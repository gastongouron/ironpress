use crate::layout::elements::{
    BoxPaint, FlexContent, FlexRow, IntoLayoutNode, LayoutElement, LayoutNode, LayoutSize,
    Positioning,
};
use crate::layout::flow_metrics::BlockMargins;
use crate::parser::css::{AncestorInfo, CssRule, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BoxSizing, ComputedStyle, Display, GridTrack, IntrinsicWidthKeyword, OverflowWrap, TextAlign,
    Transform, compute_style_with_context_with_font_metrics,
};
use crate::style::font_metrics::FontMetrics;
use crate::types::{EdgeSizes, Size};
use std::collections::HashMap;

use super::box_model::ResolvedBoxDimensions;
use super::cells::CellPaint;
use super::context::{LayoutContext, LayoutEnv};
use super::engine::{
    CounterState, ElementSiblingContext, FlexCell, FlexItemFragmentation, FlexItemRole,
    LayoutBorder, LayoutTreeContext, TextLine, apply_direct_flex_item_filters, flatten_element,
    forward_siblings,
};
use super::flex::layout_flex_container;
use super::grid::layout_grid_container;
use super::images::{
    InlineBaselineGapRounding, add_inline_replaced_baseline_gap, load_image_from_element,
};
use super::inline_formatting::{
    AtomicInlineKind, GeneratedContentStyles, IndependentFlowLayout, InlineContentSequence,
    InlineFormattingChild, InlineFormattingRole, layout_mixed_flow_children,
};
use super::paginate::estimate_element_height;
use super::roundoff::exceeds_with_roundoff;
use super::table::{TableLayoutContext, flatten_table};
use super::text::{
    InlineRunCollector, InlineTextSequence, LineStrut, TextWrapOptions, estimate_word_width,
    has_non_collapsible_text, parent_line_strut, resolve_style_font_family,
    resolved_line_height_factor, text_run_line_height_factor, used_font_size, wrap_text_runs,
};

mod row_cursor;

use row_cursor::{InlineRowCursor, InlineRowSeparator, InlineRowUnit};

fn content_only_flex_style(
    style: &ComputedStyle,
    dimensions: ResolvedBoxDimensions,
) -> ComputedStyle {
    let mut content = style.clone();
    content.display = Display::Flex;
    content.margin = EdgeSizes::ZERO;
    content.padding = EdgeSizes::ZERO;
    content.border = Default::default();
    content.reset_background();
    content.box_shadow.clear();
    content.width = Some(dimensions.content.width);
    content.height = style.height.map(|_| dimensions.content.height);
    content.min_width = None;
    content.max_width = None;
    content.min_height = None;
    content.max_height = None;
    content
}

fn min_content_anywhere_width(
    runs: &[crate::layout::engine::TextRun],
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    runs.iter()
        .filter(|r| r.inline_box.is_none())
        .flat_map(|r| {
            r.text.chars().map(|ch| {
                estimate_word_width(
                    &ch.to_string(),
                    r.font_size,
                    &r.font_family,
                    r.bold,
                    r.font_style.is_slanted(),
                    fonts,
                )
            })
        })
        .fold(0.0f32, f32::max)
}

fn inline_block_child_should_flatten(el: &ElementNode, style: &ComputedStyle) -> bool {
    matches!(el.tag, HtmlTag::Img | HtmlTag::Svg | HtmlTag::Table)
        || style.display != Display::Inline
}

fn inline_block_nested_outer_width(elements: &[LayoutNode]) -> f32 {
    elements
        .iter()
        .filter_map(|element| element.inline_flow_extent()?.max_content_outer_extent())
        .fold(0.0, f32::max)
}

fn inline_row_node(
    content: FlexContent,
    box_model: crate::layout::elements::BoxModel,
    inline_offset: crate::layout::elements::InlineOffset,
    background: Option<crate::types::Color>,
) -> LayoutNode {
    FlexRow {
        content,
        box_model,
        paint: crate::layout::elements::BoxPaint {
            background: crate::layout::elements::BackgroundPaint {
                color: background,
                ..Default::default()
            },
            ..Default::default()
        },
        inline_offset,
        ..Default::default()
    }
    .boxed()
}

struct InlineBlockChildLayout<'layout, 'dom> {
    context: &'layout LayoutContext,
    parent_style: &'layout ComputedStyle,
    ancestors: &'layout [AncestorInfo<'dom>],
    available_width: f32,
    output: &'layout mut Vec<LayoutNode>,
}

impl IndependentFlowLayout for InlineBlockChildLayout<'_, '_> {
    fn lays_out_independently(&self, element: &ElementNode, child: &InlineFormattingChild) -> bool {
        inline_block_child_should_flatten(element, &child.style)
    }

    fn layout_independently(
        &mut self,
        element: &ElementNode,
        child: &InlineFormattingChild,
        env: &mut LayoutEnv<'_>,
    ) {
        let context = self
            .context
            .with_parent_and_basis(
                self.available_width,
                self.available_width,
                None,
                self.parent_style.font_size,
            )
            .with_containing_block(None);
        flatten_element(
            element,
            LayoutTreeContext::new(self.parent_style, &context, self.ancestors)
                .for_element(child.source().as_context()),
            self.output,
            env,
        );
    }
}

fn layout_inline_block_contents(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<crate::layout::engine::TextRun>,
    ancestors: &[AncestorInfo],
    env: &mut LayoutEnv,
    layout: &mut InlineBlockChildLayout<'_, '_>,
) {
    layout_mixed_flow_children(nodes, parent_style, runs, ancestors, env, layout);
}

/// Lay out consecutive atomic inline elements as `FlexRow`s.
///
/// The bool carried with each element records whether source whitespace appeared
/// immediately before it inside the current inline formatting context.
pub(crate) fn layout_inline_block_group_with_spacing(
    elements: &[(&ElementNode, bool)],
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutNode>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    fonts: &HashMap<String, TtfFont>,
) {
    layout_inline_block_group_inner(
        elements,
        parent_style,
        ctx,
        output,
        rules,
        ancestors,
        fonts,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_inline_block_group_with_env_and_spacing(
    elements: &[(&ElementNode, bool)],
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutNode>,
    ancestors: &[AncestorInfo],
    env: &mut LayoutEnv,
) {
    layout_inline_block_group_inner(
        elements,
        parent_style,
        ctx,
        output,
        env.rules,
        ancestors,
        env.fonts,
        Some(env),
    );
}

fn inline_text_cell(
    mut runs: Vec<crate::layout::engine::TextRun>,
    parent_style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> Option<(FlexCell, f32)> {
    if runs.is_empty() {
        return None;
    }
    runs.as_mut_slice().resolve_unclaimed_boundaries(
        crate::layout::elements::TextSpacing::from_style(parent_style),
    );
    let lines = wrap_text_runs(
        runs,
        TextWrapOptions::new(
            f32::MAX,
            used_font_size(parent_style, fonts),
            text_run_line_height_factor(parent_style, fonts),
            parent_style.overflow_wrap,
        )
        .with_white_space(parent_style.white_space)
        .with_parent_strut(parent_line_strut(parent_style, fonts))
        .with_rtl(parent_style.direction_rtl)
        .with_bidi_override(parent_style.bidi_override)
        .with_bidi_plaintext(parent_style.bidi_plaintext)
        .with_word_break_keep_all(parent_style.word_break_keep_all)
        .with_line_break_anywhere(parent_style.line_break_anywhere),
        fonts,
    );
    if lines.is_empty() {
        return None;
    }
    let mut width = lines
        .iter()
        .map(|line| crate::layout::helpers::measure_runs_width(&line.runs, fonts))
        .fold(0.0f32, f32::max);
    if lines.iter().all(|line| {
        line.runs
            .iter()
            .all(|run| !has_non_collapsible_text(&run.text))
    }) {
        width = width.min(parent_style.font_size * 0.3125);
    }
    let height = lines.iter().map(|line| line.height).sum::<f32>();
    Some((
        FlexCell {
            lines,
            width,
            text_align: parent_style.text_align,
            natural_height: height,
            fragmentation: FlexItemFragmentation::definite(),
            ..Default::default()
        },
        width,
    ))
}

/// Advance of one collapsed ASCII space in the parent inline formatting context.
///
/// A space at the edge of a text fragment has no glyph run of its own: the
/// line breaker correctly discards it at that fragment edge.  Atomic inline
/// boxes split a mixed sequence into fragments, so their adjacent source
/// whitespace must be retained as an advance between those fragments instead.
fn collapsed_inline_space_advance(
    parent_style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    let glyph_advance = estimate_word_width(
        " ",
        used_font_size(parent_style, fonts),
        &resolve_style_font_family(parent_style, fonts),
        parent_style.font_weight == crate::style::computed::FontWeight::Bold,
        parent_style.font_style.is_slanted(),
        fonts,
    );
    crate::layout::elements::TextSpacing::from_style(parent_style)
        .add_internal_advance(glyph_advance, " ")
}

fn runs_have_visible_inline_content(runs: &[crate::layout::engine::TextRun]) -> bool {
    runs.iter()
        .any(|run| run.inline_box.is_some() || has_non_collapsible_text(&run.text))
}

/// The line-box extents contributed by one or more `vertical-align: middle`
/// atomic inline boxes.
///
/// CSS 2.2 defines the relation (the box centre meets the parent baseline plus
/// half the x-height), while browser layout resolves the resulting used values
/// on the CSS-pixel grid. Keeping those two dimensions together avoids treating
/// a middle-aligned table as a flex-centred item, which loses the line baseline.
#[derive(Debug, Clone, Copy, Default)]
struct MiddleAlignedLine {
    above: f32,
    below: f32,
}

impl MiddleAlignedLine {
    fn include_box(&mut self, height: f32, parent_x_height: f32) {
        let x_height = crate::fonts::round_to_css_pixel(parent_x_height.max(0.0));
        let above = crate::fonts::round_to_css_pixel((height * 0.5 + x_height * 0.5).max(0.0));
        self.above = self.above.max(above);
        self.below = self.below.max((height - above).max(0.0));
    }

    fn used_height(self, strut: LineStrut) -> f32 {
        self.above.max(strut.above) + self.below.max(strut.below)
    }

    fn text_shift(self, strut: LineStrut) -> f32 {
        (self.above - strut.above).max(0.0)
    }
}

fn parent_x_height(parent_style: &ComputedStyle, fonts: &HashMap<String, TtfFont>) -> f32 {
    let font_family = resolve_style_font_family(parent_style, fonts);
    let ratio = if let crate::style::computed::FontFamily::Custom(name) = &font_family {
        crate::system_fonts::find_font(
            fonts,
            name,
            parent_style.font_weight == crate::style::computed::FontWeight::Bold,
            parent_style.font_style.is_slanted(),
        )
        .map_or(0.5, |(_, font)| font.x_height_ratio())
    } else {
        0.5
    };
    ratio * used_font_size(parent_style, fonts)
}

#[allow(clippy::too_many_arguments)]
fn inline_atomic_cell(
    child_el: &ElementNode,
    child_style: &ComputedStyle,
    kind: AtomicInlineKind,
    ctx: &LayoutContext,
    selector_context: &SelectorContext<'_>,
    env: &mut LayoutEnv,
) -> Option<(FlexCell, f32)> {
    let ancestors = selector_context.ancestors.as_slice();
    let child_index = selector_context.child_index;
    let sibling_count = selector_context.sibling_count;
    let mut child_ancestors = selector_context.ancestors.clone();
    child_ancestors.push(selector_context.as_ancestor(child_el));

    // Replacedness is independent of whether the outer display was authored as
    // `inline` or `inline-block`. Keep the regular image path as the single
    // source of sizing, filtering, object fitting, and baseline behavior.
    if kind == AtomicInlineKind::ReplacedImage {
        let mut replaced = add_inline_replaced_baseline_gap(
            load_image_from_element(
                &mut *env.resources,
                child_el,
                ctx.available_width(),
                ctx.available_height(),
                child_style,
                env.filter_dpi,
            )?,
            child_style,
            env.fonts,
            InlineBaselineGapRounding::CssPixel,
        );
        let mut filter_style = child_style.clone();
        let filter = super::filter::ResolvedFilter::from_style(&mut filter_style, env.filter_defs);
        if filter.requires_source_surface() {
            super::filter::retain_for_fragmentation(replaced.as_mut(), &filter);
        }
        let width = replaced.replaced_element()?.geometry().size.width;
        let height = estimate_element_height(replaced.as_ref());
        return Some((
            FlexCell {
                width,
                natural_height: height,
                fragmentation: FlexItemFragmentation::definite(),
                nested_elements: vec![replaced],
                role: FlexItemRole::AtomicInline(kind),
                ..Default::default()
            },
            width,
        ));
    }

    let (width, height, nested_elements, background_color, border, padding, y_offset) =
        match child_style.display {
            Display::InlineFlex => {
                let generated_styles = GeneratedContentStyles::resolve(
                    child_el,
                    child_style,
                    env.rules,
                    selector_context,
                    env.fonts,
                );
                let dimensions = ResolvedBoxDimensions::from_style(
                    child_style,
                    Size::new(ctx.available_width(), 0.0),
                );
                let flex_style = content_only_flex_style(child_style, dimensions);
                let child_ctx = ctx.with_parent_and_basis(
                    dimensions.content.width,
                    dimensions.content.width,
                    child_style.height.map(|_| dimensions.content.height),
                    child_style.font_size,
                );
                let mut nested = Vec::new();
                layout_flex_container(
                    child_el,
                    &flex_style,
                    &child_ctx,
                    &mut nested,
                    &child_ancestors,
                    generated_styles.boxes(child_el),
                    0,
                    env,
                );
                apply_direct_flex_item_filters(
                    child_el,
                    child_style,
                    &child_ancestors,
                    env,
                    &mut nested,
                );
                (
                    dimensions.border_box.width,
                    dimensions.border_box.height,
                    nested,
                    child_style.background_color,
                    LayoutBorder::from_computed(&child_style.border, child_style.color),
                    child_style.padding,
                    0.0,
                )
            }
            Display::InlineGrid => {
                let mut grid_style = child_style.clone();
                grid_style.display = Display::Grid;
                grid_style.margin = Default::default();
                let track_len = |track: &GridTrack| match track {
                    GridTrack::Fixed(v) => *v,
                    GridTrack::Percent(p) => p * ctx.available_width(),
                    GridTrack::Minmax(min, _) => *min,
                    _ => 0.0,
                };
                let intrinsic_w = child_style
                    .grid_template_columns
                    .iter()
                    .map(track_len)
                    .sum::<f32>()
                    + child_style.column_gap
                        * child_style.grid_template_columns.len().saturating_sub(1) as f32;
                let intrinsic_h = child_style
                    .grid_template_rows
                    .iter()
                    .map(track_len)
                    .sum::<f32>()
                    + child_style.row_gap
                        * child_style.grid_template_rows.len().saturating_sub(1) as f32;
                if grid_style.width.is_none() {
                    grid_style.width = Some(intrinsic_w);
                }
                if grid_style.height.is_none() && intrinsic_h > 0.0 {
                    grid_style.height = Some(intrinsic_h);
                }
                let border_box_w = child_style.width.unwrap_or(intrinsic_w).max(0.0);
                let border_box_h = child_style.height.unwrap_or(intrinsic_h).max(0.0);
                let child_ctx = ctx.with_parent_and_basis(
                    border_box_w,
                    border_box_w,
                    Some(border_box_h),
                    child_style.font_size,
                );
                let mut nested = Vec::new();
                layout_grid_container(
                    child_el,
                    &grid_style,
                    &child_ctx,
                    &mut nested,
                    &child_ancestors,
                    0,
                    env,
                );
                (
                    border_box_w,
                    border_box_h,
                    nested,
                    None,
                    LayoutBorder::default(),
                    EdgeSizes::default(),
                    0.0,
                )
            }
            Display::InlineTable => {
                let mut table_style = child_style.clone();
                table_style.display = Display::Table;
                table_style.margin = Default::default();
                let generated_styles = GeneratedContentStyles::resolve(
                    child_el,
                    child_style,
                    env.rules,
                    selector_context,
                    env.fonts,
                );
                let mut nested = Vec::new();
                let ancestor_depth = ctx
                    .containing_block
                    .map_or(0, |containing_block| containing_block.depth);
                let positioned_depth = ancestor_depth
                    + usize::from(crate::layout::helpers::establishes_containing_block(
                        &table_style,
                    ));
                flatten_table(
                    child_el,
                    &table_style,
                    &mut nested,
                    generated_styles.boxes(child_el),
                    env,
                    TableLayoutContext::new(
                        ctx,
                        ancestors,
                        ElementSiblingContext::new(child_index, sibling_count),
                        positioned_depth,
                    ),
                );
                let width = inline_block_nested_outer_width(&nested);
                let height = nested
                    .iter()
                    .map(|element| {
                        crate::layout::paginate::estimate_element_height(element.as_ref())
                    })
                    .sum::<f32>();
                (
                    width,
                    height,
                    nested,
                    child_style.background_color,
                    LayoutBorder::from_computed(&child_style.border, child_style.color),
                    child_style.padding,
                    0.0,
                )
            }
            Display::InlineBlock => {
                let mut runs = Vec::new();
                let mut nested_elements = Vec::new();
                let mut child_layout = InlineBlockChildLayout {
                    context: ctx,
                    parent_style: child_style,
                    ancestors: &child_ancestors,
                    available_width: ctx.available_width().max(0.0),
                    output: &mut nested_elements,
                };
                layout_inline_block_contents(
                    &child_el.children,
                    child_style,
                    &mut runs,
                    &child_ancestors,
                    env,
                    &mut child_layout,
                );
                let lines = wrap_text_runs(
                    runs,
                    TextWrapOptions::new(
                        child_style.width.unwrap_or(f32::MAX).max(0.0),
                        used_font_size(child_style, env.fonts),
                        text_run_line_height_factor(child_style, env.fonts),
                        child_style.overflow_wrap,
                    )
                    .with_white_space(child_style.white_space)
                    .with_parent_strut(parent_line_strut(child_style, env.fonts))
                    .with_word_break_keep_all(child_style.word_break_keep_all)
                    .with_line_break_anywhere(child_style.line_break_anywhere),
                    env.fonts,
                );
                let content_w = child_style.width.unwrap_or_else(|| {
                    lines
                        .iter()
                        .map(|line| {
                            crate::layout::helpers::measure_runs_width(&line.runs, env.fonts)
                        })
                        .fold(0.0f32, f32::max)
                });
                let content_h = child_style.height.unwrap_or_else(|| {
                    lines.iter().map(|line| line.height).sum::<f32>()
                        + nested_elements
                            .iter()
                            .map(|element| {
                                crate::layout::paginate::estimate_element_height(element.as_ref())
                            })
                            .sum::<f32>()
                });
                let total_w = content_w
                    + child_style.padding.horizontal()
                    + child_style.border.horizontal_width();
                let total_h = content_h
                    + child_style.padding.vertical()
                    + child_style.border.vertical_width();
                return Some((
                    FlexCell {
                        lines,
                        x_offset: child_style.margin.left,
                        width: total_w,
                        text_align: child_style.text_align,
                        padding: child_style.padding,
                        border: LayoutBorder::from_computed(&child_style.border, child_style.color),
                        natural_height: total_h,
                        fragmentation: FlexItemFragmentation::definite(),
                        paint: CellPaint::from_style(
                            child_style,
                            LayoutSize::fixed(total_w, Some(total_h)),
                        ),
                        positioning: Positioning::from_style(child_style),
                        nested_elements,
                        role: FlexItemRole::AtomicInline(kind),
                        ..Default::default()
                    },
                    total_w + child_style.margin.horizontal(),
                ));
            }
            _ => return None,
        };

    Some((
        FlexCell {
            x_offset: child_style.margin.left,
            width,
            text_align: child_style.text_align,
            padding,
            border,
            natural_height: height,
            fragmentation: FlexItemFragmentation::definite(),
            paint: {
                let mut paint =
                    CellPaint::from_style(child_style, LayoutSize::fixed(width, Some(height)));
                paint.background.color = background_color;
                paint
            },
            positioning: Positioning::from_style(child_style),
            nested_elements,
            y_offset,
            role: FlexItemRole::AtomicInline(kind),
            ..Default::default()
        },
        width + child_style.margin.horizontal(),
    ))
}

pub(crate) fn layout_inline_mixed_sequence_with_env(
    sequence: InlineContentSequence<'_>,
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutNode>,
    ancestors: &[AncestorInfo],
    env: &mut LayoutEnv,
) -> bool {
    let nodes = sequence.nodes();
    let element_count = sequence
        .source_nodes()
        .iter()
        .filter(|node| matches!(node, DomNode::Element(_)))
        .count();
    let sibling_list: Vec<(String, Vec<String>)> = sequence
        .source_nodes()
        .iter()
        .filter_map(|node| match node {
            DomNode::Element(el) => Some((
                el.tag_name().to_string(),
                el.class_list().iter().map(|s| s.to_string()).collect(),
            )),
            DomNode::Text(_) => None,
        })
        .collect();
    let mut element_index = sequence.starting_element_index();
    let mut preceding_siblings: Vec<(String, Vec<String>)> = sibling_list
        .get(..element_index)
        .unwrap_or_default()
        .to_vec();
    let mut pending_runs = Vec::new();
    let mut cells = Vec::new();
    let mut cursor = InlineRowCursor::default();
    let inline_spacing = crate::layout::elements::TextSpacing::from_style(parent_style);
    let mut saw_atomic = false;
    let mut middle_aligned_line: Option<MiddleAlignedLine> = None;
    let mut last_item_was_atomic = false;
    let mut pending_trailing_space = false;
    let mut pending_space_after_atomic = false;

    sequence.append_before(
        &mut pending_runs,
        env.fonts,
        env.counter_state,
        &mut *env.resources,
    );

    for (node_index, node) in nodes.iter().enumerate() {
        match node {
            DomNode::Text(text) => {
                if last_item_was_atomic && text.chars().next().is_some_and(char::is_whitespace) {
                    pending_space_after_atomic = true;
                }
                InlineRunCollector::new(
                    env.rules,
                    env.fonts,
                    env.counter_state,
                    &mut *env.resources,
                )
                .collect(
                    sequence.item(node_index),
                    parent_style,
                    &mut pending_runs,
                    None,
                    ancestors,
                );
                pending_trailing_space = text.chars().next_back().is_some_and(char::is_whitespace);
                last_item_was_atomic = false;
            }
            DomNode::Element(el) => {
                let classes = el.class_list();
                let selector_ctx = SelectorContext {
                    ancestors: ancestors.to_vec(),
                    child_index: element_index,
                    sibling_count: element_count,
                    preceding_siblings: preceding_siblings.clone(),
                    following_siblings: forward_siblings(&sibling_list, element_index).to_vec(),
                    is_empty: false,
                };
                let child_style = compute_style_with_context_with_font_metrics(
                    el.tag,
                    el.style_attr(),
                    parent_style,
                    env.rules,
                    el.tag_name(),
                    &classes,
                    el.id(),
                    &el.attributes,
                    &selector_ctx,
                    env.font_metrics(),
                );
                if child_style.display == Display::None {
                    element_index += 1;
                    continue;
                }
                let role = InlineFormattingRole::of(el, &child_style);
                match role {
                    InlineFormattingRole::Atomic(kind) => {
                        saw_atomic = true;
                        let has_space_before_atomic = pending_trailing_space
                            && runs_have_visible_inline_content(&pending_runs);
                        let separator_after_previous_atomic =
                            pending_space_after_atomic.then(|| {
                                InlineRowSeparator::CollapsedSpace(collapsed_inline_space_advance(
                                    parent_style,
                                    env.fonts,
                                ))
                            });
                        let mut emitted_text = false;
                        if let Some((cell, advance)) = inline_text_cell(
                            std::mem::take(&mut pending_runs),
                            parent_style,
                            env.fonts,
                        ) {
                            cursor.push(
                                &mut cells,
                                cell,
                                advance,
                                InlineRowUnit::Text,
                                separator_after_previous_atomic.unwrap_or_default(),
                                inline_spacing,
                            );
                            emitted_text = true;
                        }
                        if let Some((cell, advance)) =
                            inline_atomic_cell(el, &child_style, kind, ctx, &selector_ctx, env)
                        {
                            if child_style.vertical_align
                                == crate::style::computed::VerticalAlign::Middle
                            {
                                middle_aligned_line.get_or_insert_default().include_box(
                                    cell.natural_height,
                                    parent_x_height(parent_style, env.fonts),
                                );
                            }
                            let separator = if has_space_before_atomic
                                || (!emitted_text && separator_after_previous_atomic.is_some())
                            {
                                InlineRowSeparator::CollapsedSpace(collapsed_inline_space_advance(
                                    parent_style,
                                    env.fonts,
                                ))
                            } else {
                                InlineRowSeparator::Adjacent
                            };
                            cursor.push(
                                &mut cells,
                                cell,
                                advance,
                                InlineRowUnit::Atomic,
                                separator,
                                inline_spacing,
                            );
                        }
                        pending_space_after_atomic = false;
                        last_item_was_atomic = true;
                        pending_trailing_space = false;
                    }
                    InlineFormattingRole::Text => {
                        InlineRunCollector::new(
                            env.rules,
                            env.fonts,
                            env.counter_state,
                            &mut *env.resources,
                        )
                        .collect(
                            sequence.item(node_index),
                            parent_style,
                            &mut pending_runs,
                            None,
                            ancestors,
                        );
                        pending_trailing_space = false;
                        last_item_was_atomic = false;
                    }
                    InlineFormattingRole::Hidden
                    | InlineFormattingRole::OutOfFlow
                    | InlineFormattingRole::Outside => {}
                }
                preceding_siblings.push((
                    el.tag_name().to_string(),
                    el.class_list().iter().map(|s| s.to_string()).collect(),
                ));
                element_index += 1;
            }
        }
    }
    sequence.append_after(
        &mut pending_runs,
        env.fonts,
        env.counter_state,
        &mut *env.resources,
    );
    if let Some((cell, advance)) =
        inline_text_cell(std::mem::take(&mut pending_runs), parent_style, env.fonts)
    {
        let separator = if pending_space_after_atomic {
            InlineRowSeparator::CollapsedSpace(collapsed_inline_space_advance(
                parent_style,
                env.fonts,
            ))
        } else {
            InlineRowSeparator::Adjacent
        };
        cursor.push(
            &mut cells,
            cell,
            advance,
            InlineRowUnit::Text,
            separator,
            inline_spacing,
        );
    }

    if !saw_atomic {
        return false;
    }
    if cells.is_empty() {
        // The semantic atomic source was handled, but it produced no paintable
        // cell (for example an unavailable replaced resource without fallback
        // content). Do not route the sequence through text collection, which
        // would advance generated-content counters a second time.
        return true;
    }

    let line_height = parent_style.font_size * resolved_line_height_factor(parent_style, env.fonts);
    let natural_row_height = cells
        .iter()
        .map(|cell| cell.natural_height)
        .fold(line_height, f32::max);
    let strut = parent_line_strut(parent_style, env.fonts);
    let row_height = middle_aligned_line
        .map(|middle| natural_row_height.max(middle.used_height(strut)))
        .unwrap_or(natural_row_height);
    if let Some(middle) = middle_aligned_line {
        let text_shift = middle.text_shift(strut);
        if text_shift > 0.0 {
            for cell in &mut cells {
                if !cell.lines.is_empty() {
                    cell.y_offset += text_shift;
                }
            }
        }
    }
    let parent_border = LayoutBorder::from_computed(&parent_style.border, parent_style.color);
    let plain_white_background = parent_style
        .background_color
        .is_some_and(|c| c == crate::types::Color::WHITE);
    let paints_parent_box = parent_border.has_any()
        || (parent_style.background_color.is_some() && !plain_white_background);
    // Root padding has already been folded into the effective page margin
    // before root nodes reach layout. Nested padding, on the other hand,
    // changes this row's local inline formatting origin even without visible
    // box paint. Carry only the latter to avoid applying body/html padding
    // twice while keeping unpainted padded descendants correct.
    let carries_parent_box_geometry =
        paints_parent_box || (!ancestors.is_empty() && !parent_style.padding.is_zero());
    let container_width = if carries_parent_box_geometry {
        parent_style.width.unwrap_or(ctx.available_width())
    } else {
        cursor.position()
    };
    let padding = if carries_parent_box_geometry {
        parent_style.padding
    } else {
        EdgeSizes::ZERO
    };
    output.push(inline_row_node(
        FlexContent {
            cells,
            row_height,
            alignment: if middle_aligned_line.is_some() {
                crate::style::computed::AlignItems::FlexStart
            } else {
                crate::style::computed::AlignItems::Baseline
            },
            ..Default::default()
        },
        crate::layout::elements::BoxModel {
            size: crate::layout::elements::LayoutSize::fixed(container_width, parent_style.height),
            margins: BlockMargins::new(parent_style.margin.top, parent_style.margin.bottom),
            padding,
            border: parent_border,
        },
        crate::layout::elements::InlineOffset::new(parent_style.margin.left),
        parent_style.background_color,
    ));
    true
}

#[allow(clippy::too_many_arguments)]
fn layout_inline_block_group_inner(
    elements: &[(&ElementNode, bool)],
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutNode>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    fonts: &HashMap<String, TtfFont>,
    mut env: Option<&mut LayoutEnv>,
) {
    let available_width = ctx.available_width().max(0.0);
    if elements.is_empty() {
        return;
    }

    // Lay out each inline-block element as a block to measure its size
    struct InlineBlockItem {
        kind: AtomicInlineKind,
        width: f32,
        height: f32,
        lines: Vec<TextLine>,
        padding: EdgeSizes,
        border: LayoutBorder,
        paint: CellPaint,
        positioning: Positioning,
        text_align: TextAlign,
        margins: EdgeSizes,
        nested_elements: Vec<LayoutNode>,
        space_before: bool,
        suppress_strut_descent: bool,
    }

    let mut items: Vec<InlineBlockItem> = Vec::new();
    let child_count = elements.len();
    let sibling_list: Vec<(String, Vec<String>)> = elements
        .iter()
        .map(|(el, _)| {
            (
                el.tag_name().to_string(),
                el.class_list().iter().map(|s| s.to_string()).collect(),
            )
        })
        .collect();

    for (idx, (child_el, space_before)) in elements.iter().enumerate() {
        let classes = child_el.class_list();
        let selector_ctx = SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index: idx,
            sibling_count: child_count,
            preceding_siblings: sibling_list[..idx].to_vec(),
            following_siblings: sibling_list[idx + 1..].to_vec(),
            is_empty: false,
        };
        let child_style = compute_style_with_context_with_font_metrics(
            child_el.tag,
            child_el.style_attr(),
            parent_style,
            rules,
            child_el.tag_name(),
            &classes,
            child_el.id(),
            &child_el.attributes,
            &selector_ctx,
            FontMetrics::new(fonts),
        );

        if child_style.display == Display::None {
            continue;
        }
        let InlineFormattingRole::Atomic(kind) = InlineFormattingRole::of(child_el, &child_style)
        else {
            continue;
        };

        // Determine the element width
        let has_explicit_width = child_style.width.is_some();
        let child_w = child_style.width.unwrap_or(0.0);
        let child_h = child_style.height.unwrap_or(0.0);

        let inner_width = if has_explicit_width {
            if child_style.box_sizing == BoxSizing::BorderBox {
                child_w - child_style.padding.horizontal() - child_style.border.horizontal_width()
            } else {
                child_w
            }
            .max(0.0)
        } else {
            // No explicit width: use available width for shrink-to-fit
            available_width
        };

        // Collect text runs from the inline-block element's children
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(AncestorInfo {
            element: child_el,
            child_index: idx,
            sibling_count: child_count,
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        });
        if child_style.display == Display::InlineGrid
            && let Some(env) = env.as_deref_mut()
        {
            let mut grid_style = child_style.clone();
            grid_style.display = Display::Grid;
            grid_style.margin = Default::default();
            let track_len = |track: &GridTrack| match track {
                GridTrack::Fixed(v) => *v,
                GridTrack::Percent(p) => p * available_width,
                GridTrack::Minmax(min, _) => *min,
                _ => 0.0,
            };
            let intrinsic_w = child_style
                .grid_template_columns
                .iter()
                .map(track_len)
                .sum::<f32>()
                + child_style.column_gap
                    * child_style.grid_template_columns.len().saturating_sub(1) as f32;
            let intrinsic_h = child_style
                .grid_template_rows
                .iter()
                .map(track_len)
                .sum::<f32>()
                + child_style.row_gap
                    * child_style.grid_template_rows.len().saturating_sub(1) as f32;
            if grid_style.width.is_none() {
                grid_style.width = Some(intrinsic_w);
            }
            if grid_style.height.is_none() && intrinsic_h > 0.0 {
                grid_style.height = Some(intrinsic_h);
            }
            let border_box_w = child_style.width.unwrap_or(intrinsic_w).max(0.0);
            let border_box_h = child_style.height.unwrap_or(intrinsic_h).max(0.0);
            let mut nested_elements = Vec::new();
            let child_ctx = ctx.with_parent_and_basis(
                border_box_w,
                border_box_w,
                Some(border_box_h),
                child_style.font_size,
            );
            layout_grid_container(
                child_el,
                &grid_style,
                &child_ctx,
                &mut nested_elements,
                &child_ancestors,
                0,
                env,
            );
            items.push(InlineBlockItem {
                kind,
                width: border_box_w,
                height: border_box_h,
                lines: Vec::new(),
                padding: EdgeSizes::ZERO,
                border: LayoutBorder::default(),
                paint: CellPaint {
                    box_paint: BoxPaint {
                        shadows: child_style.box_shadow.clone(),
                        group: crate::layout::elements::PaintGroup::from_style(&child_style),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                positioning: Positioning::from_style(&child_style),
                text_align: child_style.text_align,
                margins: child_style.margin,
                nested_elements,
                space_before: *space_before,
                suppress_strut_descent: false,
            });
            continue;
        }
        if child_style.display == Display::InlineFlex
            && let Some(env) = env.as_deref_mut()
        {
            let generated_styles = GeneratedContentStyles::resolve(
                child_el,
                &child_style,
                rules,
                &selector_ctx,
                fonts,
            );
            let dimensions = ResolvedBoxDimensions::from_style(
                &child_style,
                Size::new(available_width, child_h),
            );
            let flex_style = content_only_flex_style(&child_style, dimensions);
            let mut nested_elements = Vec::new();
            let child_ctx = ctx.with_parent_and_basis(
                dimensions.content.width,
                dimensions.content.width,
                child_style.height.map(|_| dimensions.content.height),
                child_style.font_size,
            );
            layout_flex_container(
                child_el,
                &flex_style,
                &child_ctx,
                &mut nested_elements,
                &child_ancestors,
                generated_styles.boxes(child_el),
                0,
                env,
            );
            items.push(InlineBlockItem {
                kind,
                width: dimensions.border_box.width,
                height: dimensions.border_box.height,
                lines: Vec::new(),
                padding: child_style.padding,
                border: LayoutBorder::from_computed(&child_style.border, child_style.color),
                paint: CellPaint::from_style(
                    &child_style,
                    LayoutSize::fixed(
                        dimensions.border_box.width,
                        Some(dimensions.border_box.height),
                    ),
                ),
                positioning: Positioning::from_style(&child_style),
                text_align: child_style.text_align,
                margins: child_style.margin,
                nested_elements,
                space_before: *space_before,
                suppress_strut_descent: false,
            });
            continue;
        }
        let mut runs = Vec::new();
        let mut nested_elements = Vec::new();
        if let Some(env) = env.as_deref_mut() {
            let mut child_layout = InlineBlockChildLayout {
                context: ctx,
                parent_style: &child_style,
                ancestors: &child_ancestors,
                available_width: inner_width.max(0.0),
                output: &mut nested_elements,
            };
            layout_inline_block_contents(
                &child_el.children,
                &child_style,
                &mut runs,
                &child_ancestors,
                env,
                &mut child_layout,
            );
        } else {
            let mut counter_state = CounterState::default();
            let mut resources = crate::security::resources::ResourceLoader::default();
            InlineRunCollector::new(rules, fonts, &mut counter_state, &mut resources)
                .collect_box_content(
                    &child_el.children,
                    &child_style,
                    &mut runs,
                    None,
                    &child_ancestors,
                );
        }

        let wrap_inner_width = if !has_explicit_width
            && child_style.width_keyword == Some(IntrinsicWidthKeyword::MinContent)
            && child_style.overflow_wrap == OverflowWrap::Anywhere
        {
            min_content_anywhere_width(&runs, fonts)
        } else {
            inner_width
        };
        let lines = if !runs.is_empty() {
            wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    wrap_inner_width,
                    used_font_size(&child_style, fonts),
                    text_run_line_height_factor(&child_style, fonts),
                    child_style.overflow_wrap,
                )
                .with_white_space(child_style.white_space)
                .with_parent_strut(parent_line_strut(&child_style, fonts))
                .with_rtl(child_style.direction_rtl)
                .with_bidi_override(child_style.bidi_override)
                .with_bidi_plaintext(child_style.bidi_plaintext)
                .with_word_break_keep_all(child_style.word_break_keep_all)
                .with_line_break_anywhere(child_style.line_break_anywhere)
                .with_hyphens_manual(child_style.hyphens_manual),
                fonts,
            )
        } else {
            Vec::new()
        };

        // Total element width including padding + border
        let content_w = if has_explicit_width {
            child_w
        } else {
            // Shrink-to-fit: widest line, measured with the REAL bundled-font
            // advances (str_width is Helvetica AFM and mis-sizes a ParitySans run).
            let line_w = lines
                .iter()
                .map(|l| crate::layout::helpers::measure_runs_width(&l.runs, fonts))
                .fold(0.0f32, f32::max);
            line_w.max(inline_block_nested_outer_width(&nested_elements))
        };
        let total_w = if child_style.box_sizing == BoxSizing::BorderBox && has_explicit_width {
            content_w
        } else {
            content_w + child_style.padding.horizontal() + child_style.border.horizontal_width()
        };

        // Total element height including padding + border
        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let nested_height: f32 = nested_elements
            .iter()
            .map(|element| crate::layout::paginate::estimate_element_height(element.as_ref()))
            .sum();
        let content_h = if child_h > 0.0 {
            child_h
        } else {
            text_height + nested_height
        };
        let total_h = if child_style.box_sizing == BoxSizing::BorderBox && child_h > 0.0 {
            child_h
        } else {
            content_h + child_style.padding.vertical() + child_style.border.vertical_width()
        };

        // CSS `position: relative` shifts an inline-block's painted box (and its
        // content) without changing its in-flow inline slot (CSS2 §9.4.3). With
        // no explicit `transform`, model the shift as a `translate()` (the
        // renderer applies a cell transform pivot-invariantly for a pure
        // translate). `left`/`top` win over `right`/`bottom`.
        let rel_transform = if child_style.position.is_relative() && child_style.transform.is_none()
        {
            let tx = child_style
                .left
                .or(child_style.right.map(|r| -r))
                .unwrap_or(0.0);
            let ty = child_style
                .top
                .or(child_style.bottom.map(|b| -b))
                .unwrap_or(0.0);
            if tx != 0.0 || ty != 0.0 {
                Some(Transform::Translate {
                    offset: crate::style::computed::CssVector::new(f64::from(tx), f64::from(ty)),
                    percentages: crate::style::computed::PercentageAxes::default(),
                })
            } else {
                None
            }
        } else {
            child_style.transform
        };

        let mut paint =
            CellPaint::from_style(&child_style, LayoutSize::fixed(total_w, Some(total_h)));
        paint.box_paint.group.transform.value = rel_transform;
        items.push(InlineBlockItem {
            kind,
            width: total_w,
            height: total_h,
            lines,
            padding: child_style.padding,
            border: LayoutBorder::from_computed(&child_style.border, child_style.color),
            paint,
            positioning: Positioning::from_style(&child_style),
            text_align: child_style.text_align,
            margins: child_style.margin,
            nested_elements,
            space_before: *space_before,
            suppress_strut_descent: child_style.width_keyword
                == Some(IntrinsicWidthKeyword::MinContent)
                && child_style.overflow_wrap == OverflowWrap::Anywhere,
        });
    }

    if items.is_empty() {
        return;
    }

    // CSS2 §10.8: every line box contains a "strut" — a zero-width inline box
    // with the block's own font and `line-height`. Even a line that holds only
    // atomic inline boxes (e.g. `<span class=chip>` with no text) is therefore at
    // least as tall as that strut, and the strut's portion *below* the baseline is
    // reserved under the in-flow boxes. Baseline-aligned inline-blocks sit above
    // the line baseline, so this descent appears as extra space at the bottom of
    // the line box — which is why a line of empty chips is taller than the chips
    // themselves. Compute the strut split about the baseline from the parent's
    // font metrics so a `font-size: 0` container (strut = 0) is unaffected.
    let strut = parent_line_strut(parent_style, fonts);
    let strut_above = strut.above;
    let strut_below = strut.below;
    let strut_lh = strut_above + strut_below;
    let parent_family = super::text::resolve_style_font_family(parent_style, fonts);
    // Position items horizontally, wrapping to new rows when they exceed available width
    let mut rows: Vec<(Vec<FlexCell>, f32)> = Vec::new(); // (cells, row_height)
    let mut current_cells: Vec<FlexCell> = Vec::new();
    let mut x = 0.0f32;
    // Tallest in-flow box on the current row (its extent above the line baseline,
    // which for these top-anchored baseline boxes is the full margin-box height).
    let mut max_item_height = 0.0f32;
    let mut row_suppress_strut_descent = false;
    let finish_row_height = |max_item_height: f32, suppress_strut_descent: bool| -> f32 {
        if suppress_strut_descent {
            max_item_height.max(strut_lh)
        } else {
            max_item_height.max(strut_above) + strut_below
        }
    };
    let inline_grid_space = estimate_word_width(
        " ",
        parent_style.font_size,
        &parent_family,
        parent_style.font_weight == crate::style::computed::FontWeight::Bold,
        parent_style.font_style.is_slanted(),
        fonts,
    )
    .min(parent_style.font_size * 0.3125);

    for item in &items {
        let item_total_w = item.margins.left + item.width + item.margins.right;
        // Wrap to new row if this item would overflow
        if !current_cells.is_empty() && exceeds_with_roundoff(x + item_total_w, available_width) {
            rows.push((
                std::mem::take(&mut current_cells),
                finish_row_height(max_item_height, row_suppress_strut_descent),
            ));
            x = 0.0;
            max_item_height = 0.0;
            row_suppress_strut_descent = false;
        }

        if !current_cells.is_empty() && item.space_before {
            x += inline_grid_space;
        }
        x += item.margins.left;
        current_cells.push(FlexCell {
            lines: item.lines.clone(),
            x_offset: x,
            width: item.width,
            // The inline-block paints at its own border-box height (`item.height`,
            // which already folds in padding + border), independent of the line
            // box. Marking it explicit-height keeps the painter from stretching it
            // to the line's cross size when the line reserves the text strut.
            natural_height: item.height,
            fragmentation: FlexItemFragmentation::definite(),
            text_align: item.text_align,
            padding: item.padding,
            border: item.border,
            paint: item.paint.clone(),
            positioning: item.positioning.clone(),
            nested_elements: item.nested_elements.clone(),
            y_offset: item.margins.top,
            role: FlexItemRole::AtomicInline(item.kind),
            ..Default::default()
        });
        x += item.width + item.margins.right;
        max_item_height = max_item_height.max(item.margins.top + item.height + item.margins.bottom);
        row_suppress_strut_descent |= item.suppress_strut_descent;
    }
    // Flush last row
    if !current_cells.is_empty() {
        rows.push((
            current_cells,
            finish_row_height(max_item_height, row_suppress_strut_descent),
        ));
    }

    for (cells, rh) in rows {
        output.push(inline_row_node(
            FlexContent {
                cells,
                row_height: rh,
                alignment: crate::style::computed::AlignItems::Baseline,
                ..Default::default()
            },
            crate::layout::elements::BoxModel {
                size: crate::layout::elements::LayoutSize::fixed(available_width, None),
                ..Default::default()
            },
            crate::layout::elements::InlineOffset::ZERO,
            None,
        ));
    }
}

#[cfg(test)]
mod cutoff_tests {
    use super::MiddleAlignedLine;
    use crate::layout::elements::{LayoutElement, LayoutElementTestExt};
    use crate::layout::engine::{layout, layout_with_rules, layout_with_rules_and_fonts};
    use crate::layout::text::LineStrut;
    use crate::parser::html::{parse_html, parse_html_with_styles};
    use crate::types::{Margin, PageSize};
    use std::collections::HashMap;

    fn collect_inline_rows(element: &dyn LayoutElement, row_lengths: &mut Vec<usize>) {
        if let Some(length) = element.inspect_flex(|row| row.content.cells.len()) {
            row_lengths.push(length);
        }
        element.visit_children(&mut |child| collect_inline_rows(child, row_lengths));
    }

    fn collect_mixed_inline_gaps(element: &dyn LayoutElement, gaps: &mut Vec<(f32, f32)>) {
        element.inspect_flex(|row| {
            let cells = &row.content.cells;
            if cells.len() == 3 {
                let before_atomic = cells[1].x_offset - (cells[0].x_offset + cells[0].width);
                let after_atomic = cells[2].x_offset - (cells[1].x_offset + cells[1].width);
                gaps.push((before_atomic, after_atomic));
            }
        });
        element.visit_children(&mut |child| collect_mixed_inline_gaps(child, gaps));
    }

    fn collect_atomic_image_rows(element: &dyn LayoutElement, rows: &mut Vec<(usize, f32)>) {
        element.inspect_flex(|row| {
            if row.content.cells.iter().any(|cell| {
                cell.nested_elements
                    .iter()
                    .any(|nested| nested.inspect_image(|_| ()).is_some())
            }) {
                rows.push((
                    row.content.cells.len(),
                    crate::layout::paginate::estimate_element_height(row),
                ));
            }
        });
        element.visit_children(&mut |child| collect_atomic_image_rows(child, rows));
    }

    fn collect_atomic_image_offsets(element: &dyn LayoutElement, offsets: &mut Vec<f32>) {
        element.inspect_flex(|row| {
            offsets.extend(row.content.cells.iter().filter_map(|cell| {
                cell.nested_elements
                    .iter()
                    .any(|nested| nested.inspect_image(|_| ()).is_some())
                    .then_some(cell.x_offset)
            }));
        });
        element.visit_children(&mut |child| collect_atomic_image_offsets(child, offsets));
    }

    #[test]
    fn inline_blocks_wrap_on_five_thousandths_of_a_point_overflow() {
        let nodes = parse_html(
            r#"<div style="width:20pt;font-size:0"><div style="display:inline-block;width:10.0025pt;height:1pt"></div><div style="display:inline-block;width:10.0025pt;height:1pt"></div></div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut row_lengths = Vec::new();
        for (_, element) in &pages[0].elements {
            collect_inline_rows(element, &mut row_lengths);
        }

        assert_eq!(
            row_lengths,
            vec![1, 1],
            "20.005pt of inline boxes must not fit in a 20pt row"
        );
    }

    #[test]
    fn inline_grid_keeps_collapsed_spaces_between_text_fragments() {
        let nodes = parse_html(
            r#"<div style="font-family:ParitySans;font-size:20px;line-height:1.5">L <span style="display:inline-grid;grid-template-columns:40px 40px;grid-template-rows:40px"><span></span><span></span></span> R</div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::new(264.0, 120.0), Margin::default());
        let mut gaps = Vec::new();
        for (_, element) in &pages[0].elements {
            collect_mixed_inline_gaps(element, &mut gaps);
        }

        assert_eq!(gaps.len(), 1, "expected one mixed inline row");
        let (before_atomic, after_atomic) = gaps[0];
        assert!(
            before_atomic > 0.0,
            "space before inline-grid was discarded"
        );
        assert!(after_atomic > 0.0, "space after inline-grid was discarded");
        assert!(
            (before_atomic - after_atomic).abs() < 0.001,
            "the two spaces share the parent inline formatting context"
        );
    }

    #[test]
    fn middle_atomic_line_uses_css_pixel_x_height_and_baseline_grid() {
        // Chromium's print layout rounds ParitySans's 10.9375px x-height to
        // 11 CSS px, then rounds the box-to-baseline distance. This models the
        // CSS2 `middle` relation without a fixture-specific paint offset.
        let strut = LineStrut {
            above: 16.5,
            below: 6.0,
        };
        let mut line = MiddleAlignedLine::default();
        line.include_box(31.5, 8.203_125);

        assert_eq!(line.above, 20.25);
        assert_eq!(line.below, 11.25);
        assert_eq!(line.used_height(strut), 31.5);
        assert_eq!(line.text_shift(strut), 3.75);

        let mut even_css_height = MiddleAlignedLine::default();
        even_css_height.include_box(30.0, 8.203_125);
        assert_eq!(even_css_height.above, 19.5);
        assert_eq!(even_css_height.below, 10.5);
    }

    #[test]
    fn native_inline_image_keeps_its_parent_line_box() {
        let nodes = parse_html(
            r#"<div style="box-sizing:border-box;width:320px;margin:24px;padding:10px;border:2px solid #22223b;background:#c9ada7;font-family:ParitySans;font-size:26px;line-height:1.2"><img style="width:40px;height:40px" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg=="></div>"#,
        )
        .unwrap();
        let font_bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans fixture font");
        let font = crate::parser::ttf::parse_ttf(font_bytes).expect("valid ParitySans fixture");
        let fonts = HashMap::from([("paritysans".to_string(), font)]);
        let pages = layout_with_rules_and_fonts(
            &nodes,
            PageSize::new(336.0, 90.0),
            Margin::uniform(0.0),
            &[],
            &fonts,
            None,
            0.0,
            Default::default(),
        );
        assert_eq!(
            pages.len(),
            1,
            "the inline image must not be laid out twice after its atomic row"
        );
        let (row_height, image_cell) = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_flex(|row| {
                    row.content
                        .cells
                        .first()
                        .cloned()
                        .map(|cell| (row.content.row_height, cell))
                })?
            })
            .expect("native inline image should create an atomic line cell");

        assert!(
            (row_height - 35.25).abs() < f32::EPSILON,
            "the image baseline reserves the 7 CSS-pixel block-end font extent; got {row_height}pt"
        );
        assert!(
            image_cell.natural_height >= 30.0,
            "the 40 CSS pixel image must contribute its 30pt content height"
        );
        assert!(
            image_cell.nested_elements.len() == 1
                && image_cell.nested_elements[0]
                    .inspect_image(|_| ())
                    .is_some()
        );
    }

    #[test]
    fn inline_block_image_remains_replaced_in_a_contextual_selector() {
        let document = parse_html_with_styles(
            r#"<style>
                .asset { display: none; }
                .feature > .own > .asset {
                    display: inline-block;
                    width: 34px;
                    height: 24px;
                }
                .own { height: 22px; }
            </style>
            <div class="feature"><div class="own"><span>A</span><img class="asset" alt="" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAIElEQVR42mM4IScHR3I9NnDEgFNCzu0EHN1ZJQJHOCUAni4lgeO2HLIAAAAASUVORK5CYII="><span>B</span></div></div>"#,
        )
        .unwrap();
        let rules = document
            .stylesheets
            .iter()
            .flat_map(|stylesheet| crate::parser::css::parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &document.nodes,
            PageSize::new(400.0, 90.0),
            Margin::uniform(0.0),
            &rules,
        );
        let mut rows = Vec::new();
        for (_, element) in &pages[0].elements {
            collect_atomic_image_rows(element.as_ref(), &mut rows);
        }

        assert_eq!(
            rows,
            vec![(3, 16.5)],
            "text and image share one row, while overflow preserves the definite 22px flow height"
        );
    }

    #[test]
    fn out_of_flow_sibling_does_not_split_inline_image_row() {
        let document = parse_html_with_styles(
            r#"<style>
                .asset { display: inline-block; width: 34px; height: 24px; }
                .out { position: absolute; right: 4px; }
            </style>
            <div style="position:relative"><span>A</span><span class="out">B</span><img class="asset" alt="" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAIElEQVR42mM4IScHR3I9NnDEgFNCzu0EHN1ZJQJHOCUAni4lgeO2HLIAAAAASUVORK5CYII="></div>"#,
        )
        .unwrap();
        let rules = document
            .stylesheets
            .iter()
            .flat_map(|stylesheet| crate::parser::css::parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &document.nodes,
            PageSize::new(400.0, 90.0),
            Margin::uniform(0.0),
            &rules,
        );
        let mut rows = Vec::new();
        for (_, element) in &pages[0].elements {
            collect_atomic_image_rows(element.as_ref(), &mut rows);
        }

        assert_eq!(
            rows.len(),
            1,
            "the positioned sibling is removed from flow without splitting the line"
        );
        assert_eq!(rows[0].0, 2, "the row contains the text and image cells");
        let mut offsets = Vec::new();
        for (_, element) in &pages[0].elements {
            collect_atomic_image_offsets(element.as_ref(), &mut offsets);
        }
        assert_eq!(offsets.len(), 1);
        assert!(
            offsets[0] > 0.0,
            "the image follows the remaining in-flow text"
        );
    }

    #[test]
    fn positioned_ancestor_keeps_image_after_in_flow_text() {
        let document = parse_html_with_styles(include_str!(
            "../../tests/parity/cases/interactions/interactions-cartesian-images-replaced-x-positioning.html"
        ))
        .unwrap();
        let rules = document
            .stylesheets
            .iter()
            .flat_map(|stylesheet| crate::parser::css::parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &document.nodes,
            PageSize::new(390.0, 150.0),
            Margin::uniform(0.0),
            &rules,
        );
        let mut offsets = Vec::new();
        for (_, element) in &pages[0].elements {
            collect_atomic_image_offsets(element.as_ref(), &mut offsets);
        }

        assert_eq!(offsets.len(), 3);
        assert!(
            offsets[0] > 0.0,
            "the positioned ancestor must not reset the image cell's inline offset: {offsets:?}"
        );
    }

    #[test]
    fn flex_auto_item_uses_mixed_row_max_content_width() {
        let document = parse_html_with_styles(include_str!(
            "../../tests/parity/cases/interactions/interactions-cartesian-flexbox-x-images-replaced.html"
        ))
        .unwrap();
        let rules = document
            .stylesheets
            .iter()
            .flat_map(|stylesheet| crate::parser::css::parse_stylesheet(stylesheet))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &document.nodes,
            PageSize::new(390.0, 150.0),
            Margin::uniform(0.0),
            &rules,
        );

        let root_row = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_flex(Clone::clone))
            .expect("the three inline-flex stages share one root row");
        let first_stage = root_row.content.cells[0].nested_elements[0]
            .inspect_flex(Clone::clone)
            .expect("the first stage remains a flex formatting context");
        let node = first_stage.content.cells[0].nested_elements[0]
            .inspect_flex(Clone::clone)
            .expect("the node remains its own flex formatting context");
        let item = &node.content.cells[0];

        assert!(
            (50.0..60.0).contains(&item.width),
            "the mixed row includes both text advances and the 25.5pt image: {}",
            item.width,
        );
        assert!(
            item.x_offset > 10.0,
            "justify-content:center positions the shrink-wrapped item: {}",
            item.x_offset,
        );
    }
}
