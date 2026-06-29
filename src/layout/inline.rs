use crate::parser::css::{AncestorInfo, CssRule, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    compute_style_with_context, BackgroundClip, BackgroundOrigin, BackgroundPosition,
    BackgroundRepeat, BackgroundSize, BoxSizing, ComputedStyle, ConicGradient, Display, GridTrack,
    IntrinsicWidthKeyword, LinearGradient, OverflowWrap, RadialGradient, TextAlign, Transform,
};
use std::collections::HashMap;

use super::context::{LayoutContext, LayoutEnv};
use super::engine::{BackgroundFields, FlexCell, LayoutBorder, LayoutElement, TextLine};
use super::flex::layout_flex_container;
use super::grid::layout_grid_container;
use super::table::flatten_table;
use super::text::{
    collect_text_runs, estimate_word_width, resolved_line_height_factor, wrap_text_runs,
    FlexTextRunCollector, TextWrapOptions,
};

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
                    r.italic,
                    fonts,
                )
            })
        })
        .fold(0.0f32, f32::max)
}

/// Check if an element computes to an atomic inline-level layout child.
pub(crate) fn element_is_inline_block(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
    preceding_siblings: &[(String, Vec<String>)],
) -> bool {
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: preceding_siblings.to_vec(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let style = compute_style_with_context(
        el.tag,
        el.style_attr(),
        parent_style,
        rules,
        el.tag_name(),
        &classes,
        el.id(),
        &el.attributes,
        &selector_ctx,
    );
    // SVGs need individual block layout (they use cm operator for viewBox).
    matches!(
        style.display,
        Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::InlineTable
    ) && el.tag != HtmlTag::Svg
        && !el
            .children
            .iter()
            .any(|c| matches!(c, DomNode::Element(e) if e.tag == HtmlTag::Svg))
}

/// Check if a natively-inline element has been styled with `display: block`
/// via CSS rules, making it a block-level element for layout purposes.
pub(crate) fn element_has_css_display_block(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
) -> bool {
    if el.tag.is_block() {
        return false; // already block by default
    }
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let style = compute_style_with_context(
        el.tag,
        el.style_attr(),
        parent_style,
        rules,
        el.tag_name(),
        &classes,
        el.id(),
        &el.attributes,
        &selector_ctx,
    );
    style.display == Display::Block
}

/// Lay out consecutive atomic inline elements as `FlexRow`s.
///
/// The bool carried with each element records whether source whitespace appeared
/// immediately before it inside the current inline formatting context.
pub(crate) fn layout_inline_block_group_with_spacing(
    elements: &[(&ElementNode, bool)],
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
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
    output: &mut Vec<LayoutElement>,
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
    runs: Vec<crate::layout::engine::TextRun>,
    parent_style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
    x: f32,
) -> Option<(FlexCell, f32)> {
    if runs.is_empty() {
        return None;
    }
    let lines = wrap_text_runs(
        runs,
        TextWrapOptions::new(
            f32::MAX,
            parent_style.font_size,
            resolved_line_height_factor(parent_style, fonts),
            parent_style.overflow_wrap,
        )
        .with_rtl(parent_style.direction_rtl)
        .with_bidi_override(parent_style.bidi_override)
        .with_bidi_plaintext(parent_style.bidi_plaintext),
        fonts,
    );
    if lines.is_empty() {
        return None;
    }
    let width = lines
        .iter()
        .map(|line| crate::layout::helpers::measure_runs_width(&line.runs, fonts))
        .fold(0.0f32, f32::max);
    let height = lines.iter().map(|line| line.height).sum::<f32>();
    Some((
        FlexCell {
            lines,
            x_offset: x,
            width,
            text_align: parent_style.text_align,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border: LayoutBorder::default(),
            natural_height: height,
            has_explicit_height: true,
            cross_min: 0.0,
            cross_max: f32::INFINITY,
            align_self: crate::style::computed::AlignSelf::Auto,
            border_radius: 0.0,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            background_clip: BackgroundClip::Border,
            transform: None,
            transform_origin: crate::style::computed::TransformOrigin::default(),
            box_shadow: Vec::new(),
            nested_elements: Vec::new(),
            y_offset: 0.0,
            line_cross_size: 0.0,
            is_positioned: false,
            z_index: 0,
        },
        width,
    ))
}

fn push_parent_space_run(
    runs: &mut Vec<crate::layout::engine::TextRun>,
    parent_style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) {
    runs.push(crate::layout::engine::TextRun {
        text: " ".to_string(),
        font_size: parent_style.font_size,
        bold: parent_style.font_weight == crate::style::computed::FontWeight::Bold,
        italic: parent_style.font_style == crate::style::computed::FontStyle::Italic,
        underline: parent_style.text_decoration_underline,
        line_through: parent_style.text_decoration_line_through,
        overline: parent_style.text_decoration_overline,
        decoration_color: parent_style.text_decoration_color.map(|c| c.to_f32_rgb()),
        color: parent_style.color.to_f32_rgb(),
        link_url: None,
        font_family: super::text::resolve_style_font_family(parent_style, fonts),
        background_color: None,
        padding: (0.0, 0.0),
        border_radius: 0.0,
        line_height_factor: resolved_line_height_factor(parent_style, fonts),
        inline_box: None,
        disable_ligatures: false,
        vertical_align: parent_style.vertical_align,
        text_shadow: parent_style.text_shadow.clone(),
    });
}

#[allow(clippy::too_many_arguments)]
fn inline_atomic_cell(
    child_el: &ElementNode,
    child_style: &ComputedStyle,
    ctx: &LayoutContext,
    ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
    x: f32,
    env: &mut LayoutEnv,
) -> Option<(FlexCell, f32)> {
    let mut child_ancestors = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        element: child_el,
        child_index,
        sibling_count,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    });

    let (width, height, nested_elements, background_color, border, padding, y_offset) =
        match child_style.display {
            Display::InlineFlex => {
                let mut flex_style = child_style.clone();
                flex_style.display = Display::Flex;
                flex_style.margin = Default::default();
                flex_style.background_color = None;
                flex_style.border = Default::default();
                let border_box_w = child_style.width.unwrap_or(ctx.available_width()).max(0.0);
                let border_box_h = child_style.height.unwrap_or(0.0).max(0.0);
                let child_ctx = ctx.with_parent_and_basis(
                    border_box_w.max(1.0),
                    border_box_w.max(1.0),
                    Some(border_box_h.max(1.0)),
                    child_style.font_size,
                );
                let mut nested = Vec::new();
                layout_flex_container(
                    child_el,
                    &flex_style,
                    &child_ctx,
                    &mut nested,
                    &child_ancestors,
                    None,
                    None,
                    0,
                    env,
                );
                (
                    border_box_w,
                    border_box_h,
                    nested,
                    child_style.background_color.map(|c| c.to_f32_rgba()),
                    LayoutBorder::from_computed(&child_style.border),
                    (
                        child_style.padding.top,
                        child_style.padding.right,
                        child_style.padding.bottom,
                        child_style.padding.left,
                    ),
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
                    border_box_w.max(1.0),
                    border_box_w.max(1.0),
                    Some(border_box_h.max(1.0)),
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
                    (0.0, 0.0, 0.0, 0.0),
                    0.0,
                )
            }
            Display::InlineTable => {
                let mut table_style = child_style.clone();
                table_style.display = Display::Table;
                table_style.margin = Default::default();
                let mut nested = Vec::new();
                flatten_table(
                    child_el,
                    &table_style,
                    ctx.available_width(),
                    &mut nested,
                    ancestors,
                    child_index,
                    sibling_count,
                    env,
                );
                let mut width = nested
                    .iter()
                    .map(crate::layout::paginate::table_row_content_width)
                    .fold(0.0f32, f32::max);
                if table_style.border_collapse == crate::style::computed::BorderCollapse::Separate
                {
                    width += table_style.border_spacing * 2.0;
                }
                let height = nested
                    .iter()
                    .map(crate::layout::paginate::estimate_element_height)
                    .sum::<f32>();
                (
                    width,
                    height,
                    nested,
                    child_style.background_color.map(|c| c.to_f32_rgba()),
                    LayoutBorder::from_computed(&child_style.border),
                    (
                        child_style.padding.top,
                        child_style.padding.right,
                        child_style.padding.bottom,
                        child_style.padding.left,
                    ),
                    -height,
                )
            }
            Display::InlineBlock => {
                let mut runs = Vec::new();
                collect_text_runs(
                    &child_el.children,
                    child_style,
                    &mut runs,
                    None,
                    env.rules,
                    env.fonts,
                    &child_ancestors,
                    env.counter_state,
                );
                let lines = wrap_text_runs(
                    runs,
                    TextWrapOptions::new(
                        child_style.width.unwrap_or(f32::MAX).max(1.0),
                        child_style.font_size,
                        resolved_line_height_factor(child_style, env.fonts),
                        child_style.overflow_wrap,
                    ),
                    env.fonts,
                );
                let content_w = child_style.width.unwrap_or_else(|| {
                    lines
                        .iter()
                        .map(|line| crate::layout::helpers::measure_runs_width(&line.runs, env.fonts))
                        .fold(0.0f32, f32::max)
                });
                let content_h = child_style
                    .height
                    .unwrap_or_else(|| lines.iter().map(|line| line.height).sum::<f32>());
                let total_w =
                    content_w + child_style.padding.left + child_style.padding.right
                        + child_style.border.horizontal_width();
                let total_h =
                    content_h + child_style.padding.top + child_style.padding.bottom
                        + child_style.border.vertical_width();
                return Some((
                    FlexCell {
                        lines,
                        x_offset: x + child_style.margin.left,
                        width: total_w,
                        text_align: child_style.text_align,
                        background_color: child_style.background_color.map(|c| c.to_f32_rgba()),
                        padding_top: child_style.padding.top,
                        padding_right: child_style.padding.right,
                        padding_bottom: child_style.padding.bottom,
                        padding_left: child_style.padding.left,
                        border: LayoutBorder::from_computed(&child_style.border),
                        natural_height: total_h,
                        has_explicit_height: true,
                        cross_min: 0.0,
                        cross_max: f32::INFINITY,
                        align_self: crate::style::computed::AlignSelf::Auto,
                        border_radius: child_style.border_radius,
                        background_gradient: None,
                        background_radial_gradient: None,
                        background_conic_gradient: None,
                        background_svg: None,
                        background_blur_radius: 0.0,
                        background_size: BackgroundSize::Auto,
                        background_position: BackgroundPosition::default(),
                        background_repeat: BackgroundRepeat::Repeat,
                        background_origin: BackgroundOrigin::Padding,
                        background_clip: BackgroundClip::Border,
                        transform: child_style.transform,
                        transform_origin: child_style.transform_origin,
                        box_shadow: child_style.box_shadow.clone(),
                        nested_elements: Vec::new(),
                        y_offset: 0.0,
                        line_cross_size: 0.0,
                        is_positioned: matches!(
                            child_style.position,
                            crate::style::computed::Position::Relative
                                | crate::style::computed::Position::Absolute
                        ),
                        z_index: child_style.z_index,
                    },
                    total_w + child_style.margin.left + child_style.margin.right,
                ));
            }
            _ => return None,
        };

    Some((
        FlexCell {
            lines: Vec::new(),
            x_offset: x + child_style.margin.left,
            width,
            text_align: child_style.text_align,
            background_color,
            padding_top: padding.0,
            padding_right: padding.1,
            padding_bottom: padding.2,
            padding_left: padding.3,
            border,
            natural_height: height,
            has_explicit_height: true,
            cross_min: 0.0,
            cross_max: f32::INFINITY,
            align_self: crate::style::computed::AlignSelf::Auto,
            border_radius: child_style.border_radius,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            background_clip: BackgroundClip::Border,
            transform: child_style.transform,
            transform_origin: child_style.transform_origin,
            box_shadow: child_style.box_shadow.clone(),
            nested_elements,
            y_offset,
            line_cross_size: 0.0,
            is_positioned: matches!(
                child_style.position,
                crate::style::computed::Position::Relative
                    | crate::style::computed::Position::Absolute
            ),
            z_index: child_style.z_index,
        },
        width + child_style.margin.left + child_style.margin.right,
    ))
}

pub(crate) fn layout_inline_mixed_sequence_with_env(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    env: &mut LayoutEnv,
) -> bool {
    let element_count = nodes
        .iter()
        .filter(|node| matches!(node, DomNode::Element(_)))
        .count();
    let sibling_list: Vec<(String, Vec<String>)> = nodes
        .iter()
        .filter_map(|node| match node {
            DomNode::Element(el) => Some((
                el.tag_name().to_string(),
                el.class_list().iter().map(|s| s.to_string()).collect(),
            )),
            DomNode::Text(_) => None,
        })
        .collect();
    let mut element_index = 0usize;
    let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
    let mut pending_runs = Vec::new();
    let mut cells = Vec::new();
    let mut x = 0.0f32;
    let mut saw_atomic = false;
    let mut last_item_was_atomic = false;
    let mut pending_trailing_space = false;

    for node in nodes {
        match node {
            DomNode::Text(text) => {
                if last_item_was_atomic && text.chars().next().is_some_and(char::is_whitespace) {
                    push_parent_space_run(&mut pending_runs, parent_style, env.fonts);
                }
                collect_text_runs(
                    std::slice::from_ref(node),
                    parent_style,
                    &mut pending_runs,
                    None,
                    env.rules,
                    env.fonts,
                    ancestors,
                    env.counter_state,
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
                    following_siblings: sibling_list[element_index + 1..].to_vec(),
                    is_empty: false,
                };
                let child_style = compute_style_with_context(
                    el.tag,
                    el.style_attr(),
                    parent_style,
                    env.rules,
                    el.tag_name(),
                    &classes,
                    el.id(),
                    &el.attributes,
                    &selector_ctx,
                );
                if child_style.display == Display::None {
                    element_index += 1;
                    continue;
                }
                if matches!(
                    child_style.display,
                    Display::InlineFlex
                        | Display::InlineGrid
                        | Display::InlineTable
                        | Display::InlineBlock
                ) {
                    if pending_trailing_space
                        && pending_runs.iter().any(|run| !run.text.is_empty())
                    {
                        push_parent_space_run(&mut pending_runs, parent_style, env.fonts);
                    }
                    if let Some((cell, advance)) =
                        inline_text_cell(std::mem::take(&mut pending_runs), parent_style, env.fonts, x)
                    {
                        x += advance;
                        cells.push(cell);
                    }
                    if let Some((cell, advance)) = inline_atomic_cell(
                        el,
                        &child_style,
                        ctx,
                        ancestors,
                        element_index,
                        element_count,
                        x,
                        env,
                    ) {
                        saw_atomic = true;
                        x += advance;
                        cells.push(cell);
                    }
                    last_item_was_atomic = true;
                    pending_trailing_space = false;
                } else {
                    collect_text_runs(
                        std::slice::from_ref(node),
                        parent_style,
                        &mut pending_runs,
                        None,
                        env.rules,
                        env.fonts,
                        ancestors,
                        env.counter_state,
                    );
                    pending_trailing_space = false;
                    last_item_was_atomic = false;
                }
                preceding_siblings.push((
                    el.tag_name().to_string(),
                    el.class_list().iter().map(|s| s.to_string()).collect(),
                ));
                element_index += 1;
            }
        }
    }
    if let Some((cell, _advance)) =
        inline_text_cell(std::mem::take(&mut pending_runs), parent_style, env.fonts, x)
    {
        cells.push(cell);
    }

    if !saw_atomic || cells.is_empty() {
        return false;
    }

    let line_height = parent_style.font_size * resolved_line_height_factor(parent_style, env.fonts);
    let row_height = cells
        .iter()
        .map(|cell| cell.natural_height)
        .fold(line_height, f32::max);
    let parent_border = LayoutBorder::from_computed(&parent_style.border);
    let paints_parent_box = parent_style.background_color.is_some() || parent_border.has_any();
    let container_width = if paints_parent_box {
        parent_style.width.unwrap_or(ctx.available_width())
    } else {
        x
    };
    let (padding_top, padding_right, padding_bottom, padding_left) = if paints_parent_box {
        (
            parent_style.padding.top,
            parent_style.padding.right,
            parent_style.padding.bottom,
            parent_style.padding.left,
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    output.push(LayoutElement::FlexRow {
        cells,
        row_height,
        margin_top: parent_style.margin.top,
        margin_bottom: parent_style.margin.bottom,
        offset_left: parent_style.margin.left,
        background_color: parent_style.background_color.map(|c| c.to_f32_rgba()),
        container_width,
        padding_top,
        padding_bottom,
        padding_left,
        padding_right,
        border: parent_border,
        border_radius: 0.0,
        box_shadow: Vec::new(),
        background_gradient: None,
        background_radial_gradient: None,
        background_conic_gradient: None,
        background_svg: None,
        background_blur_radius: 0.0,
        background_size: BackgroundSize::Auto,
        background_position: BackgroundPosition::default(),
        background_repeat: BackgroundRepeat::Repeat,
        background_origin: BackgroundOrigin::Padding,
        background_clip: BackgroundClip::Border,
        align_items: crate::style::computed::AlignItems::Baseline,
        positioned_depth: 0,
    });
    true
}

#[allow(clippy::too_many_arguments)]
fn layout_inline_block_group_inner(
    elements: &[(&ElementNode, bool)],
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    fonts: &HashMap<String, TtfFont>,
    mut env: Option<&mut LayoutEnv>,
) {
    let available_width = ctx.available_width();
    if elements.is_empty() {
        return;
    }

    // Lay out each inline-block element as a block to measure its size
    struct InlineBlockItem {
        width: f32,
        height: f32,
        lines: Vec<TextLine>,
        background_color: Option<(f32, f32, f32, f32)>,
        padding_top: f32,
        padding_right: f32,
        padding_bottom: f32,
        padding_left: f32,
        border: LayoutBorder,
        border_radius: f32,
        transform: Option<Transform>,
        transform_origin: crate::style::computed::TransformOrigin,
        background_gradient: Option<LinearGradient>,
        background_radial_gradient: Option<RadialGradient>,
        background_conic_gradient: Option<ConicGradient>,
        background_svg: Option<crate::parser::svg::SvgTree>,
        background_blur_radius: f32,
        background_size: BackgroundSize,
        background_position: BackgroundPosition,
        background_repeat: BackgroundRepeat,
        background_origin: BackgroundOrigin,
        background_clip: BackgroundClip,
        text_align: TextAlign,
        margin_top: f32,
        margin_left: f32,
        margin_right: f32,
        margin_bottom: f32,
        box_shadow: Vec<crate::style::computed::BoxShadow>,
        nested_elements: Vec<LayoutElement>,
        space_before: bool,
        is_positioned: bool,
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
        let child_style = compute_style_with_context(
            child_el.tag,
            child_el.style_attr(),
            parent_style,
            rules,
            child_el.tag_name(),
            &classes,
            child_el.id(),
            &child_el.attributes,
            &selector_ctx,
        );

        if child_style.display == Display::None {
            continue;
        }

        // Determine the element width
        let has_explicit_width = child_style.width.is_some();
        let child_w = child_style.width.unwrap_or(0.0);
        let child_h = child_style.height.unwrap_or(0.0);

        let inner_width = if has_explicit_width {
            if child_style.box_sizing == BoxSizing::BorderBox {
                child_w
                    - child_style.padding.left
                    - child_style.padding.right
                    - child_style.border.horizontal_width()
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
        if child_style.display == Display::InlineGrid && env.is_some() {
            let env = env.as_deref_mut().expect("checked above");
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
                border_box_w.max(1.0),
                border_box_w.max(1.0),
                Some(border_box_h.max(1.0)),
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
                width: border_box_w,
                height: border_box_h,
                lines: Vec::new(),
                background_color: None,
                padding_top: 0.0,
                padding_right: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                border: LayoutBorder::default(),
                border_radius: 0.0,
                transform: child_style.transform,
                transform_origin: child_style.transform_origin,
                background_gradient: None,
                background_radial_gradient: None,
                background_conic_gradient: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                text_align: child_style.text_align,
                margin_top: child_style.margin.top,
                margin_left: child_style.margin.left,
                margin_right: child_style.margin.right,
                margin_bottom: child_style.margin.bottom,
                box_shadow: child_style.box_shadow.clone(),
                nested_elements,
                space_before: *space_before,
                is_positioned: matches!(
                    child_style.position,
                    crate::style::computed::Position::Relative
                        | crate::style::computed::Position::Absolute
                ),
            });
            continue;
        }
        if child_style.display == Display::InlineFlex && env.is_some() {
            let env = env.as_deref_mut().expect("checked above");
            let mut flex_style = child_style.clone();
            flex_style.display = Display::Flex;
            flex_style.margin = Default::default();
            flex_style.background_color = None;
            flex_style.border = Default::default();
            let border_box_w = child_style.width.unwrap_or(inner_width).max(0.0);
            let border_box_h = child_style.height.unwrap_or(child_h).max(0.0);
            let mut nested_elements = Vec::new();
            let child_ctx = ctx.with_parent_and_basis(
                border_box_w.max(1.0),
                border_box_w.max(1.0),
                Some(border_box_h.max(1.0)),
                child_style.font_size,
            );
            layout_flex_container(
                child_el,
                &flex_style,
                &child_ctx,
                &mut nested_elements,
                &child_ancestors,
                None,
                None,
                0,
                env,
            );
            items.push(InlineBlockItem {
                width: border_box_w,
                height: border_box_h,
                lines: Vec::new(),
                background_color: child_style.background_color.map(|c| c.to_f32_rgba()),
                padding_top: child_style.padding.top,
                padding_right: child_style.padding.right,
                padding_bottom: child_style.padding.bottom,
                padding_left: child_style.padding.left,
                border: LayoutBorder::from_computed(&child_style.border),
                border_radius: 0.0,
                transform: child_style.transform,
                transform_origin: child_style.transform_origin,
                background_gradient: None,
                background_radial_gradient: None,
                background_conic_gradient: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                text_align: child_style.text_align,
                margin_top: child_style.margin.top,
                margin_left: child_style.margin.left,
                margin_right: child_style.margin.right,
                margin_bottom: child_style.margin.bottom,
                box_shadow: child_style.box_shadow.clone(),
                nested_elements,
                space_before: *space_before,
                is_positioned: matches!(
                    child_style.position,
                    crate::style::computed::Position::Relative
                        | crate::style::computed::Position::Absolute
                ),
            });
            continue;
        }
        let mut runs = Vec::new();
        FlexTextRunCollector {
            runs: &mut runs,
            rules,
            fonts,
        }
        .collect(
            &child_el.children,
            &child_style,
            None,
            (0.0, 0.0),
            &child_ancestors,
        );

        let wrap_inner_width = if !has_explicit_width
            && child_style.width_keyword == Some(IntrinsicWidthKeyword::MinContent)
            && child_style.overflow_wrap == OverflowWrap::Anywhere
        {
            min_content_anywhere_width(&runs, fonts).max(1.0)
        } else {
            inner_width.max(1.0)
        };
        let lines = if !runs.is_empty() {
            wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    wrap_inner_width,
                    child_style.font_size,
                    resolved_line_height_factor(&child_style, fonts),
                    child_style.overflow_wrap,
                )
                .with_rtl(child_style.direction_rtl)
                .with_bidi_override(child_style.bidi_override)
                .with_bidi_plaintext(child_style.bidi_plaintext)
                .with_word_break_keep_all(child_style.word_break_keep_all)
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
            lines
                .iter()
                .map(|l| crate::layout::helpers::measure_runs_width(&l.runs, fonts))
                .fold(0.0f32, f32::max)
        };
        let total_w = if child_style.box_sizing == BoxSizing::BorderBox && has_explicit_width {
            content_w
        } else {
            content_w
                + child_style.padding.left
                + child_style.padding.right
                + child_style.border.horizontal_width()
        };

        // Total element height including padding + border
        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let content_h = if child_h > 0.0 { child_h } else { text_height };
        let total_h = if child_style.box_sizing == BoxSizing::BorderBox {
            content_h.max(child_h)
        } else {
            content_h
                + child_style.padding.top
                + child_style.padding.bottom
                + child_style.border.vertical_width()
        };

        let bg = child_style
            .background_color
            .map(|c: crate::types::Color| c.to_f32_rgba());
        let bg_fields = BackgroundFields::from_style(&child_style);

        // CSS `position: relative` shifts an inline-block's painted box (and its
        // content) without changing its in-flow inline slot (CSS2 §9.4.3). With
        // no explicit `transform`, model the shift as a `translate()` (the
        // renderer applies a cell transform pivot-invariantly for a pure
        // translate). `left`/`top` win over `right`/`bottom`.
        let rel_transform = if child_style.position == crate::style::computed::Position::Relative
            && child_style.transform.is_none()
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
                    tx,
                    ty,
                    tx_pct: false,
                    ty_pct: false,
                })
            } else {
                None
            }
        } else {
            child_style.transform
        };

        items.push(InlineBlockItem {
            width: total_w,
            height: total_h,
            lines,
            background_color: bg,
            padding_top: child_style.padding.top,
            padding_right: child_style.padding.right,
            padding_bottom: child_style.padding.bottom,
            padding_left: child_style.padding.left,
            border: LayoutBorder::from_computed(&child_style.border),
            border_radius: child_style.border_radius,
            transform: rel_transform,
            transform_origin: child_style.transform_origin,
            background_gradient: bg_fields.gradient,
            background_radial_gradient: bg_fields.radial_gradient,
            background_conic_gradient: bg_fields.conic_gradient,
            background_svg: bg_fields.svg,
            background_blur_radius: bg_fields.blur_radius,
            background_size: bg_fields.size,
            background_position: bg_fields.position,
            background_repeat: bg_fields.repeat,
            background_origin: bg_fields.origin,
            background_clip: bg_fields.clip,
            text_align: child_style.text_align,
            margin_top: child_style.margin.top,
            margin_left: child_style.margin.left,
            margin_right: child_style.margin.right,
            margin_bottom: child_style.margin.bottom,
            box_shadow: child_style.box_shadow.clone(),
            nested_elements: Vec::new(),
            space_before: *space_before,
            // CSS 2.1 §9.9.1: a positioned inline-block (relative/absolute) is
            // painted after all non-positioned in-flow siblings in the same
            // stacking context, so it must not be hidden under a later in-flow
            // sibling it overlaps once `top`/`left` shift it.
            is_positioned: matches!(
                child_style.position,
                crate::style::computed::Position::Relative
                    | crate::style::computed::Position::Absolute
            ),
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
    let strut_lh = parent_style.font_size * resolved_line_height_factor(parent_style, fonts);
    let parent_family = super::text::resolve_style_font_family(parent_style, fonts);
    let (strut_asc, strut_desc) = crate::fonts::font_metrics_ratios(
        &parent_family,
        parent_style.font_weight == crate::style::computed::FontWeight::Bold,
        parent_style.font_style == crate::style::computed::FontStyle::Italic,
        fonts,
    );
    // CSS2 §10.8.1: split `line-height` into the font's ascent/descent plus
    // SYMMETRIC half-leading — NOT proportional to the ascent:descent ratio. The
    // two agree only at `line-height: normal` (zero leading); for a larger
    // line-height the proportional form under-reserves the below-baseline strut
    // by ~half the leading, lifting the line-box bottom.
    let content = (strut_asc + strut_desc) * parent_style.font_size;
    let half_leading = ((strut_lh - content) / 2.0).max(0.0);
    let strut_above = strut_asc * parent_style.font_size + half_leading;
    let strut_below = strut_desc * parent_style.font_size + half_leading;

    // Position items horizontally, wrapping to new rows when they exceed available width
    let mut rows: Vec<(Vec<FlexCell>, f32)> = Vec::new(); // (cells, row_height)
    let mut current_cells: Vec<FlexCell> = Vec::new();
    let mut x = 0.0f32;
    // Tallest in-flow box on the current row (its extent above the line baseline,
    // which for these top-anchored baseline boxes is the full margin-box height).
    let mut max_item_height = 0.0f32;
    // The line box must contain both the tallest box and the strut above the
    // baseline, plus the strut's descent below it.
    let finish_row_height =
        |max_item_height: f32| -> f32 { max_item_height.max(strut_above) + strut_below };
    let inline_grid_space = estimate_word_width(
        " ",
        parent_style.font_size,
        &parent_family,
        parent_style.font_weight == crate::style::computed::FontWeight::Bold,
        parent_style.font_style == crate::style::computed::FontStyle::Italic,
        fonts,
    );

    for item in &items {
        let item_total_w = item.margin_left + item.width + item.margin_right;
        // Wrap to new row if this item would overflow
        if !current_cells.is_empty() && x + item_total_w > available_width + 0.01 {
            rows.push((
                std::mem::take(&mut current_cells),
                finish_row_height(max_item_height),
            ));
            x = 0.0;
            max_item_height = 0.0;
        }

        if !current_cells.is_empty() && item.space_before {
            x += inline_grid_space;
        }
        x += item.margin_left;
        current_cells.push(FlexCell {
            lines: item.lines.clone(),
            x_offset: x,
            width: item.width,
            // The inline-block paints at its own border-box height (`item.height`,
            // which already folds in padding + border), independent of the line
            // box. Marking it explicit-height keeps the painter from stretching it
            // to the line's cross size when the line reserves the text strut.
            natural_height: item.height,
            has_explicit_height: true,
            cross_min: 0.0,
            cross_max: f32::INFINITY,
            align_self: crate::style::computed::AlignSelf::Auto,
            text_align: item.text_align,
            background_color: item.background_color,
            padding_top: item.padding_top,
            padding_right: item.padding_right,
            padding_bottom: item.padding_bottom,
            padding_left: item.padding_left,
            border: item.border,
            border_radius: item.border_radius,
            background_gradient: item.background_gradient.clone(),
            background_radial_gradient: item.background_radial_gradient.clone(),
            background_conic_gradient: item.background_conic_gradient.clone(),
            background_svg: item.background_svg.clone(),
            background_blur_radius: item.background_blur_radius,
            background_size: item.background_size,
            background_position: item.background_position,
            background_repeat: item.background_repeat,
            background_origin: item.background_origin,
            background_clip: item.background_clip,
            transform: item.transform,
            transform_origin: item.transform_origin,
            box_shadow: item.box_shadow.clone(),
            nested_elements: item.nested_elements.clone(),
            y_offset: item.margin_top,
            line_cross_size: 0.0,
            is_positioned: item.is_positioned,
            z_index: 0,
        });
        x += item.width + item.margin_right;
        max_item_height = max_item_height.max(item.margin_top + item.height + item.margin_bottom);
    }
    // Flush last row
    if !current_cells.is_empty() {
        rows.push((current_cells, finish_row_height(max_item_height)));
    }

    for (cells, rh) in rows {
        output.push(LayoutElement::FlexRow {
            cells,
            row_height: rh,
            margin_top: 0.0,
            margin_bottom: 0.0,
            offset_left: 0.0,
            background_color: None,
            container_width: available_width,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            border_radius: 0.0,
            box_shadow: Vec::new(),
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            background_clip: BackgroundClip::Border,
            // Inline-blocks are NOT flex items: they keep their own height and
            // align to the line's text baseline (`vertical-align: baseline`),
            // never stretching to fill the line box. The painter's `Baseline`
            // path preserves each cell's natural height — shifting content boxes
            // so their baseline meets the line baseline, and top-anchoring boxes
            // with no text baseline (e.g. empty chips) at cross-start. Using
            // `Stretch` here would wrongly inflate a chip's painted box to the
            // full line height once the line reserves the text strut's descent.
            align_items: crate::style::computed::AlignItems::Baseline,
            positioned_depth: 0,
        });
    }
}
