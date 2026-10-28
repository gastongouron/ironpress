//! Child-item semantics and break metadata for multi-column layout.

use crate::layout::context::LayoutEnv;
use crate::layout::elements::{Container, LayoutElement, LayoutNode, LayoutVisitor, TextBlock};
use crate::layout::paginate::estimate_element_height;
use crate::parser::css::{
    AncestorInfo, CssValue, SelectorContext, parse_inline_style, selector_matches_with_context,
    specificity,
};
use crate::parser::dom::ElementNode;
use crate::style::computed::ComputedStyle;

pub(super) struct MultiColItem {
    /// The flattened layout elements for this child (usually one Container or
    /// TextBlock, but text/anonymous content may produce several).
    pub(super) elements: Vec<LayoutNode>,
    /// Outer (margin-box) height used for balancing.
    pub(super) height: f32,
    /// Principal outer extent available to fragmentation. Definite boxes keep
    /// their hard used extent; content-dependent boxes resolve against their
    /// natural descendant extent.
    pub(super) fragmentation_height: f32,
    /// Outer (margin-box) width used for vertical writing-mode block flow.
    pub(super) width: f32,
    /// The item's trailing `margin-bottom` (the last in-flow element's bottom
    /// margin). Truncated at a column-fragment break per css-break-3 §4.2.
    pub(super) margin_bottom: f32,
    /// `column-span: all` — render as a full-width band, not inside a column.
    pub(super) span_all: bool,
    /// `break-before: column` — force this item to the next column.
    pub(super) break_before_column: bool,
    /// `break-after: column` — force following content to the next column.
    pub(super) break_after_column: bool,
    /// `break-before: avoid-column` / `avoid` — avoid a column break before it.
    pub(super) break_before_avoid_column: bool,
    /// `break-after: avoid-column` / `avoid` — avoid a column break after it.
    pub(super) break_after_avoid_column: bool,
    /// `break-inside: avoid-column` / `avoid` — keep the item in one column if
    /// it fits there.
    pub(super) break_inside_avoid_column: bool,
}

impl MultiColItem {
    pub(super) fn from_layout(elements: Vec<LayoutNode>, info: ChildMulticolInfo) -> Self {
        let height = info.definite_outer_height.unwrap_or_else(|| {
            elements
                .iter()
                .map(|element| multicol_item_element_height(element.as_ref()))
                .sum()
        });
        let width = info.definite_outer_width.unwrap_or_else(|| {
            elements
                .iter()
                .map(|element| multicol_item_element_width(element.as_ref()))
                .fold(0.0, f32::max)
        });
        let margin_bottom = element_trailing_margin_bottom(&elements);
        let fragmentation_height = elements
            .iter()
            .map(|element| {
                element
                    .fragmentable_outer_block_extent()
                    .unwrap_or_else(|| multicol_item_element_height(element.as_ref()))
            })
            .sum::<f32>()
            .max(height);
        Self {
            elements,
            height,
            fragmentation_height,
            width,
            margin_bottom,
            span_all: info.span_all,
            break_before_column: info.breaks.before_force,
            break_after_column: info.breaks.after_force,
            break_before_avoid_column: info.breaks.before_avoid,
            break_after_avoid_column: info.breaks.after_avoid,
            break_inside_avoid_column: info.breaks.inside_avoid,
        }
    }
}

pub(super) struct ChildMulticolInfo {
    pub(super) span_all: bool,
    pub(super) definite_outer_height: Option<f32>,
    pub(super) definite_outer_width: Option<f32>,
    pub(super) breaks: ColumnBreakInfo,
}

impl ChildMulticolInfo {
    pub(super) fn from_style(style: &ComputedStyle, breaks: ColumnBreakInfo) -> Self {
        let definite_outer_height = style.height.map(|height| {
            let border_padding = style.border.vertical_width() + style.padding.vertical();
            let border_box = if style.box_sizing == crate::style::computed::BoxSizing::BorderBox {
                height
            } else {
                height + border_padding
            };
            style.margin.top + border_box + style.margin.bottom
        });
        let definite_outer_width = style.width.map(|width| {
            let border_padding = style.border.horizontal_width() + style.padding.horizontal();
            let border_box = if style.box_sizing == crate::style::computed::BoxSizing::BorderBox {
                width
            } else {
                width + border_padding
            };
            style.margin.left + border_box + style.margin.right
        });
        Self {
            span_all: style.column_span_all,
            definite_outer_height,
            definite_outer_width,
            breaks,
        }
    }
}

/// Resolved forced and avoided column-break controls for one child.
#[derive(Clone, Copy, Default)]
pub(super) struct ColumnBreakInfo {
    pub(super) before_force: bool,
    pub(super) after_force: bool,
    pub(super) before_avoid: bool,
    pub(super) after_avoid: bool,
    pub(super) inside_avoid: bool,
}

/// The trailing `margin-bottom` of a laid-out item: the bottom margin of its
/// last in-flow (non-absolute) layout element, which is what adjoins a column
/// fragmentation break. Returns 0.0 for elements that carry no bottom margin.
fn element_trailing_margin_bottom(elements: &[LayoutNode]) -> f32 {
    for el in elements.iter().rev() {
        if el
            .positioning_owner()
            .is_some_and(|owner| owner.positioning().scheme.is_absolute())
        {
            continue;
        }
        if let Some(holder) = el.margin_holder() {
            return holder.margins().end;
        }
    }
    0.0
}

fn multicol_item_element_height(element: &dyn LayoutElement) -> f32 {
    #[derive(Default)]
    struct HeightVisitor(Option<f32>);

    impl LayoutVisitor for HeightVisitor {
        fn visit_text_block(&mut self, element: &TextBlock) {
            if !element.positioning.scheme.is_absolute()
                && let Some(height) = element.box_model.size.height.used()
            {
                self.0 = Some(
                    element.box_model.margins.total()
                        + height
                        + element.box_model.border.vertical_width(),
                );
            }
        }

        fn visit_container(&mut self, element: &Container) {
            if !element.positioning.scheme.is_absolute()
                && let Some(height) = element.box_model.size.height.used()
            {
                self.0 = Some(element.box_model.margins.total() + height);
            }
        }
    }

    let mut visitor = HeightVisitor::default();
    element.accept(&mut visitor);
    visitor
        .0
        .unwrap_or_else(|| estimate_element_height(element))
}

fn multicol_item_element_width(element: &dyn LayoutElement) -> f32 {
    #[derive(Default)]
    struct WidthVisitor(f32);

    impl LayoutVisitor for WidthVisitor {
        fn visit_text_block(&mut self, element: &TextBlock) {
            if !element.positioning.scheme.is_absolute()
                && let Some(width) = element.box_model.size.width.fixed_value()
            {
                self.0 = width + element.box_model.border.horizontal_width();
            }
        }

        fn visit_container(&mut self, element: &Container) {
            if !element.positioning.scheme.is_absolute() {
                self.0 = element
                    .box_model
                    .size
                    .width
                    .fixed_value()
                    .unwrap_or_default();
            }
        }
    }

    let mut visitor = WidthVisitor::default();
    element.accept(&mut visitor);
    visitor.0
}

/// Lay out a multi-column container, replacing the previous grid-emulation path.
pub(super) fn child_multicol_info(
    child_el: &ElementNode,
    parent_style: &ComputedStyle,
    env: &LayoutEnv,
    child_ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
    preceding_siblings: &[(String, Vec<String>)],
) -> ChildMulticolInfo {
    use crate::style::computed::compute_style_with_context;
    let classes = child_el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: child_ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: preceding_siblings.into(),
        following_siblings: Default::default(),
        is_empty: false,
    };
    let cs = compute_style_with_context(
        child_el.tag,
        child_el.style_attr(),
        parent_style,
        env.rules,
        child_el.tag_name(),
        &classes,
        child_el.id(),
        &child_el.attributes,
        &selector_ctx,
    );
    let breaks = resolve_child_column_breaks(
        child_el,
        env.rules,
        child_el.tag_name(),
        &classes,
        child_el.id(),
        &child_el.attributes,
        &selector_ctx,
    );
    ChildMulticolInfo::from_style(&cs, breaks)
}

fn resolve_child_column_breaks(
    child_el: &ElementNode,
    rules: &[crate::parser::css::CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &std::collections::HashMap<String, String>,
    selector_ctx: &SelectorContext,
) -> ColumnBreakInfo {
    let mut matched: Vec<(u32, &crate::parser::css::CssRule)> = Vec::new();
    for rule in rules {
        if rule.pseudo_element.is_some() {
            continue;
        }
        if selector_matches_with_context(
            &rule.selector,
            tag_name,
            classes,
            id,
            attributes,
            selector_ctx,
        ) {
            matched.push((specificity(&rule.selector), rule));
        }
    }
    matched.sort_by_key(|(spec, _)| *spec);

    let inline_map = child_el.style_attr().map(parse_inline_style);
    let mut breaks = ColumnBreakInfo::default();
    for (_, rule) in &matched {
        apply_column_break_declarations(&mut breaks, &rule.declarations, false);
    }
    if let Some(inline) = &inline_map {
        apply_column_break_declarations(&mut breaks, inline, false);
    }
    for (_, rule) in &matched {
        apply_column_break_declarations(&mut breaks, &rule.declarations, true);
    }
    if let Some(inline) = &inline_map {
        apply_column_break_declarations(&mut breaks, inline, true);
    }
    breaks
}

fn apply_column_break_declarations(
    breaks: &mut ColumnBreakInfo,
    declarations: &crate::parser::css::StyleMap,
    important: bool,
) {
    if declarations.is_important("break-before") == important {
        if let Some(CssValue::Keyword(value)) = declarations.get("break-before") {
            apply_before_column_break_value(breaks, value);
        }
    }
    if declarations.is_important("break-after") == important {
        if let Some(CssValue::Keyword(value)) = declarations.get("break-after") {
            apply_after_column_break_value(breaks, value);
        }
    }
    if declarations.is_important("break-inside") == important {
        if let Some(CssValue::Keyword(value)) = declarations.get("break-inside") {
            breaks.inside_avoid = is_column_avoid_break(value);
        }
    }
    if declarations.is_important("page-break-inside") == important {
        if let Some(CssValue::Keyword(value)) = declarations.get("page-break-inside") {
            breaks.inside_avoid = value == "avoid";
        }
    }
}

fn apply_before_column_break_value(breaks: &mut ColumnBreakInfo, value: &str) {
    breaks.before_force = value == "column";
    breaks.before_avoid = is_column_avoid_break(value);
}

fn apply_after_column_break_value(breaks: &mut ColumnBreakInfo, value: &str) {
    breaks.after_force = value == "column";
    breaks.after_avoid = is_column_avoid_break(value);
}

fn is_column_avoid_break(value: &str) -> bool {
    matches!(value, "avoid" | "avoid-column")
}
