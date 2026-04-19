use crate::parser::dom::{ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize, BoxSizing,
    ComputedStyle, ContentItem, Display, FontStyle, FontWeight, LinearGradient, ListStyleType,
    Position, RadialGradient, Visibility,
};
use std::collections::HashMap;

use super::context::ContainingBlock;
use super::engine::{CounterState, LayoutBorder, LayoutElement, TextRun};
use super::images::build_raster_background_tree;
use super::text::{
    TextWrapOptions, estimate_word_width, push_text_run_with_fallback, resolve_style_font_family,
    resolved_line_height_factor, wrap_text_runs,
};

// ---------------------------------------------------------------------------
// Group 4 — Box sizing
// ---------------------------------------------------------------------------

pub(crate) fn resolve_padding_box_height(
    content_height: f32,
    specified_height: Option<f32>,
    padding_top: f32,
    padding_bottom: f32,
    border_vertical: f32,
    box_sizing: BoxSizing,
) -> f32 {
    let content_based_height = padding_top + content_height + padding_bottom;
    match specified_height {
        Some(height) => {
            // When height is explicitly set, use it (don't expand to fit content).
            // This is essential for overflow: hidden to clip correctly.
            match box_sizing {
                BoxSizing::BorderBox => (height - border_vertical).max(0.0),
                BoxSizing::ContentBox => height + padding_top + padding_bottom,
            }
        }
        None => content_based_height,
    }
}

/// Strip the first child's top margin and the last child's bottom margin when
/// they would otherwise collapse through the parent (no top/bottom padding or
/// border). Returns the adjusted children-area height.
///
/// This mirrors CSS margin-collapsing and is used to compute the containing
/// block height for absolute-positioned descendants (e.g. ::before/::after
/// bars): their `height: 100%` should match the parent's content-box
/// excluding collapsed outer margins, not the padded wrapper height.
pub(crate) fn collapse_outer_child_margins(
    children: &[LayoutElement],
    children_height: f32,
    padding_top: f32,
    padding_bottom: f32,
    border_top: f32,
    border_bottom: f32,
) -> f32 {
    let strip_top = padding_top == 0.0 && border_top == 0.0;
    let strip_bottom = padding_bottom == 0.0 && border_bottom == 0.0;
    let first_mt = if strip_top {
        children.first().map_or(0.0, outer_margin_top)
    } else {
        0.0
    };
    let last_mb = if strip_bottom {
        children.last().map_or(0.0, outer_margin_bottom)
    } else {
        0.0
    };
    (children_height - first_mt - last_mb).max(0.0)
}

pub(crate) fn outer_margin_top(el: &LayoutElement) -> f32 {
    match el {
        LayoutElement::TextBlock { margin_top, .. }
        | LayoutElement::Container { margin_top, .. }
        | LayoutElement::FlexRow { margin_top, .. }
        | LayoutElement::GridRow { margin_top, .. }
        | LayoutElement::TableRow { margin_top, .. }
        | LayoutElement::Image { margin_top, .. }
        | LayoutElement::Svg { margin_top, .. }
        | LayoutElement::MathBlock { margin_top, .. } => *margin_top,
        _ => 0.0,
    }
}

pub(crate) fn outer_margin_bottom(el: &LayoutElement) -> f32 {
    match el {
        LayoutElement::TextBlock { margin_bottom, .. }
        | LayoutElement::Container { margin_bottom, .. }
        | LayoutElement::FlexRow { margin_bottom, .. }
        | LayoutElement::GridRow { margin_bottom, .. }
        | LayoutElement::TableRow { margin_bottom, .. }
        | LayoutElement::Image { margin_bottom, .. }
        | LayoutElement::Svg { margin_bottom, .. }
        | LayoutElement::MathBlock { margin_bottom, .. } => *margin_bottom,
        _ => 0.0,
    }
}

/// True for flow-participating block children. Absolute/fixed/float elements
/// don't participate in margin collapsing.
fn is_in_flow_block(el: &LayoutElement) -> bool {
    match el {
        LayoutElement::TextBlock {
            position, float, ..
        }
        | LayoutElement::Container {
            position, float, ..
        } => *position != Position::Absolute && *float == crate::style::computed::Float::None,
        LayoutElement::FlexRow { .. }
        | LayoutElement::GridRow { .. }
        | LayoutElement::TableRow { .. }
        | LayoutElement::Image { .. }
        | LayoutElement::Svg { .. }
        | LayoutElement::MathBlock { .. } => true,
        _ => false,
    }
}

/// Return the index of the first/last in-flow child that participates in
/// margin collapsing. Skips absolute/fixed/float children.
pub(crate) fn first_in_flow_idx(children: &[LayoutElement]) -> Option<usize> {
    children.iter().position(is_in_flow_block)
}

pub(crate) fn last_in_flow_idx(children: &[LayoutElement]) -> Option<usize> {
    children.iter().rposition(is_in_flow_block)
}

/// Take the element's margin-top (and clear it), skipping elements that
/// don't participate in margin collapsing.
pub(crate) fn take_margin_top(el: &mut LayoutElement) -> f32 {
    match el {
        LayoutElement::TextBlock { margin_top, .. }
        | LayoutElement::Container { margin_top, .. }
        | LayoutElement::FlexRow { margin_top, .. }
        | LayoutElement::GridRow { margin_top, .. }
        | LayoutElement::TableRow { margin_top, .. }
        | LayoutElement::Image { margin_top, .. }
        | LayoutElement::Svg { margin_top, .. }
        | LayoutElement::MathBlock { margin_top, .. } => {
            let m = *margin_top;
            *margin_top = 0.0;
            m
        }
        _ => 0.0,
    }
}

pub(crate) fn take_margin_bottom(el: &mut LayoutElement) -> f32 {
    match el {
        LayoutElement::TextBlock { margin_bottom, .. }
        | LayoutElement::Container { margin_bottom, .. }
        | LayoutElement::FlexRow { margin_bottom, .. }
        | LayoutElement::GridRow { margin_bottom, .. }
        | LayoutElement::TableRow { margin_bottom, .. }
        | LayoutElement::Image { margin_bottom, .. }
        | LayoutElement::Svg { margin_bottom, .. }
        | LayoutElement::MathBlock { margin_bottom, .. } => {
            let m = *margin_bottom;
            *margin_bottom = 0.0;
            m
        }
        _ => 0.0,
    }
}

/// Collapse the first in-flow child's margin-top into `container_margin_top`,
/// and the last in-flow child's margin-bottom into `container_margin_bottom`,
/// whenever there is no top/bottom padding or border to block the collapse.
///
/// This mirrors CSS 2.1 § 8.3.1: the top margin of a block box collapses with
/// the margin of its first in-flow child if the box has no border/padding/line
/// boxes above it, and symmetrically for the bottom margin.
///
/// The child's margin is zeroed so that flow layout (pagination and
/// `render_container_children`) doesn't double-count it.
pub(crate) fn collapse_margins_through_parent(
    children: &mut [LayoutElement],
    container_margin_top: &mut f32,
    container_margin_bottom: &mut f32,
    padding_top: f32,
    padding_bottom: f32,
    border_top: f32,
    border_bottom: f32,
) {
    if padding_top == 0.0
        && border_top == 0.0
        && let Some(i) = first_in_flow_idx(children)
    {
        let child_mt = take_margin_top(&mut children[i]);
        if child_mt > *container_margin_top {
            *container_margin_top = child_mt;
        }
    }
    if padding_bottom == 0.0
        && border_bottom == 0.0
        && let Some(i) = last_in_flow_idx(children)
    {
        let child_mb = take_margin_bottom(&mut children[i]);
        if child_mb > *container_margin_bottom {
            *container_margin_bottom = child_mb;
        }
    }
}

// ---------------------------------------------------------------------------
// Group 6 — Element classification
// ---------------------------------------------------------------------------

pub(crate) fn recurses_as_layout_child(tag: HtmlTag) -> bool {
    tag.is_block() || tag == HtmlTag::Svg
}

pub(crate) fn collects_as_inline_text(tag: HtmlTag) -> bool {
    tag != HtmlTag::Svg && tag.is_inline()
}

// ---------------------------------------------------------------------------
// Group 3 — List marker formatting
// ---------------------------------------------------------------------------

pub(crate) fn format_list_marker(list_style_type: ListStyleType, index: usize) -> String {
    match list_style_type {
        ListStyleType::Disc => "\u{2022} ".to_string(),
        ListStyleType::Circle => "\u{25E6} ".to_string(),
        ListStyleType::Square => "\u{25AA} ".to_string(),
        ListStyleType::Decimal => format!("{}. ", index),
        ListStyleType::DecimalLeadingZero => format!("{:02}. ", index),
        ListStyleType::LowerAlpha => format!("{}. ", to_alpha_lower(index)),
        ListStyleType::UpperAlpha => format!("{}. ", to_alpha_upper(index)),
        ListStyleType::LowerRoman => format!("{}. ", to_roman_lower(index)),
        ListStyleType::UpperRoman => format!("{}. ", to_roman_upper(index)),
        ListStyleType::None => String::new(),
    }
}
pub(crate) fn to_alpha_lower(n: usize) -> String {
    if n == 0 {
        return "a".to_string();
    }
    let mut result = String::new();
    let mut val = n;
    while val > 0 {
        val -= 1;
        result.insert(0, (b'a' + (val % 26) as u8) as char);
        val /= 26;
    }
    result
}
pub(crate) fn to_alpha_upper(n: usize) -> String {
    to_alpha_lower(n).to_uppercase()
}
pub(crate) fn to_roman_lower(n: usize) -> String {
    let vals = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut result = String::new();
    let mut remaining = n;
    for &(value, numeral) in &vals {
        while remaining >= value {
            result.push_str(numeral);
            remaining -= value;
        }
    }
    if result.is_empty() {
        "0".to_string()
    } else {
        result
    }
}
pub(crate) fn to_roman_upper(n: usize) -> String {
    to_roman_lower(n).to_uppercase()
}

// ---------------------------------------------------------------------------
// Group 1 — Pseudo-element helpers
// ---------------------------------------------------------------------------

pub(crate) fn resolve_content(
    items: &[ContentItem],
    attributes: &HashMap<String, String>,
    counter_state: &CounterState,
) -> String {
    let mut result = String::new();
    for item in items {
        match item {
            ContentItem::String(s) => result.push_str(s),
            ContentItem::Attr(name) => {
                if let Some(val) = attributes.get(name) {
                    result.push_str(val);
                }
            }
            ContentItem::Counter(name) => {
                result.push_str(&counter_state.get(name).to_string());
            }
            ContentItem::Counters(name, sep) => {
                result.push_str(&counter_state.get_all(name, sep));
            }
        }
    }
    result
}

pub(crate) fn measure_runs_width(runs: &[TextRun], fonts: &HashMap<String, TtfFont>) -> f32 {
    runs.iter()
        .map(|run| {
            estimate_word_width(
                &run.text,
                run.font_size,
                &run.font_family,
                run.bold,
                run.italic,
                fonts,
            )
        })
        .sum()
}

pub(crate) fn pseudo_is_block_like(pseudo_style: &ComputedStyle) -> bool {
    pseudo_style.display == Display::Block || pseudo_style.position == Position::Absolute
}

pub(crate) fn append_pseudo_inline_run(
    runs: &mut Vec<TextRun>,
    pseudo_style: Option<&ComputedStyle>,
    el: &ElementNode,
    fonts: &HashMap<String, TtfFont>,
    counter_state: &CounterState,
) {
    if let Some(pseudo_style) = pseudo_style {
        if !pseudo_is_block_like(pseudo_style) {
            runs.push(build_pseudo_inline_run(
                pseudo_style,
                el,
                fonts,
                counter_state,
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn push_block_pseudo(
    output: &mut Vec<LayoutElement>,
    pseudo_style: Option<&ComputedStyle>,
    el: &ElementNode,
    available_width: f32,
    fonts: &HashMap<String, TtfFont>,
    containing_block_info: Option<ContainingBlock>,
    positioned_ancestor_depth: usize,
    counter_state: &CounterState,
) {
    if let Some(pseudo_style) = pseudo_style {
        if pseudo_is_block_like(pseudo_style) {
            let pseudo_cb = if pseudo_style.position == Position::Absolute {
                containing_block_info
            } else {
                None
            };
            output.push(build_pseudo_block(
                pseudo_style,
                el,
                available_width,
                fonts,
                pseudo_cb,
                positioned_ancestor_depth,
                counter_state,
            ));
        }
    }
}

/// Build a `LayoutElement::TextBlock` for a `::before` or `::after` pseudo-element
/// that uses `display: block` (or `position: absolute`).
pub(crate) fn build_pseudo_block(
    pseudo_style: &ComputedStyle,
    el: &ElementNode,
    available_width: f32,
    fonts: &HashMap<String, TtfFont>,
    containing_block_info: Option<ContainingBlock>,
    positioned_ancestor_depth: usize,
    counter_state: &CounterState,
) -> LayoutElement {
    let content_text = resolve_content(&pseudo_style.content, &el.attributes, counter_state);
    let mut block_w = available_width;
    if let Some(cb) = containing_block_info
        && let Some(percent) = pseudo_style.percentage_sizing.width
    {
        block_w = cb.width * percent / 100.0;
    }
    if let Some(w) = pseudo_style.width {
        block_w = w.min(available_width);
    }
    if let Some(cb) = containing_block_info {
        if let Some(percent) = pseudo_style.percentage_sizing.min_width {
            block_w = block_w.max(cb.width * percent / 100.0);
        }
        if let Some(percent) = pseudo_style.percentage_sizing.max_width {
            block_w = block_w.min(cb.width * percent / 100.0);
        }
    }

    let inner_w = if pseudo_style.box_sizing == BoxSizing::BorderBox {
        block_w
            - pseudo_style.padding.left
            - pseudo_style.padding.right
            - pseudo_style.border.horizontal_width()
    } else {
        block_w - pseudo_style.padding.left - pseudo_style.padding.right
    }
    .max(0.0);

    let mut lines = Vec::new();
    let mut runs = Vec::new();
    if !content_text.is_empty() {
        push_text_run_with_fallback(
            TextRun {
                text: content_text,
                font_size: pseudo_style.font_size,
                bold: pseudo_style.font_weight == FontWeight::Bold,
                italic: pseudo_style.font_style == FontStyle::Italic,
                underline: pseudo_style.text_decoration_underline,
                line_through: pseudo_style.text_decoration_line_through,
                overline: pseudo_style.text_decoration_overline,
                color: pseudo_style.color.to_f32_rgb(),
                link_url: None,
                font_family: resolve_style_font_family(pseudo_style, fonts),
                background_color: None,
                padding: (0.0, 0.0),
                border_radius: 0.0,
            },
            &mut runs,
            fonts,
        );
        lines = wrap_text_runs(
            runs.clone(),
            TextWrapOptions::new(
                inner_w,
                pseudo_style.font_size,
                resolved_line_height_factor(pseudo_style, fonts),
                pseudo_style.overflow_wrap,
            )
            .with_rtl(pseudo_style.direction_rtl),
            fonts,
        );
    }

    if pseudo_style.position == Position::Absolute
        && pseudo_style.width.is_none()
        && pseudo_style.min_width.is_none()
    {
        let content_w = measure_runs_width(&runs, fonts);
        block_w = if pseudo_style.box_sizing == BoxSizing::BorderBox {
            content_w
                + pseudo_style.padding.left
                + pseudo_style.padding.right
                + pseudo_style.border.horizontal_width()
        } else {
            content_w + pseudo_style.padding.left + pseudo_style.padding.right
        };
    }

    let bg = pseudo_style.background_color.map(|c| c.to_f32_rgba());
    let border = LayoutBorder::from_computed(&pseudo_style.border);
    let BackgroundFields {
        gradient: background_gradient,
        radial_gradient: background_radial_gradient,
        svg: background_svg,
        blur_radius: background_blur_radius,
        size: background_size,
        position: background_position,
        repeat: background_repeat,
        origin: background_origin,
    } = BackgroundFields::from_style(pseudo_style);

    let explicit_width = if pseudo_style.position == Position::Absolute
        || pseudo_style.width.is_some()
        || pseudo_style.min_width.is_some()
    {
        Some(block_w)
    } else {
        None
    };

    let effective_height = {
        let mut h = pseudo_style.height;
        if let Some(cb) = containing_block_info
            && let Some(percent) = pseudo_style.percentage_sizing.height
        {
            h = Some(cb.height * percent / 100.0);
        }
        if let Some(min_h) = pseudo_style.min_height {
            h = Some(h.map_or(min_h, |v| v.max(min_h)));
        }
        if let Some(cb) = containing_block_info
            && let Some(percent) = pseudo_style.percentage_sizing.min_height
        {
            let min_h = cb.height * percent / 100.0;
            h = Some(h.map_or(min_h, |v| v.max(min_h)));
        }
        if let Some(max_h) = pseudo_style.max_height {
            h = h.map(|v| v.min(max_h));
        }
        if let Some(cb) = containing_block_info
            && let Some(percent) = pseudo_style.percentage_sizing.max_height
        {
            let max_h = cb.height * percent / 100.0;
            h = h.map_or(Some(max_h), |v| Some(v.min(max_h)));
        }
        h
    };
    let text_height: f32 = lines.iter().map(|l| l.height).sum();
    let padding_box_height = resolve_padding_box_height(
        text_height,
        effective_height,
        pseudo_style.padding.top,
        pseudo_style.padding.bottom,
        border.vertical_width(),
        pseudo_style.box_sizing,
    );

    // Resolve bottom/right into top/left when a containing block is present.
    // This allows pagination and rendering to only deal with top/left offsets.
    let (resolved_top, resolved_left) = if let Some(cb) = containing_block_info {
        let elem_h = padding_box_height;
        let elem_w = explicit_width.unwrap_or(block_w);
        let top_from_percent = pseudo_style
            .percentage_insets
            .top
            .map(|percent| cb.height * percent / 100.0);
        let bottom_from_percent = pseudo_style
            .percentage_insets
            .bottom
            .map(|percent| cb.height * percent / 100.0);
        let left_from_percent = pseudo_style
            .percentage_insets
            .left
            .map(|percent| cb.width * percent / 100.0);
        let right_from_percent = pseudo_style
            .percentage_insets
            .right
            .map(|percent| cb.width * percent / 100.0);

        let top = if let Some(top) = top_from_percent.or(pseudo_style.top) {
            top
        } else if let Some(bottom) = bottom_from_percent.or(pseudo_style.bottom) {
            cb.height - elem_h - bottom
        } else {
            0.0
        };
        let left = if let Some(left) = left_from_percent.or(pseudo_style.left) {
            left
        } else if let Some(right) = right_from_percent.or(pseudo_style.right) {
            cb.width - elem_w - right
        } else {
            0.0
        };
        (top, left)
    } else {
        (
            pseudo_style.top.unwrap_or(0.0),
            pseudo_style.left.unwrap_or(0.0),
        )
    };

    LayoutElement::TextBlock {
        lines,
        margin_top: pseudo_style.margin.top,
        margin_bottom: pseudo_style.margin.bottom,
        text_align: pseudo_style.text_align,
        background_color: bg,
        padding_top: pseudo_style.padding.top,
        padding_bottom: pseudo_style.padding.bottom,
        padding_left: pseudo_style.padding.left,
        padding_right: pseudo_style.padding.right,
        border,
        block_width: explicit_width,
        block_height: effective_height.map(|_| padding_box_height),
        opacity: pseudo_style.opacity,
        float: pseudo_style.float,
        clear: pseudo_style.clear,
        position: pseudo_style.position,
        offset_top: resolved_top,
        offset_left: resolved_left,
        offset_bottom: pseudo_style.bottom.unwrap_or(0.0),
        offset_right: pseudo_style.right.unwrap_or(0.0),
        containing_block: containing_block_info,
        box_shadow: pseudo_style.box_shadow,
        visible: pseudo_style.visibility == Visibility::Visible,
        clip_rect: None,
        transform: pseudo_style.transform,
        border_radius: pseudo_style.border_radius,
        outline_width: pseudo_style.outline_width,
        outline_color: pseudo_style.outline_color.map(|c| c.to_f32_rgb()),
        text_indent: pseudo_style.text_indent,
        letter_spacing: pseudo_style.letter_spacing,
        word_spacing: pseudo_style.word_spacing,
        vertical_align: pseudo_style.vertical_align,
        background_gradient,
        background_radial_gradient,
        background_svg,
        background_blur_radius,
        background_size,
        background_position,
        background_repeat,
        background_origin,
        z_index: pseudo_style.z_index,
        repeat_on_each_page: false,
        positioned_depth: if pseudo_style.position == Position::Relative
            || pseudo_style.position == Position::Absolute
        {
            positioned_ancestor_depth + 1
        } else {
            positioned_ancestor_depth
        },
        heading_level: None,
        clip_children_count: 0,
    }
}

/// Build a `TextRun` for an inline `::before` or `::after` pseudo-element.
pub(crate) fn build_pseudo_inline_run(
    pseudo_style: &ComputedStyle,
    el: &ElementNode,
    fonts: &HashMap<String, TtfFont>,
    counter_state: &CounterState,
) -> TextRun {
    let content_text = resolve_content(&pseudo_style.content, &el.attributes, counter_state);
    TextRun {
        text: content_text,
        font_size: pseudo_style.font_size,
        bold: pseudo_style.font_weight == FontWeight::Bold,
        italic: pseudo_style.font_style == FontStyle::Italic,
        underline: pseudo_style.text_decoration_underline,
        line_through: pseudo_style.text_decoration_line_through,
        overline: pseudo_style.text_decoration_overline,
        color: pseudo_style.color.to_f32_rgb(),
        link_url: None,
        font_family: resolve_style_font_family(pseudo_style, fonts),
        background_color: pseudo_style.background_color.map(|c| c.to_f32_rgba()),
        padding: (0.0, 0.0),
        border_radius: 0.0,
    }
}

// ---------------------------------------------------------------------------
// Group 2 — Background/visual helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct BackgroundFields {
    pub(crate) gradient: Option<LinearGradient>,
    pub(crate) radial_gradient: Option<RadialGradient>,
    pub(crate) svg: Option<crate::parser::svg::SvgTree>,
    pub(crate) blur_radius: f32,
    pub(crate) size: BackgroundSize,
    pub(crate) position: BackgroundPosition,
    pub(crate) repeat: BackgroundRepeat,
    pub(crate) origin: BackgroundOrigin,
}

impl BackgroundFields {
    pub(crate) fn from_style(style: &ComputedStyle) -> Self {
        Self {
            gradient: style.background_gradient.clone(),
            radial_gradient: style.background_radial_gradient.clone(),
            svg: background_svg_for_style(style),
            blur_radius: style.blur_radius,
            size: style.background_size,
            position: style.background_position,
            repeat: style.background_repeat,
            origin: style.background_origin,
        }
    }

    pub(crate) fn none() -> Self {
        Self {
            gradient: None,
            radial_gradient: None,
            svg: None,
            blur_radius: 0.0,
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::Repeat,
            origin: BackgroundOrigin::Padding,
        }
    }
}

pub(crate) fn has_background_paint(style: &ComputedStyle) -> bool {
    style.background_color.is_some()
        || style.background_gradient.is_some()
        || style.background_radial_gradient.is_some()
        || style.background_image.is_some()
        || style.background_svg.is_some()
}

pub(crate) fn background_svg_for_style(
    style: &ComputedStyle,
) -> Option<crate::parser::svg::SvgTree> {
    style.background_svg.clone().or_else(|| {
        style
            .background_image
            .as_deref()
            .and_then(build_raster_background_tree)
    })
}

pub(crate) fn aspect_ratio_height(width: f32, style: &ComputedStyle) -> Option<f32> {
    style
        .aspect_ratio
        .filter(|ratio| *ratio > 0.0)
        .map(|ratio| width / ratio)
        .filter(|height| *height > 0.0)
}

// ---------------------------------------------------------------------------
// Group 6 — Paint order
// ---------------------------------------------------------------------------

pub(crate) fn layout_element_paint_order(element: &LayoutElement) -> (i32, i32) {
    match element {
        LayoutElement::TextBlock {
            repeat_on_each_page: true,
            ..
        } => (i32::MIN, 0),
        LayoutElement::TextBlock { z_index, .. } => (0, *z_index),
        _ => (0, 0),
    }
}

// ---------------------------------------------------------------------------
// Group 6 — Heading level
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Returns the heading level (1-6) for a tag, or None if not a heading.
pub(crate) fn heading_level(tag: HtmlTag) -> Option<u8> {
    match tag {
        HtmlTag::H1 => Some(1),
        HtmlTag::H2 => Some(2),
        HtmlTag::H3 => Some(3),
        HtmlTag::H4 => Some(4),
        HtmlTag::H5 => Some(5),
        HtmlTag::H6 => Some(6),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Group 5 — Positioning helpers
// ---------------------------------------------------------------------------

/// Resolve the containing block for an element that is `position: absolute`.
/// If the element is absolute and `abs_cb` is `Some`, returns `abs_cb` and
/// resolves bottom/right offsets into top/left. Otherwise returns `None`
/// and leaves offsets unchanged.
pub(crate) fn resolve_abs_containing_block(
    style: &ComputedStyle,
    abs_cb: Option<ContainingBlock>,
    elem_height: f32,
    elem_width: f32,
) -> (Option<ContainingBlock>, f32, f32) {
    if style.position != Position::Absolute {
        return (None, style.top.unwrap_or(0.0), style.left.unwrap_or(0.0));
    }
    let cb = match abs_cb {
        Some(cb) => cb,
        None => return (None, style.top.unwrap_or(0.0), style.left.unwrap_or(0.0)),
    };

    let top_from_percent = style.percentage_insets.top.map(|p| cb.height * p / 100.0);
    let bottom_from_percent = style
        .percentage_insets
        .bottom
        .map(|p| cb.height * p / 100.0);
    let left_from_percent = style.percentage_insets.left.map(|p| cb.width * p / 100.0);
    let right_from_percent = style.percentage_insets.right.map(|p| cb.width * p / 100.0);

    let resolved_top = if let Some(top) = top_from_percent.or(style.top) {
        top
    } else if let Some(bottom) = bottom_from_percent.or(style.bottom) {
        cb.height - elem_height - bottom
    } else {
        0.0
    };
    let resolved_left = if let Some(left) = left_from_percent.or(style.left) {
        left
    } else if let Some(right) = right_from_percent.or(style.right) {
        cb.width - elem_width - right
    } else {
        0.0
    };

    (Some(cb), resolved_top, resolved_left)
}

/// Patch absolute-positioned children in a flattened element list with
/// the parent's containing block info. This resolves bottom/right offsets
/// into top/left and sets the `containing_block` field.
pub(crate) fn patch_absolute_children_containing_block(
    elements: &mut [LayoutElement],
    cb: ContainingBlock,
) {
    for element in elements.iter_mut() {
        if let LayoutElement::TextBlock {
            position,
            containing_block,
            offset_top,
            offset_left,
            offset_bottom,
            offset_right,
            block_width,
            block_height,
            lines,
            padding_top,
            padding_bottom,
            padding_left: _,
            padding_right: _,
            border,
            ..
        } = element
        {
            if *position == Position::Absolute && containing_block.is_none() {
                // Compute element dimensions for right/bottom resolution
                let text_h: f32 = lines.iter().map(|l| l.height).sum();
                let elem_h = block_height
                    .unwrap_or(*padding_top + text_h + *padding_bottom + border.vertical_width());
                let elem_w = block_width.unwrap_or_else(|| {
                    // Estimate width from text content for right-offset resolution
                    lines
                        .iter()
                        .map(|l| {
                            l.runs
                                .iter()
                                .map(|r| {
                                    crate::fonts::str_width(
                                        &r.text,
                                        r.font_size,
                                        &r.font_family,
                                        r.bold,
                                    )
                                })
                                .sum::<f32>()
                        })
                        .fold(0.0f32, f32::max)
                });

                // Resolve right -> left
                if *offset_left == 0.0 && *offset_right > 0.0 {
                    *offset_left = cb.width - elem_w - *offset_right;
                }
                // Resolve bottom -> top
                if *offset_top == 0.0 && *offset_bottom > 0.0 {
                    *offset_top = cb.height - elem_h - *offset_bottom;
                }

                *containing_block = Some(cb);
            }
        }
    }
}
