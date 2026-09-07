//! Intrinsic inline sizing shared by block and flex formatting contexts.

use super::helpers::{collects_as_inline_text, recurses_as_layout_child};
use super::text::{collapse_whitespace, estimate_word_width, resolve_style_font_family};
use super::traversal::ElementSiblingPosition;
use crate::parser::css::{AncestorInfo, CssRule, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::{
    BoxSizing, ComputedStyle, Display, FontFamily, FontWeight, IntrinsicWidthKeyword,
    compute_style_with_context,
};
use std::collections::VecDeque;

/// css-sizing-3 § 5.1 intrinsic inline sizes for one box edge.
///
/// The function returning this value defines whether that edge is the content
/// box or border box. Keeping both sizes together prevents callers from mixing
/// a min-content measurement with a max-content box adjustment.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct IntrinsicWidths {
    min_content: f32,
    max_content: f32,
}

impl IntrinsicWidths {
    /// Resolve a CSS intrinsic keyword against the available border-box width.
    pub(crate) fn resolve(self, keyword: IntrinsicWidthKeyword, available_width: f32) -> f32 {
        match keyword {
            IntrinsicWidthKeyword::MinContent => self.min_content,
            IntrinsicWidthKeyword::MaxContent => self.max_content,
            IntrinsicWidthKeyword::FitContent => self
                .max_content
                .min(self.min_content.max(available_width.max(0.0))),
        }
    }

    /// Append one atomic inline box to the current inline formatting sequence.
    /// Its max-content contribution stays on the same unwrapped line, while
    /// its min-content contribution remains one unbreakable unit.
    fn append_atomic(&mut self, box_widths: Self) {
        self.min_content = self.min_content.max(box_widths.min_content);
        self.max_content += box_widths.max_content;
    }
}

fn box_horizontal_extra(style: &ComputedStyle) -> f32 {
    style.padding.horizontal() + style.border.horizontal_width()
}

/// Inputs required at one step of a recursive intrinsic-size measurement.
/// Selector ancestry belongs here because it is part of computed style.
#[derive(Clone, Copy)]
struct IntrinsicMeasurement<'context, 'dom> {
    rules: &'context [CssRule],
    fonts: &'context dyn crate::font_registry::FontRegistry,
    ancestors: &'context [AncestorInfo<'dom>],
    selector: &'context SelectorContext<'dom>,
}

/// Compute an element's intrinsic border-box widths without its outer margins.
pub(crate) fn intrinsic_border_box_widths(
    el: &ElementNode,
    style: &ComputedStyle,
    rules: &[CssRule],
    fonts: &dyn crate::font_registry::FontRegistry,
    ancestors: &[AncestorInfo],
    selector: &SelectorContext,
) -> IntrinsicWidths {
    intrinsic_border_box_widths_with(
        el,
        style,
        IntrinsicMeasurement {
            rules,
            fonts,
            ancestors,
            selector,
        },
    )
}

/// Compute content-driven border-box widths while ignoring the preferred
/// `width` and min/max constraints, as required by `flex-basis: content`.
pub(crate) fn content_intrinsic_border_box_widths(
    el: &ElementNode,
    style: &ComputedStyle,
    rules: &[CssRule],
    fonts: &dyn crate::font_registry::FontRegistry,
    ancestors: &[AncestorInfo],
    selector: &SelectorContext,
) -> IntrinsicWidths {
    let inner = content_intrinsic_widths(
        el,
        style,
        IntrinsicMeasurement {
            rules,
            fonts,
            ancestors,
            selector,
        },
    );
    let extra = box_horizontal_extra(style);
    IntrinsicWidths {
        min_content: inner.min_content + extra,
        max_content: inner.max_content + extra,
    }
}

fn intrinsic_border_box_widths_with(
    el: &ElementNode,
    style: &ComputedStyle,
    measurement: IntrinsicMeasurement<'_, '_>,
) -> IntrinsicWidths {
    let content = content_intrinsic_widths(el, style, measurement);
    let content = style.width.map_or(content, |width| {
        let width = if style.box_sizing == BoxSizing::BorderBox {
            (width - box_horizontal_extra(style)).max(0.0)
        } else {
            width
        };
        IntrinsicWidths {
            min_content: width,
            max_content: width,
        }
    });

    let mut minimum = content.min_content;
    let mut maximum = content.max_content;
    if let Some(width) = style.max_width {
        let cap = content_box_constraint(width, style);
        minimum = minimum.min(cap);
        maximum = maximum.min(cap);
    }
    if let Some(width) = style.min_width {
        let floor = content_box_constraint(width, style);
        minimum = minimum.max(floor);
        maximum = maximum.max(floor);
    }
    let extra = box_horizontal_extra(style);
    IntrinsicWidths {
        min_content: minimum + extra,
        max_content: maximum + extra,
    }
}

fn content_box_constraint(width: f32, style: &ComputedStyle) -> f32 {
    if style.box_sizing == BoxSizing::BorderBox {
        (width - box_horizontal_extra(style)).max(0.0)
    } else {
        width
    }
}

/// Compute content-box min/max-content widths from an element's children.
fn content_intrinsic_widths(
    el: &ElementNode,
    style: &ComputedStyle,
    measurement: IntrinsicMeasurement<'_, '_>,
) -> IntrinsicWidths {
    let mut contributions = Vec::new();
    let mut inline = IntrinsicWidths::default();
    let text = TextMeasurement::new(style, measurement.fonts);
    let mut positions = element_positions(el);
    let mut child_ancestors = measurement.ancestors.to_vec();
    child_ancestors.push(measurement.selector.as_ancestor(el));

    for child in &el.children {
        match child {
            DomNode::Text(value) => text.accumulate(value, &mut inline),
            DomNode::Element(child_el) => {
                let Some(child_position) = positions.pop_front() else {
                    continue;
                };
                let child_selector = child_position
                    .as_context()
                    .selector_context(&child_ancestors, child_el.children.is_empty());
                let child_style = child_style(child_el, style, measurement.rules, &child_selector);
                if child_style.display == Display::None || child_style.position.is_absolute() {
                    continue;
                }

                let is_flex_item = matches!(style.display, Display::Flex | Display::InlineFlex);
                let is_atomic_inline = matches!(child_el.tag, crate::parser::dom::HtmlTag::Img)
                    || matches!(
                        child_style.display,
                        Display::InlineBlock
                            | Display::InlineFlex
                            | Display::InlineGrid
                            | Display::InlineTable
                    );
                let establishes_block = is_flex_item
                    || matches!(
                        child_style.display,
                        Display::Block
                            | Display::ListItem
                            | Display::Flex
                            | Display::Grid
                            | Display::Table
                    )
                    || (recurses_as_layout_child(child_el.tag) && !is_atomic_inline);
                if establishes_block {
                    flush_inline(&mut contributions, &mut inline);
                    let border_box = intrinsic_border_box_widths_with(
                        child_el,
                        &child_style,
                        IntrinsicMeasurement {
                            rules: measurement.rules,
                            fonts: measurement.fonts,
                            ancestors: &child_ancestors,
                            selector: &child_selector,
                        },
                    );
                    contributions.push(border_box.with_extra(child_style.margin.horizontal()));
                } else if is_atomic_inline {
                    let border_box = intrinsic_border_box_widths_with(
                        child_el,
                        &child_style,
                        IntrinsicMeasurement {
                            rules: measurement.rules,
                            fonts: measurement.fonts,
                            ancestors: &child_ancestors,
                            selector: &child_selector,
                        },
                    );
                    inline.append_atomic(border_box.with_extra(child_style.margin.horizontal()));
                } else {
                    accumulate_inline_element(
                        child_el,
                        &child_style,
                        IntrinsicMeasurement {
                            rules: measurement.rules,
                            fonts: measurement.fonts,
                            ancestors: &child_ancestors,
                            selector: &child_selector,
                        },
                        &mut inline,
                    );
                }
            }
        }
    }
    flush_inline(&mut contributions, &mut inline);
    combine_contributions(style, &contributions)
}

impl IntrinsicWidths {
    fn with_extra(self, extra: f32) -> Self {
        Self {
            min_content: self.min_content + extra,
            max_content: self.max_content + extra,
        }
    }
}

fn combine_contributions(
    style: &ComputedStyle,
    contributions: &[IntrinsicWidths],
) -> IntrinsicWidths {
    if matches!(style.display, Display::Flex | Display::InlineFlex) && style.flex_direction.is_row()
    {
        let gap = style.column_gap.max(style.gap) * contributions.len().saturating_sub(1) as f32;
        let max_content = contributions
            .iter()
            .map(|width| width.max_content)
            .sum::<f32>()
            + gap;
        let min_content = if style.flex_wrap.wraps() {
            contributions
                .iter()
                .map(|width| width.min_content)
                .fold(0.0, f32::max)
        } else {
            contributions
                .iter()
                .map(|width| width.min_content)
                .sum::<f32>()
                + gap
        };
        IntrinsicWidths {
            min_content,
            max_content,
        }
    } else {
        IntrinsicWidths {
            min_content: contributions
                .iter()
                .map(|width| width.min_content)
                .fold(0.0, f32::max),
            max_content: contributions
                .iter()
                .map(|width| width.max_content)
                .fold(0.0, f32::max),
        }
    }
}

fn element_positions(el: &ElementNode) -> VecDeque<ElementSiblingPosition> {
    let elements = el
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(element) => Some(element),
            DomNode::Text(_) => None,
        })
        .collect::<Vec<_>>();
    (0..elements.len())
        .map(|index| ElementSiblingPosition::from_element_siblings(&elements, index))
        .collect()
}

fn child_style(
    el: &ElementNode,
    parent: &ComputedStyle,
    rules: &[CssRule],
    selector: &SelectorContext,
) -> ComputedStyle {
    let class_refs = el.class_list();
    compute_style_with_context(
        el.tag,
        el.style_attr(),
        parent,
        rules,
        el.tag_name(),
        &class_refs,
        el.id(),
        &el.attributes,
        selector,
    )
}

fn flush_inline(contributions: &mut Vec<IntrinsicWidths>, inline: &mut IntrinsicWidths) {
    if inline.min_content > 0.0 || inline.max_content > 0.0 {
        contributions.push(*inline);
    }
    *inline = IntrinsicWidths::default();
}

struct TextMeasurement<'a> {
    font_size: f32,
    family: FontFamily,
    bold: bool,
    italic: bool,
    fonts: &'a dyn crate::font_registry::FontRegistry,
}

impl<'a> TextMeasurement<'a> {
    fn new(style: &ComputedStyle, fonts: &'a dyn crate::font_registry::FontRegistry) -> Self {
        Self {
            font_size: style.font_size,
            family: resolve_style_font_family(style, fonts),
            bold: style.font_weight == FontWeight::Bold,
            italic: style.font_style.is_slanted(),
            fonts,
        }
    }

    fn accumulate(&self, text: &str, widths: &mut IntrinsicWidths) {
        let collapsed = collapse_whitespace(text);
        if collapsed.is_empty() {
            return;
        }
        widths.max_content += self.width(&collapsed);
        for word in collapsed.split(' ').filter(|word| !word.is_empty()) {
            widths.min_content = widths.min_content.max(self.width(word));
        }
    }

    fn width(&self, text: &str) -> f32 {
        estimate_word_width(
            text,
            self.font_size,
            &self.family,
            self.bold,
            self.italic,
            self.fonts,
        )
    }
}

fn accumulate_inline_element(
    el: &ElementNode,
    style: &ComputedStyle,
    measurement: IntrinsicMeasurement<'_, '_>,
    inline: &mut IntrinsicWidths,
) {
    let text = TextMeasurement::new(style, measurement.fonts);
    let mut positions = element_positions(el);
    let mut child_ancestors = measurement.ancestors.to_vec();
    child_ancestors.push(measurement.selector.as_ancestor(el));

    for child in &el.children {
        match child {
            DomNode::Text(value) => text.accumulate(value, inline),
            DomNode::Element(child_el) => {
                let Some(child_position) = positions.pop_front() else {
                    continue;
                };
                if !collects_as_inline_text(child_el.tag) {
                    continue;
                }
                let child_selector = child_position
                    .as_context()
                    .selector_context(&child_ancestors, child_el.children.is_empty());
                let child_style = child_style(child_el, style, measurement.rules, &child_selector);
                accumulate_inline_element(
                    child_el,
                    &child_style,
                    IntrinsicMeasurement {
                        rules: measurement.rules,
                        fonts: measurement.fonts,
                        ancestors: &child_ancestors,
                        selector: &child_selector,
                    },
                    inline,
                );
            }
        }
    }
}

/// Resolve a block box's border-box width for an intrinsic `width` keyword.
pub(crate) fn resolve_intrinsic_keyword_width(
    el: &ElementNode,
    style: &ComputedStyle,
    keyword: IntrinsicWidthKeyword,
    available_width: f32,
    rules: &[CssRule],
    fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    // A min/max-width keyword constrains the preferred width; it does not
    // measure that preferred width. Using `intrinsic_border_box_widths` here
    // would feed `width` back into its own constraint (for example making
    // `width: 200px; max-width: min-content` resolve to 200px).
    let widths = content_intrinsic_border_box_widths(
        el,
        style,
        rules,
        fonts,
        &[],
        &SelectorContext::default(),
    );
    let stretch = (available_width - style.margin.horizontal()).max(0.0);
    widths.resolve(keyword, stretch).max(0.0)
}
