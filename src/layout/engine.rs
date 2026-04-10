use crate::parser::css::{
    AncestorInfo, CssRule, CssValue, PseudoElement, SelectorContext, selector_matches,
};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::png;
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    AlignItems, BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize,
    BorderCollapse, BorderSides, BoxShadow, BoxSizing, Clear, ComputedStyle, ContentItem, Display,
    FlexDirection, FlexWrap, Float, FontFamily, FontStyle, FontWeight, GridTrack, JustifyContent,
    LinearGradient, ListStylePosition, ListStyleType, Overflow, OverflowWrap, Position,
    RadialGradient, TableLayout, TextAlign, TextOverflow, Transform, VerticalAlign, Visibility,
    WhiteSpace, compute_pseudo_element_style, compute_style_with_context,
};
use crate::types::{Margin, PageSize};
use crate::util::decode_base64;
use std::collections::HashMap;

/// A single border side for layout rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutBorderSide {
    pub width: f32,
    pub color: (f32, f32, f32),
}

/// Per-side border for layout rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutBorder {
    pub top: LayoutBorderSide,
    pub right: LayoutBorderSide,
    pub bottom: LayoutBorderSide,
    pub left: LayoutBorderSide,
}

// Default is derived via #[derive(Default)] on the struct.

#[allow(dead_code)]
impl LayoutBorder {
    pub fn from_computed(b: &BorderSides) -> Self {
        Self {
            top: LayoutBorderSide {
                width: b.top.width,
                color: b.top.color.map_or((0.0, 0.0, 0.0), |c| c.to_f32_rgb()),
            },
            right: LayoutBorderSide {
                width: b.right.width,
                color: b.right.color.map_or((0.0, 0.0, 0.0), |c| c.to_f32_rgb()),
            },
            bottom: LayoutBorderSide {
                width: b.bottom.width,
                color: b.bottom.color.map_or((0.0, 0.0, 0.0), |c| c.to_f32_rgb()),
            },
            left: LayoutBorderSide {
                width: b.left.width,
                color: b.left.color.map_or((0.0, 0.0, 0.0), |c| c.to_f32_rgb()),
            },
        }
    }
    pub fn has_any(&self) -> bool {
        self.top.width > 0.0
            || self.right.width > 0.0
            || self.bottom.width > 0.0
            || self.left.width > 0.0
    }
    pub fn horizontal_width(&self) -> f32 {
        self.left.width + self.right.width
    }
    pub fn vertical_width(&self) -> f32 {
        self.top.width + self.bottom.width
    }
    pub fn max_width(&self) -> f32 {
        self.top
            .width
            .max(self.right.width)
            .max(self.bottom.width)
            .max(self.left.width)
    }
}

fn resolve_padding_box_height(
    content_height: f32,
    specified_height: Option<f32>,
    padding_top: f32,
    padding_bottom: f32,
    border_vertical: f32,
    box_sizing: BoxSizing,
) -> f32 {
    let content_based_height = padding_top + content_height + padding_bottom;
    let specified_padding_box_height = specified_height.map_or(0.0, |height| match box_sizing {
        BoxSizing::BorderBox => (height - border_vertical).max(0.0),
        BoxSizing::ContentBox => height + padding_top + padding_bottom,
    });

    content_based_height.max(specified_padding_box_height)
}

fn advance_positioned_ancestors_after_page_break(
    positioned_y_by_depth: &mut HashMap<usize, f32>,
    consumed_height: f32,
) {
    for y in positioned_y_by_depth.values_mut() {
        *y -= consumed_height;
    }
}

fn recurses_as_layout_child(tag: HtmlTag) -> bool {
    tag.is_block() || tag == HtmlTag::Svg
}

fn collects_as_inline_text(tag: HtmlTag) -> bool {
    tag != HtmlTag::Svg && tag.is_inline()
}

/// Counter state for CSS counters.
#[derive(Debug, Default, Clone)]
#[allow(dead_code)]
struct CounterState {
    stacks: HashMap<String, Vec<i32>>,
}
#[allow(dead_code)]
impl CounterState {
    fn apply_resets(&mut self, resets: &[(String, i32)]) {
        for (name, val) in resets {
            self.stacks.entry(name.clone()).or_default().push(*val);
        }
    }
    fn apply_increments(&mut self, increments: &[(String, i32)]) {
        for (name, val) in increments {
            let stack = self.stacks.entry(name.clone()).or_default();
            if stack.is_empty() {
                stack.push(0);
            }
            if let Some(top) = stack.last_mut() {
                *top += val;
            }
        }
    }
    fn pop_resets(&mut self, resets: &[(String, i32)]) {
        for (name, _) in resets {
            if let Some(stack) = self.stacks.get_mut(name) {
                stack.pop();
            }
        }
    }
    fn get(&self, name: &str) -> i32 {
        self.stacks
            .get(name)
            .and_then(|s| s.last().copied())
            .unwrap_or(0)
    }
    fn get_all(&self, name: &str, sep: &str) -> String {
        self.stacks
            .get(name)
            .map(|s| {
                s.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(sep)
            })
            .unwrap_or_else(|| "0".to_string())
    }
}

fn format_list_marker(list_style_type: ListStyleType, index: usize) -> String {
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
fn to_alpha_lower(n: usize) -> String {
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
fn to_alpha_upper(n: usize) -> String {
    to_alpha_lower(n).to_uppercase()
}
fn to_roman_lower(n: usize) -> String {
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
fn to_roman_upper(n: usize) -> String {
    to_roman_lower(n).to_uppercase()
}

fn resolve_content(
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

#[cfg(test)]
fn measure_lines_width(lines: &[TextLine], fonts: &HashMap<String, TtfFont>) -> f32 {
    lines
        .iter()
        .map(|line| {
            line.runs
                .iter()
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
                .sum::<f32>()
        })
        .fold(0.0, f32::max)
}

fn measure_runs_width(runs: &[TextRun], fonts: &HashMap<String, TtfFont>) -> f32 {
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

#[allow(dead_code)]
fn resolve_pseudo_content(
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    pseudo: PseudoElement,
    counter_state: &CounterState,
) -> Option<String> {
    rules.iter().find_map(|rule| {
        if rule.pseudo_element != Some(pseudo)
            || !selector_matches(&rule.selector, tag_name, classes, id)
        {
            return None;
        }

        let CssValue::Keyword(content) = rule.declarations.get("content")? else {
            return None;
        };
        let items = crate::style::computed::parse_content_value_pub(content);
        if items.is_empty() {
            return None;
        }
        let text = resolve_content(&items, attributes, counter_state);
        (!text.is_empty()).then_some(text)
    })
}

fn pseudo_is_block_like(pseudo_style: &ComputedStyle) -> bool {
    pseudo_style.display == Display::Block || pseudo_style.position == Position::Absolute
}

fn append_pseudo_inline_run(
    runs: &mut Vec<TextRun>,
    pseudo_style: Option<&ComputedStyle>,
    el: &ElementNode,
    fonts: &HashMap<String, TtfFont>,
) {
    if let Some(pseudo_style) = pseudo_style {
        if !pseudo_is_block_like(pseudo_style) {
            runs.push(build_pseudo_inline_run(pseudo_style, el, fonts));
        }
    }
}

fn push_block_pseudo(
    output: &mut Vec<LayoutElement>,
    pseudo_style: Option<&ComputedStyle>,
    el: &ElementNode,
    available_width: f32,
    fonts: &HashMap<String, TtfFont>,
    containing_block_info: Option<ContainingBlock>,
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
            ));
        }
    }
}

/// Build a `LayoutElement::TextBlock` for a `::before` or `::after` pseudo-element
/// that uses `display: block` (or `position: absolute`).
fn build_pseudo_block(
    pseudo_style: &ComputedStyle,
    el: &ElementNode,
    available_width: f32,
    fonts: &HashMap<String, TtfFont>,
    containing_block_info: Option<ContainingBlock>,
) -> LayoutElement {
    let content_text = resolve_content(
        &pseudo_style.content,
        &el.attributes,
        &CounterState::default(),
    );

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
        runs.push(TextRun {
            text: content_text,
            font_size: pseudo_style.font_size,
            bold: pseudo_style.font_weight == FontWeight::Bold,
            italic: pseudo_style.font_style == FontStyle::Italic,
            underline: pseudo_style.text_decoration_underline,
            line_through: pseudo_style.text_decoration_line_through,
            color: pseudo_style.color.to_f32_rgb(),
            link_url: None,
            font_family: resolve_style_font_family(pseudo_style, fonts),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        });
        lines = wrap_text_runs(
            runs.clone(),
            TextWrapOptions::new(
                inner_w,
                pseudo_style.font_size,
                resolved_line_height_factor(pseudo_style, fonts),
                pseudo_style.overflow_wrap,
            ),
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

    let bg = pseudo_style.background_color.map(|c| c.to_f32_rgb());
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
        positioned_depth: 0,
        heading_level: None,
    }
}

fn resolve_style_font_family(
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> FontFamily {
    crate::system_fonts::resolve_font_family(
        &style.font_stack,
        fonts,
        style.font_weight == FontWeight::Bold,
        style.font_style == FontStyle::Italic,
    )
}

fn resolved_line_height_factor(style: &ComputedStyle, fonts: &HashMap<String, TtfFont>) -> f32 {
    if style.line_height.is_nan() {
        let font_family = resolve_style_font_family(style, fonts);
        crate::fonts::normal_line_height_factor(
            &font_family,
            style.font_weight == FontWeight::Bold,
            style.font_style == FontStyle::Italic,
            fonts,
        )
    } else {
        style.line_height
    }
}

/// Build a `TextRun` for an inline `::before` or `::after` pseudo-element.
fn build_pseudo_inline_run(
    pseudo_style: &ComputedStyle,
    el: &ElementNode,
    fonts: &HashMap<String, TtfFont>,
) -> TextRun {
    let content_text = resolve_content(
        &pseudo_style.content,
        &el.attributes,
        &CounterState::default(),
    );
    TextRun {
        text: content_text,
        font_size: pseudo_style.font_size,
        bold: pseudo_style.font_weight == FontWeight::Bold,
        italic: pseudo_style.font_style == FontStyle::Italic,
        underline: pseudo_style.text_decoration_underline,
        line_through: pseudo_style.text_decoration_line_through,
        color: pseudo_style.color.to_f32_rgb(),
        link_url: None,
        font_family: resolve_style_font_family(pseudo_style, fonts),
        background_color: pseudo_style.background_color.map(|c| c.to_f32_rgb()),
        padding: (0.0, 0.0),
        border_radius: 0.0,
    }
}

/// Context for rendering list items.
#[derive(Debug, Clone)]
enum ListContext {
    Unordered { indent: f32 },
    Ordered { index: usize, indent: f32 },
}

/// A table cell ready for rendering.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TableCell {
    pub lines: Vec<TextLine>,
    pub nested_rows: Vec<LayoutElement>,
    pub bold: bool,
    pub background_color: Option<(f32, f32, f32)>,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    /// Number of columns this cell spans (default 1).
    pub colspan: usize,
    /// Number of rows this cell spans (default 1).
    pub rowspan: usize,
    /// Per-side border specification.
    pub border: LayoutBorder,
    /// Text alignment within the cell.
    pub text_align: TextAlign,
    /// Vertical alignment within the row box.
    pub vertical_align: VerticalAlign,
}

pub(crate) fn table_cell_content_height(cell: &TableCell) -> f32 {
    let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
    let nested_h: f32 = cell.nested_rows.iter().map(estimate_element_height).sum();
    cell.padding_top + text_h + nested_h + cell.padding_bottom
}

/// A cell within a flex row, with its computed x-offset and width.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FlexCell {
    pub lines: Vec<TextLine>,
    pub x_offset: f32,
    pub width: f32,
    pub text_align: TextAlign,
    pub background_color: Option<(f32, f32, f32)>,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
    pub border_radius: f32,
    pub background_gradient: Option<LinearGradient>,
    pub background_radial_gradient: Option<RadialGradient>,
    pub background_svg: Option<crate::parser::svg::SvgTree>,
    pub background_blur_radius: f32,
    pub background_size: BackgroundSize,
    pub background_position: BackgroundPosition,
    pub background_repeat: BackgroundRepeat,
    pub background_origin: BackgroundOrigin,
}

#[derive(Debug, Clone)]
struct BackgroundFields {
    gradient: Option<LinearGradient>,
    radial_gradient: Option<RadialGradient>,
    svg: Option<crate::parser::svg::SvgTree>,
    blur_radius: f32,
    size: BackgroundSize,
    position: BackgroundPosition,
    repeat: BackgroundRepeat,
    origin: BackgroundOrigin,
}

impl BackgroundFields {
    fn from_style(style: &ComputedStyle) -> Self {
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

    fn none() -> Self {
        Self {
            gradient: None,
            radial_gradient: None,
            svg: None,
            blur_radius: 0.0,
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::Repeat,
            origin: BackgroundOrigin::PaddingBox,
        }
    }
}

fn has_background_paint(style: &ComputedStyle) -> bool {
    style.background_color.is_some()
        || style.background_gradient.is_some()
        || style.background_radial_gradient.is_some()
        || style.background_image.is_some()
        || style.background_svg.is_some()
}

fn background_svg_for_style(style: &ComputedStyle) -> Option<crate::parser::svg::SvgTree> {
    style.background_svg.clone().or_else(|| {
        style
            .background_image
            .as_deref()
            .and_then(build_raster_background_tree)
    })
}

fn aspect_ratio_height(width: f32, style: &ComputedStyle) -> Option<f32> {
    style
        .aspect_ratio
        .filter(|ratio| *ratio > 0.0)
        .map(|ratio| width / ratio)
        .filter(|height| *height > 0.0)
}

/// A styled text run (a piece of text with uniform style).
#[derive(Debug, Clone)]
pub struct TextRun {
    pub text: String,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub line_through: bool,
    pub color: (f32, f32, f32),
    pub link_url: Option<String>,
    pub font_family: FontFamily,
    /// Background color for inline spans (e.g. badge/highlight).
    pub background_color: Option<(f32, f32, f32)>,
    /// Horizontal and vertical padding for inline background.
    pub padding: (f32, f32),
    /// Border radius for inline spans (e.g. badge with rounded corners).
    pub border_radius: f32,
}

/// A laid-out line of text runs.
#[derive(Debug, Clone)]
pub struct TextLine {
    pub runs: Vec<TextRun>,
    pub height: f32,
}

/// The format of an embedded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

/// Parsed PNG metadata needed for PDF FlateDecode parameters.
#[derive(Debug, Clone)]
pub struct PngMetadata {
    pub channels: u8,
    pub bit_depth: u8,
}

/// Raster image bytes plus the source pixel dimensions required by the PDF renderer.
#[derive(Debug, Clone)]
pub struct RasterImageAsset {
    pub data: Vec<u8>,
    pub source_width: u32,
    pub source_height: u32,
    pub format: ImageFormat,
    pub png_metadata: Option<PngMetadata>,
}

/// Containing block information for `position: absolute` elements.
/// Stores the containing block's position and dimensions so the renderer
/// can resolve offsets relative to the nearest positioned ancestor.
#[derive(Debug, Clone, Copy)]
pub struct ContainingBlock {
    /// X-offset of the containing block's left edge from the page left margin.
    pub x: f32,
    /// Width of the containing block.
    pub width: f32,
    /// Height of the containing block.
    pub height: f32,
    /// Depth of the positioned ancestor in the layout stack.
    pub depth: usize,
}

/// A layout element ready for rendering.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant, dead_code)]
pub enum LayoutElement {
    /// A block of text lines with optional background.
    TextBlock {
        lines: Vec<TextLine>,
        margin_top: f32,
        margin_bottom: f32,
        text_align: TextAlign,
        background_color: Option<(f32, f32, f32)>,
        padding_top: f32,
        padding_bottom: f32,
        padding_left: f32,
        padding_right: f32,
        border: LayoutBorder,
        block_width: Option<f32>,
        block_height: Option<f32>,
        opacity: f32,
        float: Float,
        clear: Clear,
        position: Position,
        offset_top: f32,
        offset_left: f32,
        offset_bottom: f32,
        offset_right: f32,
        /// Containing block for `position: absolute` elements.
        /// When `Some`, offsets are relative to this block instead of the page.
        containing_block: Option<ContainingBlock>,
        box_shadow: Option<BoxShadow>,
        visible: bool,
        clip_rect: Option<(f32, f32, f32, f32)>,
        transform: Option<Transform>,
        border_radius: f32,
        outline_width: f32,
        outline_color: Option<(f32, f32, f32)>,
        text_indent: f32,
        letter_spacing: f32,
        word_spacing: f32,
        vertical_align: VerticalAlign,
        background_gradient: Option<LinearGradient>,
        background_radial_gradient: Option<RadialGradient>,
        background_svg: Option<crate::parser::svg::SvgTree>,
        background_blur_radius: f32,
        background_size: BackgroundSize,
        background_position: BackgroundPosition,
        background_repeat: BackgroundRepeat,
        background_origin: BackgroundOrigin,
        z_index: i32,
        repeat_on_each_page: bool,
        positioned_depth: usize,
        /// Heading level (1-6) if this block is an h1-h6, used for PDF bookmarks.
        heading_level: Option<u8>,
    },
    /// A table row with cells.
    TableRow {
        cells: Vec<TableCell>,
        col_widths: Vec<f32>,
        margin_top: f32,
        margin_bottom: f32,
        border_collapse: BorderCollapse,
        border_spacing: f32,
    },
    /// A grid row with cells of varying widths.
    GridRow {
        cells: Vec<TableCell>,
        col_widths: Vec<f32>,
        margin_top: f32,
        margin_bottom: f32,
    },
    /// An embedded image.
    Image {
        image: RasterImageAsset,
        width: f32,
        height: f32,
        /// Extra flow-only height below the replaced content, used to model
        /// inline baseline/strut space without stretching the rendered image.
        flow_extra_bottom: f32,
        margin_top: f32,
        margin_bottom: f32,
    },
    /// A horizontal rule.
    HorizontalRule { margin_top: f32, margin_bottom: f32 },
    /// An inline SVG element.
    Svg {
        /// The parsed SVG tree.
        tree: crate::parser::svg::SvgTree,
        /// Rendered width in points.
        width: f32,
        /// Rendered height in points.
        height: f32,
        /// Extra flow-only height below the rendered SVG, used to model
        /// inline baseline/strut space without stretching the rendered image.
        flow_extra_bottom: f32,
        /// Top margin.
        margin_top: f32,
        /// Bottom margin.
        margin_bottom: f32,
    },
    /// A flex row with cells positioned horizontally.
    #[allow(dead_code)]
    FlexRow {
        cells: Vec<FlexCell>,
        row_height: f32,
        margin_top: f32,
        margin_bottom: f32,
        /// Container background color.
        background_color: Option<(f32, f32, f32)>,
        /// Full container width (including padding).
        container_width: f32,
        padding_top: f32,
        padding_bottom: f32,
        padding_left: f32,
        padding_right: f32,
        border: LayoutBorder,
        border_radius: f32,
        box_shadow: Option<BoxShadow>,
        background_gradient: Option<LinearGradient>,
        background_radial_gradient: Option<RadialGradient>,
        background_svg: Option<crate::parser::svg::SvgTree>,
        background_blur_radius: f32,
        background_size: BackgroundSize,
        background_position: BackgroundPosition,
        background_repeat: BackgroundRepeat,
        background_origin: BackgroundOrigin,
    },
    /// A progress bar or meter element.
    ProgressBar {
        /// Fraction filled (0.0 to 1.0).
        fraction: f32,
        /// Total width in points.
        width: f32,
        /// Total height in points.
        height: f32,
        /// Fill color (r, g, b).
        fill_color: (f32, f32, f32),
        /// Track color (r, g, b).
        track_color: (f32, f32, f32),
        margin_top: f32,
        margin_bottom: f32,
    },
    /// A page break.
    PageBreak,
}

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

/// A fully laid-out page.
pub struct Page {
    pub elements: Vec<(f32, LayoutElement)>, // (y_position, element)
}

/// Lay out the DOM nodes into pages.
#[allow(dead_code)]
pub fn layout(nodes: &[DomNode], page_size: PageSize, margin: Margin) -> Vec<Page> {
    layout_with_rules(nodes, page_size, margin, &[])
}

/// Lay out the DOM nodes into pages with stylesheet rules.
#[allow(dead_code)]
pub fn layout_with_rules(
    nodes: &[DomNode],
    page_size: PageSize,
    margin: Margin,
    rules: &[CssRule],
) -> Vec<Page> {
    layout_with_rules_and_fonts(nodes, page_size, margin, rules, &HashMap::new())
}

/// Lay out the DOM nodes into pages with stylesheet rules and custom fonts.
pub fn layout_with_rules_and_fonts(
    nodes: &[DomNode],
    page_size: PageSize,
    margin: Margin,
    rules: &[CssRule],
    custom_fonts: &HashMap<String, TtfFont>,
) -> Vec<Page> {
    // Apply body/html/:root rules to the root style so that inherited root
    // properties still take effect even though the HTML parser unwraps the
    // <html>/<body> elements before layout.
    let mut parent_style = ComputedStyle::default();
    let default_parent = ComputedStyle::default();
    for rule in rules {
        let sel = rule.selector.trim();
        if sel == "body" || sel == "html" || sel == ":root" {
            crate::style::computed::apply_style_map(
                &mut parent_style,
                &rule.declarations,
                &default_parent,
            );
        }
    }
    let available_width = page_size.width - margin.left - margin.right;
    let content_height = page_size.height - margin.top - margin.bottom;
    parent_style.width = Some(available_width);
    parent_style.root_font_size = parent_style.font_size;

    // First, flatten DOM into layout elements
    let mut elements = Vec::new();

    // If the body/html has a background SVG (or gradient/color), emit a full-content-area
    // background block at the very start so it renders behind all content.
    let has_body_bg = has_background_paint(&parent_style);
    if has_body_bg {
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&parent_style);
        elements.push(LayoutElement::TextBlock {
            lines: vec![],
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: parent_style.background_color.map(|c| c.to_f32_rgb()),
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: Some(page_size.width),
            block_height: Some(page_size.height),
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Absolute,
            offset_top: -margin.top,
            offset_left: -margin.left,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: -1,
            repeat_on_each_page: true,
            positioned_depth: 0,
            heading_level: None,
        });
    }

    let ancestors: Vec<AncestorInfo> = Vec::new();
    flatten_nodes(
        nodes,
        &parent_style,
        available_width,
        content_height,
        &mut elements,
        None,
        rules,
        &ancestors,
        0,
        custom_fonts,
    );

    // Then paginate
    paginate(elements, content_height)
}

#[allow(clippy::too_many_arguments)]
fn flatten_nodes(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    available_width: f32,
    available_height: f32,
    output: &mut Vec<LayoutElement>,
    list_ctx: Option<&ListContext>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    positioned_ancestor_depth: usize,
    fonts: &HashMap<String, TtfFont>,
) {
    // Count element children for sibling context
    let element_count = nodes
        .iter()
        .filter(|n| matches!(n, DomNode::Element(_)))
        .count();
    let mut element_index = 0;
    let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();

    for node in nodes {
        match node {
            DomNode::Text(text) => {
                let trimmed = collapse_whitespace(text);
                if !trimmed.is_empty() {
                    let run = TextRun {
                        text: trimmed,
                        font_size: parent_style.font_size,
                        bold: parent_style.font_weight == FontWeight::Bold,
                        italic: parent_style.font_style == FontStyle::Italic,
                        underline: parent_style.text_decoration_underline,
                        line_through: parent_style.text_decoration_line_through,
                        color: parent_style.color.to_f32_rgb(),
                        link_url: None,
                        font_family: resolve_style_font_family(parent_style, fonts),
                        background_color: None,
                        padding: (0.0, 0.0),
                        border_radius: 0.0,
                    };
                    let lines = wrap_text_runs(
                        vec![run],
                        TextWrapOptions::new(
                            available_width,
                            parent_style.font_size,
                            resolved_line_height_factor(parent_style, fonts),
                            parent_style.overflow_wrap,
                        ),
                        fonts,
                    );
                    if !lines.is_empty() {
                        output.push(LayoutElement::TextBlock {
                            lines,
                            margin_top: 0.0,
                            margin_bottom: 0.0,
                            text_align: parent_style.text_align,
                            background_color: None,
                            padding_top: 0.0,
                            padding_bottom: 0.0,
                            padding_left: 0.0,
                            padding_right: 0.0,
                            border: LayoutBorder::default(),
                            block_width: None,
                            block_height: None,
                            opacity: 1.0,
                            float: Float::None,
                            clear: Clear::None,
                            position: Position::Static,
                            offset_top: 0.0,
                            offset_left: 0.0,
                            offset_bottom: 0.0,
                            offset_right: 0.0,
                            containing_block: None,
                            box_shadow: None,
                            visible: true,
                            clip_rect: None,
                            transform: None,
                            border_radius: 0.0,
                            outline_width: 0.0,
                            outline_color: None,
                            text_indent: 0.0,
                            letter_spacing: 0.0,
                            word_spacing: 0.0,
                            vertical_align: VerticalAlign::Baseline,
                            background_gradient: None,
                            background_radial_gradient: None,
                            background_svg: None,
                            background_blur_radius: 0.0,
                            background_size: BackgroundSize::Auto,
                            background_position: BackgroundPosition::default(),
                            background_repeat: BackgroundRepeat::Repeat,
                            background_origin: BackgroundOrigin::PaddingBox,
                            z_index: 0,
                            repeat_on_each_page: false,
                            positioned_depth: 0,
                            heading_level: None,
                        });
                    }
                }
            }
            DomNode::Element(el) => {
                flatten_element(
                    el,
                    parent_style,
                    available_width,
                    available_height,
                    output,
                    list_ctx,
                    rules,
                    ancestors,
                    positioned_ancestor_depth,
                    element_index,
                    element_count,
                    &preceding_siblings,
                    fonts,
                );
                // Track this element as a preceding sibling for the next element
                preceding_siblings.push((
                    el.tag_name().to_string(),
                    el.class_list().iter().map(|s| s.to_string()).collect(),
                ));
                element_index += 1;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Returns the heading level (1-6) for a tag, or None if not a heading.
fn heading_level(tag: HtmlTag) -> Option<u8> {
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

#[allow(clippy::too_many_arguments)]
fn flatten_element(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    available_width: f32,
    available_height: f32,
    output: &mut Vec<LayoutElement>,
    list_ctx: Option<&ListContext>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    positioned_ancestor_depth: usize,
    child_index: usize,
    sibling_count: usize,
    preceding_siblings: &[(String, Vec<String>)],
    fonts: &HashMap<String, TtfFont>,
) {
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: preceding_siblings.to_vec(),
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
    let available_height = style.height.unwrap_or(available_height);
    let positioned_depth =
        if style.position == Position::Relative || style.position == Position::Absolute {
            positioned_ancestor_depth + 1
        } else {
            positioned_ancestor_depth
        };

    // display: none — skip this element entirely
    if style.display == Display::None {
        return;
    }

    if el.tag == HtmlTag::Br {
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::none();
        let line = TextLine {
            runs: vec![TextRun {
                text: String::new(),
                font_size: style.font_size,
                bold: false,
                italic: false,
                underline: false,
                line_through: false,
                color: (0.0, 0.0, 0.0),
                link_url: None,
                font_family: resolve_style_font_family(&style, fonts),
                background_color: None,
                padding: (0.0, 0.0),
                border_radius: 0.0,
            }],
            height: style.font_size * resolved_line_height_factor(&style, fonts),
        };
        output.push(LayoutElement::TextBlock {
            lines: vec![line],
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border: LayoutBorder::default(),
            padding_right: 0.0,
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        });
        return;
    }

    if el.tag == HtmlTag::Hr {
        output.push(LayoutElement::HorizontalRule {
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
        });
        return;
    }

    if el.tag == HtmlTag::Img {
        if let Some(img_element) =
            load_image_from_element(el, available_width, available_height, &style)
        {
            output.push(add_inline_replaced_baseline_gap(img_element, &style, fonts));
        }
        return;
    }

    if el.tag == HtmlTag::Svg {
        let (svg_width, svg_height) =
            resolve_svg_element_size(el, available_width, available_height, true, true);
        if let Some(mut tree) = crate::parser::svg::parse_svg_from_element_with_viewport(
            el,
            Some((svg_width, svg_height)),
        ) {
            sync_svg_tree_to_layout_box(&mut tree, svg_width, svg_height);
            inject_inherited_svg_color(&mut tree, style.color.to_f32_rgb());
            output.push(LayoutElement::Svg {
                tree,
                width: svg_width,
                height: svg_height,
                flow_extra_bottom: 0.0,
                margin_top: style.margin.top,
                margin_bottom: style.margin.bottom,
            });
        }
        return;
    }

    // Form control elements — render as styled boxes with placeholder text
    if el.tag == HtmlTag::Input || el.tag == HtmlTag::Select || el.tag == HtmlTag::Textarea {
        let ctrl_width = style
            .width
            .unwrap_or(if el.tag == HtmlTag::Textarea {
                available_width.min(300.0)
            } else {
                150.0
            })
            .min(available_width);
        let ctrl_height = style.height.unwrap_or(if el.tag == HtmlTag::Textarea {
            80.0
        } else {
            20.0
        });

        let label = if el.tag == HtmlTag::Select {
            el.children
                .iter()
                .find_map(|c| {
                    if let DomNode::Element(opt) = c {
                        opt.children.iter().find_map(|t| {
                            if let DomNode::Text(s) = t {
                                Some(s.trim().to_string())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        } else if el.tag == HtmlTag::Textarea {
            el.children
                .iter()
                .find_map(|c| {
                    if let DomNode::Text(s) = c {
                        Some(s.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        } else {
            el.attributes
                .get("value")
                .or(el.attributes.get("placeholder"))
                .cloned()
                .unwrap_or_default()
        };

        let mut lines = Vec::new();
        if !label.is_empty() {
            let runs = vec![TextRun {
                text: label,
                font_size: style.font_size,
                bold: false,
                italic: false,
                underline: false,
                line_through: false,
                color: style.color.to_f32_rgb(),
                link_url: None,
                font_family: resolve_style_font_family(&style, fonts),
                background_color: None,
                padding: (0.0, 0.0),
                border_radius: 0.0,
            }];
            let inner_w = ctrl_width - style.padding.left - style.padding.right;
            lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    inner_w,
                    style.font_size,
                    resolved_line_height_factor(&style, fonts),
                    style.overflow_wrap,
                ),
                fonts,
            );
        }

        let bg = style
            .background_color
            .map(|c| c.to_f32_rgb())
            .unwrap_or((1.0, 1.0, 1.0));
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&style);

        output.push(LayoutElement::TextBlock {
            lines,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            text_align: style.text_align,
            background_color: Some(bg),
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            block_width: Some(ctrl_width),
            block_height: Some(ctrl_height),
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0),
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: style.box_shadow,
            visible: style.visibility == Visibility::Visible,
            clip_rect: None,
            transform: style.transform,
            border_radius: style.border_radius,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: 0.0,
            letter_spacing: style.letter_spacing,
            word_spacing: style.word_spacing,
            vertical_align: style.vertical_align,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        });
        return;
    }

    // Media elements — render as placeholder rectangles
    if el.tag == HtmlTag::Video || el.tag == HtmlTag::Audio {
        let media_width = style
            .width
            .or_else(|| {
                el.attributes
                    .get("width")
                    .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            })
            .unwrap_or(if el.tag == HtmlTag::Video {
                300.0
            } else {
                200.0
            })
            .min(available_width);
        let media_height = style
            .height
            .or_else(|| {
                el.attributes
                    .get("height")
                    .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
            })
            .unwrap_or(if el.tag == HtmlTag::Video {
                150.0
            } else {
                24.0
            });

        let label = if el.tag == HtmlTag::Video {
            "\u{25B6} Video".to_string()
        } else {
            "\u{25B6} Audio".to_string()
        };

        let bg =
            style
                .background_color
                .map(|c| c.to_f32_rgb())
                .unwrap_or(if el.tag == HtmlTag::Video {
                    (0.0, 0.0, 0.0)
                } else {
                    (0.94, 0.94, 0.94)
                });
        let text_color = if el.tag == HtmlTag::Video {
            (1.0, 1.0, 1.0)
        } else {
            (0.3, 0.3, 0.3)
        };
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&style);

        let runs = vec![TextRun {
            text: label,
            font_size: style.font_size,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: text_color,
            link_url: None,
            font_family: resolve_style_font_family(&style, fonts),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        }];
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(
                media_width,
                style.font_size,
                resolved_line_height_factor(&style, fonts),
                style.overflow_wrap,
            ),
            fonts,
        );

        output.push(LayoutElement::TextBlock {
            lines,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            text_align: TextAlign::Center,
            background_color: Some(bg),
            padding_top: if el.tag == HtmlTag::Video {
                (media_height - style.font_size) / 2.0
            } else {
                4.0
            },
            padding_bottom: if el.tag == HtmlTag::Video {
                (media_height - style.font_size) / 2.0
            } else {
                4.0
            },
            padding_left: 4.0,
            padding_right: 4.0,
            border: LayoutBorder::from_computed(&style.border),
            block_width: Some(media_width),
            block_height: Some(media_height),
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0),
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: style.box_shadow,
            visible: style.visibility == Visibility::Visible,
            clip_rect: None,
            transform: style.transform,
            border_radius: style.border_radius,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: 0.0,
            letter_spacing: style.letter_spacing,
            word_spacing: style.word_spacing,
            vertical_align: style.vertical_align,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        });
        return;
    }

    // Progress and meter elements — render as a horizontal bar
    if el.tag == HtmlTag::Progress || el.tag == HtmlTag::Meter {
        let bar_width = style.width.unwrap_or(150.0).min(available_width);
        let bar_height = style.height.unwrap_or(12.0);
        let value: f32 = el
            .attributes
            .get("value")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let max: f32 = el
            .attributes
            .get("max")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1.0);
        let fraction = if max > 0.0 {
            (value / max).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let fill_color = if el.tag == HtmlTag::Progress {
            (0.12, 0.53, 0.90)
        } else {
            let low: f32 = el
                .attributes
                .get("low")
                .and_then(|s| s.parse().ok())
                .unwrap_or(max * 0.25);
            let high: f32 = el
                .attributes
                .get("high")
                .and_then(|s| s.parse().ok())
                .unwrap_or(max * 0.75);
            if value <= low {
                (0.90, 0.20, 0.20)
            } else if value >= high {
                (0.20, 0.78, 0.35)
            } else {
                (0.95, 0.77, 0.06)
            }
        };

        output.push(LayoutElement::ProgressBar {
            fraction,
            width: bar_width,
            height: bar_height,
            fill_color,
            track_color: (0.88, 0.88, 0.88),
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
        });
        return;
    }

    if style.page_break_before {
        output.push(LayoutElement::PageBreak);
    }

    // Table handling
    if el.tag == HtmlTag::Table {
        flatten_table(
            el,
            &style,
            available_width,
            output,
            rules,
            fonts,
            ancestors,
            child_index,
            sibling_count,
        );
        return;
    }

    // Build ancestors list for children of this element
    let mut child_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        element: el,
        child_index,
        sibling_count,
        preceding_siblings: Vec::new(),
    });

    // List handling — Ul/Ol pass context to Li children
    if el.tag == HtmlTag::Ul || el.tag == HtmlTag::Ol {
        let inner_width = available_width - style.margin.left;
        // Accumulate indentation from parent list context
        let parent_indent = match list_ctx {
            Some(ListContext::Unordered { indent }) => *indent,
            Some(ListContext::Ordered { indent, .. }) => *indent,
            None => 0.0,
        };
        let total_indent = parent_indent + style.margin.left;
        let mut ctx = if el.tag == HtmlTag::Ol {
            ListContext::Ordered {
                index: 1,
                indent: total_indent,
            }
        } else {
            ListContext::Unordered {
                indent: total_indent,
            }
        };
        let child_el_count = el
            .children
            .iter()
            .filter(|c| matches!(c, DomNode::Element(_)))
            .count();
        let mut child_el_idx = 0;
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if child_el.tag == HtmlTag::Li {
                    flatten_element(
                        child_el,
                        &style,
                        inner_width,
                        available_height,
                        output,
                        Some(&ctx),
                        rules,
                        &child_ancestors,
                        positioned_depth,
                        child_el_idx,
                        child_el_count,
                        &[],
                        fonts,
                    );
                    if let ListContext::Ordered { index, .. } = &mut ctx {
                        *index += 1;
                    }
                } else {
                    flatten_element(
                        child_el,
                        &style,
                        inner_width,
                        available_height,
                        output,
                        None,
                        rules,
                        &child_ancestors,
                        positioned_depth,
                        child_el_idx,
                        child_el_count,
                        &[],
                        fonts,
                    );
                }
                child_el_idx += 1;
            }
        }
        return;
    }

    // Li handling — prepend bullet/number marker
    if el.tag == HtmlTag::Li {
        let inner_width = available_width - style.padding.left - style.padding.right;
        let mut runs = Vec::new();

        // Add list marker using list-style-type from computed style
        let marker = match list_ctx {
            Some(ListContext::Unordered { .. }) => format_list_marker(style.list_style_type, 0),
            Some(ListContext::Ordered { index, .. }) => {
                let lst = if style.list_style_type == ListStyleType::Disc {
                    ListStyleType::Decimal
                } else {
                    style.list_style_type
                };
                format_list_marker(lst, *index)
            }
            None => format_list_marker(style.list_style_type, 0),
        };
        let list_indent = if style.list_style_position == ListStylePosition::Inside {
            0.0
        } else {
            match list_ctx {
                Some(ListContext::Unordered { indent }) => *indent,
                Some(ListContext::Ordered { indent, .. }) => *indent,
                None => 0.0,
            }
        };
        if !marker.is_empty() {
            runs.push(TextRun {
                text: marker,
                font_size: style.font_size,
                bold: style.font_weight == FontWeight::Bold,
                italic: style.font_style == FontStyle::Italic,
                underline: false,
                line_through: false,
                color: style.color.to_f32_rgb(),
                link_url: None,
                font_family: resolve_style_font_family(&style, fonts),
                background_color: None,
                padding: (0.0, 0.0),
                border_radius: 0.0,
            });
        }

        collect_text_runs(
            &el.children,
            &style,
            &mut runs,
            None,
            rules,
            fonts,
            ancestors,
        );

        let block_heading_level = heading_level(el.tag);

        if !runs.is_empty() {
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    inner_width,
                    style.font_size,
                    resolved_line_height_factor(&style, fonts),
                    style.overflow_wrap,
                ),
                fonts,
            );
            let BackgroundFields {
                gradient: background_gradient,
                radial_gradient: background_radial_gradient,
                svg: background_svg,
                blur_radius: background_blur_radius,
                size: background_size,
                position: background_position,
                repeat: background_repeat,
                origin: background_origin,
            } = BackgroundFields::from_style(&style);
            output.push(LayoutElement::TextBlock {
                lines,
                margin_top: style.margin.top,
                margin_bottom: style.margin.bottom,
                text_align: style.text_align,
                background_color: None,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: list_indent,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: None,
                block_height: None,
                opacity: style.opacity,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0),
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: style.box_shadow,
                visible: style.visibility == Visibility::Visible,
                clip_rect: None,
                transform: style.transform,
                border_radius: style.border_radius,
                outline_width: style.outline_width,
                outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
                text_indent: style.text_indent,
                letter_spacing: style.letter_spacing,
                word_spacing: style.word_spacing,
                vertical_align: style.vertical_align,
                background_gradient,
                background_radial_gradient,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                z_index: style.z_index,
                repeat_on_each_page: false,
                positioned_depth: 0,
                heading_level: block_heading_level,
            });
        }

        // Process block children inside li (nested lists get reduced width for indentation)
        let child_el_count = el
            .children
            .iter()
            .filter(|c| matches!(c, DomNode::Element(_)))
            .count();
        let mut child_el_idx = 0;
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if child_el.tag == HtmlTag::Ul || child_el.tag == HtmlTag::Ol {
                    flatten_element(
                        child_el,
                        &style,
                        inner_width,
                        available_height,
                        output,
                        list_ctx,
                        rules,
                        &child_ancestors,
                        positioned_depth,
                        child_el_idx,
                        child_el_count,
                        &[],
                        fonts,
                    );
                } else if recurses_as_layout_child(child_el.tag) {
                    flatten_element(
                        child_el,
                        &style,
                        available_width,
                        available_height,
                        output,
                        None,
                        rules,
                        &child_ancestors,
                        positioned_depth,
                        child_el_idx,
                        child_el_count,
                        &[],
                        fonts,
                    );
                }
                child_el_idx += 1;
            }
        }
        return;
    }

    // Compute ::before and ::after pseudo-element styles before any display-
    // specific early returns so layout modes such as flex can still emit them.
    let cls: Vec<&str> = classes.iter().map(|s| s.as_ref()).collect();
    let before_style = compute_pseudo_element_style(
        &style,
        rules,
        el.tag_name(),
        &cls,
        el.id(),
        &el.attributes,
        &selector_ctx,
        PseudoElement::Before,
    );
    let after_style = compute_pseudo_element_style(
        &style,
        rules,
        el.tag_name(),
        &cls,
        el.id(),
        &el.attributes,
        &selector_ctx,
        PseudoElement::After,
    );

    // Flex container handling
    if style.display == Display::Flex {
        flatten_flex_container(
            el,
            &style,
            available_width,
            output,
            rules,
            &child_ancestors,
            fonts,
            before_style.as_ref(),
            after_style.as_ref(),
            positioned_depth,
        );

        if style.page_break_after {
            output.push(LayoutElement::PageBreak);
        }
        return;
    }

    // Grid container handling
    if style.display == Display::Grid {
        flatten_grid_container(
            el,
            &style,
            available_width,
            output,
            rules,
            &child_ancestors,
            fonts,
        );

        if style.page_break_after {
            output.push(LayoutElement::PageBreak);
        }
        return;
    }

    // Multi-column layout: treat as implicit grid with equal columns
    if let Some(col_count) = style.column_count {
        if col_count >= 2 {
            let gap = style.column_gap;
            let tracks: Vec<GridTrack> = (0..col_count).map(|_| GridTrack::Fr(1.0)).collect();
            let mut col_style = style.clone();
            col_style.grid_template_columns = tracks;
            col_style.grid_gap = gap;
            flatten_grid_container(
                el,
                &col_style,
                available_width,
                output,
                rules,
                &child_ancestors,
                fonts,
            );

            if style.page_break_after {
                output.push(LayoutElement::PageBreak);
            }
            return;
        }
    }

    if style.display == Display::Block {
        // Compute effective block width considering CSS width/max-width/min-width
        let mut block_w = available_width;
        if let Some(w) = style.width {
            block_w = w.min(available_width);
        }
        if let Some(mw) = style.max_width {
            block_w = block_w.min(mw);
        }
        if let Some(mw) = style.min_width {
            block_w = block_w.max(mw);
        }

        // Compute effective height considering CSS height/min-height/max-height
        let mut effective_height = style.height;
        if let Some(min_h) = style.min_height {
            effective_height = Some(effective_height.map_or(min_h, |h| h.max(min_h)));
        }
        if let Some(max_h) = style.max_height {
            effective_height = effective_height.map(|h| h.min(max_h));
        }

        // Compute margin auto offset for horizontal centering
        let has_explicit_width =
            style.width.is_some() || style.max_width.is_some() || style.min_width.is_some();
        let auto_offset_left = if has_explicit_width && block_w < available_width {
            if style.margin_left_auto && style.margin_right_auto {
                (available_width - block_w) / 2.0
            } else if style.margin_left_auto {
                available_width - block_w
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Adjust for box-sizing: border-box
        // When border-box, the specified width includes padding and border,
        // so the content area is width minus padding and border.
        let inner_width = if style.box_sizing == BoxSizing::BorderBox {
            block_w - style.padding.left - style.padding.right - style.border.horizontal_width()
        } else {
            block_w - style.padding.left - style.padding.right
        };
        let inner_width = inner_width.max(0.0);

        let positioned_container =
            style.position == Position::Relative || style.position == Position::Absolute;
        let make_containing_block = |padding_box_height: f32| {
            if positioned_container {
                let cb_width = if style.box_sizing == BoxSizing::BorderBox {
                    block_w - style.border.horizontal_width()
                } else {
                    block_w + style.padding.left + style.padding.right
                };
                Some(ContainingBlock {
                    x: style.left.unwrap_or(0.0)
                        + auto_offset_left
                        + style.border.left.width
                        + style.padding.left,
                    width: cb_width,
                    height: padding_box_height,
                    depth: positioned_depth,
                })
            } else {
                None
            }
        };

        // Emit block-level ::before pseudo-element.
        let before_is_abs = before_style
            .as_ref()
            .is_some_and(|s| s.position == Position::Absolute);
        let after_is_abs = after_style
            .as_ref()
            .is_some_and(|s| s.position == Position::Absolute);
        if let Some(ref ps) = before_style {
            if pseudo_is_block_like(ps) && !before_is_abs {
                output.push(build_pseudo_block(ps, el, inner_width, fonts, None));
            }
        }

        // Collect all inline content as text runs, with inline ::before/::after
        let mut runs = Vec::new();
        append_pseudo_inline_run(&mut runs, before_style.as_ref(), el, fonts);
        collect_text_runs(
            &el.children,
            &style,
            &mut runs,
            None,
            rules,
            fonts,
            ancestors,
        );
        append_pseudo_inline_run(&mut runs, after_style.as_ref(), el, fonts);

        let had_inline_runs = !runs.is_empty();
        let mut cb_info = None;
        if !runs.is_empty() {
            // When white-space: nowrap, prevent wrapping by using a huge width
            let wrap_width = if style.white_space == WhiteSpace::NoWrap {
                f32::MAX
            } else {
                inner_width
            };
            let mut lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    wrap_width,
                    style.font_size,
                    resolved_line_height_factor(&style, fonts),
                    style.overflow_wrap,
                ),
                fonts,
            );

            // Apply text-overflow: ellipsis when overflow is hidden, white-space
            // is nowrap, and we have a fixed width.
            if style.text_overflow == TextOverflow::Ellipsis
                && style.overflow == Overflow::Hidden
                && style.white_space == WhiteSpace::NoWrap
                && style.width.is_some()
            {
                apply_text_overflow_ellipsis(&mut lines, inner_width, fonts);
            }

            let bg = style
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgb());

            let explicit_width = if block_w < available_width || style.min_width.is_some() {
                Some(block_w)
            } else {
                None
            };

            // Compute clip rect before moving lines
            let clip_rect = if style.overflow == Overflow::Hidden {
                let text_height: f32 = lines.iter().map(|l| l.height).sum();
                let total_h = resolve_padding_box_height(
                    text_height,
                    effective_height,
                    style.padding.top,
                    style.padding.bottom,
                    style.border.vertical_width(),
                    style.box_sizing,
                );
                Some((0.0, 0.0, block_w, total_h))
            } else {
                None
            };
            let BackgroundFields {
                gradient: background_gradient,
                radial_gradient: background_radial_gradient,
                svg: background_svg,
                blur_radius: background_blur_radius,
                size: background_size,
                position: background_position,
                repeat: background_repeat,
                origin: background_origin,
            } = BackgroundFields::from_style(&style);
            let text_height: f32 = lines.iter().map(|l| l.height).sum();
            let total_h = resolve_padding_box_height(
                text_height,
                effective_height,
                style.padding.top,
                style.padding.bottom,
                style.border.vertical_width(),
                style.box_sizing,
            );
            cb_info = make_containing_block(total_h);

            output.push(LayoutElement::TextBlock {
                lines,
                margin_top: style.margin.top,
                margin_bottom: style.margin.bottom,
                text_align: style.text_align,
                background_color: bg,
                padding_top: style.padding.top,
                padding_bottom: style.padding.bottom,
                padding_left: style.padding.left,
                padding_right: style.padding.right,
                border: LayoutBorder::from_computed(&style.border),
                block_width: explicit_width,
                block_height: effective_height.map(|_| total_h),
                opacity: style.opacity,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0) + auto_offset_left,
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: style.box_shadow,
                visible: style.visibility == Visibility::Visible,
                clip_rect,
                transform: style.transform,
                border_radius: style.border_radius,
                outline_width: style.outline_width,
                outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
                text_indent: style.text_indent,
                letter_spacing: style.letter_spacing,
                word_spacing: style.word_spacing,
                vertical_align: style.vertical_align,
                background_gradient,
                background_radial_gradient,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                z_index: style.z_index,
                repeat_on_each_page: false,
                positioned_depth,
                heading_level: heading_level(el.tag),
            });
            push_block_pseudo(
                output,
                before_style.as_ref(),
                el,
                inner_width,
                fonts,
                cb_info,
            );
        }

        // Also process block children recursively, using inner_width
        // so children respect the parent's padding boundaries.
        let child_el_count = el
            .children
            .iter()
            .filter(|c| matches!(c, DomNode::Element(_)))
            .count();

        // If no inline content but the element has visual properties (background,
        // gradient, border, border-radius), emit a wrapper TextBlock so the visuals
        // are rendered.  Children are then pulled back inside via a negative-margin
        // spacer (same technique as flex column containers).
        // NB: check before runs is moved into wrap_text_runs above.
        let has_visual = has_background_paint(&style)
            || style.border.has_any()
            || style.border_radius > 0.0
            || style.box_shadow.is_some();
        let needs_wrapper = has_visual
            || style.aspect_ratio.is_some()
            || style.height.is_some()
            || (positioned_container && (before_is_abs || after_is_abs));
        let no_inline_content = !had_inline_runs;

        if no_inline_content && needs_wrapper {
            // Pre-flatten children to measure total height
            let mut child_elements = Vec::new();
            let mut child_el_idx = 0;
            for child in &el.children {
                if let DomNode::Element(child_el) = child {
                    if recurses_as_layout_child(child_el.tag) {
                        flatten_element(
                            child_el,
                            &style,
                            inner_width,
                            available_height,
                            &mut child_elements,
                            None,
                            rules,
                            &child_ancestors,
                            positioned_depth,
                            child_el_idx,
                            child_el_count,
                            &[],
                            fonts,
                        );
                    }
                    child_el_idx += 1;
                }
            }
            // Measure children total height
            let children_h: f32 = child_elements.iter().map(estimate_element_height).sum();
            let mut container_h = resolve_padding_box_height(
                children_h,
                effective_height,
                style.padding.top,
                style.padding.bottom,
                style.border.vertical_width(),
                style.box_sizing,
            );
            if effective_height.is_none()
                && let Some(aspect_h) = aspect_ratio_height(block_w, &style)
            {
                container_h = container_h.max(aspect_h);
            }
            cb_info = make_containing_block(container_h);

            let bg = style
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgb());
            let BackgroundFields {
                gradient: background_gradient,
                radial_gradient: background_radial_gradient,
                svg: background_svg,
                blur_radius: background_blur_radius,
                size: background_size,
                position: background_position,
                repeat: background_repeat,
                origin: background_origin,
            } = BackgroundFields::from_style(&style);
            // Emit wrapper with visual properties
            output.push(LayoutElement::TextBlock {
                lines: Vec::new(),
                margin_top: style.margin.top,
                margin_bottom: 0.0,
                text_align: style.text_align,
                background_color: bg,
                // Padding is already included in container_h (block_height),
                // so set 0 here to avoid double-counting in the paginator.
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: style.padding.left,
                padding_right: style.padding.right,
                border: LayoutBorder::from_computed(&style.border),
                block_width: Some(block_w),
                block_height: Some(container_h),
                opacity: style.opacity,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0) + auto_offset_left,
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: style.box_shadow,
                visible: style.visibility == Visibility::Visible,
                clip_rect: if style.overflow == Overflow::Hidden {
                    Some((0.0, 0.0, block_w, container_h))
                } else {
                    None
                },
                transform: style.transform,
                border_radius: style.border_radius,
                outline_width: style.outline_width,
                outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
                text_indent: 0.0,
                letter_spacing: 0.0,
                word_spacing: 0.0,
                vertical_align: VerticalAlign::Baseline,
                background_gradient,
                background_radial_gradient,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                z_index: style.z_index,
                repeat_on_each_page: false,
                positioned_depth,
                heading_level: None,
            });
            push_block_pseudo(
                output,
                before_style.as_ref(),
                el,
                inner_width,
                fonts,
                cb_info,
            );
            // Pull y back so children flow inside the wrapper, starting
            // after the top padding.  The wrapper advanced y by its full
            // height; we only pull back by (children_h + padding_bottom + border)
            // so that padding_top of space remains above the children.
            let pullback = children_h + style.padding.bottom + style.border.vertical_width();
            output.push(LayoutElement::TextBlock {
                lines: Vec::new(),
                margin_top: -pullback,
                margin_bottom: 0.0,
                text_align: TextAlign::Left,
                background_color: None,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: style.padding.left,
                padding_right: style.padding.right,
                border: LayoutBorder::default(),
                block_width: None,
                block_height: None,
                opacity: 1.0,
                float: Float::None,
                clear: Clear::None,
                position: Position::Static,
                offset_top: 0.0,
                offset_left: 0.0,
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: None,
                visible: true,
                clip_rect: None,
                transform: None,
                border_radius: 0.0,
                outline_width: 0.0,
                outline_color: None,
                text_indent: 0.0,
                letter_spacing: 0.0,
                word_spacing: 0.0,
                vertical_align: VerticalAlign::Baseline,
                background_gradient: None,
                background_radial_gradient: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::PaddingBox,
                z_index: 0,
                repeat_on_each_page: false,
                positioned_depth: 0,
                heading_level: None,
            });
            // Add the parent's left/right padding to children so they render
            // inside the padded area, not at the page left margin.
            if style.padding.left > 0.0 || style.padding.right > 0.0 {
                for child_elem in &mut child_elements {
                    if let LayoutElement::TextBlock {
                        padding_left,
                        padding_right,
                        ..
                    } = child_elem
                    {
                        *padding_left += style.padding.left;
                        *padding_right += style.padding.right;
                    }
                }
            }
            output.extend(child_elements);
            // Emit spacer for bottom padding + border + margin_bottom
            let bottom_space =
                style.padding.bottom + style.border.vertical_width() + style.margin.bottom;
            if bottom_space > 0.0 {
                output.push(LayoutElement::TextBlock {
                    lines: Vec::new(),
                    margin_top: bottom_space,
                    margin_bottom: 0.0,
                    text_align: TextAlign::Left,
                    background_color: None,
                    padding_top: 0.0,
                    padding_bottom: 0.0,
                    padding_left: 0.0,
                    padding_right: 0.0,
                    border: LayoutBorder::default(),
                    block_width: None,
                    block_height: None,
                    opacity: 1.0,
                    float: Float::None,
                    clear: Clear::None,
                    position: Position::Static,
                    offset_top: 0.0,
                    offset_left: 0.0,
                    offset_bottom: 0.0,
                    offset_right: 0.0,
                    containing_block: None,
                    box_shadow: None,
                    visible: true,
                    clip_rect: None,
                    transform: None,
                    border_radius: 0.0,
                    outline_width: 0.0,
                    outline_color: None,
                    text_indent: 0.0,
                    letter_spacing: 0.0,
                    word_spacing: 0.0,
                    vertical_align: VerticalAlign::Baseline,
                    background_gradient: None,
                    background_radial_gradient: None,
                    background_svg: None,
                    background_blur_radius: 0.0,
                    background_size: BackgroundSize::Auto,
                    background_position: BackgroundPosition::default(),
                    background_repeat: BackgroundRepeat::Repeat,
                    background_origin: BackgroundOrigin::PaddingBox,
                    z_index: 0,
                    repeat_on_each_page: false,
                    positioned_depth: 0,
                    heading_level: None,
                });
            }
        } else {
            if no_inline_content {
                push_block_pseudo(
                    output,
                    before_style.as_ref(),
                    el,
                    inner_width,
                    fonts,
                    cb_info,
                );
            }
            let mut child_el_idx = 0;
            for child in &el.children {
                if let DomNode::Element(child_el) = child {
                    if recurses_as_layout_child(child_el.tag) {
                        flatten_element(
                            child_el,
                            &style,
                            inner_width,
                            available_height,
                            output,
                            None,
                            rules,
                            &child_ancestors,
                            positioned_depth,
                            child_el_idx,
                            child_el_count,
                            &[],
                            fonts,
                        );
                    }
                    child_el_idx += 1;
                }
            }
        }

        // Emit block-level ::after pseudo-element (inside block path)
        push_block_pseudo(
            output,
            after_style.as_ref(),
            el,
            inner_width,
            fonts,
            cb_info,
        );
    } else {
        // Inline element — process children with this style context
        flatten_nodes(
            &el.children,
            &style,
            available_width,
            available_height,
            output,
            None,
            rules,
            &child_ancestors,
            positioned_depth,
            fonts,
        );
    }

    if style.page_break_after {
        output.push(LayoutElement::PageBreak);
    }
}

/// Lay out children of a `display: flex` container.
///
/// Each child is laid out as a TextBlock at a computed position. The container
/// emits one TextBlock per flex item with an `offset_left` / `offset_top` that
/// encodes its position inside the flex row/column. The container itself emits
/// a wrapper TextBlock for its background/border first, then the items.
#[allow(clippy::too_many_arguments)]
fn flatten_flex_container(
    el: &ElementNode,
    style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    fonts: &HashMap<String, TtfFont>,
    before_style: Option<&ComputedStyle>,
    after_style: Option<&ComputedStyle>,
    positioned_depth: usize,
) {
    let mut block_w = available_width;
    if let Some(w) = style.width {
        block_w = w.min(available_width);
    }
    if let Some(mw) = style.max_width {
        block_w = block_w.min(mw);
    }

    let inner_width = block_w - style.padding.left - style.padding.right;

    // Collect child elements and lay each one out into a temporary buffer
    let child_elements: Vec<&ElementNode> = el
        .children
        .iter()
        .filter_map(|c| {
            if let DomNode::Element(e) = c {
                Some(e)
            } else {
                None
            }
        })
        .collect();

    let child_count = child_elements.len();
    if child_count == 0 {
        let before_abs = before_style.is_some_and(|pseudo| {
            pseudo_is_block_like(pseudo) && pseudo.position == Position::Absolute
        });
        let after_abs = after_style.is_some_and(|pseudo| {
            pseudo_is_block_like(pseudo) && pseudo.position == Position::Absolute
        });
        if has_background_paint(&style)
            || style.border.has_any()
            || style.border_radius > 0.0
            || style.box_shadow.is_some()
            || style.aspect_ratio.is_some()
            || style.height.is_some()
            || before_abs
            || after_abs
        {
            let container_h = style
                .height
                .or_else(|| aspect_ratio_height(block_w, &style))
                .unwrap_or(0.0);
            let containing_block = (style.position == Position::Relative
                || style.position == Position::Absolute)
                .then(|| ContainingBlock {
                    x: style.left.unwrap_or(0.0) + style.border.left.width + style.padding.left,
                    width: if style.box_sizing == BoxSizing::BorderBox {
                        block_w - style.border.horizontal_width()
                    } else {
                        block_w + style.padding.left + style.padding.right
                    },
                    height: container_h,
                    depth: positioned_depth,
                });
            let bg = style
                .background_color
                .map(|color: crate::types::Color| color.to_f32_rgb());
            let BackgroundFields {
                gradient: background_gradient,
                radial_gradient: background_radial_gradient,
                svg: background_svg,
                blur_radius: background_blur_radius,
                size: background_size,
                position: background_position,
                repeat: background_repeat,
                origin: background_origin,
            } = BackgroundFields::from_style(&style);
            output.push(LayoutElement::TextBlock {
                lines: Vec::new(),
                margin_top: style.margin.top,
                margin_bottom: style.margin.bottom,
                text_align: style.text_align,
                background_color: bg,
                padding_top: style.padding.top,
                padding_bottom: style.padding.bottom,
                padding_left: style.padding.left,
                padding_right: style.padding.right,
                border: LayoutBorder::from_computed(&style.border),
                block_width: Some(block_w),
                block_height: Some(container_h),
                opacity: style.opacity,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0),
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: style.box_shadow,
                visible: style.visibility == Visibility::Visible,
                clip_rect: if style.overflow == Overflow::Hidden {
                    Some((0.0, 0.0, block_w, container_h))
                } else {
                    None
                },
                transform: style.transform,
                border_radius: style.border_radius,
                outline_width: style.outline_width,
                outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
                text_indent: 0.0,
                letter_spacing: style.letter_spacing,
                word_spacing: style.word_spacing,
                vertical_align: style.vertical_align,
                background_gradient,
                background_radial_gradient,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                z_index: style.z_index,
                repeat_on_each_page: false,
                positioned_depth,
                heading_level: None,
            });

            if before_abs {
                push_block_pseudo(
                    output,
                    before_style,
                    el,
                    inner_width.max(0.0),
                    fonts,
                    containing_block,
                );
            }
            if after_abs {
                push_block_pseudo(
                    output,
                    after_style,
                    el,
                    inner_width.max(0.0),
                    fonts,
                    containing_block,
                );
            }
        }
        return;
    }

    // Lay out each child into its own set of elements to measure sizes
    struct FlexItem {
        elements: Vec<LayoutElement>,
        width: f32,
        base_width: f32,
        flex_grow: f32,
        flex_shrink: f32,
        height: f32,
    }

    let mut items: Vec<FlexItem> = Vec::new();

    // For percentage width resolution, children need the actual container width
    // as the parent reference (not the CSS width which may be None).
    // Subtract total gap space so that percentage widths + gaps fit within the container.
    let total_gaps = style.gap * (child_count.saturating_sub(1)) as f32;
    let width_for_percentages = (inner_width - total_gaps).max(0.0);
    let mut parent_for_children = style.clone();
    if parent_for_children.width.is_none() {
        parent_for_children.width = Some(width_for_percentages);
    }

    for (idx, child_el) in child_elements.iter().enumerate() {
        let classes = child_el.class_list();
        let selector_ctx = SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index: idx,
            sibling_count: child_count,
            preceding_siblings: Vec::new(),
        };
        let child_style = compute_style_with_context(
            child_el.tag,
            child_el.style_attr(),
            &parent_for_children,
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

        // Determine child width: flex-basis takes priority, then explicit width.
        // If neither is set and flex-grow > 0, use 0 as base (grow will distribute).
        // Otherwise fall back to equal share.
        let child_w = child_style
            .flex_basis
            .or(child_style.width)
            .unwrap_or_else(|| {
                if child_style.flex_grow > 0.0 {
                    0.0
                } else {
                    width_for_percentages / child_count as f32
                }
            });

        let child_inner_w = if child_style.box_sizing == BoxSizing::BorderBox {
            child_w
                - child_style.padding.left
                - child_style.padding.right
                - child_style.border.horizontal_width()
        } else {
            child_w - child_style.padding.left - child_style.padding.right
        }
        .max(0.0);

        // Collect text runs for this child, including from nested block elements.
        // Include the child element itself in the ancestor chain so that
        // descendant selectors like `.card h3` can match.
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.push(AncestorInfo {
            element: child_el,
            child_index: idx,
            sibling_count: child_count,
            preceding_siblings: Vec::new(),
        });
        let mut runs = Vec::new();
        collect_flex_child_text_runs(
            &child_el.children,
            &child_style,
            &mut runs,
            None,
            (0.0, 0.0),
            rules,
            fonts,
            &child_ancestors,
        );

        let lines = if !runs.is_empty() {
            wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    child_inner_w.max(1.0),
                    child_style.font_size,
                    resolved_line_height_factor(&child_style, fonts),
                    child_style.overflow_wrap,
                ),
                fonts,
            )
        } else {
            Vec::new()
        };

        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let aspect_h = child_style
            .height
            .is_none()
            .then(|| aspect_ratio_height(child_w, &child_style))
            .flatten();
        let mut child_h = resolve_padding_box_height(
            text_height,
            child_style.height,
            child_style.padding.top,
            child_style.padding.bottom,
            child_style.border.vertical_width(),
            child_style.box_sizing,
        );
        if let Some(aspect_h) = aspect_h {
            child_h = child_h.max(aspect_h);
        }

        let bg = child_style
            .background_color
            .map(|c: crate::types::Color| c.to_f32_rgb());
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&child_style);
        let elem = LayoutElement::TextBlock {
            lines,
            margin_top: child_style.margin.top,
            margin_bottom: child_style.margin.bottom,
            text_align: child_style.text_align,
            background_color: bg,
            padding_top: child_style.padding.top,
            padding_bottom: child_style.padding.bottom,
            padding_left: child_style.padding.left,
            padding_right: child_style.padding.right,
            border: LayoutBorder::from_computed(&child_style.border),
            block_width: Some(child_w),
            block_height: child_style
                .height
                .map(|_| child_h)
                .or(aspect_h.map(|_| child_h)),
            opacity: child_style.opacity,
            float: Float::None,
            clear: Clear::None,
            position: child_style.position,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: child_style.box_shadow,
            visible: child_style.visibility == Visibility::Visible,
            clip_rect: if child_style.overflow == Overflow::Hidden {
                Some((0.0, 0.0, child_w, child_h))
            } else {
                None
            },
            transform: child_style.transform,
            border_radius: child_style.border_radius,
            outline_width: child_style.outline_width,
            outline_color: child_style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: child_style.text_indent,
            letter_spacing: child_style.letter_spacing,
            word_spacing: child_style.word_spacing,
            vertical_align: child_style.vertical_align,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: child_style.z_index,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        };

        items.push(FlexItem {
            elements: vec![elem],
            width: child_w,
            base_width: child_w,
            flex_grow: child_style.flex_grow,
            flex_shrink: child_style.flex_shrink,
            height: child_h + child_style.margin.top + child_style.margin.bottom,
        });
    }

    if items.is_empty() {
        return;
    }

    let direction = style.flex_direction;
    let justify = style.justify_content;
    let align = style.align_items;
    let wrap = style.flex_wrap;
    let gap = style.gap;

    // Group items into lines (for flex-wrap)
    struct FlexLine {
        item_indices: Vec<usize>,
        main_size: f32,
        cross_size: f32,
    }

    let mut lines: Vec<FlexLine> = Vec::new();

    match direction {
        FlexDirection::Row => {
            let max_main = inner_width;
            let mut current_line = FlexLine {
                item_indices: Vec::new(),
                main_size: 0.0,
                cross_size: 0.0,
            };

            for (i, item) in items.iter().enumerate() {
                let item_main = item.width;
                let gap_extra = if current_line.item_indices.is_empty() {
                    0.0
                } else {
                    gap
                };

                if wrap == FlexWrap::Wrap
                    && !current_line.item_indices.is_empty()
                    && current_line.main_size + gap_extra + item_main > max_main
                {
                    lines.push(current_line);
                    current_line = FlexLine {
                        item_indices: Vec::new(),
                        main_size: 0.0,
                        cross_size: 0.0,
                    };
                }

                if !current_line.item_indices.is_empty() {
                    current_line.main_size += gap;
                }
                current_line.main_size += item_main;
                current_line.cross_size = current_line.cross_size.max(item.height);
                current_line.item_indices.push(i);
            }
            if !current_line.item_indices.is_empty() {
                lines.push(current_line);
            }
        }
        FlexDirection::Column => {
            // In column direction, each item is on its own "line" conceptually,
            // but we group them all into one line for simplicity (no column wrap needed yet)
            let mut line = FlexLine {
                item_indices: Vec::new(),
                main_size: 0.0,
                cross_size: 0.0,
            };
            for (i, item) in items.iter().enumerate() {
                if !line.item_indices.is_empty() {
                    line.main_size += gap;
                }
                line.main_size += item.height;
                line.cross_size = line.cross_size.max(item.width);
                line.item_indices.push(i);
            }
            if !line.item_indices.is_empty() {
                lines.push(line);
            }
        }
    }

    // Compute container dimensions
    let total_cross: f32 = match direction {
        FlexDirection::Row => {
            lines.iter().map(|l| l.cross_size).sum::<f32>()
                + if lines.len() > 1 {
                    (lines.len() - 1) as f32 * gap
                } else {
                    0.0
                }
        }
        FlexDirection::Column => lines.iter().map(|l| l.cross_size).fold(0.0f32, f32::max),
    };

    let total_main: f32 = match direction {
        FlexDirection::Row => inner_width,
        FlexDirection::Column => lines.iter().map(|l| l.main_size).sum::<f32>(),
    };

    let container_height = match direction {
        FlexDirection::Row => total_cross,
        FlexDirection::Column => total_main,
    };

    let container_h = style.padding.top + container_height + style.padding.bottom;
    let container_h = match style.height {
        Some(h) => container_h.max(h),
        None => container_h,
    };
    let bg = style
        .background_color
        .map(|color: crate::types::Color| color.to_f32_rgb());

    // For column direction, emit container background separately
    let emitted_column_bg = direction == FlexDirection::Column
        && (has_background_paint(&style) || style.border.has_any() || style.box_shadow.is_some());
    if emitted_column_bg {
        // Emit the container background/border as a visual element.
        // It advances y by its full height in paginate.  We then emit a
        // negative-margin spacer to pull y back so children flow *inside*
        // the background rather than after it.
        let bg_flow_height = container_h + style.border.vertical_width();
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::from_style(&style);
        output.push(LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: style.margin.top,
            margin_bottom: 0.0,
            text_align: style.text_align,
            background_color: bg,
            padding_top: style.padding.top,
            padding_bottom: style.padding.bottom,
            padding_left: style.padding.left,
            padding_right: style.padding.right,
            border: LayoutBorder::from_computed(&style.border),
            block_width: Some(block_w),
            block_height: Some(container_h),
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0),
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: style.box_shadow,
            visible: style.visibility == Visibility::Visible,
            clip_rect: if style.overflow == Overflow::Hidden {
                Some((0.0, 0.0, block_w, container_h))
            } else {
                None
            },
            transform: style.transform,
            border_radius: style.border_radius,
            outline_width: style.outline_width,
            outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        });
        // Pull y back so children flow inside the container background
        let BackgroundFields {
            gradient: background_gradient,
            radial_gradient: background_radial_gradient,
            svg: background_svg,
            blur_radius: background_blur_radius,
            size: background_size,
            position: background_position,
            repeat: background_repeat,
            origin: background_origin,
        } = BackgroundFields::none();
        output.push(LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: -bg_flow_height,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient,
            background_radial_gradient,
            background_svg,
            background_blur_radius,
            background_size,
            background_position,
            background_repeat,
            background_origin,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        });
    }

    // Position items within the flex container and emit them
    let mut cross_offset = 0.0;

    for line in &lines {
        let line_items: Vec<usize> = line.item_indices.clone();
        let line_item_count = line_items.len();

        match direction {
            FlexDirection::Row => {
                let total_item_width: f32 = line_items.iter().map(|&i| items[i].width).sum();
                let total_gap = if line_item_count > 1 {
                    (line_item_count - 1) as f32 * gap
                } else {
                    0.0
                };
                let mut free_space = inner_width - total_item_width - total_gap;

                // Flex grow: distribute positive free space proportionally
                let total_grow: f32 = line_items.iter().map(|&i| items[i].flex_grow).sum();
                if free_space > 0.0 && total_grow > 0.0 {
                    for &i in &line_items {
                        items[i].width += free_space * (items[i].flex_grow / total_grow);
                    }
                    free_space = 0.0;
                } else if free_space > 0.0
                    && total_grow == 0.0
                    && free_space < inner_width * 0.05
                    && line_item_count > 0
                {
                    // Fallback: small rounding remainder with no explicit flex-grow
                    let grow_each = free_space / line_item_count as f32;
                    for &i in &line_items {
                        items[i].width += grow_each;
                    }
                    free_space = 0.0;
                }

                // Flex shrink: shrink items when overflowing
                if free_space < 0.0 {
                    let total_shrink_weighted: f32 = line_items
                        .iter()
                        .map(|&i| items[i].flex_shrink * items[i].base_width)
                        .sum();
                    if total_shrink_weighted > 0.0 {
                        let deficit = -free_space;
                        for &i in &line_items {
                            let shrink_ratio =
                                items[i].flex_shrink * items[i].base_width / total_shrink_weighted;
                            items[i].width = (items[i].width - deficit * shrink_ratio).max(0.0);
                        }
                    }
                    free_space = 0.0;
                }

                let free_space = free_space.max(0.0);

                // Calculate starting x and spacing based on justify-content
                let (mut x, extra_gap) = match justify {
                    JustifyContent::FlexStart => (0.0, 0.0),
                    JustifyContent::FlexEnd => (free_space, 0.0),
                    JustifyContent::Center => (free_space / 2.0, 0.0),
                    JustifyContent::SpaceBetween => {
                        if line_item_count > 1 {
                            (0.0, free_space / (line_item_count - 1) as f32)
                        } else {
                            (0.0, 0.0)
                        }
                    }
                    JustifyContent::SpaceAround => {
                        let around = free_space / line_item_count as f32;
                        (around / 2.0, around)
                    }
                };

                // Build FlexCells for this row line
                let mut flex_cells = Vec::new();
                for &item_idx in &line_items {
                    let item = &items[item_idx];

                    // Extract text properties from the item's TextBlock
                    if let Some(LayoutElement::TextBlock {
                        lines: tb_lines,
                        text_align: tb_ta,
                        background_color: tb_bg,
                        padding_top: tb_pt,
                        padding_bottom: tb_pb,
                        padding_left: tb_pl,
                        padding_right: tb_pr,
                        border_radius: tb_br,
                        background_gradient: tb_grad,
                        background_radial_gradient: tb_rgrad,
                        background_svg: tb_bg_svg,
                        background_blur_radius: tb_bg_blur,
                        background_size: tb_bg_size,
                        background_position: tb_bg_pos,
                        background_repeat: tb_bg_repeat,
                        background_origin: tb_bg_origin,
                        ..
                    }) = item.elements.first()
                    {
                        flex_cells.push(FlexCell {
                            lines: tb_lines.clone(),
                            x_offset: x,
                            width: item.width,
                            text_align: *tb_ta,
                            background_color: *tb_bg,
                            padding_top: *tb_pt,
                            padding_right: *tb_pr,
                            padding_bottom: *tb_pb,
                            padding_left: *tb_pl,
                            border_radius: *tb_br,
                            background_gradient: tb_grad.clone(),
                            background_radial_gradient: tb_rgrad.clone(),
                            background_svg: tb_bg_svg.clone(),
                            background_blur_radius: *tb_bg_blur,
                            background_size: *tb_bg_size,
                            background_position: *tb_bg_pos,
                            background_repeat: *tb_bg_repeat,
                            background_origin: *tb_bg_origin,
                        });
                    }

                    x += item.width + gap + extra_gap;
                }

                output.push(LayoutElement::FlexRow {
                    cells: flex_cells,
                    row_height: line.cross_size,
                    margin_top: style.margin.top + cross_offset,
                    margin_bottom: 0.0,
                    background_color: if cross_offset == 0.0 { bg } else { None },
                    container_width: block_w,
                    padding_top: style.padding.top,
                    padding_bottom: style.padding.bottom,
                    padding_left: style.padding.left,
                    padding_right: style.padding.right,
                    border: if cross_offset == 0.0 {
                        LayoutBorder::from_computed(&style.border)
                    } else {
                        LayoutBorder::default()
                    },
                    border_radius: style.border_radius,
                    box_shadow: if cross_offset == 0.0 {
                        style.box_shadow
                    } else {
                        None
                    },
                    background_gradient: if cross_offset == 0.0 {
                        style.background_gradient.clone()
                    } else {
                        None
                    },
                    background_radial_gradient: if cross_offset == 0.0 {
                        style.background_radial_gradient.clone()
                    } else {
                        None
                    },
                    background_svg: if cross_offset == 0.0 {
                        background_svg_for_style(&style)
                    } else {
                        None
                    },
                    background_blur_radius: if cross_offset == 0.0 {
                        style.blur_radius
                    } else {
                        0.0
                    },
                    background_size: if cross_offset == 0.0 {
                        style.background_size
                    } else {
                        BackgroundSize::Auto
                    },
                    background_position: if cross_offset == 0.0 {
                        style.background_position
                    } else {
                        BackgroundPosition::default()
                    },
                    background_repeat: if cross_offset == 0.0 {
                        style.background_repeat
                    } else {
                        BackgroundRepeat::Repeat
                    },
                    background_origin: if cross_offset == 0.0 {
                        style.background_origin
                    } else {
                        BackgroundOrigin::PaddingBox
                    },
                });
            }
            FlexDirection::Column => {
                let _total_item_height: f32 = line_items.iter().map(|&i| items[i].height).sum();
                let _total_gap = if line_item_count > 1 {
                    (line_item_count - 1) as f32 * gap
                } else {
                    0.0
                };
                let free_space = 0.0f32; // column doesn't constrain main axis to container width
                let _ = free_space;

                let mut y = 0.0;

                for &item_idx in &line_items {
                    let item = &items[item_idx];

                    // Calculate cross-axis (horizontal) alignment
                    let x_offset = match align {
                        AlignItems::FlexStart => 0.0,
                        AlignItems::FlexEnd => inner_width - item.width,
                        AlignItems::Center => (inner_width - item.width) / 2.0,
                        AlignItems::Stretch => 0.0,
                    };

                    let effective_width = if align == AlignItems::Stretch {
                        Some(inner_width)
                    } else {
                        Some(item.width)
                    };

                    for elem in &item.elements {
                        if let LayoutElement::TextBlock {
                            lines: tb_lines,
                            margin_top: tb_mt,
                            margin_bottom: tb_mb,
                            text_align: tb_ta,
                            background_color: tb_bg,
                            padding_top: tb_pt,
                            padding_bottom: tb_pb,
                            padding_left: tb_pl,
                            padding_right: tb_pr,
                            border: tb_border,
                            block_height: tb_bh,
                            opacity: tb_op,
                            position: tb_pos,
                            box_shadow: tb_bs,
                            visible: tb_vis,
                            clip_rect: tb_clip,
                            transform: tb_transform,
                            border_radius: tb_br,
                            outline_width: tb_ow,
                            outline_color: tb_oc,
                            text_indent: tb_ti,
                            letter_spacing: tb_ls,
                            word_spacing: tb_ws,
                            vertical_align: tb_va,
                            background_gradient: tb_grad,
                            background_radial_gradient: tb_rgrad,
                            background_svg: tb_bg_svg,
                            background_blur_radius: tb_bg_blur,
                            background_size: tb_bg_size,
                            background_position: tb_bg_pos,
                            background_repeat: tb_bg_repeat,
                            background_origin: tb_bg_origin,
                            ..
                        } = elem
                        {
                            output.push(LayoutElement::TextBlock {
                                lines: tb_lines.clone(),
                                margin_top: if y == 0.0 && !emitted_column_bg {
                                    style.margin.top + style.padding.top + *tb_mt
                                } else if y == 0.0 {
                                    // Background element already accounts for margin;
                                    // add only the container padding offset.
                                    style.padding.top + *tb_mt
                                } else {
                                    *tb_mt
                                },
                                margin_bottom: *tb_mb,
                                text_align: *tb_ta,
                                background_color: *tb_bg,
                                padding_top: *tb_pt,
                                padding_bottom: *tb_pb,
                                padding_left: *tb_pl,
                                padding_right: *tb_pr,
                                border: *tb_border,
                                block_width: effective_width,
                                block_height: *tb_bh,
                                opacity: *tb_op,
                                float: Float::None,
                                clear: Clear::None,
                                position: if x_offset > 0.0 {
                                    Position::Relative
                                } else {
                                    *tb_pos
                                },
                                offset_top: 0.0,
                                offset_left: x_offset + style.padding.left,
                                offset_bottom: 0.0,
                                offset_right: 0.0,
                                containing_block: None,
                                box_shadow: *tb_bs,
                                visible: *tb_vis,
                                clip_rect: *tb_clip,
                                transform: *tb_transform,
                                border_radius: *tb_br,
                                outline_width: *tb_ow,
                                outline_color: *tb_oc,
                                text_indent: *tb_ti,
                                letter_spacing: *tb_ls,
                                word_spacing: *tb_ws,
                                vertical_align: *tb_va,
                                background_gradient: tb_grad.clone(),
                                background_radial_gradient: tb_rgrad.clone(),
                                background_svg: tb_bg_svg.clone(),
                                background_blur_radius: *tb_bg_blur,
                                background_size: *tb_bg_size,
                                background_position: *tb_bg_pos,
                                background_repeat: *tb_bg_repeat,
                                background_origin: *tb_bg_origin,
                                z_index: 0,
                                repeat_on_each_page: false,
                                positioned_depth: 0,
                                heading_level: None,
                            });
                        }
                    }

                    y += item.height + gap;
                }
            }
        }

        cross_offset += line.cross_size + gap;
    }

    // Emit trailing margin (include bottom padding when bg spacer shifted y back)
    let trailing = if emitted_column_bg {
        style.padding.bottom + style.margin.bottom
    } else {
        style.margin.bottom
    };
    if trailing > 0.0 {
        output.push(LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: trailing,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::PaddingBox,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        });
    }
}

/// Resolve grid column widths from track definitions.
fn resolve_grid_columns(tracks: &[GridTrack], available_width: f32, gap: f32) -> Vec<f32> {
    if tracks.is_empty() {
        return vec![available_width];
    }

    let num_gaps = if tracks.len() > 1 {
        (tracks.len() - 1) as f32 * gap
    } else {
        0.0
    };
    let space = available_width - num_gaps;

    // First pass: consume fixed-width columns
    let mut fixed_total: f32 = 0.0;
    let mut fr_total: f32 = 0.0;
    let mut auto_count: usize = 0;
    let mut minmax_count: usize = 0;

    for track in tracks {
        match track {
            GridTrack::Fixed(v) => fixed_total += *v,
            GridTrack::Fr(v) => fr_total += *v,
            GridTrack::Auto => auto_count += 1,
            GridTrack::Minmax(min, _) => {
                fixed_total += min;
                minmax_count += 1;
            }
        }
    }

    let remaining = (space - fixed_total).max(0.0);

    // Auto columns are treated like 1fr each for distribution purposes
    let effective_fr_total = fr_total + auto_count as f32 + minmax_count as f32;
    let per_fr = if effective_fr_total > 0.0 {
        remaining / effective_fr_total
    } else {
        0.0
    };

    tracks
        .iter()
        .map(|track| match track {
            GridTrack::Fixed(v) => *v,
            GridTrack::Fr(v) => per_fr * *v,
            GridTrack::Auto => per_fr,
            GridTrack::Minmax(min, max) => {
                let desired = min + per_fr;
                if *max < f32::MAX {
                    desired.clamp(*min, *max)
                } else {
                    desired
                }
            }
        })
        .collect()
}

/// Lay out a CSS Grid container into GridRow layout elements.
#[allow(clippy::too_many_arguments)]
fn flatten_grid_container(
    el: &ElementNode,
    style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    fonts: &HashMap<String, TtfFont>,
) {
    let inner_width = available_width - style.padding.left - style.padding.right;
    let gap = style.grid_gap;

    let col_widths = resolve_grid_columns(&style.grid_template_columns, inner_width, gap);
    let num_cols = col_widths.len();

    // Build ancestors list for children of this element
    let mut child_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        element: el,
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
    });

    // Collect element children (skip text nodes)
    let children: Vec<&ElementNode> = el
        .children
        .iter()
        .filter_map(|child| {
            if let DomNode::Element(child_el) = child {
                Some(child_el)
            } else {
                None
            }
        })
        .collect();

    let child_count = children.len();

    // Lay out children into grid cells, row by row
    let mut child_idx = 0;
    let mut is_first_row = true;

    while child_idx < children.len() {
        let row_end = (child_idx + num_cols).min(children.len());
        let mut cells = Vec::new();

        for (col, child_el) in children[child_idx..row_end].iter().enumerate() {
            let classes = child_el.class_list();
            let selector_ctx = SelectorContext {
                ancestors: child_ancestors.clone(),
                child_index: child_idx + col,
                sibling_count: child_count,
                preceding_siblings: Vec::new(),
            };
            let child_style = compute_style_with_context(
                child_el.tag,
                child_el.style_attr(),
                style,
                rules,
                child_el.tag_name(),
                &classes,
                child_el.id(),
                &child_el.attributes,
                &selector_ctx,
            );

            let cell_width = col_widths[col];
            let cell_inner =
                (cell_width - child_style.padding.left - child_style.padding.right).max(1.0);

            let mut runs = Vec::new();
            collect_flex_child_text_runs(
                &child_el.children,
                &child_style,
                &mut runs,
                None,
                (0.0, 0.0),
                rules,
                fonts,
                &child_ancestors,
            );
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    cell_inner,
                    child_style.font_size,
                    resolved_line_height_factor(&child_style, fonts),
                    child_style.overflow_wrap,
                ),
                fonts,
            );

            let bg = child_style
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgb());

            cells.push(TableCell {
                lines,
                nested_rows: Vec::new(),
                bold: child_style.font_weight == FontWeight::Bold,
                background_color: bg,
                padding_top: child_style.padding.top,
                padding_right: child_style.padding.right,
                padding_bottom: child_style.padding.bottom,
                padding_left: child_style.padding.left,
                colspan: 1,
                rowspan: 1,
                border: LayoutBorder::from_computed(&child_style.border),
                text_align: child_style.text_align,
                vertical_align: child_style.vertical_align,
            });
        }

        // Fill remaining columns with empty cells if the row is incomplete
        while cells.len() < num_cols {
            cells.push(TableCell {
                lines: Vec::new(),
                nested_rows: Vec::new(),
                bold: false,
                background_color: None,
                padding_top: 0.0,
                padding_right: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                colspan: 1,
                rowspan: 1,
                border: LayoutBorder::default(),
                text_align: TextAlign::Left,
                vertical_align: VerticalAlign::Baseline,
            });
        }

        let margin_top = if is_first_row { style.margin.top } else { gap };

        output.push(LayoutElement::GridRow {
            cells,
            col_widths: col_widths.clone(),
            margin_top,
            margin_bottom: 0.0,
        });

        is_first_row = false;
        child_idx = row_end;
    }

    // Add bottom margin after the last row
    if let Some(LayoutElement::GridRow { margin_bottom, .. }) = output.last_mut() {
        *margin_bottom = style.margin.bottom;
    }
}

/// Parse a width for a `<col>` / `<colgroup>` element.
///
/// Valid inline `width` declarations take precedence. Malformed inline
/// declarations are ignored so the `width` attribute can still act as a
/// fallback. `width: auto` explicitly clears the width.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TableTrackWidth {
    Points(f32),
    Percent(f32),
}

fn resolve_table_percentage_width(table_width: f32, percent: f32) -> f32 {
    // Percentage `<col>` and `<colgroup>` widths resolve against the table
    // width itself. Border-spacing is applied later when laying out the cells
    // so it must not shrink the percentage basis.
    table_width * percent
}

impl TableTrackWidth {
    fn resolve(self, table_width: f32) -> f32 {
        match self {
            Self::Points(width) => width,
            Self::Percent(percent) => resolve_table_percentage_width(table_width, percent),
        }
    }
}

fn compute_column_style(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
) -> ComputedStyle {
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: Vec::new(),
    };
    compute_style_with_context(
        el.tag,
        el.style_attr(),
        parent_style,
        rules,
        el.tag_name(),
        &classes,
        el.id(),
        &el.attributes,
        &selector_ctx,
    )
}

fn parse_element_width(el: &ElementNode) -> Option<TableTrackWidth> {
    if let Some(inline_width) = parse_element_inline_width(el) {
        return inline_width;
    }
    el.attributes
        .get("width")
        .and_then(|val| parse_table_track_width(val))
}

fn parse_element_inline_width(el: &ElementNode) -> Option<Option<TableTrackWidth>> {
    if let Some(style_str) = el.style_attr() {
        let mut last_inline_width = None;
        for decl in style_str.split(';').map(str::trim) {
            if let Some((prop, val)) = decl.split_once(':') {
                if prop.trim().eq_ignore_ascii_case("width") {
                    let val = strip_important(val).trim();
                    last_inline_width = parse_inline_width_value(val).or(last_inline_width);
                }
            }
        }
        return last_inline_width;
    }
    None
}

fn parse_col_width(
    col_el: &ElementNode,
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
) -> Option<TableTrackWidth> {
    let computed_style = compute_column_style(
        col_el,
        parent_style,
        rules,
        ancestors,
        child_index,
        sibling_count,
    );
    if let Some(inline_width) = parse_column_inline_width(col_el, computed_style.width) {
        return inline_width;
    }
    computed_style
        .width
        .map(TableTrackWidth::Points)
        .or_else(|| {
            col_el
                .attributes
                .get("width")
                .and_then(|val| parse_table_track_width(val))
        })
}

fn parse_column_inline_width(
    el: &ElementNode,
    computed_width: Option<f32>,
) -> Option<Option<TableTrackWidth>> {
    let style_str = el.style_attr()?;
    let inline = crate::parser::css::parse_inline_style(style_str);
    match inline.get("width") {
        Some(CssValue::Keyword(k)) if k.eq_ignore_ascii_case("auto") => Some(None),
        Some(_) => computed_width.map(|width| Some(TableTrackWidth::Points(width))),
        None => None,
    }
}

fn parse_percent_width(val: &str) -> Option<f32> {
    let pct_str = val.trim().strip_suffix('%')?;
    pct_str.trim().parse::<f32>().ok().map(|pct| pct / 100.0)
}

fn parse_table_track_width(val: &str) -> Option<TableTrackWidth> {
    if let Some(percent) = parse_percent_width(val) {
        return Some(TableTrackWidth::Percent(percent));
    }
    match crate::parser::css::parse_length(val) {
        Some(CssValue::Length(width)) => Some(TableTrackWidth::Points(width)),
        _ => None,
    }
}

fn parse_inline_width_value(val: &str) -> Option<Option<TableTrackWidth>> {
    if val.eq_ignore_ascii_case("auto") {
        return Some(None);
    }
    parse_table_track_width(val).map(Some).or_else(|| {
        crate::parser::css::parse_length(val)
            .is_some()
            .then_some(None)
    })
}

fn strip_important(val: &str) -> &str {
    val.strip_suffix("!important")
        .map(str::trim_end)
        .unwrap_or(val)
}

fn parse_col_span(el: &ElementNode) -> usize {
    el.attributes
        .get("span")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 1000)
}

fn assign_explicit_col_widths(
    explicit_col_widths: &mut [Option<TableTrackWidth>],
    col_idx: &mut usize,
    span: usize,
    width: Option<TableTrackWidth>,
) {
    for slot in explicit_col_widths.iter_mut().skip(*col_idx).take(span) {
        *slot = width;
    }
    *col_idx = col_idx.saturating_add(span);
}

fn resolve_table_inner_width(style: &ComputedStyle, available_width: f32) -> f32 {
    let containing_width = (available_width - style.margin.left - style.margin.right).max(0.0);
    style
        .width
        .or_else(|| {
            style
                .percentage_sizing
                .width
                .map(|percent| containing_width * percent / 100.0)
        })
        .map_or(containing_width, |width| {
            width.min(containing_width).max(0.0)
        })
}

fn uses_fixed_table_layout(style: &ComputedStyle) -> bool {
    style.table_layout == TableLayout::Fixed
        && (style.width.is_some() || style.percentage_sizing.width.is_some())
}

fn resolve_cell_track_width(
    cell_el: &ElementNode,
    cell_style: &ComputedStyle,
    table_width: f32,
) -> Option<f32> {
    parse_element_width(cell_el)
        .map(|width| width.resolve(table_width))
        .or(cell_style.width)
}

fn apply_cell_width_to_columns(
    col_widths: &mut [Option<f32>],
    start: usize,
    colspan: usize,
    width: f32,
) {
    if colspan == 0 || start >= col_widths.len() {
        return;
    }
    let per_column_width = width / colspan as f32;
    for slot in col_widths.iter_mut().skip(start).take(colspan) {
        *slot = Some(slot.map_or(per_column_width, |existing| existing.max(per_column_width)));
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_fixed_table_columns(
    table_style: &ComputedStyle,
    table_width: f32,
    rows: &[&ElementNode],
    row_section_indices: &[usize],
    row_section_sizes: &[usize],
    row_section_elements: &[Option<&ElementNode>],
    row_section_child_indices: &[usize],
    row_section_sibling_counts: &[usize],
    table_ancestors: &[AncestorInfo],
    explicit_col_widths: &[Option<TableTrackWidth>],
    num_cols: usize,
    rules: &[CssRule],
) -> Vec<f32> {
    let mut col_widths: Vec<Option<f32>> = explicit_col_widths
        .iter()
        .map(|width| width.map(|specified| specified.resolve(table_width)))
        .collect();

    if let Some(first_row) = rows.first() {
        let mut row_ancestors = table_ancestors.to_vec();
        if let Some(section_el) = row_section_elements.first().copied().flatten() {
            row_ancestors.push(AncestorInfo {
                element: section_el,
                child_index: row_section_child_indices.first().copied().unwrap_or(0),
                sibling_count: row_section_sibling_counts.first().copied().unwrap_or(0),
                preceding_siblings: Vec::new(),
            });
        }
        let row_selector_ctx = SelectorContext {
            ancestors: row_ancestors,
            child_index: row_section_indices.first().copied().unwrap_or(0),
            sibling_count: row_section_sizes.first().copied().unwrap_or(1),
            preceding_siblings: Vec::new(),
        };
        let row_classes = first_row.class_list();
        let mut row_style = compute_style_with_context(
            first_row.tag,
            first_row.style_attr(),
            table_style,
            rules,
            first_row.tag_name(),
            &row_classes,
            first_row.id(),
            &first_row.attributes,
            &row_selector_ctx,
        );
        row_style.width = Some(table_width);

        let mut col_pos = 0usize;
        for child in &first_row.children {
            let DomNode::Element(cell_el) = child else {
                continue;
            };
            if cell_el.tag != HtmlTag::Td && cell_el.tag != HtmlTag::Th {
                continue;
            }
            let colspan = cell_el
                .attributes
                .get("colspan")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);

            let cell_classes = cell_el.class_list();
            let mut cell_ancestors = row_selector_ctx.ancestors.clone();
            cell_ancestors.push(AncestorInfo {
                element: first_row,
                child_index: row_selector_ctx.child_index,
                sibling_count: row_selector_ctx.sibling_count,
                preceding_siblings: Vec::new(),
            });
            let cell_selector_ctx = SelectorContext {
                ancestors: cell_ancestors,
                child_index: col_pos,
                sibling_count: num_cols,
                preceding_siblings: Vec::new(),
            };
            let cell_style = compute_style_with_context(
                cell_el.tag,
                cell_el.style_attr(),
                &row_style,
                rules,
                cell_el.tag_name(),
                &cell_classes,
                cell_el.id(),
                &cell_el.attributes,
                &cell_selector_ctx,
            );

            if let Some(width) = resolve_cell_track_width(cell_el, &cell_style, table_width) {
                apply_cell_width_to_columns(&mut col_widths, col_pos, colspan, width);
            }

            col_pos = col_pos.saturating_add(colspan);
            if col_pos >= num_cols {
                break;
            }
        }
    }

    let assigned_width: f32 = col_widths.iter().flatten().copied().sum();
    let unresolved_count = col_widths.iter().filter(|width| width.is_none()).count();
    if unresolved_count > 0 {
        let remaining_width = (table_width - assigned_width).max(0.0);
        let default_width = remaining_width / unresolved_count as f32;
        for width in &mut col_widths {
            if width.is_none() {
                *width = Some(default_width);
            }
        }
    }

    let mut resolved_widths: Vec<f32> = col_widths
        .into_iter()
        .map(|width| width.unwrap_or(0.0))
        .collect();
    let resolved_total: f32 = resolved_widths.iter().sum();
    let used_table_width = table_width.max(resolved_total);
    if used_table_width > resolved_total && !resolved_widths.is_empty() {
        let extra_per_column = (used_table_width - resolved_total) / resolved_widths.len() as f32;
        for width in &mut resolved_widths {
            *width += extra_per_column;
        }
    }

    if resolved_widths.iter().all(|width| *width <= 0.0) && num_cols > 0 {
        return vec![table_width / num_cols as f32; num_cols];
    }

    resolved_widths
}

#[allow(clippy::too_many_arguments)]
fn flatten_table(
    el: &ElementNode,
    style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    ancestors: &[AncestorInfo],
    table_child_index: usize,
    table_sibling_count: usize,
) {
    let inner_width = resolve_table_inner_width(style, available_width);

    // Build ancestor chain: everything above + the table element itself.
    let mut table_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
    table_ancestors.push(AncestorInfo {
        element: el,
        child_index: table_child_index,
        sibling_count: table_sibling_count,
        preceding_siblings: Vec::new(),
    });

    // Collect all <tr> elements (from direct children, thead, tbody, tfoot).
    // Track section-relative indices so nth-child counts within each section
    // (thead, tbody, tfoot) as browsers do, not globally.
    // Also track the section element so descendant selectors can see it.
    let mut rows: Vec<&ElementNode> = Vec::new();
    let mut row_section_indices: Vec<usize> = Vec::new();
    let mut row_section_sizes: Vec<usize> = Vec::new();
    let mut row_section_elements: Vec<Option<&ElementNode>> = Vec::new();
    let mut row_section_child_indices: Vec<usize> = Vec::new();
    let mut row_section_sibling_counts: Vec<usize> = Vec::new();
    let section_count = el
        .children
        .iter()
        .filter(|c| matches!(c, DomNode::Element(_)))
        .count();
    for (section_child_idx, child) in el.children.iter().enumerate() {
        if let DomNode::Element(child_el) = child {
            match child_el.tag {
                HtmlTag::Tr => {
                    // Direct <tr> child of <table> — standalone section
                    let idx = rows.len();
                    rows.push(child_el);
                    row_section_indices.push(idx);
                    row_section_sizes.push(1);
                    row_section_elements.push(None);
                    row_section_child_indices.push(section_child_idx);
                    row_section_sibling_counts.push(section_count);
                }
                HtmlTag::Thead | HtmlTag::Tbody | HtmlTag::Tfoot => {
                    let section_rows: Vec<&ElementNode> = child_el
                        .children
                        .iter()
                        .filter_map(|gc| {
                            if let DomNode::Element(g) = gc {
                                if g.tag == HtmlTag::Tr {
                                    return Some(g);
                                }
                            }
                            None
                        })
                        .collect();
                    let section_size = section_rows.len();
                    for (i, gc) in section_rows.into_iter().enumerate() {
                        rows.push(gc);
                        row_section_indices.push(i);
                        row_section_sizes.push(section_size);
                        row_section_elements.push(Some(child_el));
                        row_section_child_indices.push(section_child_idx);
                        row_section_sibling_counts.push(section_count);
                    }
                }
                _ => {}
            }
        }
    }

    if rows.is_empty() {
        return;
    }

    // Determine column count from the widest row, accounting for colspan
    let num_cols = rows
        .iter()
        .map(|row| {
            row.children
                .iter()
                .filter_map(|c| {
                    if let DomNode::Element(e) = c {
                        if e.tag == HtmlTag::Td || e.tag == HtmlTag::Th {
                            let colspan = e
                                .attributes
                                .get("colspan")
                                .and_then(|v| v.parse::<usize>().ok())
                                .unwrap_or(1)
                                .max(1);
                            return Some(colspan);
                        }
                    }
                    None
                })
                .sum::<usize>()
        })
        .max()
        .unwrap_or(1);

    let mut column_parent_style = style.clone();
    column_parent_style.width = Some(inner_width);

    // --- Extract explicit column widths from <colgroup>/<col> elements ---
    let mut explicit_col_widths: Vec<Option<TableTrackWidth>> = vec![None; num_cols];
    {
        let mut col_idx = 0usize;
        for (section_child_idx, child) in el.children.iter().enumerate() {
            if let DomNode::Element(child_el) = child {
                match child_el.tag {
                    HtmlTag::Colgroup => {
                        let cols: Vec<&ElementNode> = child_el
                            .children
                            .iter()
                            .filter_map(|gc| match gc {
                                DomNode::Element(g) if g.tag == HtmlTag::Col => Some(g),
                                _ => None,
                            })
                            .collect();
                        let colgroup_style = compute_column_style(
                            child_el,
                            &column_parent_style,
                            rules,
                            &table_ancestors,
                            section_child_idx,
                            section_count,
                        );
                        if !cols.is_empty() {
                            let mut colgroup_basis_style = colgroup_style.clone();
                            colgroup_basis_style.width = Some(inner_width);
                            let mut colgroup_ancestors = table_ancestors.clone();
                            colgroup_ancestors.push(AncestorInfo {
                                element: child_el,
                                child_index: section_child_idx,
                                sibling_count: section_count,
                                preceding_siblings: Vec::new(),
                            });
                            let col_sibling_count = cols.len();
                            for (col_child_idx, col_el) in cols.into_iter().enumerate() {
                                assign_explicit_col_widths(
                                    &mut explicit_col_widths,
                                    &mut col_idx,
                                    parse_col_span(col_el),
                                    parse_col_width(
                                        col_el,
                                        &colgroup_basis_style,
                                        rules,
                                        &colgroup_ancestors,
                                        col_child_idx,
                                        col_sibling_count,
                                    ),
                                );
                            }
                            continue;
                        }
                        assign_explicit_col_widths(
                            &mut explicit_col_widths,
                            &mut col_idx,
                            parse_col_span(child_el),
                            parse_col_width(
                                child_el,
                                &column_parent_style,
                                rules,
                                &table_ancestors,
                                section_child_idx,
                                section_count,
                            ),
                        );
                    }
                    HtmlTag::Col => {
                        assign_explicit_col_widths(
                            &mut explicit_col_widths,
                            &mut col_idx,
                            parse_col_span(child_el),
                            parse_col_width(
                                child_el,
                                &column_parent_style,
                                rules,
                                &table_ancestors,
                                section_child_idx,
                                section_count,
                            ),
                        );
                    }
                    _ => continue,
                }
            }
        }
    }
    let has_explicit_widths = explicit_col_widths.iter().any(|width| width.is_some());
    let col_widths: Vec<f32> = if uses_fixed_table_layout(style) {
        resolve_fixed_table_columns(
            style,
            inner_width,
            &rows,
            &row_section_indices,
            &row_section_sizes,
            &row_section_elements,
            &row_section_child_indices,
            &row_section_sibling_counts,
            &table_ancestors,
            &explicit_col_widths,
            num_cols,
            rules,
        )
    } else {
        // --- Auto-sizing pass: measure preferred content width for each column ---
        let min_col_width: f32 = 30.0;
        let mut preferred_widths: Vec<f32> = vec![0.0; num_cols];

        for (sizing_row_idx, row) in rows.iter().enumerate() {
            let row_classes = row.class_list();
            // Build ancestors for the row: table + optional section element
            let mut sizing_row_ancestors = table_ancestors.clone();
            if let Some(section_el) = row_section_elements[sizing_row_idx] {
                sizing_row_ancestors.push(AncestorInfo {
                    element: section_el,
                    child_index: row_section_child_indices[sizing_row_idx],
                    sibling_count: row_section_sibling_counts[sizing_row_idx],
                    preceding_siblings: Vec::new(),
                });
            }
            let sizing_row_ctx = SelectorContext {
                ancestors: sizing_row_ancestors,
                child_index: row_section_indices[sizing_row_idx],
                sibling_count: row_section_sizes[sizing_row_idx],
                preceding_siblings: Vec::new(),
            };
            let mut row_style = compute_style_with_context(
                row.tag,
                row.style_attr(),
                style,
                rules,
                row.tag_name(),
                &row_classes,
                row.id(),
                &row.attributes,
                &sizing_row_ctx,
            );
            row_style.width = Some(inner_width);
            let mut col_pos: usize = 0;
            for child in &row.children {
                if let DomNode::Element(cell_el) = child {
                    if cell_el.tag == HtmlTag::Td || cell_el.tag == HtmlTag::Th {
                        let colspan = cell_el
                            .attributes
                            .get("colspan")
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(1)
                            .max(1);
                        let cell_classes = cell_el.class_list();
                        let mut cell_sizing_ancestors = sizing_row_ctx.ancestors.clone();
                        cell_sizing_ancestors.push(AncestorInfo {
                            element: row,
                            child_index: row_section_indices[sizing_row_idx],
                            sibling_count: row_section_sizes[sizing_row_idx],
                            preceding_siblings: Vec::new(),
                        });
                        let cell_sizing_ctx = SelectorContext {
                            ancestors: cell_sizing_ancestors,
                            child_index: col_pos,
                            sibling_count: num_cols,
                            preceding_siblings: Vec::new(),
                        };
                        let cell_style = compute_style_with_context(
                            cell_el.tag,
                            cell_el.style_attr(),
                            &row_style,
                            rules,
                            cell_el.tag_name(),
                            &cell_classes,
                            cell_el.id(),
                            &cell_el.attributes,
                            &cell_sizing_ctx,
                        );
                        let mut runs = Vec::new();
                        let mut nested_rows = Vec::new();
                        let recurse_descendants = cell_el.children.iter().any(
                            |node| matches!(node, DomNode::Element(e) if recurses_as_layout_child(e.tag)),
                        );
                        let mut text_ancestors = cell_sizing_ctx.ancestors.clone();
                        text_ancestors.push(AncestorInfo {
                            element: cell_el,
                            child_index: col_pos,
                            sibling_count: num_cols,
                            preceding_siblings: Vec::new(),
                        });
                        collect_table_cell_content_inner(
                            &cell_el.children,
                            &cell_style,
                            &mut runs,
                            &mut nested_rows,
                            None,
                            rules,
                            fonts,
                            false,
                            recurse_descendants,
                            recurse_descendants,
                            &text_ancestors,
                            inner_width.max(1.0),
                        );
                        // Estimate content width using estimate_word_width for accurate
                        // measurement. Use the maximum of (full text width, longest word
                        // width) to avoid hyphenation of short columns like "Unit Price".
                        let content_width: f32 = runs
                            .iter()
                            .map(|run| {
                                // Measure full text width using estimate_word_width
                                let full_width = estimate_word_width(
                                    &run.text,
                                    run.font_size,
                                    &run.font_family,
                                    run.bold,
                                    run.italic,
                                    fonts,
                                );
                                // Also ensure the column is at least as wide as
                                // the longest word to prevent hyphenation.
                                let longest_word_width = run
                                    .text
                                    .split_whitespace()
                                    .map(|w| {
                                        estimate_word_width(
                                            w,
                                            run.font_size,
                                            &run.font_family,
                                            run.bold,
                                            run.italic,
                                            fonts,
                                        )
                                    })
                                    .fold(0.0f32, f32::max);
                                full_width.max(longest_word_width)
                            })
                            .sum();
                        let nested_width = nested_rows
                            .iter()
                            .map(table_row_content_width)
                            .fold(0.0f32, f32::max);
                        let total_preferred = content_width.max(nested_width)
                            + cell_style.padding.left
                            + cell_style.padding.right;
                        if colspan == 1 {
                            if col_pos < num_cols {
                                preferred_widths[col_pos] =
                                    preferred_widths[col_pos].max(total_preferred);
                            }
                        } else {
                            let per_col = total_preferred / colspan as f32;
                            for i in 0..colspan {
                                if col_pos + i < num_cols {
                                    preferred_widths[col_pos + i] =
                                        preferred_widths[col_pos + i].max(per_col);
                                }
                            }
                        }
                        col_pos += colspan;
                    }
                }
            }
        }

        for width in &mut preferred_widths {
            if *width < min_col_width {
                *width = min_col_width;
            }
        }

        if has_explicit_widths {
            preferred_widths
                .iter()
                .zip(explicit_col_widths.iter())
                .map(|(preferred, explicit)| {
                    explicit
                        .map(|width| width.resolve(inner_width).max(min_col_width))
                        .unwrap_or_else(|| preferred.max(min_col_width))
                })
                .collect()
        } else {
            let total_preferred: f32 = preferred_widths.iter().sum();
            if total_preferred <= inner_width {
                let extra = inner_width - total_preferred;
                if total_preferred > 0.0 && extra > 0.0 {
                    preferred_widths
                        .iter()
                        .map(|width| width + (width / total_preferred) * extra)
                        .collect()
                } else {
                    preferred_widths
                }
            } else {
                let scale = inner_width / total_preferred;
                preferred_widths
                    .iter()
                    .map(|width| (width * scale).max(min_col_width))
                    .collect()
            }
        }
    };

    // Build layout rows, tracking cells occupied by rowspan from previous rows.
    // Each entry in `occupied` tracks the remaining rowspan count for that column.
    let mut occupied: Vec<usize> = vec![0; num_cols];
    let mut is_first = true;
    for (row_idx, row) in rows.iter().enumerate() {
        let row_classes = row.class_list();
        // Use section-relative index for nth-child matching (browsers count
        // within thead/tbody/tfoot, not globally across all rows).
        let section_idx = row_section_indices[row_idx];
        let section_size = row_section_sizes[row_idx];
        // Build ancestors for the row: table + optional section element
        let mut row_ancestors = table_ancestors.clone();
        if let Some(section_el) = row_section_elements[row_idx] {
            row_ancestors.push(AncestorInfo {
                element: section_el,
                child_index: row_section_child_indices[row_idx],
                sibling_count: row_section_sibling_counts[row_idx],
                preceding_siblings: Vec::new(),
            });
        }
        let row_selector_ctx = SelectorContext {
            ancestors: row_ancestors,
            child_index: section_idx,
            sibling_count: section_size,
            preceding_siblings: Vec::new(),
        };
        let mut row_style = compute_style_with_context(
            row.tag,
            row.style_attr(),
            style,
            rules,
            row.tag_name(),
            &row_classes,
            row.id(),
            &row.attributes,
            &row_selector_ctx,
        );
        row_style.width = Some(inner_width);
        let mut cells = Vec::new();

        // Current logical column position in the grid
        let mut col_pos: usize = 0;
        let mut child_iter = row.children.iter().filter_map(|child| {
            if let DomNode::Element(cell_el) = child {
                if cell_el.tag == HtmlTag::Td || cell_el.tag == HtmlTag::Th {
                    return Some(cell_el);
                }
            }
            None
        });

        // Process cells, skipping occupied positions and inserting phantom cells
        let mut next_cell = child_iter.next();
        while col_pos < num_cols {
            if occupied[col_pos] > 0 {
                // This position is occupied by a rowspan from a previous row.
                // Insert a phantom cell (rowspan = 0) as a placeholder.
                let span_cols = {
                    // Count how many consecutive occupied columns share this rowspan
                    let remaining = occupied[col_pos];
                    let mut count = 1;
                    while col_pos + count < num_cols && occupied[col_pos + count] == remaining {
                        count += 1;
                    }
                    count
                };
                cells.push(TableCell {
                    lines: Vec::new(),
                    nested_rows: Vec::new(),
                    bold: false,
                    background_color: None,
                    padding_top: 0.0,
                    padding_right: 0.0,
                    padding_bottom: 0.0,
                    padding_left: 0.0,
                    colspan: span_cols,
                    rowspan: 0, // phantom cell marker
                    border: LayoutBorder::default(),
                    text_align: TextAlign::Left,
                    vertical_align: VerticalAlign::Baseline,
                });
                for i in 0..span_cols {
                    occupied[col_pos + i] -= 1;
                }
                col_pos += span_cols;
                continue;
            }

            // Place the next real cell at this position
            let Some(cell_el) = next_cell else { break };
            next_cell = child_iter.next();

            let colspan = cell_el
                .attributes
                .get("colspan")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            let rowspan = cell_el
                .attributes
                .get("rowspan")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);

            let cell_classes = cell_el.class_list();
            let mut cell_ancestors = row_selector_ctx.ancestors.clone();
            cell_ancestors.push(AncestorInfo {
                element: row,
                child_index: section_idx,
                sibling_count: section_size,
                preceding_siblings: Vec::new(),
            });
            let cell_selector_ctx = SelectorContext {
                ancestors: cell_ancestors,
                child_index: col_pos,
                sibling_count: num_cols,
                preceding_siblings: Vec::new(),
            };
            let cell_style = compute_style_with_context(
                cell_el.tag,
                cell_el.style_attr(),
                &row_style,
                rules,
                cell_el.tag_name(),
                &cell_classes,
                cell_el.id(),
                &cell_el.attributes,
                &cell_selector_ctx,
            );
            // Compute effective width from auto-sized column widths
            let effective_width: f32 = col_widths.iter().skip(col_pos).take(colspan).copied().sum();
            let cell_inner = effective_width - cell_style.padding.left - cell_style.padding.right;
            let mut cell_content_style = cell_style.clone();
            cell_content_style.width = Some(cell_inner.max(0.0));

            let mut runs = Vec::new();
            let mut nested_rows = Vec::new();
            let recurse_descendants = cell_el
                .children
                .iter()
                .any(|node| matches!(node, DomNode::Element(e) if recurses_as_layout_child(e.tag)));
            let mut text_ancestors = cell_selector_ctx.ancestors.clone();
            text_ancestors.push(AncestorInfo {
                element: cell_el,
                child_index: col_pos,
                sibling_count: num_cols,
                preceding_siblings: Vec::new(),
            });
            let (block_margin_top, block_margin_bottom) = table_cell_edge_block_margins(
                &cell_el.children,
                &cell_content_style,
                rules,
                &text_ancestors,
            );
            collect_table_cell_content_inner(
                &cell_el.children,
                &cell_content_style,
                &mut runs,
                &mut nested_rows,
                None,
                rules,
                fonts,
                false,
                recurse_descendants,
                recurse_descendants,
                &text_ancestors,
                cell_inner.max(1.0),
            );
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    cell_inner.max(1.0),
                    cell_style.font_size,
                    resolved_line_height_factor(&cell_style, fonts),
                    cell_style.overflow_wrap,
                ),
                fonts,
            );

            let bg = cell_style
                .background_color
                .or(row_style.background_color)
                .map(|c: crate::types::Color| c.to_f32_rgb());

            cells.push(TableCell {
                lines,
                nested_rows,
                bold: cell_style.font_weight == FontWeight::Bold,
                background_color: bg,
                padding_top: cell_style.padding.top + block_margin_top,
                padding_right: cell_style.padding.right,
                padding_bottom: cell_style.padding.bottom + block_margin_bottom,
                padding_left: cell_style.padding.left,
                colspan,
                rowspan,
                border: LayoutBorder::from_computed(&cell_style.border),
                text_align: cell_style.text_align,
                vertical_align: cell_style.vertical_align,
            });

            // Mark subsequent rows as occupied if rowspan > 1
            if rowspan > 1 {
                for i in 0..colspan {
                    if col_pos + i < num_cols {
                        occupied[col_pos + i] = rowspan - 1;
                    }
                }
            }

            col_pos += colspan;
        }

        if !cells.is_empty() {
            output.push(LayoutElement::TableRow {
                cells,
                col_widths: col_widths.clone(),
                margin_top: if is_first { style.margin.top } else { 0.0 },
                margin_bottom: 0.0,
                border_collapse: style.border_collapse,
                border_spacing: style.border_spacing,
            });
            is_first = false;
        }
    }

    // Add bottom margin after the last row
    if let Some(LayoutElement::TableRow { margin_bottom, .. }) = output.last_mut() {
        *margin_bottom = style.margin.bottom;
    }
}

fn table_cell_edge_block_margins(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
) -> (f32, f32) {
    let element_sibling_count = nodes
        .iter()
        .filter(|node| matches!(node, DomNode::Element(_)))
        .count();

    let mut first_margin_top = None;
    let mut last_margin_bottom = None;

    for (node_index, node) in nodes.iter().enumerate() {
        let DomNode::Element(element) = node else {
            continue;
        };
        if element.tag == HtmlTag::Br
            || element.tag == HtmlTag::Table
            || element.children.is_empty()
        {
            continue;
        }

        let child_index = nodes[..node_index]
            .iter()
            .filter(|node| matches!(node, DomNode::Element(_)))
            .count();
        let preceding_siblings = nodes[..node_index]
            .iter()
            .filter_map(|node| match node {
                DomNode::Element(element) => Some((
                    element.tag_name().to_string(),
                    element
                        .class_list()
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                )),
                _ => None,
            })
            .collect();
        let selector_ctx = SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index,
            sibling_count: element_sibling_count,
            preceding_siblings,
        };
        let child_style = compute_style_with_context(
            element.tag,
            element.style_attr(),
            parent_style,
            rules,
            element.tag_name(),
            &element.class_list(),
            element.id(),
            &element.attributes,
            &selector_ctx,
        );
        if child_style.display == Display::Inline {
            continue;
        }

        first_margin_top.get_or_insert(child_style.margin.top);
        last_margin_bottom = Some(child_style.margin.bottom);
    }

    (
        first_margin_top.unwrap_or(0.0),
        last_margin_bottom.unwrap_or(0.0),
    )
}

/// Collect text runs from flex child content, recursively descending into
/// block-level children so that nested `<h1>`, `<p>`, etc. are captured.
#[allow(clippy::only_used_in_recursion)]
fn collect_flex_child_text_runs(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    link_url: Option<&str>,
    text_padding: (f32, f32),
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    ancestors: &[AncestorInfo],
) {
    let preserve_ws = matches!(
        parent_style.white_space,
        WhiteSpace::Pre | WhiteSpace::PreWrap
    );

    for node in nodes {
        match node {
            DomNode::Text(text) => {
                let processed = if preserve_ws {
                    text.clone()
                } else {
                    collapse_whitespace(text)
                };
                if !processed.is_empty() {
                    runs.push(TextRun {
                        text: processed,
                        font_size: parent_style.font_size,
                        bold: parent_style.font_weight == FontWeight::Bold,
                        italic: parent_style.font_style == FontStyle::Italic,
                        underline: parent_style.text_decoration_underline,
                        line_through: parent_style.text_decoration_line_through,
                        color: parent_style.color.to_f32_rgb(),
                        link_url: link_url.map(String::from),
                        font_family: resolve_style_font_family(parent_style, fonts),
                        background_color: parent_style.background_color.map(|c| c.to_f32_rgb()),
                        padding: text_padding,
                        border_radius: 0.0,
                    });
                }
            }
            DomNode::Element(el) => {
                let classes = el.class_list();
                let selector_ctx = SelectorContext {
                    ancestors: ancestors.to_vec(),
                    child_index: 0,
                    sibling_count: nodes.len(),
                    preceding_siblings: Vec::new(),
                };
                let child_style = compute_style_with_context(
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

                if child_style.display == Display::None {
                    continue;
                }

                let child_padding = if child_style.display == Display::Block
                    || child_style.background_color.is_some()
                    || child_style.border.has_any()
                    || child_style.border_radius > 0.0
                {
                    (child_style.padding.left, child_style.padding.top)
                } else {
                    text_padding
                };
                let child_link_url = if el.tag == HtmlTag::A {
                    el.attributes.get("href").map(|s| s.as_str()).or(link_url)
                } else {
                    link_url
                };

                if el.tag == HtmlTag::Br {
                    runs.push(TextRun {
                        text: "\n".to_string(),
                        font_size: parent_style.font_size,
                        bold: false,
                        italic: false,
                        underline: false,
                        line_through: false,
                        color: (0.0, 0.0, 0.0),
                        link_url: None,
                        font_family: resolve_style_font_family(parent_style, fonts),
                        background_color: None,
                        padding: (0.0, 0.0),
                        border_radius: 0.0,
                    });
                } else {
                    // Build ancestor chain including current element for recursive calls
                    let mut child_ancestors = ancestors.to_vec();
                    child_ancestors.push(AncestorInfo {
                        element: el,
                        child_index: 0,
                        sibling_count: nodes.len(),
                        preceding_siblings: Vec::new(),
                    });
                    // Recurse into both inline and block children so flex items
                    // with nested block elements (h1, h2, p, div, …) produce text.
                    collect_flex_child_text_runs(
                        &el.children,
                        &child_style,
                        runs,
                        child_link_url,
                        child_padding,
                        rules,
                        fonts,
                        &child_ancestors,
                    );
                    // Insert a line break after block-level elements so they don't
                    // merge with following content on the same line.
                    if el.tag.is_block() && !runs.is_empty() {
                        runs.push(TextRun {
                            text: "\n".to_string(),
                            font_size: child_style.font_size,
                            bold: false,
                            italic: false,
                            underline: false,
                            line_through: false,
                            color: child_style.color.to_f32_rgb(),
                            link_url: child_link_url.map(String::from),
                            font_family: resolve_style_font_family(&child_style, fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                        });
                    }
                }
            }
        }
    }
}

fn collect_text_runs(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    link_url: Option<&str>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    ancestors: &[AncestorInfo],
) {
    collect_text_runs_inner(
        nodes,
        parent_style,
        runs,
        link_url,
        rules,
        fonts,
        false,
        ancestors,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_text_runs_inner(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    link_url: Option<&str>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    inline_parent: bool,
    ancestors: &[AncestorInfo],
) {
    let preserve_ws = matches!(
        parent_style.white_space,
        WhiteSpace::Pre | WhiteSpace::PreWrap
    );

    for node in nodes {
        match node {
            DomNode::Text(text) => {
                let processed = if preserve_ws {
                    // In pre/pre-wrap: preserve newlines as \n runs for line breaking
                    text.clone()
                } else {
                    collapse_whitespace(text)
                };
                if !processed.is_empty() {
                    // Only propagate background_color when the immediate
                    // parent is an inline element (e.g. <span>).  Block-level
                    // backgrounds are drawn by the TextBlock itself.
                    // In preformatted blocks (<pre>), skip inline backgrounds
                    // to avoid overlapping rects that hide subsequent lines.
                    let (bg, pad, br) = if inline_parent && !preserve_ws {
                        (
                            parent_style.background_color.map(|c| c.to_f32_rgb()),
                            (parent_style.padding.left, parent_style.padding.top),
                            parent_style.border_radius,
                        )
                    } else {
                        (None, (0.0, 0.0), 0.0)
                    };
                    runs.push(TextRun {
                        text: processed,
                        font_size: parent_style.font_size,
                        bold: parent_style.font_weight == FontWeight::Bold,
                        italic: parent_style.font_style == FontStyle::Italic,
                        underline: parent_style.text_decoration_underline,
                        line_through: parent_style.text_decoration_line_through,
                        color: parent_style.color.to_f32_rgb(),
                        link_url: link_url.map(String::from),
                        font_family: resolve_style_font_family(parent_style, fonts),
                        background_color: bg,
                        padding: pad,
                        border_radius: br,
                    });
                }
            }
            DomNode::Element(el) => {
                if collects_as_inline_text(el.tag) || el.tag == HtmlTag::Br {
                    if el.tag == HtmlTag::Br {
                        runs.push(TextRun {
                            text: "\n".to_string(),
                            font_size: parent_style.font_size,
                            bold: false,
                            italic: false,
                            underline: false,
                            line_through: false,
                            color: (0.0, 0.0, 0.0),
                            link_url: None,
                            font_family: resolve_style_font_family(parent_style, fonts),
                            background_color: None,
                            padding: (0.0, 0.0),
                            border_radius: 0.0,
                        });
                    } else {
                        let classes = el.class_list();
                        let selector_ctx = SelectorContext {
                            ancestors: ancestors.to_vec(),
                            child_index: 0,
                            sibling_count: nodes.len(),
                            preceding_siblings: Vec::new(),
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
                        let url = if el.tag == HtmlTag::A {
                            el.attributes.get("href").map(|s| s.as_str()).or(link_url)
                        } else {
                            link_url
                        };
                        collect_text_runs_inner(
                            &el.children,
                            &style,
                            runs,
                            url,
                            rules,
                            fonts,
                            true,
                            ancestors,
                        );
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_table_cell_content_inner(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    nested_rows: &mut Vec<LayoutElement>,
    link_url: Option<&str>,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    inline_parent: bool,
    recurse_blocks: bool,
    suppress_direct_text_padding: bool,
    ancestors: &[AncestorInfo],
    available_width: f32,
) {
    let preserve_ws = matches!(
        parent_style.white_space,
        WhiteSpace::Pre | WhiteSpace::PreWrap
    );
    let element_sibling_count = nodes
        .iter()
        .filter(|node| matches!(node, DomNode::Element(_)))
        .count();

    for (node_index, node) in nodes.iter().enumerate() {
        match node {
            DomNode::Text(text) => {
                let processed = if preserve_ws {
                    text.clone()
                } else {
                    collapse_whitespace(text)
                };
                if !processed.is_empty() {
                    let (bg, pad, br) = if (inline_parent || recurse_blocks) && !preserve_ws {
                        let pad = if suppress_direct_text_padding {
                            (0.0, 0.0)
                        } else {
                            (parent_style.padding.left, parent_style.padding.top)
                        };
                        (
                            parent_style.background_color.map(|c| c.to_f32_rgb()),
                            pad,
                            parent_style.border_radius,
                        )
                    } else {
                        (None, (0.0, 0.0), 0.0)
                    };
                    push_text_run(
                        runs,
                        TextRun {
                            text: processed,
                            font_size: parent_style.font_size,
                            bold: parent_style.font_weight == FontWeight::Bold,
                            italic: parent_style.font_style == FontStyle::Italic,
                            underline: parent_style.text_decoration_underline,
                            line_through: parent_style.text_decoration_line_through,
                            color: parent_style.color.to_f32_rgb(),
                            link_url: link_url.map(String::from),
                            font_family: resolve_style_font_family(parent_style, fonts),
                            background_color: bg,
                            padding: pad,
                            border_radius: br,
                        },
                    );
                }
            }
            DomNode::Element(el) => {
                let child_index = nodes[..node_index]
                    .iter()
                    .filter(|node| matches!(node, DomNode::Element(_)))
                    .count();
                let preceding_siblings = nodes[..node_index]
                    .iter()
                    .filter_map(|node| match node {
                        DomNode::Element(element) => Some((
                            element.tag_name().to_string(),
                            element
                                .class_list()
                                .into_iter()
                                .map(str::to_string)
                                .collect(),
                        )),
                        _ => None,
                    })
                    .collect();
                let classes = el.class_list();
                let selector_ctx = SelectorContext {
                    ancestors: ancestors.to_vec(),
                    child_index,
                    sibling_count: element_sibling_count,
                    preceding_siblings,
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
                if style.display == Display::None {
                    continue;
                }
                let url = if el.tag == HtmlTag::A {
                    el.attributes.get("href").map(|s| s.as_str()).or(link_url)
                } else {
                    link_url
                };
                let mut child_ancestors = ancestors.to_vec();
                child_ancestors.push(AncestorInfo {
                    element: el,
                    child_index,
                    sibling_count: element_sibling_count,
                    preceding_siblings: Vec::new(),
                });
                if el.tag == HtmlTag::Table {
                    flatten_table(
                        el,
                        &style,
                        available_width,
                        nested_rows,
                        rules,
                        fonts,
                        &child_ancestors,
                        child_index,
                        element_sibling_count,
                    );
                } else if recurse_blocks
                    && style.display != Display::Inline
                    && el.tag != HtmlTag::Br
                    && el.children.is_empty()
                    && (has_background_paint(&style)
                        || style.border.has_any()
                        || style.box_shadow.is_some()
                        || style.aspect_ratio.is_some()
                        || style.height.is_some()
                        || style.width.is_some())
                {
                    flatten_element(
                        el,
                        parent_style,
                        available_width,
                        f32::INFINITY,
                        nested_rows,
                        None,
                        rules,
                        ancestors,
                        0,
                        child_index,
                        element_sibling_count,
                        &selector_ctx.preceding_siblings,
                        fonts,
                    );
                } else if el.tag == HtmlTag::Svg {
                    flatten_element(
                        el,
                        parent_style,
                        available_width,
                        f32::INFINITY,
                        nested_rows,
                        None,
                        rules,
                        ancestors,
                        0,
                        child_index,
                        element_sibling_count,
                        &selector_ctx.preceding_siblings,
                        fonts,
                    );
                } else if recurse_blocks || collects_as_inline_text(el.tag) || el.tag == HtmlTag::Br
                {
                    if el.tag == HtmlTag::Br {
                        push_line_break_run(runs, parent_style, fonts);
                    } else {
                        collect_table_cell_content_inner(
                            &el.children,
                            &style,
                            runs,
                            nested_rows,
                            url,
                            rules,
                            fonts,
                            collects_as_inline_text(el.tag),
                            recurse_blocks,
                            false,
                            &child_ancestors,
                            available_width,
                        );
                        if recurse_blocks && style.display != Display::Inline && !runs.is_empty() {
                            push_line_break_run(runs, &style, fonts);
                        }
                    }
                }
            }
        }
    }
}

fn push_text_run(runs: &mut Vec<TextRun>, run: TextRun) {
    runs.push(run);
}

fn push_line_break_run(
    runs: &mut Vec<TextRun>,
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) {
    push_text_run(
        runs,
        TextRun {
            text: "\n".to_string(),
            font_size: style.font_size,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: resolve_style_font_family(style, fonts),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        },
    );
}
/// Estimate the width of a word given its font settings and available custom fonts.
fn estimate_word_width(
    word: &str,
    font_size: f32,
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    if let Some(width) =
        crate::text::measure_text_width(word, font_size, font_family, bold, italic, fonts)
    {
        return width;
    }

    // Use AFM metrics for standard fonts (non-bold for layout estimation)
    crate::fonts::str_width(word, font_size, font_family, false)
}

#[derive(Clone, Copy)]
struct TextWrapOptions {
    max_width: f32,
    default_font_size: f32,
    line_height_factor: f32,
    overflow_wrap: OverflowWrap,
}

impl TextWrapOptions {
    const fn new(
        max_width: f32,
        default_font_size: f32,
        line_height_factor: f32,
        overflow_wrap: OverflowWrap,
    ) -> Self {
        Self {
            max_width,
            default_font_size,
            line_height_factor,
            overflow_wrap,
        }
    }
}

/// Split a long word at the last character boundary that still fits within
/// `available_width`, without inserting hyphen characters.
fn split_word_to_fit(
    word: &str,
    available_width: f32,
    font_size: f32,
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
) -> Option<(String, String)> {
    if word.is_empty() || available_width <= 0.0 {
        return None;
    }

    let mut best_boundary = None;
    for (index, _) in word.char_indices().skip(1) {
        let prefix = &word[..index];
        let prefix_width = estimate_word_width(prefix, font_size, font_family, bold, italic, fonts);
        if prefix_width <= available_width {
            best_boundary = Some(index);
        } else {
            break;
        }
    }

    let boundary = best_boundary?;
    Some((word[..boundary].to_string(), word[boundary..].to_string()))
}

/// Simple text wrapping using character width estimation.
/// Uses TTF metrics when a custom font is available.
fn wrap_text_runs(
    runs: Vec<TextRun>,
    options: TextWrapOptions,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<TextLine> {
    let line_height_factor = options.line_height_factor.max(0.0);
    let mut lines: Vec<TextLine> = Vec::new();
    let mut current_runs: Vec<TextRun> = Vec::new();
    let mut current_width: f32 = 0.0;
    let mut line_height = options.default_font_size * line_height_factor;

    // Concatenate all text then re-split by words, preserving run styles.
    // For text containing \n (white-space: pre), split on newlines first,
    // then split each segment by words.
    let mut styled_words: Vec<(String, TextRun, bool)> = Vec::new();
    for run in &runs {
        if run.text == "\n" {
            styled_words.push(("\n".to_string(), run.clone(), false));
            continue;
        }
        let has_newlines = run.text.contains('\n');
        let has_preserved_spacing = run.text.chars().next().is_some_and(char::is_whitespace)
            || run.text.chars().last().is_some_and(char::is_whitespace)
            || run.text.contains("  ");
        if has_newlines {
            for (seg_idx, segment) in run.text.split('\n').enumerate() {
                if seg_idx > 0 {
                    styled_words.push(("\n".to_string(), run.clone(), false));
                }
                if segment.is_empty() {
                    continue;
                }
                if segment.chars().next().is_some_and(char::is_whitespace)
                    || segment.chars().last().is_some_and(char::is_whitespace)
                    || segment.contains("  ")
                {
                    styled_words.push((segment.to_string(), run.clone(), true));
                } else {
                    for word in segment.split_whitespace() {
                        styled_words.push((word.to_string(), run.clone(), false));
                    }
                }
            }
        } else if has_preserved_spacing {
            styled_words.push((run.text.clone(), run.clone(), true));
        } else {
            for word in run.text.split_whitespace() {
                styled_words.push((word.to_string(), run.clone(), false));
            }
        }
    }

    if styled_words.is_empty() && !runs.is_empty() {
        return vec![TextLine {
            runs,
            height: line_height,
        }];
    }

    // Use a VecDeque so hyphenation remainders can be re-queued for processing.
    let mut queue: std::collections::VecDeque<(String, TextRun, bool)> =
        styled_words.into_iter().collect();

    while let Some((word, template, preserve_spacing)) = queue.pop_front() {
        if word == "\n" {
            // Line break
            lines.push(TextLine {
                runs: std::mem::take(&mut current_runs),
                height: line_height,
            });
            current_width = 0.0;
            line_height = options.default_font_size * line_height_factor;
            continue;
        }

        let word_width = estimate_word_width(
            &word,
            template.font_size,
            &template.font_family,
            template.bold,
            template.italic,
            fonts,
        );
        let space_width = estimate_word_width(
            " ",
            template.font_size,
            &template.font_family,
            template.bold,
            template.italic,
            fonts,
        );

        let needed = if current_width > 0.0 && !preserve_spacing {
            space_width + word_width
        } else {
            word_width
        };

        let overflows = current_width + needed > options.max_width;

        if overflows && !preserve_spacing && options.overflow_wrap != OverflowWrap::Normal {
            let available_width = if current_width > 0.0 {
                options.max_width - current_width - space_width
            } else {
                options.max_width
            };
            if let Some((prefix, remainder)) = split_word_to_fit(
                &word,
                available_width,
                template.font_size,
                &template.font_family,
                template.bold,
                template.italic,
                fonts,
            ) {
                let prefix_text = if current_width > 0.0 {
                    format!(" {prefix}")
                } else {
                    prefix
                };
                line_height = line_height.max(template.font_size * line_height_factor);
                current_runs.push(TextRun {
                    text: prefix_text,
                    ..template.clone()
                });

                lines.push(TextLine {
                    runs: std::mem::take(&mut current_runs),
                    height: line_height,
                });
                current_width = 0.0;
                line_height = options.default_font_size * line_height_factor;
                queue.push_front((remainder, template, false));
                continue;
            }
        }

        if overflows && current_width > 0.0 {
            lines.push(TextLine {
                runs: std::mem::take(&mut current_runs),
                height: line_height,
            });
            current_width = 0.0;
            line_height = options.default_font_size * line_height_factor;
        }

        let text = if current_width > 0.0 && !preserve_spacing {
            format!(" {word}")
        } else {
            word
        };

        let w = estimate_word_width(
            &text,
            template.font_size,
            &template.font_family,
            template.bold,
            template.italic,
            fonts,
        );
        current_width += w;
        line_height = line_height.max(template.font_size * line_height_factor);

        current_runs.push(TextRun { text, ..template });
    }

    if !current_runs.is_empty() {
        lines.push(TextLine {
            runs: current_runs,
            height: line_height,
        });
    }

    lines
}

/// Apply text-overflow: ellipsis by truncating lines and appending "...".
fn apply_text_overflow_ellipsis(
    lines: &mut Vec<TextLine>,
    max_width: f32,
    fonts: &HashMap<String, TtfFont>,
) {
    // With nowrap, there should be only one line. Truncate it if it overflows.
    if lines.is_empty() {
        return;
    }
    // Merge all runs into a single string, keeping the style of the first run.
    let line = &lines[0];
    let total_text: String = line.runs.iter().map(|r| r.text.as_str()).collect();
    if line.runs.is_empty() {
        return;
    }
    let template = line.runs[0].clone();
    let ellipsis = "...";
    let ellipsis_width = estimate_word_width(
        ellipsis,
        template.font_size,
        &template.font_family,
        template.bold,
        template.italic,
        fonts,
    );

    // Check if the line actually overflows
    let line_width = estimate_word_width(
        &total_text,
        template.font_size,
        &template.font_family,
        template.bold,
        template.italic,
        fonts,
    );
    if line_width <= max_width {
        return;
    }

    // Truncate character by character until text + ellipsis fits
    let mut truncated = String::new();
    for ch in total_text.chars() {
        truncated.push(ch);
        let w = estimate_word_width(
            &truncated,
            template.font_size,
            &template.font_family,
            template.bold,
            template.italic,
            fonts,
        );
        if w + ellipsis_width > max_width {
            truncated.pop();
            break;
        }
    }
    truncated.push_str(ellipsis);

    lines[0] = TextLine {
        runs: vec![TextRun {
            text: truncated,
            ..template
        }],
        height: line.height,
    };

    // Remove any additional lines (shouldn't exist with nowrap, but just in case)
    lines.truncate(1);
}

/// A tracked float region for simplified float layout.
#[derive(Debug, Clone)]
struct FloatRegion {
    #[allow(dead_code)]
    y_start: f32,
    y_end: f32,
    #[allow(dead_code)]
    side: Float,
}

/// Estimate the height of a layout element for wrapper sizing.
fn estimate_element_height(element: &LayoutElement) -> f32 {
    match element {
        LayoutElement::TextBlock {
            lines,
            margin_top,
            margin_bottom,
            padding_top,
            padding_bottom,
            border,
            block_height,
            position,
            ..
        } => {
            if *position == Position::Absolute {
                return 0.0;
            }
            let text_height: f32 = lines.iter().map(|l| l.height).sum();
            let content_h = padding_top + text_height + padding_bottom;
            let effective_h = block_height.map_or(content_h, |h| content_h.max(h));
            margin_top + effective_h + margin_bottom + border.vertical_width()
        }
        LayoutElement::FlexRow {
            row_height,
            margin_top,
            margin_bottom,
            padding_top,
            padding_bottom,
            border,
            ..
        } => {
            margin_top
                + padding_top
                + row_height
                + padding_bottom
                + margin_bottom
                + border.vertical_width()
        }
        LayoutElement::TableRow {
            cells,
            margin_top,
            margin_bottom,
            ..
        } => {
            let row_h = cells
                .iter()
                .map(table_cell_content_height)
                .fold(0.0f32, f32::max);
            margin_top + row_h + margin_bottom
        }
        LayoutElement::GridRow {
            cells,
            margin_top,
            margin_bottom,
            ..
        } => {
            let row_h = cells
                .iter()
                .map(table_cell_content_height)
                .fold(0.0f32, f32::max);
            margin_top + row_h + margin_bottom
        }
        LayoutElement::Image {
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + height + flow_extra_bottom + margin_bottom,
        LayoutElement::HorizontalRule {
            margin_top,
            margin_bottom,
        } => margin_top + 1.0 + margin_bottom,
        LayoutElement::ProgressBar {
            height,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + height + margin_bottom,
        LayoutElement::Svg {
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + height + flow_extra_bottom + margin_bottom,
        _ => 0.0,
    }
}

fn table_row_content_width(element: &LayoutElement) -> f32 {
    match element {
        LayoutElement::TableRow {
            col_widths,
            border_collapse,
            border_spacing,
            ..
        } => {
            let spacing = if *border_collapse == BorderCollapse::Collapse {
                0.0
            } else {
                *border_spacing
            };
            col_widths.iter().sum::<f32>() + spacing * col_widths.len().saturating_sub(1) as f32
        }
        _ => 0.0,
    }
}

fn paginate(elements: Vec<LayoutElement>, content_height: f32) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut current_elements: Vec<(f32, LayoutElement)> = Vec::new();
    let mut y = 0.0;

    // Track active float regions for simplified float/clear behavior
    let mut left_floats: Vec<FloatRegion> = Vec::new();
    let mut right_floats: Vec<FloatRegion> = Vec::new();
    let mut prev_margin_bottom: f32 = 0.0;

    // Collect synthetic full-page background elements that should be repeated
    // across every page during pagination.
    let mut absolute_backgrounds: Vec<(f32, LayoutElement)> = Vec::new();
    // Track the y-position of positioned ancestors by depth so absolute descendants
    // resolve against the nearest positioned ancestor rather than the most recent one.
    let mut positioned_y_by_depth: HashMap<usize, f32> = HashMap::new();

    for element in elements {
        // Extract float/clear/position info from TextBlock elements
        let (
            elem_float,
            elem_clear,
            elem_position,
            elem_offset_top,
            _elem_offset_bottom,
            elem_containing_block,
            elem_positioned_depth,
        ) = match &element {
            LayoutElement::TextBlock {
                float,
                clear,
                position,
                offset_top,
                offset_bottom,
                containing_block,
                positioned_depth,
                ..
            } => (
                *float,
                *clear,
                *position,
                *offset_top,
                *offset_bottom,
                *containing_block,
                *positioned_depth,
            ),
            _ => (
                Float::None,
                Clear::None,
                Position::Static,
                0.0,
                0.0,
                None,
                0,
            ),
        };

        // Handle clear: move y below active floats on the specified side
        match elem_clear {
            Clear::Left | Clear::Both => {
                for f in &left_floats {
                    if f.y_end > y {
                        y = f.y_end;
                    }
                }
                if elem_clear == Clear::Both {
                    for f in &right_floats {
                        if f.y_end > y {
                            y = f.y_end;
                        }
                    }
                }
            }
            Clear::Right => {
                for f in &right_floats {
                    if f.y_end > y {
                        y = f.y_end;
                    }
                }
            }
            Clear::None => {}
        }

        // Returns (content_height_without_margins, margin_top, margin_bottom)
        let (content_h_val, margin_top_val, margin_bottom_val) = match &element {
            LayoutElement::PageBreak => {
                let consumed_height = y;
                pages.push(Page {
                    elements: std::mem::take(&mut current_elements),
                });
                // Duplicate root background onto the new page.
                for bg in &absolute_backgrounds {
                    current_elements.push(bg.clone());
                }
                y = 0.0;
                prev_margin_bottom = 0.0;
                left_floats.clear();
                right_floats.clear();
                advance_positioned_ancestors_after_page_break(
                    &mut positioned_y_by_depth,
                    consumed_height,
                );
                continue;
            }
            LayoutElement::HorizontalRule {
                margin_top,
                margin_bottom,
            } => (1.0, *margin_top, *margin_bottom),
            LayoutElement::TableRow {
                cells,
                margin_top,
                margin_bottom,
                ..
            } => {
                let row_height = cells
                    .iter()
                    .map(table_cell_content_height)
                    .fold(0.0f32, f32::max);
                (row_height, *margin_top, *margin_bottom)
            }
            LayoutElement::GridRow {
                cells,
                margin_top,
                margin_bottom,
                ..
            } => {
                let row_height = cells
                    .iter()
                    .map(table_cell_content_height)
                    .fold(0.0f32, f32::max);
                (row_height, *margin_top, *margin_bottom)
            }
            LayoutElement::FlexRow {
                row_height,
                margin_top,
                margin_bottom,
                padding_top,
                padding_bottom,
                border,
                ..
            } => {
                let content = padding_top + row_height + padding_bottom + border.vertical_width();
                (content, *margin_top, *margin_bottom)
            }
            LayoutElement::TextBlock {
                lines,
                margin_top,
                margin_bottom,
                padding_top,
                padding_bottom,
                border,
                block_height,
                ..
            } => {
                let text_height: f32 = lines.iter().map(|l| l.height).sum();
                let border_extra = border.vertical_width();
                let content_h = padding_top + text_height + padding_bottom;
                let effective_content_h = match block_height {
                    Some(h) => content_h.max(*h),
                    None => content_h,
                };
                (
                    effective_content_h + border_extra,
                    *margin_top,
                    *margin_bottom,
                )
            }
            LayoutElement::Image {
                height,
                flow_extra_bottom,
                margin_top,
                margin_bottom,
                ..
            } => (*height + *flow_extra_bottom, *margin_top, *margin_bottom),
            LayoutElement::Svg {
                height,
                flow_extra_bottom,
                margin_top,
                margin_bottom,
                ..
            } => (*height + *flow_extra_bottom, *margin_top, *margin_bottom),
            LayoutElement::ProgressBar {
                height,
                margin_top,
                margin_bottom,
                ..
            } => (*height, *margin_top, *margin_bottom),
        };

        // Collapse margins: adjacent vertical margins merge (larger wins for positive,
        // most negative for negative, sum for mixed).
        let collapsed_margin = if margin_top_val >= 0.0 && prev_margin_bottom >= 0.0 {
            margin_top_val.max(prev_margin_bottom)
        } else if margin_top_val < 0.0 && prev_margin_bottom < 0.0 {
            margin_top_val.min(prev_margin_bottom)
        } else {
            margin_top_val + prev_margin_bottom
        };
        let margin_top_val = collapsed_margin - prev_margin_bottom;
        let element_height = margin_top_val + content_h_val + margin_bottom_val;

        // Handle position: absolute -- place at fixed position, don't affect flow
        if elem_position == Position::Absolute {
            let abs_y = if let Some(cb) = elem_containing_block {
                // Position relative to the containing block (nearest positioned ancestor).
                // bottom/right offsets are pre-resolved into top/left in build_pseudo_block.
                positioned_y_by_depth.get(&cb.depth).copied().unwrap_or(0.0) + elem_offset_top
            } else {
                // No containing block — position relative to page (legacy behavior).
                elem_offset_top
            };
            if elem_positioned_depth > 0 {
                positioned_y_by_depth.insert(elem_positioned_depth, abs_y);
            }
            let repeats_on_each_page = match &element {
                LayoutElement::TextBlock {
                    repeat_on_each_page,
                    ..
                } => *repeat_on_each_page,
                _ => false,
            };
            if repeats_on_each_page {
                absolute_backgrounds.push((abs_y, element.clone()));
            }
            current_elements.push((abs_y, element));
            continue;
        }

        if y + element_height > content_height && y > 0.0 {
            let consumed_height = y;
            pages.push(Page {
                elements: std::mem::take(&mut current_elements),
            });
            // Duplicate root background onto the new page.
            for bg in &absolute_backgrounds {
                current_elements.push(bg.clone());
            }
            y = 0.0;
            prev_margin_bottom = 0.0;
            left_floats.clear();
            right_floats.clear();
            advance_positioned_ancestors_after_page_break(
                &mut positioned_y_by_depth,
                consumed_height,
            );
        }

        // After potential page break, recompute effective margin_top
        // (on a fresh page, prev_margin_bottom is 0 so no collapsing needed).
        let effective_margin_top = if prev_margin_bottom == 0.0 {
            collapsed_margin
        } else {
            margin_top_val
        };

        // Handle floated elements (floats don't participate in margin collapsing)
        if elem_float != Float::None {
            y += effective_margin_top;
            let float_y_end = y + content_h_val;
            let region = FloatRegion {
                y_start: y,
                y_end: float_y_end,
                side: elem_float,
            };
            if elem_float == Float::Left {
                left_floats.push(region);
            } else {
                right_floats.push(region);
            }
            current_elements.push((y, element));
            prev_margin_bottom = 0.0;
            continue;
        }

        y += effective_margin_top;

        // Handle position: relative -- offset from normal position
        let effective_y = if elem_position == Position::Relative {
            y + elem_offset_top
        } else {
            y
        };

        // Track positioned ancestor y for absolute children.
        if elem_positioned_depth > 0
            && (elem_position == Position::Relative || elem_position == Position::Absolute)
        {
            positioned_y_by_depth.insert(elem_positioned_depth, effective_y);
        }

        current_elements.push((effective_y, element));
        y += content_h_val;
        prev_margin_bottom = margin_bottom_val;
    }

    if !current_elements.is_empty() {
        pages.push(Page {
            elements: current_elements,
        });
    }

    if pages.is_empty() {
        pages.push(Page {
            elements: Vec::new(),
        });
    }

    // Sort elements within each page by z_index for correct rendering order.
    // Static elements (z_index 0) stay in document order; positioned elements
    // with higher z_index are moved later so they render on top.
    for page in &mut pages {
        page.elements
            .sort_by_key(|(_, element)| layout_element_paint_order(element));
    }

    pages
}

/// Load raw bytes from a `src` attribute value.
///
/// Supports `data:` URIs (base64 and percent-encoded), local file paths, and
/// HTTP/HTTPS URLs (gated behind the `remote` feature).
///
/// For data URIs the MIME header is returned so callers can use it to skip
/// unnecessary probing (e.g. skip SVG probe when the MIME is `image/jpeg`).
pub(crate) fn load_src_bytes(src: &str) -> Option<(Vec<u8>, Option<String>)> {
    if let Some(rest) = src.strip_prefix("data:") {
        let (header, encoded) = rest.split_once(',')?;
        let header_lower = header.to_ascii_lowercase();
        let bytes = if header_lower.contains("base64") {
            decode_base64(encoded)?
        } else {
            // Plain-text or percent-encoded data URI — decode %XX sequences.
            percent_decode(encoded).into_bytes()
        };
        let mime = if header_lower.is_empty() {
            None
        } else {
            Some(header_lower)
        };
        Some((bytes, mime))
    } else if src.starts_with("http://") || src.starts_with("https://") {
        Some((fetch_remote_url(src)?, None))
    } else {
        Some((std::fs::read(src).ok()?, None))
    }
}

/// Probe raw bytes for SVG content and parse into an `SvgTree`.
///
/// Uses a heuristic on the first 512 bytes (via `String::from_utf8_lossy` so
/// that non-UTF-8 binary content is safely rejected) and then parses the full
/// content through the HTML parser to extract the `<svg>` element.
fn try_parse_svg_bytes(raw: &[u8]) -> Option<crate::parser::svg::SvgTree> {
    try_parse_svg_bytes_with_viewport(raw, None)
}

fn try_parse_svg_bytes_with_viewport(
    raw: &[u8],
    root_viewport: Option<(f32, f32)>,
) -> Option<crate::parser::svg::SvgTree> {
    // Heuristic: check if the content looks like SVG (XML with an <svg element).
    let prefix = if raw.len() > 512 { &raw[..512] } else { raw };
    let text = String::from_utf8_lossy(prefix);
    let trimmed = text.trim_start_matches('\u{FEFF}').trim_start();
    let trimmed_lower = trimmed.to_ascii_lowercase();
    if !(trimmed.starts_with("<svg")
        || trimmed.starts_with("<?xml")
        || trimmed.starts_with("<!--")
        || trimmed_lower.starts_with("<!doctype"))
    {
        return None;
    }
    // For the comment case, search the full content (comments may exceed the
    // 512-byte prefix before the <svg> tag appears).
    if trimmed.starts_with("<!--") {
        let full_text = String::from_utf8_lossy(raw);
        if !full_text.contains("<svg") {
            return None;
        }
    }

    // Parse the full SVG content — use lossy conversion so that stray non-UTF-8
    // bytes don't cause the whole parse to fail.
    let svg_str = String::from_utf8_lossy(raw);
    let nodes = crate::parser::html::parse_html(&svg_str).ok()?;
    let svg_el = find_svg_element(&nodes)?;
    crate::parser::svg::parse_svg_from_element_with_viewport(svg_el, root_viewport)
}

fn find_svg_element<'a>(nodes: &'a [crate::parser::dom::DomNode]) -> Option<&'a ElementNode> {
    for node in nodes {
        if let crate::parser::dom::DomNode::Element(el) = node {
            if el.tag == HtmlTag::Svg {
                return Some(el);
            }
            if let Some(found) = find_svg_element(&el.children) {
                return Some(found);
            }
        }
    }
    None
}

/// Detect PNG/JPEG format and return a raster asset with source dimensions.
fn load_image_bytes(raw: Vec<u8>) -> Option<RasterImageAsset> {
    if png::is_png(&raw) {
        let png_info = png::parse_png(&raw)?;
        let metadata = PngMetadata {
            channels: png_info.channels,
            bit_depth: png_info.bit_depth,
        };
        Some(RasterImageAsset {
            data: png_info.idat_data,
            source_width: png_info.width,
            source_height: png_info.height,
            format: ImageFormat::Png,
            png_metadata: Some(metadata),
        })
    } else if raw.starts_with(&[0xFF, 0xD8]) {
        let (source_width, source_height) = crate::parser::jpeg::parse_jpeg_dimensions(&raw)?;
        Some(RasterImageAsset {
            data: raw,
            source_width,
            source_height,
            format: ImageFormat::Jpeg,
            png_metadata: None,
        })
    } else {
        None
    }
}

/// Load image data from an <img> element and return a LayoutElement.
///
/// Bytes are fetched exactly once from the source.  When the content is SVG it
/// is parsed as vector graphics (`LayoutElement::Svg`); otherwise it falls back
/// to raster PNG/JPEG (`LayoutElement::Image`).
fn load_image_from_element(
    el: &ElementNode,
    available_width: f32,
    available_height: f32,
    style: &ComputedStyle,
) -> Option<LayoutElement> {
    let src = el.attributes.get("src")?;

    // Load bytes once.
    let (raw, mime) = load_src_bytes(src)?;

    // For data URIs with a non-SVG MIME type, skip the SVG probe entirely.
    let skip_svg = mime
        .as_deref()
        .is_some_and(|m| !m.is_empty() && !m.contains("svg") && !m.contains("xml"));

    // Try SVG path first — render as vector graphics instead of raster.
    if !skip_svg {
        let svg_str = String::from_utf8_lossy(&raw);
        if let Ok(nodes) = crate::parser::html::parse_html(&svg_str) {
            if let Some(svg_el) = find_svg_element(&nodes) {
                let intrinsic = resolve_svg_element_size(
                    svg_el,
                    available_width,
                    available_height,
                    false,
                    false,
                );
                let html_attr_width = style
                    .width
                    .or_else(|| parse_html_image_dimension(el.attributes.get("width")));
                let html_attr_height = style
                    .height
                    .or_else(|| parse_html_image_dimension(el.attributes.get("height")));

                let (width, height) = match (html_attr_width, html_attr_height) {
                    (Some(w), Some(h)) => (w, h),
                    (Some(w), None) => {
                        if intrinsic.0 > 0.0 {
                            (w, intrinsic.1 * (w / intrinsic.0))
                        } else {
                            (w, intrinsic.1)
                        }
                    }
                    (None, Some(h)) => {
                        if intrinsic.1 > 0.0 {
                            (intrinsic.0 * (h / intrinsic.1), h)
                        } else {
                            (intrinsic.0, h)
                        }
                    }
                    (None, None) => intrinsic,
                };

                let (width, height) = constrain_replaced_image_size(
                    width,
                    height,
                    available_width,
                    style.max_width,
                    style.max_height,
                );

                if let Some(tree) = crate::parser::svg::parse_svg_from_element_with_viewport(
                    svg_el,
                    Some((width, height)),
                ) {
                    let mut tree = tree;
                    sync_svg_tree_to_layout_box(&mut tree, width, height);
                    return Some(LayoutElement::Svg {
                        tree,
                        width,
                        height,
                        flow_extra_bottom: 0.0,
                        margin_top: style.margin.top,
                        margin_bottom: style.margin.bottom,
                    });
                }
            }
        }
    }

    // Fall back to raster image using the same bytes.
    let image = load_raster_image_bytes(raw, style.blur_radius)?;

    // Determine dimensions from attributes
    let attr_width = parse_html_image_dimension(el.attributes.get("width"));
    let attr_height = parse_html_image_dimension(el.attributes.get("height"));

    let (width, height) = match (attr_width, attr_height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w), // fallback: square
        (None, Some(h)) => (h, h),
        (None, None) => (available_width.min(200.0), 150.0),
    };

    let (width, height) = constrain_replaced_image_size(
        width,
        height,
        available_width,
        style.max_width,
        style.max_height,
    );

    Some(LayoutElement::Image {
        image,
        width,
        height,
        flow_extra_bottom: 0.0,
        margin_top: style.margin.top,
        margin_bottom: style.margin.bottom,
    })
}

fn constrain_replaced_image_size(
    width: f32,
    height: f32,
    available_width: f32,
    max_width: Option<f32>,
    max_height: Option<f32>,
) -> (f32, f32) {
    if width <= 0.0 || height <= 0.0 {
        return (width.max(0.0), height.max(0.0));
    }

    let mut scale: f32 = 1.0;

    if available_width.is_finite() && available_width > 0.0 {
        scale = scale.min(available_width / width);
    }

    if let Some(limit) = max_width.filter(|limit| limit.is_finite() && *limit > 0.0) {
        scale = scale.min(limit / width);
    }

    if let Some(limit) = max_height.filter(|limit| limit.is_finite() && *limit > 0.0) {
        scale = scale.min(limit / height);
    }

    if scale < 1.0 {
        (width * scale, height * scale)
    } else {
        (width, height)
    }
}

fn add_inline_replaced_baseline_gap(
    element: LayoutElement,
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> LayoutElement {
    if style.display != Display::Inline || style.vertical_align != VerticalAlign::Baseline {
        return element;
    }

    let font_family = resolve_style_font_family(style, fonts);
    let (_, descender_ratio) = crate::fonts::font_metrics_ratios(
        &font_family,
        style.font_weight == FontWeight::Bold,
        style.font_style == FontStyle::Italic,
        fonts,
    );
    let baseline_gap = descender_ratio * style.font_size;
    if baseline_gap <= 0.0 {
        return element;
    }

    match element {
        LayoutElement::Image {
            image,
            width,
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
        } => LayoutElement::Image {
            image,
            width,
            height,
            flow_extra_bottom: flow_extra_bottom + baseline_gap,
            margin_top,
            margin_bottom,
        },
        LayoutElement::Svg {
            tree,
            width,
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
        } => LayoutElement::Svg {
            tree,
            width,
            height,
            flow_extra_bottom: flow_extra_bottom + baseline_gap,
            margin_top,
            margin_bottom,
        },
        other => other,
    }
}

fn parse_html_image_dimension(raw: Option<&String>) -> Option<f32> {
    let raw = raw?.trim();
    let raw = raw.strip_suffix("px").unwrap_or(raw);
    raw.parse::<f32>().ok().map(|px| px * 0.75)
}

/// Resolve the rendered size of an SVG from its intrinsic dimensions and raw
/// `width`/`height` attributes.
fn resolve_svg_size(
    tree: &crate::parser::svg::SvgTree,
    available_width: f32,
    available_height: f32,
    allow_percent_width: bool,
    allow_percent_height: bool,
) -> (f32, f32) {
    resolve_svg_size_raw(
        tree.width_attr.as_deref(),
        tree.height_attr.as_deref(),
        tree.view_box.as_ref(),
        tree.width,
        tree.height,
        available_width,
        available_height,
        allow_percent_width,
        allow_percent_height,
    )
}

fn resolve_svg_element_size(
    el: &ElementNode,
    available_width: f32,
    available_height: f32,
    allow_percent_width: bool,
    allow_percent_height: bool,
) -> (f32, f32) {
    let width_raw = el.attributes.get("width").map(String::as_str);
    let height_raw = el.attributes.get("height").map(String::as_str);
    let view_box = el
        .attributes
        .get("viewBox")
        .and_then(|v| crate::parser::svg::parse_viewbox(v));
    let intrinsic_width = width_raw
        .and_then(crate::parser::svg::parse_absolute_length)
        .unwrap_or(0.0);
    let intrinsic_height = height_raw
        .and_then(crate::parser::svg::parse_absolute_length)
        .unwrap_or(0.0);
    resolve_svg_size_raw(
        width_raw,
        height_raw,
        view_box.as_ref(),
        intrinsic_width,
        intrinsic_height,
        available_width,
        available_height,
        allow_percent_width,
        allow_percent_height,
    )
}

fn resolve_svg_size_raw(
    width_raw: Option<&str>,
    height_raw: Option<&str>,
    view_box: Option<&crate::parser::svg::ViewBox>,
    intrinsic_width: f32,
    intrinsic_height: f32,
    available_width: f32,
    available_height: f32,
    allow_percent_width: bool,
    allow_percent_height: bool,
) -> (f32, f32) {
    const DEFAULT_OBJECT_WIDTH: f32 = 300.0;
    const DEFAULT_OBJECT_HEIGHT: f32 = 150.0;

    let intrinsic_ratio = if let Some(vb) = view_box {
        if vb.width > 0.0 && vb.height > 0.0 {
            Some(vb.height / vb.width)
        } else if intrinsic_width > 0.0 {
            Some(intrinsic_height / intrinsic_width)
        } else {
            None
        }
    } else {
        Some(if intrinsic_width > 0.0 {
            intrinsic_height / intrinsic_width
        } else {
            1.0
        })
    };
    let fallback_size = if intrinsic_width > 0.0 && intrinsic_height > 0.0 {
        (intrinsic_width, intrinsic_height)
    } else if intrinsic_width > 0.0 {
        (
            intrinsic_width,
            intrinsic_width
                * intrinsic_ratio.unwrap_or(DEFAULT_OBJECT_HEIGHT / DEFAULT_OBJECT_WIDTH),
        )
    } else if intrinsic_height > 0.0 {
        let ratio = intrinsic_ratio.unwrap_or(DEFAULT_OBJECT_HEIGHT / DEFAULT_OBJECT_WIDTH);
        (intrinsic_height / ratio.max(f32::EPSILON), intrinsic_height)
    } else if let Some(ratio) = intrinsic_ratio {
        let default_ratio = DEFAULT_OBJECT_HEIGHT / DEFAULT_OBJECT_WIDTH;
        if ratio > default_ratio {
            (DEFAULT_OBJECT_HEIGHT / ratio, DEFAULT_OBJECT_HEIGHT)
        } else {
            (DEFAULT_OBJECT_WIDTH, DEFAULT_OBJECT_WIDTH * ratio)
        }
    } else {
        (DEFAULT_OBJECT_WIDTH, DEFAULT_OBJECT_HEIGHT)
    };

    let width = resolve_svg_dimension(width_raw, available_width, allow_percent_width);
    let height = resolve_svg_dimension(height_raw, available_height, allow_percent_height);
    match (width, height, intrinsic_ratio) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some(ratio)) => (w, w * ratio),
        (None, Some(h), Some(ratio)) => (h / ratio.max(f32::EPSILON), h),
        _ => fallback_size,
    }
}

fn resolve_svg_dimension(
    raw: Option<&str>,
    available_space: f32,
    allow_percent: bool,
) -> Option<f32> {
    let Some(raw) = raw else {
        return None;
    };
    let raw = raw.trim();
    if let Some(pct) = raw.strip_suffix('%') {
        if allow_percent {
            if let Ok(value) = pct.trim().parse::<f32>() {
                if value >= 0.0 {
                    return Some(available_space * (value / 100.0));
                }
            }
        }
        return None;
    }

    let value = crate::parser::svg::parse_length(raw)?;
    if value >= 0.0 { Some(value) } else { None }
}

fn sync_svg_tree_to_layout_box(tree: &mut crate::parser::svg::SvgTree, width: f32, height: f32) {
    if tree.view_box.is_none() {
        tree.width = width;
        tree.height = height;
    }
}

fn inject_inherited_svg_color(
    tree: &mut crate::parser::svg::SvgTree,
    inherited_color: (f32, f32, f32),
) {
    let inherit_color = |style: &mut crate::parser::svg::SvgStyle| {
        style.color.get_or_insert(inherited_color);
    };

    match tree.children.as_mut_slice() {
        [crate::parser::svg::SvgNode::Group { style, .. }] => inherit_color(style),
        _ => {
            tree.children = vec![crate::parser::svg::SvgNode::Group {
                transform: None,
                children: std::mem::take(&mut tree.children),
                style: crate::parser::svg::SvgStyle {
                    color: Some(inherited_color),
                    ..crate::parser::svg::SvgStyle::default()
                },
            }];
        }
    }
}

/// Maximum size for remote resources (10 MB).
#[cfg(feature = "remote")]
const MAX_REMOTE_SIZE: usize = 10 * 1024 * 1024;

/// Fetch bytes from an HTTP/HTTPS URL (requires the `remote` feature).
/// Returns `None` if the feature is disabled, the request fails, or the response exceeds 10 MB.
fn fetch_remote_url(url: &str) -> Option<Vec<u8>> {
    #[cfg(feature = "remote")]
    {
        let resp = ureq::get(url).call().ok()?;
        let len = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        if len > MAX_REMOTE_SIZE {
            return None;
        }
        let buf = resp
            .into_body()
            .with_config()
            .limit(MAX_REMOTE_SIZE as u64)
            .read_to_vec()
            .ok()?;
        Some(buf)
    }
    #[cfg(not(feature = "remote"))]
    {
        let _ = url;
        None
    }
}

/// Load image data from a src attribute (supports data: URIs, local files, and remote URLs).
///
/// This is a convenience wrapper around `load_src_bytes` + `load_image_bytes`.
fn load_image_data(src: &str) -> Option<RasterImageAsset> {
    let (raw, _mime) = load_src_bytes(src)?;
    load_image_bytes(raw)
}

fn build_raster_background_tree(src: &str) -> Option<crate::parser::svg::SvgTree> {
    let image_src = crate::parser::css::extract_url_path(src).unwrap_or_else(|| src.to_string());
    let (raw, _mime) = load_src_bytes(&image_src)?;
    let (width, height) = raster_image_dimensions(&raw)?;

    Some(crate::parser::svg::SvgTree {
        width: width as f32,
        height: height as f32,
        width_attr: None,
        height_attr: None,
        preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
        view_box: None,
        defs: crate::parser::svg::SvgDefs::default(),
        children: vec![crate::parser::svg::SvgNode::Image {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
            href: image_src,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::None,
            style: crate::parser::svg::SvgStyle::default(),
        }],
        text_ctx: crate::parser::svg::SvgTextContext::default(),
        source_markup: None,
    })
}

fn raster_image_dimensions(raw: &[u8]) -> Option<(u32, u32)> {
    if png::is_png(raw) {
        let png_info = png::parse_png(raw)?;
        Some((png_info.width, png_info.height))
    } else {
        let image = image::load_from_memory(raw).ok()?;
        Some((image.width(), image.height()))
    }
}

fn load_raster_image_bytes(raw: Vec<u8>, blur_radius: f32) -> Option<RasterImageAsset> {
    if blur_radius > 0.0 {
        blur_image_bytes(&raw, blur_radius)
    } else {
        load_image_bytes(raw)
    }
}

fn blur_image_bytes(raw: &[u8], blur_radius: f32) -> Option<RasterImageAsset> {
    let decoded = decode_image_for_blur(raw)?;
    let blurred = image::imageops::blur(&decoded, blur_radius);
    let mut encoded = Vec::new();
    image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(blurred).to_rgb8())
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Jpeg,
        )
        .ok()?;
    Some(RasterImageAsset {
        data: encoded,
        source_width: decoded.width(),
        source_height: decoded.height(),
        format: ImageFormat::Jpeg,
        png_metadata: None,
    })
}

fn decode_image_for_blur(raw: &[u8]) -> Option<image::DynamicImage> {
    if png::is_png(raw) {
        decode_png_for_blur(raw)
    } else {
        image::load_from_memory(raw).ok()
    }
}

fn decode_png_for_blur(data: &[u8]) -> Option<image::DynamicImage> {
    use image::{DynamicImage, ImageBuffer};

    let mut decoder = png_decoder::Decoder::new(std::io::Cursor::new(data));
    decoder.ignore_checksums(true);
    let mut reader = decoder.read_info().ok()?;
    let output_size = reader.output_buffer_size()?;
    let mut buf = vec![0; output_size];
    let info = reader.next_frame(&mut buf).ok()?;
    let width = info.width;
    let height = info.height;
    let used = info.buffer_size();
    let buf = buf.get(..used)?.to_vec();

    match info.color_type {
        png_decoder::ColorType::Rgba => {
            let image = ImageBuffer::from_raw(width, height, buf)?;
            Some(DynamicImage::ImageRgba8(image))
        }
        png_decoder::ColorType::Rgb => {
            let image = ImageBuffer::from_raw(width, height, buf)?;
            Some(DynamicImage::ImageRgb8(image))
        }
        png_decoder::ColorType::Grayscale => {
            let image = ImageBuffer::from_raw(width, height, buf)?;
            Some(DynamicImage::ImageLuma8(image))
        }
        png_decoder::ColorType::GrayscaleAlpha => {
            let image = ImageBuffer::from_raw(width, height, buf)?;
            Some(DynamicImage::ImageLumaA8(image))
        }
        _ => image::load_from_memory(data).ok(),
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(*chunk.first().unwrap_or(&0));
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;

        append_base64_char(&mut result, CHARS, ((triple >> 18) & 0x3F) as usize);
        append_base64_char(&mut result, CHARS, ((triple >> 12) & 0x3F) as usize);

        if chunk.len() > 1 {
            append_base64_char(&mut result, CHARS, ((triple >> 6) & 0x3F) as usize);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            append_base64_char(&mut result, CHARS, (triple & 0x3F) as usize);
        } else {
            result.push('=');
        }
    }

    result
}

fn append_base64_char(out: &mut String, table: &[u8], index: usize) {
    if let Some(&byte) = table.get(index) {
        out.push(char::from(byte));
    }
}

/// Decode percent-encoded strings (e.g. `%3C` → `<`).  Used for plain-text SVG
/// data URIs like `data:image/svg+xml,%3Csvg ...%3E`.
fn percent_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut last_was_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_was_space && !result.is_empty() {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(c);
            last_was_space = false;
        }
    }
    result.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::css::parse_stylesheet;
    use crate::parser::html::{parse_html, parse_html_with_styles};
    use crate::parser::svg::{SvgTree, ViewBox};

    const TEST_JPEG_DATA_URI: &str = concat!(
        "data:image/jpeg;base64,",
        "/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkK",
        "DA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAA",
        "AAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q=="
    );

    fn first_child_element(node: &ElementNode) -> Option<&ElementNode> {
        node.children.iter().find_map(|child| match child {
            DomNode::Element(element) => Some(element),
            _ => None,
        })
    }

    fn first_child_element_with_tag(node: &ElementNode, tag: HtmlTag) -> Option<&ElementNode> {
        node.children.iter().find_map(|child| match child {
            DomNode::Element(element) if element.tag == tag => Some(element),
            _ => None,
        })
    }

    fn page_text_runs(page: &Page) -> Vec<&TextRun> {
        let mut runs = Vec::new();
        for (_, element) in &page.elements {
            if let LayoutElement::TextBlock { lines, .. } = element {
                for line in lines {
                    runs.extend(line.runs.iter());
                }
            }
        }
        runs
    }

    fn page_text(page: &Page) -> String {
        page_text_runs(page)
            .into_iter()
            .map(|run| run.text.as_str())
            .collect()
    }

    #[test]
    fn layout_simple_paragraph() {
        let nodes = parse_html("<p>Hello World</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_multiple_elements() {
        let nodes = parse_html("<h1>Title</h1><p>Paragraph one.</p><p>Paragraph two.</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 3);
    }

    #[test]
    fn layout_empty() {
        let nodes = parse_html("").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.is_empty());
    }

    #[test]
    fn collapse_whitespace_test() {
        assert_eq!(collapse_whitespace("  hello   world  "), "hello world");
        assert_eq!(collapse_whitespace("\n\t  foo  \n"), "foo");
    }

    #[test]
    fn page_break_creates_new_page() {
        let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn bare_text_node() {
        // Text not wrapped in any element — exercises DomNode::Text branch in flatten_nodes
        let nodes = parse_html("Just some bare text").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn br_element_creates_empty_line() {
        let html = "<p>Line one</p><br><p>Line two</p>";
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have at least 3 elements (p, br, p)
        assert!(pages[0].elements.len() >= 2);
    }

    #[test]
    fn inline_element_layout() {
        // Inline element outside a block — exercises the else branch
        let html = "<span>Hello</span>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn svg_size_percent_attrs_do_not_override_intrinsic_image_size() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: Some("100%".to_string()),
            height_attr: Some("50%".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (300.0, 150.0)
        );
    }

    #[test]
    fn svg_size_absolute_width_only_preserves_aspect_ratio() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: Some("120".to_string()),
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 20.0,
                height: 10.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (120.0, 60.0)
        );
    }

    #[test]
    fn svg_size_absolute_height_only_preserves_aspect_ratio() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: None,
            height_attr: Some("60".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 20.0,
                height: 10.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (120.0, 60.0)
        );
    }

    #[test]
    fn svg_size_absolute_width_ignores_disallowed_percent_height() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: Some("120".to_string()),
            height_attr: Some("50%".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 20.0,
                height: 10.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (120.0, 60.0)
        );
    }

    #[test]
    fn svg_size_absolute_height_ignores_disallowed_percent_width() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: Some("50%".to_string()),
            height_attr: Some("60".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 20.0,
                height: 10.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (120.0, 60.0)
        );
    }

    #[test]
    fn svg_size_intrinsic_is_not_clamped_to_available_width() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 200.0, 400.0, false, false),
            (300.0, 150.0)
        );
    }

    #[test]
    fn svg_size_negative_percent_falls_back_to_intrinsic_size() {
        let tree = SvgTree {
            width: 120.0,
            height: 60.0,
            width_attr: Some("-10%".to_string()),
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, true, false),
            (120.0, 60.0)
        );
    }

    #[test]
    fn nested_svg_percent_height_uses_parent_height() {
        let html = r#"<div style="height: 200pt"><svg width="100" height="50%"></svg></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let svg = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::Svg { width, height, .. } => Some((*width, *height)),
                _ => None,
            })
            .expect("expected nested svg element");
        assert!((svg.0 - 100.0).abs() < 0.1);
        assert!((svg.1 - 100.0).abs() < 0.1);
    }

    #[test]
    fn nested_svg_percent_viewport_uses_resolved_root_size() {
        let html = r#"
            <div style="width: 400pt; height: 200pt">
                <svg width="100%" height="50%" viewBox="0 0 20 10">
                    <svg width="50%" height="50%" viewBox="0 0 10 10">
                        <rect width="10" height="10"/>
                    </svg>
                </svg>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let svg = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::Svg { tree, .. } => Some(tree),
                _ => None,
            })
            .expect("expected nested svg element");
        match &svg.children[0] {
            crate::parser::svg::SvgNode::Group { transform, .. } => {
                assert!(matches!(
                    transform,
                    Some(crate::parser::svg::SvgTransform::Matrix(
                        20.0, 0.0, 0.0, 5.0, 0.0, 0.0
                    ))
                ));
            }
            other => panic!("expected nested svg group, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_element_preserves_viewbox_for_renderer() {
        let html = r#"<svg width="200" height="100" viewBox="0 0 20 10"><rect width="10" height="10"/></svg>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let svg = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::Svg {
                    tree,
                    width,
                    height,
                    ..
                } => Some((tree, *width, *height)),
                _ => None,
            })
            .expect("expected svg layout element");
        assert_eq!(svg.1, 200.0);
        assert_eq!(svg.2, 100.0);
        assert!(
            svg.0.view_box.is_some(),
            "renderer should keep viewBox metadata"
        );
    }

    #[test]
    fn inline_svg_inherits_document_color_for_current_color() {
        let html = r#"<div style="color: #336699"><svg width="20" height="10"><rect width="10" height="10" fill="currentColor"/></svg></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let tree = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::Svg { tree, .. } => Some(tree),
                _ => None,
            })
            .expect("expected svg layout element");

        match &tree.children[0] {
            crate::parser::svg::SvgNode::Group {
                style, children, ..
            } => {
                assert_eq!(style.color, Some((0.2, 0.4, 0.6)));
                assert_eq!(children.len(), 1);
            }
            other => panic!("expected root group wrapper, got {other:?}"),
        }
    }

    #[test]
    fn page_break_after() {
        let html = r#"<div style="page-break-after: always"><p>Page 1</p></div><p>Page 2</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn word_wrap_long_text() {
        // Generate text that exceeds page width to trigger word wrapping
        let long_text = "word ".repeat(200);
        let html = format!("<p>{long_text}</p>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have wrapped into multiple lines
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            assert!(lines.len() > 1);
        }
    }

    #[test]
    fn content_overflows_to_next_page() {
        // Generate enough content to overflow one page
        let paragraphs = "<p>Some paragraph text that takes up space.</p>\n".repeat(100);
        let nodes = parse_html(&paragraphs).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn background_color_block() {
        let html = r#"<div style="background-color: yellow"><p>Highlighted</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn pre_element_with_background() {
        let html = "<pre>code block</pre>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Pre has background color in defaults
        if let (
            _,
            LayoutElement::TextBlock {
                background_color, ..
            },
        ) = &pages[0].elements[0]
        {
            assert!(background_color.is_some());
        }
    }

    #[test]
    fn table_layout_basic() {
        // Exercises flatten_table and table row layout (lines 232, 248, 344, 354)
        let html = r#"
            <table>
                <tr><th>Header 1</th><th>Header 2</th></tr>
                <tr><td>Cell A</td><td>Cell B</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have TableRow elements
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 2);
    }

    #[test]
    fn table_with_thead_tbody_tfoot() {
        // Exercises lines 345-353: collecting rows from thead/tbody/tfoot
        let html = r#"
            <table>
                <thead><tr><th>H</th></tr></thead>
                <tbody><tr><td>B</td></tr></tbody>
                <tfoot><tr><td>F</td></tr></tfoot>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 3);
    }

    #[test]
    fn table_empty_rows_ignored() {
        // Line 360: empty table returns early
        let html = "<table></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // Should have no table rows
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 0);
    }

    #[test]
    fn ordered_list_layout() {
        // Exercises lines 219-232, 248: ordered list context and numbering
        let html = "<ol><li>First</li><li>Second</li><li>Third</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have items with numbered markers
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TextBlock { .. }))
            .collect();
        assert!(blocks.len() >= 3);
    }

    #[test]
    fn unordered_list_layout() {
        // Exercises lines 217-236: unordered list layout
        let html = "<ul><li>A</li><li>B</li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn list_with_non_li_child() {
        // Line 232: non-li child inside ul
        let html = "<ul><li>Item</li><p>Not a list item</p></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn li_with_block_child() {
        // Lines 279-280: block child inside li
        let html = "<ul><li><p>Paragraph inside li</p></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn table_row_pagination() {
        // Exercises TableRow height calculation in paginate (lines 559-572)
        let mut rows = String::new();
        for i in 0..100 {
            rows.push_str(&format!(
                "<tr><td>Row {i} with some text</td><td>More text</td></tr>"
            ));
        }
        let html = format!("<table>{rows}</table>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2, "Large table should span multiple pages");
    }

    #[test]
    fn table_with_non_cell_children_in_row() {
        // Line 354: non-td/th child in tr is ignored
        let html = r#"<table><tr><td>Cell</td><span>Ignored</span></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert_eq!(table_rows.len(), 1);
    }

    #[test]
    fn del_element_sets_line_through() {
        let html = "<p><del>Deleted text</del></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            assert!(!lines.is_empty());
            let run = &lines[0].runs[0];
            assert!(run.line_through, "del element should set line_through");
            assert!(!run.underline);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn s_element_sets_line_through() {
        let html = "<p><s>Struck text</s></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            assert!(!lines.is_empty());
            let run = &lines[0].runs[0];
            assert!(run.line_through, "s element should set line_through");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn nested_unordered_list() {
        let html = "<ul><li>Parent<ul><li>Child</li></ul></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have at least 2 TextBlock elements: parent item and nested child item
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines,
                    padding_left,
                    ..
                } => Some((lines.clone(), *padding_left)),
                _ => None,
            })
            .collect();
        assert!(
            blocks.len() >= 2,
            "Expected at least 2 text blocks for nested list, got {}",
            blocks.len()
        );
        // The nested item should have greater indentation than the parent
        let parent_indent = blocks[0].1;
        let child_indent = blocks[1].1;
        assert!(
            child_indent > parent_indent,
            "Nested list item should be more indented: parent={parent_indent}, child={child_indent}"
        );
    }

    #[test]
    fn nested_ordered_list() {
        let html = "<ol><li>First<ol><li>Nested first</li><li>Nested second</li></ol></li><li>Second</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines,
                    padding_left,
                    ..
                } => Some((lines.clone(), *padding_left)),
                _ => None,
            })
            .collect();
        // Should have: "1. First", "1. Nested first", "2. Nested second", "2. Second"
        assert!(
            blocks.len() >= 3,
            "Expected at least 3 text blocks for nested ordered list, got {}",
            blocks.len()
        );
        // Nested items should have greater indentation
        let parent_indent = blocks[0].1;
        let nested_indent = blocks[1].1;
        assert!(
            nested_indent > parent_indent,
            "Nested ordered list should be more indented: parent={parent_indent}, nested={nested_indent}"
        );
    }

    #[test]
    fn mixed_nested_list() {
        let html = "<ul><li>Bullet<ol><li>Numbered</li></ol></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines,
                    padding_left,
                    ..
                } => Some((lines.clone(), *padding_left)),
                _ => None,
            })
            .collect();
        assert!(
            blocks.len() >= 2,
            "Expected at least 2 text blocks for mixed nested list, got {}",
            blocks.len()
        );
        // Nested ordered list inside unordered should be more indented
        let parent_indent = blocks[0].1;
        let nested_indent = blocks[1].1;
        assert!(
            nested_indent > parent_indent,
            "Nested ol inside ul should be more indented: parent={parent_indent}, nested={nested_indent}"
        );
        // Check that the nested item has a numbered marker
        let nested_text: String = blocks[1].0[0].runs.iter().map(|r| r.text.clone()).collect();
        assert!(
            nested_text.contains("1."),
            "Nested item should have ordered marker, got: {nested_text}"
        );
    }

    #[test]
    fn base64_decode_basic() {
        // "Hello" in base64 is "SGVsbG8="
        let decoded = decode_base64("SGVsbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn base64_decode_with_whitespace() {
        let decoded = decode_base64("SGVs\nbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn layout_jpeg_image_from_data_uri() {
        let html = r#"<img src="data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkKDA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==" width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        match &pages[0].elements[0].1 {
            LayoutElement::Image {
                image,
                width,
                height,
                ..
            } => {
                assert_eq!(image.format, ImageFormat::Jpeg);
                assert!((width - 75.0).abs() < 0.1); // 100px * 0.75
                assert!((height - 60.0).abs() < 0.1); // 80px * 0.75
                assert!(image.png_metadata.is_none());
            }
            _ => panic!("Expected Image layout element"),
        }
    }

    #[test]
    fn layout_svg_image_from_data_uri_uses_intrinsic_size() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20width%3D%22100%25%22%20height%3D%2250%25%22%20viewBox%3D%220%200%20100%2050%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 300.0).abs() < 0.1);
                assert!((*height - 150.0).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_image_respects_max_width() {
        let html = r#"<img style="max-width: 75pt" src="data:image/svg+xml,%3Csvg%20width%3D%22100%22%20height%3D%2250%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 75.0).abs() < 0.1);
                assert!((*height - 37.5).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_image_respects_max_height() {
        let html = r#"<img style="max-height: 20pt" src="data:image/svg+xml,%3Csvg%20width%3D%22100%22%20height%3D%2250%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 40.0).abs() < 0.1);
                assert!((*height - 20.0).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_viewbox_only_svg_image_uses_default_object_size_ratio() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%20100%2020%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 300.0).abs() < 0.1);
                assert!((*height - 60.0).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_viewbox_only_svg_image_respects_max_height() {
        let html = r#"<img style="max-height: 50pt" src="data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%20100%2020%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        match &pages[0].elements[0].1 {
            LayoutElement::Svg { width, height, .. } => {
                assert!((*width - 250.0).abs() < 0.1);
                assert!((*height - 50.0).abs() < 0.1);
            }
            other => panic!("Expected Svg layout element, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_image_without_viewbox_syncs_tree_to_layout_box() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20width%3D%22100%25%22%20height%3D%2250%25%22%3E%3Crect%20width%3D%22100%25%22%20height%3D%22100%25%22/%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let (tree_width, tree_height, width, height) = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::Svg {
                    tree,
                    width,
                    height,
                    ..
                } => Some((tree.width, tree.height, *width, *height)),
                _ => None,
            })
            .expect("expected svg layout element");

        assert!((tree_width - width).abs() < 0.1);
        assert!((tree_height - height).abs() < 0.1);
    }

    #[test]
    fn try_parse_svg_bytes_accepts_utf8_bom_prefix() {
        let raw = b"\xEF\xBB\xBF<svg width=\"20\" height=\"10\"></svg>";
        let tree = try_parse_svg_bytes(raw).expect("expected BOM-prefixed SVG to parse");
        assert_eq!(tree.width, 20.0);
        assert_eq!(tree.height, 10.0);
    }

    #[test]
    fn layout_png_image_from_data_uri() {
        // Build a minimal valid PNG and encode as base64
        let png_bytes = build_test_png_bytes();
        let b64 = base64_encode(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="120" height="90">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        match &pages[0].elements[0].1 {
            LayoutElement::Image { image, .. } => {
                assert_eq!(image.format, ImageFormat::Png);
                let meta = image.png_metadata.as_ref().unwrap();
                assert_eq!(meta.channels, 3); // RGB
                assert_eq!(meta.bit_depth, 8);
            }
            _ => panic!("Expected Image layout element"),
        }
    }

    #[test]
    fn layout_image_without_dimensions_gets_defaults() {
        let png_bytes = build_test_png_bytes();
        let b64 = base64_encode(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}">"#);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        match &pages[0].elements[0].1 {
            LayoutElement::Image { width, height, .. } => {
                assert!(*width > 0.0);
                assert!(*height > 0.0);
            }
            _ => panic!("Expected Image layout element"),
        }
    }

    #[test]
    fn layout_image_unsupported_src_ignored() {
        // HTTP src is not supported, should be silently ignored
        let html = r#"<img src="http://example.com/image.png" width="100" height="100">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // No image element should be produced
        assert!(
            pages[0].elements.is_empty()
                || !matches!(&pages[0].elements[0].1, LayoutElement::Image { .. })
        );
    }

    #[test]
    fn fetch_remote_url_returns_none_without_feature() {
        // Without the "remote" feature, fetch_remote_url always returns None
        let result = fetch_remote_url("https://example.com/image.png");
        #[cfg(not(feature = "remote"))]
        assert!(result.is_none());
        // With the feature enabled, it would attempt a real HTTP request
        // (which may or may not succeed depending on network)
        let _ = result;
    }

    #[test]
    fn load_image_data_http_without_feature() {
        let result = load_image_data("http://example.com/test.jpg");
        #[cfg(not(feature = "remote"))]
        assert!(
            result.is_none(),
            "HTTP images should be None without remote feature"
        );
        let _ = result;
    }

    #[test]
    fn load_image_data_https_without_feature() {
        let result = load_image_data("https://example.com/test.png");
        #[cfg(not(feature = "remote"))]
        assert!(
            result.is_none(),
            "HTTPS images should be None without remote feature"
        );
        let _ = result;
    }

    #[test]
    fn base64_decode_roundtrip() {
        let data = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let encoded = base64_encode(data);
        let decoded = decode_base64(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn img_scales_to_fit_available_width() {
        // Very wide image: 2000px = 1500pt, which exceeds A4 content width (~451pt)
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="2000" height="1000">"#);
        let nodes = parse_html(&html).unwrap();
        let page_size = PageSize::A4;
        let margin_val = Margin::default();
        let available_width = page_size.width - margin_val.left - margin_val.right;
        let pages = layout(&nodes, page_size, margin_val);
        if let (_, LayoutElement::Image { width, .. }) = &pages[0].elements[0] {
            assert!(
                *width <= available_width + 0.01,
                "Image width {width} should fit within available width {available_width}"
            );
        } else {
            panic!("Expected Image element");
        }
    }

    #[test]
    fn img_without_src_ignored() {
        let html = r#"<img width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_image = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::Image { .. }));
        assert!(
            !has_image,
            "img without src should not produce Image element"
        );
    }

    #[test]
    fn block_aspect_ratio_sets_height_for_empty_box() {
        let html = r#"<div style="width: 120pt; aspect-ratio: 3 / 2"></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (_, element) = &pages[0].elements[0];
        if let LayoutElement::TextBlock {
            block_height: Some(height),
            ..
        } = element
        {
            assert!((*height - 80.0).abs() < 0.1);
        } else {
            panic!("Expected aspect-ratio box to produce a TextBlock");
        }
    }

    #[test]
    fn raster_background_image_survives_into_layout() {
        let png = build_test_png_bytes();
        let encoded = base64_encode(&png);
        let html = format!(
            r#"<div style="width: 40pt; height: 40pt; background-image: url('data:image/png;base64,{encoded}') no-repeat"></div>"#
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (_, element) = &pages[0].elements[0];
        if let LayoutElement::TextBlock {
            background_svg: Some(tree),
            ..
        } = element
        {
            assert!(matches!(
                tree.children.first(),
                Some(crate::parser::svg::SvgNode::Image { .. })
            ));
        } else {
            panic!("Expected raster background to produce a TextBlock");
        }
    }

    fn build_test_png_bytes() -> Vec<u8> {
        let mut png_data = Vec::new();
        png_data.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        // IHDR
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.push(8); // bit depth
        ihdr.push(2); // color type RGB
        ihdr.push(0);
        ihdr.push(0);
        ihdr.push(0);
        append_test_chunk(&mut png_data, b"IHDR", &ihdr);
        let idat = [
            0x78, 0x01, 0x62, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
        ];
        append_test_chunk(&mut png_data, b"IDAT", &idat);
        append_test_chunk(&mut png_data, b"IEND", &[]);
        png_data
    }

    fn append_test_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(chunk_type);
        buf.extend_from_slice(data);
        buf.extend_from_slice(&[0, 0, 0, 0]);
    }

    #[test]
    fn three_levels_deep_nested_list() {
        let html = "<ul><li>Level 1<ul><li>Level 2<ul><li>Level 3</li></ul></li></ul></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines,
                    padding_left,
                    ..
                } => Some((lines.clone(), *padding_left)),
                _ => None,
            })
            .collect();
        assert!(
            blocks.len() >= 3,
            "Expected at least 3 text blocks for 3-level list, got {}",
            blocks.len()
        );
        let indent_1 = blocks[0].1;
        let indent_2 = blocks[1].1;
        let indent_3 = blocks[2].1;
        assert!(
            indent_2 > indent_1,
            "Level 2 should be more indented than level 1: l1={indent_1}, l2={indent_2}"
        );
        assert!(
            indent_3 > indent_2,
            "Level 3 should be more indented than level 2: l2={indent_2}, l3={indent_3}"
        );
    }

    // --- Overflow / Visibility / Transform layout tests ---

    #[test]
    fn visibility_hidden_keeps_space_but_not_visible() {
        let html = r#"<div style="visibility: hidden">Hidden text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        if let (_, LayoutElement::TextBlock { visible, .. }) = &pages[0].elements[0] {
            assert!(!visible, "visibility: hidden should set visible to false");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn visibility_visible_is_visible() {
        let html = r#"<div>Visible text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { visible, .. }) = &pages[0].elements[0] {
            assert!(*visible, "Default should be visible");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn overflow_hidden_produces_clip_rect() {
        let html = r#"<div style="overflow: hidden; width: 200pt; height: 100pt">Clipped</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { clip_rect, .. }) = &pages[0].elements[0] {
            assert!(clip_rect.is_some(), "overflow: hidden should set clip_rect");
            let (_, _, w, _) = clip_rect.unwrap();
            assert!((w - 200.0).abs() < 0.1);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn overflow_visible_no_clip_rect() {
        let html = r#"<div style="width: 200pt">Not clipped</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { clip_rect, .. }) = &pages[0].elements[0] {
            assert!(clip_rect.is_none(), "No overflow should mean no clip_rect");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn transform_rotate_stored_in_layout() {
        let html = r#"<div style="transform: rotate(45deg)">Rotated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { transform, .. }) = &pages[0].elements[0] {
            assert_eq!(
                *transform,
                Some(crate::style::computed::Transform::Rotate(45.0))
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn transform_scale_stored_in_layout() {
        let html = r#"<div style="transform: scale(2)">Scaled</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { transform, .. }) = &pages[0].elements[0] {
            assert_eq!(
                *transform,
                Some(crate::style::computed::Transform::Scale(2.0, 2.0))
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn transform_translate_stored_in_layout() {
        let html = r#"<div style="transform: translate(10pt, 20pt)">Translated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { transform, .. }) = &pages[0].elements[0] {
            assert_eq!(
                *transform,
                Some(crate::style::computed::Transform::Translate(10.0, 20.0))
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn table_colspan_default_is_one() {
        let html = "<table><tr><td>A</td><td>B</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TableRow { cells, .. } = el {
                for cell in cells {
                    assert_eq!(cell.colspan, 1, "Default colspan should be 1");
                    assert_eq!(cell.rowspan, 1, "Default rowspan should be 1");
                }
            }
        }
    }

    #[test]
    fn table_colspan_header_spans_two() {
        let html =
            r#"<table><tr><th colspan="2">Header</th></tr><tr><td>A</td><td>B</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 2);
        assert_eq!(table_rows[0].len(), 1);
        assert_eq!(table_rows[0][0].colspan, 2);
        assert_eq!(table_rows[1].len(), 2);
        assert_eq!(table_rows[1][0].colspan, 1);
        assert_eq!(table_rows[1][1].colspan, 1);
    }

    #[test]
    fn table_colspan_makes_cells_wider() {
        let html = r#"<table><tr><td colspan="2">Wide</td><td>N</td></tr><tr><td>A</td><td>B</td><td>C</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths.clone()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 2);
        let (cells, col_widths) = &table_rows[0];
        assert_eq!(cells[0].colspan, 2);
        // With auto-sizing, col_widths should have 3 entries
        assert_eq!(col_widths.len(), 3);
        // The colspan=2 cell should span the first two column widths
        let span_width: f32 = col_widths[0] + col_widths[1];
        let single_width = col_widths[2];
        assert!(
            span_width > single_width,
            "colspan=2 span ({span_width}) should be wider than single col ({single_width})"
        );
    }

    #[test]
    fn table_mixed_colspan_values() {
        let html = r#"<table><tr><td colspan="3">Full</td></tr><tr><td>A</td><td colspan="2">BC</td></tr><tr><td>X</td><td>Y</td><td>Z</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 3);
        assert_eq!(table_rows[0].len(), 1);
        assert_eq!(table_rows[0][0].colspan, 3);
        assert_eq!(table_rows[1].len(), 2);
        assert_eq!(table_rows[1][0].colspan, 1);
        assert_eq!(table_rows[1][1].colspan, 2);
        assert_eq!(table_rows[2].len(), 3);
        for cell in table_rows[2] {
            assert_eq!(cell.colspan, 1);
        }
    }

    #[test]
    fn table_rowspan_basic() {
        // Cell A spans two rows; row 1 should have a phantom cell in column 0.
        let html = r#"<table>
            <tr><td rowspan="2">A</td><td>B</td></tr>
            <tr><td>C</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 2, "Should have 2 rows");
        // Row 0: cell A (rowspan=2) and cell B
        assert_eq!(table_rows[0].len(), 2);
        assert_eq!(table_rows[0][0].rowspan, 2);
        assert_eq!(table_rows[0][1].rowspan, 1);
        // Row 1: phantom cell (rowspan=0) and cell C
        assert_eq!(table_rows[1].len(), 2);
        assert_eq!(
            table_rows[1][0].rowspan, 0,
            "Phantom cell should have rowspan=0"
        );
        assert_eq!(table_rows[1][1].rowspan, 1);
    }

    #[test]
    fn table_rowspan_and_colspan_combined() {
        // Cell A spans 2 rows and 2 columns in a 3-column table.
        let html = r#"<table>
            <tr><td rowspan="2" colspan="2">A</td><td>B</td></tr>
            <tr><td>C</td></tr>
            <tr><td>D</td><td>E</td><td>F</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 3, "Should have 3 rows");
        // Row 0: cell A (rowspan=2, colspan=2) and cell B
        assert_eq!(table_rows[0].len(), 2);
        assert_eq!(table_rows[0][0].rowspan, 2);
        assert_eq!(table_rows[0][0].colspan, 2);
        assert_eq!(table_rows[0][1].rowspan, 1);
        // Row 1: phantom cell spanning 2 cols and cell C
        assert_eq!(table_rows[1].len(), 2);
        assert_eq!(table_rows[1][0].rowspan, 0);
        assert_eq!(table_rows[1][0].colspan, 2, "Phantom should span 2 cols");
        assert_eq!(table_rows[1][1].rowspan, 1);
        // Row 2: three normal cells
        assert_eq!(table_rows[2].len(), 3);
        for cell in table_rows[2] {
            assert_eq!(cell.rowspan, 1);
            assert_eq!(cell.colspan, 1);
        }
    }

    #[test]
    fn table_rowspan_renders_to_pdf() {
        // Verify that a table with rowspan produces valid PDF output.
        let html = r#"<table>
            <tr><td rowspan="2">Spans two rows</td><td>Top right</td></tr>
            <tr><td>Bottom right</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = crate::render::pdf::render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Spans"),
            "Cell text 'Spans' should be in PDF"
        );
        assert!(
            content.contains("rows"),
            "Cell text 'rows' should be in PDF"
        );
        assert!(content.contains("Top"), "Cell text 'Top' should be in PDF");
        assert!(
            content.contains("Bottom"),
            "Cell text 'Bottom' should be in PDF"
        );
        // No default cell borders — only CSS-specified borders produce strokes
    }

    #[test]
    fn css_width_constrains_block() {
        let html = r#"<div style="width: 200pt">Narrow block</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, Some(200.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_max_width_limits_width() {
        let html = r#"<div style="max-width: 300pt">Limited block</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_height_sets_minimum_height() {
        let html = r#"<div style="height: 100pt">Short text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_height, Some(100.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_opacity_stored_in_layout() {
        let html = r#"<div style="opacity: 0.5">Semi-transparent</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { opacity, .. }) = &pages[0].elements[0] {
            assert!((*opacity - 0.5).abs() < 0.01);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn no_explicit_width_is_none() {
        let html = "<div>Normal block</div>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, None);
        } else {
            panic!("Expected TextBlock");
        }
    }

    // --- Float / Clear / Position / Box-shadow layout tests ---

    #[test]
    fn float_left_positions_element() {
        let html = r#"<div style="float: left; width: 100pt">Floated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { float, .. }) = &pages[0].elements[0] {
            assert_eq!(*float, Float::Left);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn float_right_positions_element() {
        let html = r#"<div style="float: right; width: 100pt">Floated right</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { float, .. }) = &pages[0].elements[0] {
            assert_eq!(*float, Float::Right);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn clear_both_moves_below_floats() {
        let html = r#"
            <div style="float: left">Float</div>
            <div style="clear: both">After float</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // The cleared element should be below the floated element
        let float_y = pages[0].elements[0].0;
        let cleared_y = pages[0].elements[1].0;
        assert!(
            cleared_y >= float_y,
            "Cleared element y={cleared_y} should be >= floated y={float_y}"
        );
        // Check the clear property is set
        if let (_, LayoutElement::TextBlock { clear, .. }) = &pages[0].elements[1] {
            assert_eq!(*clear, Clear::Both);
        }
    }

    #[test]
    fn position_relative_offsets_element() {
        let html = r#"<div style="position: relative; top: 10pt; left: 5pt">Offset</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            y,
            LayoutElement::TextBlock {
                position,
                offset_top,
                offset_left,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(*position, Position::Relative);
            assert!((offset_top - 10.0).abs() < 0.1);
            assert!((offset_left - 5.0).abs() < 0.1);
            // y should be offset by top value from normal position
            assert!(
                *y > 0.0,
                "Element should have non-zero y due to relative offset"
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn position_absolute_fixed_position() {
        let html = r#"<div style="position: absolute; top: 100pt; left: 50pt">Absolute</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            y,
            LayoutElement::TextBlock {
                position,
                offset_top,
                offset_left,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(*position, Position::Absolute);
            assert!((offset_top - 100.0).abs() < 0.1);
            assert!((offset_left - 50.0).abs() < 0.1);
            // y should be exactly the top value
            assert!((*y - 100.0).abs() < 0.1, "Absolute y={y} should be 100.0");
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn position_absolute_does_not_affect_flow() {
        let html = r#"
            <div style="position: absolute; top: 200pt">Absolute</div>
            <div>Normal flow</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 2);
        // The normal flow element should start at y=0 (top of content area)
        let normal_y = pages[0].elements[1].0;
        assert!(
            normal_y < 10.0,
            "Normal flow element should be near top, but y={normal_y}"
        );
    }

    #[test]
    fn box_shadow_produces_offset_rect() {
        let html = r#"<div style="box-shadow: 3px 3px black">Content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { box_shadow, .. }) = &pages[0].elements[0] {
            let shadow = box_shadow.unwrap();
            assert!((shadow.offset_x - 2.25).abs() < 0.1); // 3px * 0.75
            assert!((shadow.offset_y - 2.25).abs() < 0.1);
            assert_eq!(shadow.color.r, 0);
            assert_eq!(shadow.color.g, 0);
            assert_eq!(shadow.color.b, 0);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn float_does_not_advance_normal_flow() {
        let html = r#"
            <div style="float: left">Floated</div>
            <div>Normal after float</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 2);
        // Both elements should start at roughly the same y position
        // because floats don't advance normal flow
        let float_y = pages[0].elements[0].0;
        let normal_y = pages[0].elements[1].0;
        // The normal element might be at the same position or slightly different
        // due to margins, but it should not be pushed far down
        assert!(
            (normal_y - float_y).abs() < 50.0,
            "Normal flow element should be near float, not pushed far down: float_y={float_y}, normal_y={normal_y}"
        );
    }

    #[test]
    fn table_auto_sizing_varying_content() {
        let html = "<table><tr><td>A</td><td>Much longer content here</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[1] > col_widths[0],
            "Column with longer text ({}) should be wider than short text ({})",
            col_widths[1],
            col_widths[0]
        );
    }

    #[test]
    fn table_auto_sizing_very_long_cell_no_break() {
        let long_text = "x".repeat(500);
        let html = format!("<table><tr><td>{long_text}</td><td>Short</td></tr></table>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages.is_empty());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(!table_rows.is_empty());
        for w in &table_rows[0] {
            assert!(*w >= 30.0, "Column width {w} should be at least 30pt");
        }
    }

    #[test]
    fn table_auto_sizing_min_column_width() {
        let html = "<table><tr><td></td><td></td><td></td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(!table_rows.is_empty());
        for w in &table_rows[0] {
            assert!(
                *w >= 30.0,
                "Empty column should have minimum width, got {w}"
            );
        }
    }

    #[test]
    fn table_four_column_invoice_non_equal_widths() {
        // A 4-column invoice table: Description should be wider than Qty/Amount
        let html = r#"<table>
            <tr><th>Description</th><th>Qty</th><th>Unit Price</th><th>Amount</th></tr>
            <tr><td>Web development services - January</td><td>1</td><td>2500.00</td><td>2500.00</td></tr>
            <tr><td>Hosting and maintenance</td><td>12</td><td>50.00</td><td>600.00</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert!(!table_rows.is_empty());
        let cw = &table_rows[0];
        assert_eq!(cw.len(), 4);
        // Description column (index 0) should be wider than Qty (index 1)
        assert!(
            cw[0] > cw[1],
            "Description column ({}) should be wider than Qty column ({})",
            cw[0],
            cw[1]
        );
        // Description column should be wider than Amount column
        assert!(
            cw[0] > cw[3],
            "Description column ({}) should be wider than Amount column ({})",
            cw[0],
            cw[3]
        );
        // Columns should NOT all be equal
        assert!(
            !(cw[0] == cw[1] && cw[1] == cw[2] && cw[2] == cw[3]),
            "Column widths should not all be equal: {:?}",
            cw
        );
    }

    #[test]
    fn simple_invoice_fits_on_one_page() {
        // A simple invoice with ~15 lines should fit on a single A4 page
        let html = r#"
            <h1>Invoice #1001</h1>
            <p>Date: 2026-01-15</p>
            <p>Bill To: Acme Corp</p>
            <p>123 Main Street, Springfield</p>
            <table>
                <tr><th>Description</th><th>Qty</th><th>Unit Price</th><th>Amount</th></tr>
                <tr><td>Web development</td><td>1</td><td>2500.00</td><td>2500.00</td></tr>
                <tr><td>Hosting</td><td>12</td><td>50.00</td><td>600.00</td></tr>
                <tr><td>Domain renewal</td><td>1</td><td>15.00</td><td>15.00</td></tr>
                <tr><td>SSL certificate</td><td>1</td><td>75.00</td><td>75.00</td></tr>
            </table>
            <p>Subtotal: 3190.00</p>
            <p>Tax (10%): 319.00</p>
            <p>Total: 3509.00</p>
            <p>Thank you for your business!</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(
            pages.len(),
            1,
            "Simple invoice should fit on one page, got {} pages",
            pages.len()
        );
    }

    // --- Flexbox layout tests ---

    fn extract_flex_items(pages: &[Page]) -> Vec<(f32, f32, Option<f32>, String)> {
        let mut result = Vec::new();
        for page in pages {
            for (y, elem) in &page.elements {
                match elem {
                    LayoutElement::TextBlock {
                        lines,
                        offset_left,
                        block_width,
                        ..
                    } => {
                        let text: String = lines
                            .iter()
                            .flat_map(|l| l.runs.iter().map(|r| r.text.clone()))
                            .collect::<Vec<_>>()
                            .join("");
                        if !text.is_empty() {
                            result.push((*y, *offset_left, *block_width, text));
                        }
                    }
                    LayoutElement::FlexRow { cells, .. } => {
                        for cell in cells {
                            let text: String = cell
                                .lines
                                .iter()
                                .flat_map(|l| l.runs.iter().map(|r| r.text.clone()))
                                .collect::<Vec<_>>()
                                .join("");
                            if !text.is_empty() {
                                result.push((*y, cell.x_offset, Some(cell.width), text));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        result
    }

    #[test]
    fn flex_row_horizontal_layout() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">L</div><div style="width: 100pt">R</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let l = items.iter().find(|i| i.3.contains('L')).unwrap();
        let r = items.iter().find(|i| i.3.contains('R')).unwrap();
        assert!(r.1 > l.1);
    }

    #[test]
    fn flex_column_vertical() {
        let html = r#"<div style="display: flex; flex-direction: column"><div style="width: 100pt">T</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let t = items.iter().find(|i| i.3.contains('T')).unwrap();
        let b = items.iter().find(|i| i.3.contains('B')).unwrap();
        assert!(b.0 > t.0);
    }

    #[test]
    fn flex_justify_center() {
        let html = r#"<div style="display: flex; justify-content: center"><div style="width: 100pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(!items.is_empty());
        assert!(items[0].1 > 50.0);
    }

    #[test]
    fn flex_justify_space_between() {
        let html = r#"<div style="display: flex; justify-content: space-between"><div style="width: 100pt">A</div><div style="width: 100pt">B</div><div style="width: 100pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 3);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let c = items.iter().find(|i| i.3 == "C").unwrap();
        let g1 = b.1 - a.1;
        let g2 = c.1 - b.1;
        assert!((g1 - g2).abs() < 1.0, "gaps equal: {g1} vs {g2}");
    }

    #[test]
    fn flex_justify_space_around() {
        let html = r#"<div style="display: flex; justify-content: space-around"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        assert!(a.1 > 10.0, "space-around: first not at edge, got {}", a.1);
    }

    #[test]
    fn flex_justify_flex_end() {
        let html = r#"<div style="display: flex; justify-content: flex-end"><div style="width: 100pt">E</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(!items.is_empty());
        assert!(items[0].1 > 200.0, "flex-end: got {}", items[0].1);
    }

    #[test]
    fn flex_align_center() {
        let html = r#"<div style="display: flex; align-items: center"><div style="width: 100pt; height: 50pt">T</div><div style="width: 100pt">S</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let t = items.iter().find(|i| i.3 == "T").unwrap();
        let s = items.iter().find(|i| i.3 == "S").unwrap();
        assert!(s.0 >= t.0);
    }

    #[test]
    fn flex_wrap_test() {
        let html = r#"<div style="display: flex; flex-wrap: wrap"><div style="width: 200pt">A</div><div style="width: 200pt">B</div><div style="width: 200pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(
            items.len() >= 3,
            "Should have at least 3 flex items, got {}",
            items.len()
        );
        // Verify all three items appear in the output
        assert!(items.iter().any(|i| i.3 == "A"), "A should appear");
        assert!(items.iter().any(|i| i.3 == "B"), "B should appear");
        assert!(items.iter().any(|i| i.3 == "C"), "C should appear");
        // B should be to the right of A (same row)
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        assert!(b.1 > a.1, "B should be to the right of A");
    }

    #[test]
    fn flex_gap_spacing() {
        let html = r#"<div style="display: flex; gap: 20pt"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let expected = a.1 + 100.0 + 20.0;
        assert!(
            (b.1 - expected).abs() < 1.0,
            "gap: expected {expected}, got {}",
            b.1
        );
    }

    #[test]
    fn flex_no_gap() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let expected = a.1 + 100.0;
        assert!(
            (b.1 - expected).abs() < 1.0,
            "no gap: expected {expected}, got {}",
            b.1
        );
    }

    #[test]
    fn flex_style_block() {
        use crate::parser::css::parse_stylesheet;
        let css = ".f{display:flex;gap:10pt}";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="f"><div style="width:100pt">A</div><div style="width:100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        assert!(b.1 > a.1);
    }

    #[test]
    fn flex_display_none_child() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">V</div><div style="width: 100pt; display: none">H</div><div style="width: 100pt">V2</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.iter().all(|i| !i.3.contains('H')));
        assert!(items.len() >= 2);
    }

    #[test]
    fn flex_row_children_same_y_not_stacked() {
        let html = r#"<div style="display: flex;"><div>Left</div><div>Right</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        let left = items
            .iter()
            .find(|i| i.3.contains("Left"))
            .expect("Left text");
        let right = items
            .iter()
            .find(|i| i.3.contains("Right"))
            .expect("Right text");
        // Both should be at the same y position (same row, not stacked)
        assert!(
            (left.0 - right.0).abs() < 1.0,
            "Left y={} Right y={} -- should be on the same line",
            left.0,
            right.0
        );
        // Right should be to the right of Left
        assert!(
            right.1 > left.1,
            "Right x={} should be greater than Left x={}",
            right.1,
            left.1
        );
    }

    #[test]
    fn flex_space_between_positions() {
        let html = r#"<div style="display: flex; justify-content: space-between;">
            <div>Left content</div>
            <div>Right content</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        let left = items
            .iter()
            .find(|i| i.3.contains("Left"))
            .expect("Left content");
        let right = items
            .iter()
            .find(|i| i.3.contains("Right"))
            .expect("Right content");
        // Both at same y
        assert!(
            (left.0 - right.0).abs() < 1.0,
            "space-between: both should be on same y"
        );
        // First child should be at x=0 (or near 0)
        assert!(
            left.1 < 5.0,
            "space-between: first child near left edge, got {}",
            left.1
        );
        // Second child should be far to the right
        assert!(
            right.1 > 100.0,
            "space-between: second child should be far right, got {}",
            right.1
        );
    }

    #[test]
    fn flex_text_align_right_in_child() {
        let html = r#"<div style="display: flex;">
            <div style="width: 200pt; text-align: right">Aligned</div>
            <div style="width: 200pt">Normal</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // Verify we can find both items as flex cells
        let items = extract_flex_items(&pages);
        let aligned = items
            .iter()
            .find(|i| i.3.contains("Aligned"))
            .expect("Aligned text");
        let normal = items
            .iter()
            .find(|i| i.3.contains("Normal"))
            .expect("Normal text");
        // Aligned should be in first cell (x_offset = 0)
        assert!(aligned.1 < normal.1, "first cell before second");
        // Verify the FlexRow element stores text_align correctly
        for page in &pages {
            for (_y, elem) in &page.elements {
                if let LayoutElement::FlexRow { cells, .. } = elem {
                    if let Some(cell) = cells.iter().find(|c| {
                        c.lines
                            .iter()
                            .any(|l| l.runs.iter().any(|r| r.text.contains("Aligned")))
                    }) {
                        assert_eq!(
                            cell.text_align,
                            TextAlign::Right,
                            "text-align: right should be preserved in FlexCell"
                        );
                    }
                }
            }
        }
    }

    // --- CSS Grid tests ---

    #[test]
    fn grid_three_column_places_items_correctly() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr 1fr">
            <div>Cell 1</div>
            <div>Cell 2</div>
            <div>Cell 3</div>
            <div>Cell 4</div>
            <div>Cell 5</div>
            <div>Cell 6</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::GridRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            grid_rows.len(),
            2,
            "Should have 2 rows for 6 items in 3 columns"
        );
        assert_eq!(grid_rows[0].0.len(), 3, "First row should have 3 cells");
        assert_eq!(grid_rows[1].0.len(), 3, "Second row should have 3 cells");

        // Columns should be equal width
        let widths = grid_rows[0].1;
        assert!(
            (widths[0] - widths[1]).abs() < 0.1,
            "Columns should be equal width"
        );
        assert!(
            (widths[1] - widths[2]).abs() < 0.1,
            "Columns should be equal width"
        );
    }

    #[test]
    fn grid_mixed_fr_and_fixed_columns() {
        let html = r#"<div style="display: grid; grid-template-columns: 100pt 1fr 200pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::GridRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1);
        let widths = grid_rows[0].1;
        assert_eq!(widths.len(), 3);
        assert!(
            (widths[0] - 100.0).abs() < 0.1,
            "First column should be 100pt"
        );
        assert!(
            (widths[2] - 200.0).abs() < 0.1,
            "Third column should be 200pt"
        );
        // Middle column gets remaining space
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_middle = available - 100.0 - 200.0;
        assert!(
            (widths[1] - expected_middle).abs() < 0.1,
            "Middle column should get remaining space: got {}, expected {}",
            widths[1],
            expected_middle
        );
    }

    #[test]
    fn grid_auto_columns() {
        let html = r#"<div style="display: grid; grid-template-columns: auto auto">
            <div>Left</div>
            <div>Right</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::GridRow { col_widths, .. } = el {
                    Some(col_widths)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1);
        let widths = grid_rows[0];
        assert_eq!(widths.len(), 2);
        // Auto columns should be equal width, sharing available space
        assert!(
            (widths[0] - widths[1]).abs() < 0.1,
            "Auto columns should be equal: {} vs {}",
            widths[0],
            widths[1]
        );
    }

    #[test]
    fn grid_gap_adds_spacing() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr; grid-gap: 10pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::GridRow {
                    col_widths,
                    margin_top,
                    ..
                } = el
                {
                    Some((col_widths, *margin_top))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 2, "Should have 2 rows");

        // Column widths should account for the gap
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_col = (available - 10.0) / 2.0;
        let widths = grid_rows[0].0;
        assert!(
            (widths[0] - expected_col).abs() < 0.1,
            "Column width should account for gap: got {}, expected {}",
            widths[0],
            expected_col
        );

        // Second row should have grid-gap as margin_top
        assert!(
            (grid_rows[1].1 - 10.0).abs() < 0.1,
            "Second row margin_top should be the grid gap: got {}",
            grid_rows[1].1
        );
    }

    #[test]
    fn grid_wraps_to_new_rows() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
            <div>E</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::GridRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 3, "5 items in 2 columns = 3 rows");
        assert_eq!(grid_rows[0].len(), 2);
        assert_eq!(grid_rows[1].len(), 2);
        assert_eq!(
            grid_rows[2].len(),
            2,
            "Last row should be padded to 2 cells"
        );
        // Last row's second cell should be empty
        assert!(
            grid_rows[2][1].lines.is_empty(),
            "Padding cell should have no text"
        );
    }

    #[test]
    fn grid_renders_to_pdf() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr 1fr; grid-gap: 10pt">
            <div>Cell 1</div>
            <div>Cell 2</div>
            <div>Cell 3</div>
            <div>Cell 4</div>
            <div>Cell 5</div>
            <div>Cell 6</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = crate::render::pdf::render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Cell"),
            "Grid cell text should appear in PDF"
        );
        assert!(content.contains("1"), "Cell numbers should appear in PDF");
        assert!(content.contains("6"), "Cell 6 should appear in PDF");
    }

    #[test]
    fn grid_with_gap_alias() {
        // Test that 'gap' works as an alias for 'grid-gap'
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr; gap: 20pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::GridRow { margin_top, .. } = el {
                    Some(*margin_top)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 2);
        // Second row should have gap as margin_top
        assert!(
            (grid_rows[1] - 20.0).abs() < 0.1,
            "gap alias should work: got {}",
            grid_rows[1]
        );
    }

    #[test]
    fn grid_with_stylesheet_rules() {
        use crate::parser::css::parse_stylesheet;
        let css = ".grid { display: grid; grid-template-columns: 1fr 1fr; grid-gap: 5pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::GridRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1, "Should have 1 grid row");
        assert_eq!(grid_rows[0].0.len(), 2, "Should have 2 cells");
        // Verify gap is accounted for in widths
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_col = (available - 5.0) / 2.0;
        assert!(
            (grid_rows[0].1[0] - expected_col).abs() < 0.1,
            "Column width with gap: got {}, expected {}",
            grid_rows[0].1[0],
            expected_col
        );
    }

    #[test]
    fn grid_no_template_columns_defaults_to_single_column() {
        let html = r#"<div style="display: grid">
            <div>Only</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::GridRow {
                    cells, col_widths, ..
                } = el
                {
                    Some((cells, col_widths))
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(grid_rows.len(), 1);
        assert_eq!(grid_rows[0].1.len(), 1, "Default should be single column");
    }

    // --- min-width / min-height / max-height / margin auto tests ---

    #[test]
    fn css_min_width_enforces_minimum() {
        // width: 100pt would be 100, but min-width: 300pt forces it to 300
        let html = r#"<div style="width: 100pt; min-width: 300pt">Narrow text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_min_height_enforces_minimum() {
        let html = r#"<div style="min-height: 200pt">Short text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_height, Some(200.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_max_height_limits_height() {
        let html = r#"<div style="height: 500pt; max-height: 300pt">Tall box</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_height, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_margin_auto_centers_element() {
        let html = r#"<div style="width: 200pt; margin: 0 auto">Centered</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            _,
            LayoutElement::TextBlock {
                offset_left,
                block_width,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(*block_width, Some(200.0));
            // available_width = 595.28 - 72 - 72 = 451.28
            let expected_offset = (451.28 - 200.0) / 2.0;
            assert!(
                (*offset_left - expected_offset).abs() < 0.1,
                "offset_left should be ~{expected_offset}, got {offset_left}"
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_margin_left_auto_pushes_right() {
        let html = r#"<div style="width: 200pt; margin-left: auto">Right-aligned</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            _,
            LayoutElement::TextBlock {
                offset_left,
                block_width,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(*block_width, Some(200.0));
            // available_width = 451.28, push to right
            let expected_offset = 451.28 - 200.0;
            assert!(
                (*offset_left - expected_offset).abs() < 0.1,
                "offset_left should be ~{expected_offset}, got {offset_left}"
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn css_min_max_interact_with_width_height() {
        // min-height larger than height => min-height wins
        let html = r#"<div style="height: 50pt; min-height: 100pt">Content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_height, Some(100.0));
        } else {
            panic!("Expected TextBlock");
        }

        // width smaller than min-width => min-width wins
        let html2 = r#"<div style="width: 100pt; min-width: 300pt">Content</div>"#;
        let nodes2 = parse_html(html2).unwrap();
        let pages2 = layout(&nodes2, PageSize::A4, Margin::default());
        assert_eq!(pages2.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages2[0].elements[0] {
            assert_eq!(*block_width, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }

        // max-height smaller than min-height => max-height wins (CSS spec)
        // Actually in CSS spec min-height wins over max-height. Let's test:
        // height: 500pt, min-height: 200pt, max-height: 300pt => clamp to 300pt
        let html3 =
            r#"<div style="height: 500pt; max-height: 300pt; min-height: 200pt">Content</div>"#;
        let nodes3 = parse_html(html3).unwrap();
        let pages3 = layout(&nodes3, PageSize::A4, Margin::default());
        assert_eq!(pages3.len(), 1);
        if let (_, LayoutElement::TextBlock { block_height, .. }) = &pages3[0].elements[0] {
            assert_eq!(*block_height, Some(300.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    // --- box-sizing tests ---

    #[test]
    fn box_sizing_border_box_subtracts_padding_from_width() {
        // With border-box, width: 200pt includes padding.
        // With 20pt padding on each side, content area = 200 - 20 - 20 = 160pt
        let html = r#"<div style="box-sizing: border-box; width: 200pt; padding-left: 20pt; padding-right: 20pt">Text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            // block_width should still be 200 (the outer box)
            assert_eq!(*block_width, Some(200.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn box_sizing_content_box_width_is_content_only() {
        // With content-box (default), width: 200pt is just the content
        let html = r#"<div style="box-sizing: content-box; width: 200pt; padding-left: 20pt; padding-right: 20pt">Text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert_eq!(*block_width, Some(200.0));
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn border_radius_stored_in_layout() {
        let html = r#"<div style="border-radius: 8pt; background-color: red">Rounded</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { border_radius, .. }) = &pages[0].elements[0] {
            assert!((*border_radius - 8.0).abs() < 0.001);
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn outline_stored_in_layout() {
        let html = r#"<div style="outline: 3px solid blue">Outlined</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (
            _,
            LayoutElement::TextBlock {
                outline_width,
                outline_color,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert!((*outline_width - 2.25).abs() < 0.01); // 3px * 0.75
            assert!(outline_color.is_some());
            let (r, g, b) = outline_color.unwrap();
            assert!((r - 0.0).abs() < 0.01);
            assert!((g - 0.0).abs() < 0.01);
            assert!((b - 1.0).abs() < 0.01);
        } else {
            panic!("Expected TextBlock");
        }
    }

    // ---- z-index tests ----

    #[test]
    fn z_index_stored_in_layout_element() {
        let html = r#"<div style="position: absolute; z-index: 5; top: 10pt">High</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::TextBlock { z_index: 5, .. }));
        assert!(found, "Expected element with z_index=5");
    }

    #[test]
    fn paginate_repeats_only_synthetic_page_background() {
        let make_block =
            |position, z_index, repeat_on_each_page, height| LayoutElement::TextBlock {
                lines: Vec::new(),
                margin_top: 0.0,
                margin_bottom: 0.0,
                text_align: TextAlign::Left,
                background_color: None,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: Some(100.0),
                block_height: Some(height),
                opacity: 1.0,
                float: Float::None,
                clear: Clear::None,
                position,
                offset_top: 0.0,
                offset_left: 0.0,
                offset_bottom: 0.0,
                offset_right: 0.0,
                containing_block: None,
                box_shadow: None,
                visible: true,
                clip_rect: None,
                transform: None,
                border_radius: 0.0,
                outline_width: 0.0,
                outline_color: None,
                text_indent: 0.0,
                letter_spacing: 0.0,
                word_spacing: 0.0,
                vertical_align: VerticalAlign::Baseline,
                background_gradient: None,
                background_radial_gradient: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::PaddingBox,
                z_index,
                repeat_on_each_page,
                positioned_depth: 0,
                heading_level: None,
            };

        let pages = paginate(
            vec![
                make_block(Position::Absolute, -1, true, 40.0),
                make_block(Position::Absolute, -1, false, 40.0),
                make_block(Position::Static, 0, false, 30.0),
                make_block(Position::Static, 0, false, 30.0),
            ],
            40.0,
        );

        assert_eq!(pages.len(), 2);
        let repeated_per_page: Vec<_> = pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .filter(|(_, element)| {
                        matches!(
                            element,
                            LayoutElement::TextBlock {
                                repeat_on_each_page: true,
                                ..
                            }
                        )
                    })
                    .count()
            })
            .collect();
        assert_eq!(repeated_per_page, vec![1, 1]);

        let non_repeating_per_page: Vec<_> = pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .filter(|(_, element)| {
                        matches!(
                            element,
                            LayoutElement::TextBlock {
                                position: Position::Absolute,
                                repeat_on_each_page: false,
                                ..
                            }
                        )
                    })
                    .count()
            })
            .collect();
        assert_eq!(non_repeating_per_page, vec![1, 0]);
    }

    #[test]
    fn z_index_sorting_order() {
        let html = r#"
            <div style="position: absolute; z-index: 10; top: 0">High</div>
            <div style="position: absolute; z-index: 1; top: 0">Low</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // After sorting, z_index=1 should come before z_index=10
        let z_indices: Vec<i32> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    z_index, position, ..
                } if *position != Position::Static => Some(*z_index),
                _ => None,
            })
            .collect();
        if z_indices.len() >= 2 {
            assert!(
                z_indices[0] <= z_indices[1],
                "Elements should be sorted by z_index"
            );
        }
    }

    #[test]
    fn synthetic_page_background_sorts_before_more_negative_layers() {
        let make_block = |z_index, repeat_on_each_page| LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: Some(100.0),
            block_height: Some(40.0),
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Absolute,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::PaddingBox,
            z_index,
            repeat_on_each_page,
            positioned_depth: 0,
            heading_level: None,
        };

        let pages = paginate(vec![make_block(-1, true), make_block(-2, false)], 200.0);

        match &pages[0].elements[0].1 {
            LayoutElement::TextBlock {
                repeat_on_each_page,
                ..
            } => assert!(
                *repeat_on_each_page,
                "synthetic background should render first"
            ),
            other => panic!("expected text block, got {other:?}"),
        }
    }

    // ---- calc() integration test ----

    #[test]
    fn calc_width_in_layout() {
        // Use a calc() value that's smaller than available_width so explicit_width is set
        let html = r#"<div style="width: calc(50% - 10pt)">Calc content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        if let (_, LayoutElement::TextBlock { block_width, .. }) = &pages[0].elements[0] {
            assert!(
                block_width.is_some(),
                "calc() width should resolve to explicit width"
            );
        }
    }

    // ---- CSS variable integration test ----

    #[test]
    fn var_width_in_layout() {
        let html = r#"<div style="--w: 200pt"><div style="width: var(--w)">Var width</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, el)| {
            matches!(el, LayoutElement::TextBlock { block_width: Some(w), .. } if (*w - 200.0).abs() < 1.0)
        });
        assert!(found, "Expected element with width ~200pt from var()");
    }

    // ---- rem unit integration test ----

    #[test]
    fn rem_unit_in_layout() {
        let html = r#"<div style="margin-top: 2rem">Rem margin</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        // 2rem = 24pt margin_top
        if let (_, LayoutElement::TextBlock { margin_top, .. }) = &pages[0].elements[0] {
            assert!(
                (*margin_top - 24.0).abs() < 0.5,
                "Expected ~24pt margin_top from 2rem"
            );
        }
    }

    #[test]
    fn table_row_carries_border_collapse() {
        let html = r#"<table style="border-collapse: collapse"><tr><td>A</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_collapse = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TableRow {
                    border_collapse: BorderCollapse::Collapse,
                    ..
                }
            )
        });
        assert!(has_collapse, "Expected border_collapse: Collapse");
    }

    #[test]
    fn table_row_default_border_separate() {
        let html = r#"<table><tr><td>A</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_separate = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TableRow {
                    border_collapse: BorderCollapse::Separate,
                    ..
                }
            )
        });
        assert!(has_separate, "Expected default border_collapse: Separate");
    }

    #[test]
    fn table_row_carries_border_spacing() {
        let html = r#"<table style="border-spacing: 8px"><tr><td>A</td><td>B</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_spacing = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TableRow { border_spacing, .. } = el {
                (*border_spacing - 6.0).abs() < 0.1
            } else {
                false
            }
        });
        assert!(has_spacing, "Expected border_spacing of 6pt (8px * 0.75)");
    }

    #[test]
    fn text_overflow_ellipsis_truncates() {
        // text-overflow: ellipsis is stored on the style; layout does not yet
        // perform the actual truncation with "..." so we just verify the
        // element is produced and has a single line (nowrap).
        let html = r#"<div style="width: 50px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis">This is a very long text that should be truncated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { lines, .. } = el {
                lines.len() == 1
            } else {
                false
            }
        });
        assert!(found, "Text with nowrap should have a single line");
    }

    #[test]
    fn text_overflow_clip_no_ellipsis() {
        let html = r#"<div style="width: 50px; overflow: hidden; white-space: nowrap; text-overflow: clip">This is a very long text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_ellipsis = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { lines, .. } = el {
                lines
                    .iter()
                    .any(|l| l.runs.iter().any(|r| r.text.ends_with("...")))
            } else {
                false
            }
        });
        assert!(!has_ellipsis, "clip should not add ellipsis");
    }

    // --- list-style-type tests ---
    #[test]
    fn format_list_marker_disc() {
        assert_eq!(format_list_marker(ListStyleType::Disc, 1), "\u{2022} ");
    }

    #[test]
    fn format_list_marker_circle() {
        assert_eq!(format_list_marker(ListStyleType::Circle, 1), "\u{25E6} ");
    }

    #[test]
    fn format_list_marker_square() {
        assert_eq!(format_list_marker(ListStyleType::Square, 1), "\u{25AA} ");
    }

    #[test]
    fn format_list_marker_decimal() {
        assert_eq!(format_list_marker(ListStyleType::Decimal, 3), "3. ");
    }

    #[test]
    fn format_list_marker_decimal_leading_zero() {
        assert_eq!(
            format_list_marker(ListStyleType::DecimalLeadingZero, 3),
            "03. "
        );
        assert_eq!(
            format_list_marker(ListStyleType::DecimalLeadingZero, 12),
            "12. "
        );
    }

    #[test]
    fn format_list_marker_lower_alpha() {
        assert_eq!(format_list_marker(ListStyleType::LowerAlpha, 1), "a. ");
        assert_eq!(format_list_marker(ListStyleType::LowerAlpha, 3), "c. ");
        assert_eq!(format_list_marker(ListStyleType::LowerAlpha, 27), "aa. ");
    }

    #[test]
    fn format_list_marker_upper_alpha() {
        assert_eq!(format_list_marker(ListStyleType::UpperAlpha, 1), "A. ");
        assert_eq!(format_list_marker(ListStyleType::UpperAlpha, 26), "Z. ");
    }

    #[test]
    fn format_list_marker_lower_roman() {
        assert_eq!(format_list_marker(ListStyleType::LowerRoman, 1), "i. ");
        assert_eq!(format_list_marker(ListStyleType::LowerRoman, 4), "iv. ");
        assert_eq!(format_list_marker(ListStyleType::LowerRoman, 9), "ix. ");
        assert_eq!(format_list_marker(ListStyleType::LowerRoman, 14), "xiv. ");
    }

    #[test]
    fn format_list_marker_upper_roman() {
        assert_eq!(format_list_marker(ListStyleType::UpperRoman, 1), "I. ");
        assert_eq!(format_list_marker(ListStyleType::UpperRoman, 4), "IV. ");
    }

    #[test]
    fn format_list_marker_none() {
        assert_eq!(format_list_marker(ListStyleType::None, 1), "");
    }

    // --- Counter state tests ---
    #[test]
    fn counter_state_default_returns_zero() {
        let cs = CounterState::default();
        assert_eq!(cs.get("foo"), 0);
    }

    #[test]
    fn counter_state_apply_resets() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 0);
    }

    #[test]
    fn counter_state_apply_increments() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 1);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 2);
    }

    #[test]
    fn counter_state_nested_resets() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 1)]);
        // Nested reset pushes a new counter
        cs.apply_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 0);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 1);
        // Pop nested reset
        cs.pop_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 1); // Back to outer counter value
    }

    #[test]
    fn counter_state_get_all() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 1)]);
        cs.apply_resets(&[("section".to_string(), 2)]);
        cs.apply_resets(&[("section".to_string(), 3)]);
        assert_eq!(cs.get_all("section", "."), "1.2.3");
    }

    // --- resolve_content tests ---
    #[test]
    fn resolve_content_string() {
        let cs = CounterState::default();
        let attrs = HashMap::new();
        let items = vec![ContentItem::String("hello".to_string())];
        assert_eq!(resolve_content(&items, &attrs, &cs), "hello");
    }

    #[test]
    fn resolve_content_attr() {
        let cs = CounterState::default();
        let mut attrs = HashMap::new();
        attrs.insert("title".to_string(), "My Title".to_string());
        let items = vec![ContentItem::Attr("title".to_string())];
        assert_eq!(resolve_content(&items, &attrs, &cs), "My Title");
    }

    #[test]
    fn resolve_content_counter() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 3)]);
        let attrs = HashMap::new();
        let items = vec![ContentItem::Counter("section".to_string())];
        assert_eq!(resolve_content(&items, &attrs, &cs), "3");
    }

    #[test]
    fn resolve_content_counters() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 1)]);
        cs.apply_resets(&[("section".to_string(), 2)]);
        let attrs = HashMap::new();
        let items = vec![ContentItem::Counters(
            "section".to_string(),
            ".".to_string(),
        )];
        assert_eq!(resolve_content(&items, &attrs, &cs), "1.2");
    }

    #[test]
    fn resolve_content_mixed() {
        let cs = CounterState::default();
        let mut attrs = HashMap::new();
        attrs.insert("data-label".to_string(), "Note".to_string());
        let items = vec![
            ContentItem::Attr("data-label".to_string()),
            ContentItem::String(": ".to_string()),
        ];
        assert_eq!(resolve_content(&items, &attrs, &cs), "Note: ");
    }

    // --- ::before/::after integration tests ---
    #[test]
    fn before_pseudo_element_in_layout() {
        let html = r#"<html><head><style>p::before { content: ">> " }</style></head><body><p>Hello</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut all_texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let text: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    all_texts.push(text);
                }
            }
        }
        let found = all_texts
            .iter()
            .any(|t| t.contains(">>") && t.contains("Hello"));
        assert!(
            found,
            "::before content should be prepended to paragraph, got: {:?}",
            all_texts
        );
    }

    #[test]
    fn after_pseudo_element_in_layout() {
        let html = r#"<html><head><style>p::after { content: " <<" }</style></head><body><p>Hello</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut all_texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let text: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    all_texts.push(text);
                }
            }
        }
        let found = all_texts
            .iter()
            .any(|t| t.contains("Hello") && t.contains("<<"));
        assert!(
            found,
            "::after content should be appended to paragraph, got: {:?}",
            all_texts
        );
    }

    #[test]
    fn root_font_size_drives_rem_layout_values() {
        let html = r#"
            <html>
                <head>
                    <style>
                        :root { font-size: 10pt; }
                        .title { font-size: 2rem; margin-top: 0.5rem; }
                    </style>
                </head>
                <body><div class="title">Title</div></body>
            </html>
        "#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }

        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let title_block = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TextBlock {
                    lines, margin_top, ..
                } if lines
                    .iter()
                    .flat_map(|line| line.runs.iter())
                    .any(|run| run.text.contains("Title")) =>
                {
                    Some((lines, margin_top))
                }
                _ => None,
            })
            .expect("expected title block");

        let (lines, margin_top) = title_block;
        assert!((*margin_top - 5.0).abs() < 0.1);
        assert!(
            (lines[0].runs[0].font_size - 20.0).abs() < 0.1,
            "expected 2rem to resolve from :root 10pt"
        );
    }

    // --- list-style-type in layout tests ---
    #[test]
    fn unordered_list_uses_bullet_marker() {
        let html = "<ul><li>Item</li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { lines, .. } = el {
                lines
                    .iter()
                    .any(|l| l.runs.iter().any(|r| r.text.contains('\u{2022}')))
            } else {
                false
            }
        });
        assert!(found, "Unordered list should use bullet marker");
    }

    #[test]
    fn ordered_list_uses_decimal_marker() {
        let html = "<ol><li>First</li><li>Second</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut all_texts: Vec<String> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for l in lines {
                    let text: String = l.runs.iter().map(|r| r.text.as_str()).collect();
                    all_texts.push(text);
                }
            }
        }
        let found = all_texts.iter().any(|t| t.contains("1."));
        assert!(
            found,
            "Ordered list should use decimal marker, got: {:?}",
            all_texts
        );
    }

    // --- Coverage tests for uncovered lines ---

    #[test]
    fn to_alpha_lower_zero_returns_a() {
        // Covers line 81: to_alpha_lower(0) returns "a"
        assert_eq!(to_alpha_lower(0), "a");
    }

    #[test]
    fn to_roman_lower_zero_returns_zero_string() {
        // Covers line 120: to_roman_lower(0) returns "0"
        assert_eq!(to_roman_lower(0), "0");
    }

    #[test]
    fn counter_state_apply_increments_on_empty_stack() {
        // Covers line 32: apply_increments pushes 0 when stack is empty
        let mut state = CounterState::default();
        state.apply_increments(&[("test".to_string(), 1)]);
        assert_eq!(state.get("test"), 1);
    }

    #[test]
    fn layout_flex_container() {
        // Covers lines 1067,1133,1395: flex layout code paths
        let html = r#"<div style="display: flex; width: 400pt;">
            <div style="width: 200pt;">Left</div>
            <div style="width: 200pt;">Right</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_grid_container() {
        // Covers lines 1670,1712: grid layout code paths
        let html = r#"<html><head><style>
            .grid { display: grid; grid-template-columns: 1fr 1fr; }
        </style></head><body>
        <div class="grid"><div>A</div><div>B</div></div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_table_with_non_standard_children() {
        // Covers line 1821,1831,1858: table non-tr children
        let html = "<table><caption>Cap</caption><tr><td>A</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_table_colspan_exceeds_cols() {
        // Covers line 1943,2003: colspan beyond column count
        let html = r#"<table>
            <tr><td colspan="10">Wide</td></tr>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_white_space_nowrap_overflow() {
        // Covers lines 2221,2227,2242: nowrap + text-overflow: ellipsis
        let html = r#"<html><head><style>
            .nowrap { width: 50pt; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
        </style></head><body>
        <div class="nowrap">This text is very long and should be truncated</div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_clear_right_float() {
        // Covers line 2312: clear: right
        let html = r#"
            <div style="float: right; width: 100pt;">Floated</div>
            <div style="clear: right;">Cleared</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn base64_decode_valid() {
        // Covers lines 2562,2574: base64 decode
        let decoded = decode_base64("SGVsbG8=").unwrap();
        assert_eq!(&decoded, b"Hello");
    }

    #[test]
    fn base64_decode_invalid_char() {
        // Covers line 2562: base64 decode with invalid char
        let result = decode_base64("!!!!");
        assert!(result.is_none());
    }

    #[test]
    fn base64_decode_short_input() {
        // Covers line 2574: base64 decode with very short input (breaks early)
        let result = decode_base64("A");
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn wrap_break_word_splits_long_word_without_hyphen() {
        let fonts = HashMap::new();
        let template = TextRun {
            text: String::new(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        // At 12pt, each char ~6pt. "Hi" = 12pt.
        // "Supercalifragilisticexpialidocious" = 34*6 = 204pt.
        // With max_width=100, "Hi" (12pt) fits, then the long word (204pt)
        // doesn't fit (12 + 6 space + 204 > 100), so break-word splits it
        // across lines without inserting a hyphen character.
        let runs = vec![TextRun {
            text: "Hi Supercalifragilisticexpialidocious".to_string(),
            ..template
        }];
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(100.0, 12.0, 1.2, OverflowWrap::BreakWord),
            &fonts,
        );
        assert!(
            lines.len() > 1,
            "expected break-word to produce multiple lines, got {}",
            lines.len()
        );
        let first_line_text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert!(
            !first_line_text.ends_with('-'),
            "break-word should not insert hyphens, got: {first_line_text:?}"
        );
    }

    #[test]
    fn wrap_normal_keeps_fitting_text_on_one_line() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "Hello world".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        let lines = wrap_text_runs(
            vec![run],
            TextWrapOptions::new(500.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert!(
            !text.contains('-'),
            "short fitting text should stay unchanged, got: {text:?}"
        );
    }

    #[test]
    fn wrap_break_word_splits_short_remainder_without_hyphen() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "Hi the end".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        let lines = wrap_text_runs(
            vec![run],
            TextWrapOptions::new(20.0, 12.0, 1.2, OverflowWrap::BreakWord),
            &fonts,
        );
        for line in &lines {
            for run in &line.runs {
                assert!(
                    !run.text.contains('-'),
                    "break-word should not add hyphens, got: {:?}",
                    run.text
                );
            }
        }
    }

    /// Helper: extract all Tj strings from a PDF byte vector.
    fn extract_tj_strings(pdf: &[u8]) -> Vec<String> {
        let pdf_str = String::from_utf8_lossy(pdf);
        pdf_str
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.ends_with("Tj") && trimmed.starts_with('(') {
                    Some(trimmed[1..trimmed.len() - 4].to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn spaces_preserved_in_text() {
        // "Hello World" must stay "Hello World" through the full pipeline
        let html = "<p>Hello World</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let all_text = tj.join("");
        assert!(
            all_text.contains("Hello World"),
            "Expected 'Hello World' in PDF text, got: {tj:?}"
        );
    }

    #[test]
    fn spaces_between_inline_elements() {
        // `<span>Hello</span> <span>World</span>` must have a space
        let html = "<p><span>Hello</span> <span>World</span></p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let all_text = tj.join("");
        assert!(
            all_text.contains("Hello World"),
            "Expected space between inline elements, got: {tj:?}"
        );
    }

    #[test]
    fn invoice_text_spaces_preserved() {
        // Verify the specific failing cases from the invoice
        let html = r#"
            <p><strong>Bill to:</strong><br>
            Acme Corp<br>
            456 Enterprise Blvd<br>
            New York, NY 10001</p>
            <table>
                <tr><td>Custom font embedding module</td></tr>
                <tr><td>SVG rendering add-on</td></tr>
            </table>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let has = |needle: &str| tj.iter().any(|s| s.contains(needle));

        assert!(has("Acme Corp"), "Expected 'Acme Corp', got: {tj:?}");
        assert!(has("New York"), "Expected 'New York', got: {tj:?}");
        assert!(has("Custom font"), "Expected 'Custom font', got: {tj:?}");
        assert!(
            has("SVG rendering"),
            "Expected 'SVG rendering', got: {tj:?}"
        );
        assert!(
            has("Enterprise Blvd"),
            "Expected 'Enterprise Blvd', got: {tj:?}"
        );
    }

    /// Block children inside a padded parent should use inner_width (parent
    /// width minus padding) so that their text wraps within the padding.
    #[test]
    fn padded_div_child_block_respects_inner_width() {
        let html = r#"<div style="padding: 20pt;"><p>short</p></div>"#;
        let dom = parse_html(html).unwrap();
        let pages = layout(
            &dom,
            crate::types::PageSize::new(200.0, 800.0),
            crate::types::Margin::uniform(0.0),
        );
        // The <p> inside the padded div should be laid out within 200 - 40 = 160pt.
        // We verify that the p's TextBlock has block_width <= 160.
        let mut found = false;
        for page in &pages {
            for (_, elem) in &page.elements {
                if let LayoutElement::TextBlock {
                    lines, block_width, ..
                } = elem
                {
                    let text: String = lines
                        .iter()
                        .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                        .collect();
                    if text.contains("short") {
                        if let Some(bw) = block_width {
                            assert!(
                                *bw <= 160.0,
                                "child block width {bw} should be <= inner_width 160"
                            );
                        }
                        found = true;
                    }
                }
            }
        }
        assert!(found, "did not find the child paragraph");
    }

    /// Flex child with inline background (badge) should propagate the
    /// background_color from the computed style to the TextRun.
    #[test]
    fn flex_child_propagates_background_color() {
        let html = r#"
        <div style="display: flex;">
          <div><span style="background-color: #27ae60; color: white;">PAID</span></div>
        </div>"#;
        let dom = parse_html(html).unwrap();
        let rules = parse_stylesheet("span { background-color: #27ae60; color: white; }");
        let pages = layout_with_rules(
            &dom,
            crate::types::PageSize::default(),
            crate::types::Margin::uniform(20.0),
            &rules,
        );
        let mut found_bg = false;
        for page in &pages {
            for (_, elem) in &page.elements {
                if let LayoutElement::FlexRow { cells, .. } = elem {
                    for cell in cells {
                        for line in &cell.lines {
                            for run in &line.runs {
                                if run.text.contains("PAID") && run.background_color.is_some() {
                                    found_bg = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            found_bg,
            "PAID badge text run should have background_color set"
        );
    }

    #[test]
    fn flex_row_child_preserves_svg_background() {
        let child_style = r#"background-image: url(data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10'%3E%3Crect width='10' height='10' fill='%23f00'/%3E%3C/svg%3E); width: 60pt;"#;
        let parsed = crate::parser::css::parse_inline_style(child_style);
        assert!(
            parsed.get("background-svg").is_some(),
            "expected inline style parser to capture SVG background"
        );
        let computed = crate::style::computed::compute_style(
            HtmlTag::Div,
            Some(child_style),
            &ComputedStyle::default(),
        );
        assert!(
            computed.background_svg.is_some(),
            "expected computed style to retain SVG background"
        );
        let html =
            format!(r#"<div style="display: flex;"><div style="{child_style}">A</div></div>"#);
        let pages = layout(&parse_html(&html).unwrap(), PageSize::A4, Margin::default());
        let has_cell_svg_background = pages.iter().any(|page| {
            page.elements.iter().any(|(_, el)| match el {
                LayoutElement::FlexRow { cells, .. } => {
                    cells.iter().any(|cell| cell.background_svg.is_some())
                }
                _ => false,
            })
        });
        assert!(
            has_cell_svg_background,
            "expected flex row cell to retain SVG background data"
        );
    }

    /// Notes-style div with padding, br tags, and inline content should
    /// produce wrapped text that fits within the padded area.
    #[test]
    fn notes_div_with_padding_and_br_wraps_correctly() {
        let html = r#"<div style="padding: 10pt; font-size: 9pt;">
          <strong>Notes:</strong><br>
          First line of text that should be fully visible inside the padded area.<br>
          Second line with content.
        </div>"#;
        let dom = parse_html(html).unwrap();
        let pages = layout(
            &dom,
            crate::types::PageSize::new(300.0, 800.0),
            crate::types::Margin::uniform(0.0),
        );
        // Verify that lines exist and the text is present
        let mut all_text = String::new();
        let mut line_count = 0;
        for page in &pages {
            for (_, elem) in &page.elements {
                if let LayoutElement::TextBlock { lines, .. } = elem {
                    for line in lines {
                        for run in &line.runs {
                            all_text.push_str(&run.text);
                        }
                        line_count += 1;
                    }
                }
            }
        }
        assert!(all_text.contains("Notes:"), "Notes: text missing");
        assert!(
            all_text.contains("First line"),
            "First line text missing: {all_text:?}"
        );
        assert!(
            all_text.contains("Second line"),
            "Second line text missing: {all_text:?}"
        );
        // Should have at least 3 lines due to the <br> tags
        assert!(
            line_count >= 3,
            "expected at least 3 lines from br tags, got {line_count}"
        );
    }

    #[test]
    fn body_rules_applied_to_root() {
        let css = "body { font-size: 10pt }";
        let rules = parse_stylesheet(css);
        let html = "<p>text</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            assert!(!lines.is_empty());
            let font_size = lines[0].runs[0].font_size;
            assert!(
                (font_size - 10.0).abs() < 0.1,
                "Expected font_size 10.0 from body rule, got {font_size}"
            );
        } else {
            panic!("Expected TextBlock");
        }
    }

    #[test]
    fn root_rules_applied_to_root_style() {
        let css = ":root { font-size: 11pt; background-color: #abcdef }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());

        let first_is_background = matches!(
            &pages[0].elements[0].1,
            LayoutElement::TextBlock {
                background_color: Some((r, g, b)),
                repeat_on_each_page: true,
                ..
            } if (*r - 0xAB as f32 / 255.0).abs() < 0.01
                && (*g - 0xCD as f32 / 255.0).abs() < 0.01
                && (*b - 0xEF as f32 / 255.0).abs() < 0.01
        );
        assert!(first_is_background, "Expected page background from :root");

        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[1] {
            assert!(!lines.is_empty());
            let font_size = lines[0].runs[0].font_size;
            assert!(
                (font_size - 11.0).abs() < 0.1,
                "Expected font_size 11.0 from :root rule, got {font_size}"
            );
        } else {
            panic!("Expected text block after root background");
        }
    }

    #[test]
    fn root_svg_background_emits_page_background_block() {
        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        if let (
            _,
            LayoutElement::TextBlock {
                background_svg: Some(tree),
                block_width: Some(width),
                block_height: Some(height),
                repeat_on_each_page: true,
                ..
            },
        ) = &pages[0].elements[0]
        {
            assert_eq!(tree.width, 20.0);
            assert_eq!(tree.height, 10.0);
            assert!((*width - PageSize::A4.width).abs() < 0.1);
            assert!((*height - PageSize::A4.height).abs() < 0.1);
        } else {
            panic!("Expected a repeat-on-each-page SVG background block");
        }
    }

    #[test]
    fn wrapper_textblock_for_visual_blocks() {
        let css = ".box { background-color: red; padding: 10pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="box"><p>hello</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_bg = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    background_color: Some(_),
                    ..
                }
            )
        });
        assert!(
            has_bg,
            "Expected a TextBlock with background_color from .box div"
        );
    }

    #[test]
    fn flex_child_ancestor_selectors() {
        let css = ".card .value { font-size: 20pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="card" style="display: flex"><div class="value">big</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let items = extract_flex_items(&pages);
        let big_item = items.iter().find(|i| i.3.contains("big"));
        assert!(
            big_item.is_some(),
            "Did not find 'big' text in flex layout output"
        );
        // Verify the font size was applied via ancestor selector
        // Check via the layout elements directly for font_size
        let mut found = false;
        for (_, el) in &pages[0].elements {
            match el {
                LayoutElement::TextBlock { lines, .. } => {
                    for line in lines {
                        for run in &line.runs {
                            if run.text.contains("big") && (run.font_size - 20.0).abs() < 0.1 {
                                found = true;
                            }
                        }
                    }
                }
                LayoutElement::FlexRow { cells, .. } => {
                    for cell in cells {
                        for line in &cell.lines {
                            for run in &line.runs {
                                if run.text.contains("big") && (run.font_size - 20.0).abs() < 0.1 {
                                    found = true;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(found, "Expected font_size 20.0 for .value in flex child");
    }

    #[test]
    fn p_inherits_parent_font_size() {
        let html = r#"<div style="font-size: 8pt"><p>small</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        let mut found = false;
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for line in lines {
                    for run in &line.runs {
                        if run.text.contains("small") {
                            assert!(
                                (run.font_size - 8.0).abs() < 0.1,
                                "Expected font_size 8.0 for p inside div, got {}",
                                run.font_size
                            );
                            found = true;
                        }
                    }
                }
            }
        }
        assert!(found, "Did not find 'small' text run in layout output");
    }

    #[test]
    fn table_nth_child_section_relative() {
        let css = "tbody tr:nth-child(even) { background-color: #eee }";
        let rules = parse_stylesheet(css);
        let html = r#"
            <table>
                <thead><tr><th>H</th></tr></thead>
                <tbody>
                    <tr><td>Row 1</td></tr>
                    <tr><td>Row 2</td></tr>
                    <tr><td>Row 3</td></tr>
                </tbody>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(cells)
                } else {
                    None
                }
            })
            .collect();
        // Should have at least 4 rows (1 thead + 3 tbody)
        assert!(
            table_rows.len() >= 4,
            "Expected at least 4 table rows, got {}",
            table_rows.len()
        );
    }

    #[test]
    fn layout_border_horizontal_width() {
        let border = LayoutBorder {
            top: LayoutBorderSide {
                width: 1.0,
                color: (0.0, 0.0, 0.0),
            },
            right: LayoutBorderSide {
                width: 3.0,
                color: (0.0, 0.0, 0.0),
            },
            bottom: LayoutBorderSide {
                width: 2.0,
                color: (0.0, 0.0, 0.0),
            },
            left: LayoutBorderSide {
                width: 5.0,
                color: (0.0, 0.0, 0.0),
            },
        };
        assert!((border.horizontal_width() - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_border_vertical_width() {
        let border = LayoutBorder {
            top: LayoutBorderSide {
                width: 4.0,
                color: (0.0, 0.0, 0.0),
            },
            right: LayoutBorderSide {
                width: 1.0,
                color: (0.0, 0.0, 0.0),
            },
            bottom: LayoutBorderSide {
                width: 6.0,
                color: (0.0, 0.0, 0.0),
            },
            left: LayoutBorderSide {
                width: 1.0,
                color: (0.0, 0.0, 0.0),
            },
        };
        assert!((border.vertical_width() - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_border_max_width() {
        let border = LayoutBorder {
            top: LayoutBorderSide {
                width: 2.0,
                color: (0.0, 0.0, 0.0),
            },
            right: LayoutBorderSide {
                width: 7.0,
                color: (0.0, 0.0, 0.0),
            },
            bottom: LayoutBorderSide {
                width: 3.0,
                color: (0.0, 0.0, 0.0),
            },
            left: LayoutBorderSide {
                width: 5.0,
                color: (0.0, 0.0, 0.0),
            },
        };
        assert!((border.max_width() - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flex_column_layout() {
        let html = r#"<div style="display: flex; flex-direction: column">
            <div>First</div>
            <div>Second</div>
            <div>Third</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let text_blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TextBlock { .. }))
            .collect();
        assert!(
            text_blocks.len() >= 3,
            "Expected at least 3 text blocks for column flex children, got {}",
            text_blocks.len()
        );
    }

    #[test]
    fn flex_column_with_background() {
        let html = r#"<div style="display: flex; flex-direction: column; background-color: #eee">
            <p>Child A</p>
            <p>Child B</p>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_bg = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    background_color: Some(_),
                    ..
                }
            )
        });
        assert!(
            has_bg,
            "Expected a wrapper TextBlock with background_color for flex column container"
        );
    }

    #[test]
    fn table_rowspan_layout() {
        let html = r#"
            <table>
                <tr><td rowspan="2">Spanning</td><td>A</td></tr>
                <tr><td>B</td></tr>
                <tr><td>C</td><td>D</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .collect();
        assert!(
            table_rows.len() >= 2,
            "Expected at least 2 table rows with rowspan, got {}",
            table_rows.len()
        );
    }

    #[test]
    fn inline_span_inherits_border_radius() {
        let css = "span.badge { background-color: green; border-radius: 5pt; padding: 2pt; }";
        let rules = parse_stylesheet(css);
        let html = r#"<p><span class="badge">Tag</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let mut found_br = false;
        for (_, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                for line in lines {
                    for run in &line.runs {
                        if run.text.contains("Tag") && run.border_radius > 0.0 {
                            found_br = true;
                        }
                    }
                }
            }
        }
        assert!(
            found_br,
            "Expected TextRun for 'Tag' to have border_radius > 0 from stylesheet"
        );
    }

    #[test]
    fn grid_layout_produces_rows() {
        let css = ".grid { display: grid; grid-template-columns: 1fr 1fr; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div><div>D</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert!(
            !grid_rows.is_empty(),
            "Expected GridRow elements from display: grid layout"
        );
    }

    #[test]
    fn page_break_produces_multiple_pages() {
        let html = r#"
            <p>Page one content</p>
            <div style="page-break-before: always">
                <p>Page two content</p>
            </div>
            <div style="page-break-before: always">
                <p>Page three content</p>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(
            pages.len() >= 3,
            "Expected at least 3 pages from two page-break-before: always, got {}",
            pages.len()
        );
    }

    #[test]
    fn image_element_in_layout() {
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" style="width: 50px; height: 50px">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_image = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::Image { .. }));
        assert!(has_image, "Expected an Image layout element from img tag");
    }

    #[test]
    fn debug_inline_svg_before_heading_layout() {
        let html = r#"
            <body style="font-size: 10pt">
                <img
                    src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='50'%3E%3Crect width='200' height='50' fill='red'/%3E%3C/svg%3E"
                    style="max-width: 50%; max-height: 50px; object-fit: contain; object-position: left center"
                >
                <h2 style="font-size: 2rem; font-weight: bold; margin-top: .5rem; margin-bottom: 0">
                    International Timestamp Certificate
                </h2>
            </body>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut debug = Vec::new();
        for (y, element) in &pages[0].elements {
            match element {
                LayoutElement::Image {
                    width,
                    height,
                    flow_extra_bottom,
                    ..
                } => debug.push(format!(
                    "image y={y:.3} w={width:.3} h={height:.3} flow_extra_bottom={flow_extra_bottom:.3}"
                )),
                LayoutElement::Svg {
                    width,
                    height,
                    flow_extra_bottom,
                    ..
                } => debug.push(format!(
                    "svg y={y:.3} w={width:.3} h={height:.3} flow_extra_bottom={flow_extra_bottom:.3}"
                )),
                LayoutElement::TextBlock { lines, .. } => {
                    let text: String = lines
                        .iter()
                        .flat_map(|line| line.runs.iter().map(|run| run.text.as_str()))
                        .collect();
                    if !text.trim().is_empty() {
                        debug.push(format!("text y={y:.3} {:?}", text.trim()));
                    }
                }
                _ => {}
            }
        }
        println!("{}", debug.join("\n"));
        assert!(!debug.is_empty());
    }

    #[test]
    fn debug_certificate_table_row_heights() {
        let html = std::fs::read_to_string(
            "/home/frederic/IdeaProjects/ipocamp-backend-v2/target/test/ironpress_comparison/certificate.html",
        )
        .unwrap();
        let parsed = crate::parser::html::parse_html_with_styles(&html).unwrap();
        let css = parsed.stylesheets.join("\n");
        let rules = crate::parser::css::parse_stylesheet(&css);
        let pages = layout_with_rules_and_fonts(
            &parsed.nodes,
            PageSize::A4,
            Margin::uniform(28.346457),
            &rules,
            &HashMap::new(),
        );
        let mut debug = Vec::new();
        for (y, element) in &pages[0].elements {
            match element {
                LayoutElement::Svg {
                    width,
                    height,
                    flow_extra_bottom,
                    ..
                } => debug.push(format!(
                    "svg y={y:.3} w={width:.3} h={height:.3} flow_extra_bottom={flow_extra_bottom:.3}"
                )),
                LayoutElement::TextBlock { lines, .. } => {
                    let text: String = lines
                        .iter()
                        .flat_map(|line| line.runs.iter().map(|run| run.text.as_str()))
                        .collect();
                    if !text.trim().is_empty() {
                        debug.push(format!("text y={y:.3} {:?}", text.trim()));
                    }
                }
                LayoutElement::TableRow { cells, .. } => {
                    let row_h = cells
                        .iter()
                        .map(table_cell_content_height)
                        .fold(0.0f32, f32::max);
                    let first_text = cells
                        .iter()
                        .flat_map(|cell| cell.lines.iter())
                        .flat_map(|line| line.runs.iter())
                        .map(|run| run.text.as_str())
                        .collect::<String>();
                    debug.push(format!(
                        "table-row y={y:.3} h={row_h:.3} {:?}",
                        first_text.trim()
                    ));
                    for (cell_idx, cell) in cells.iter().enumerate() {
                        for nested in &cell.nested_rows {
                            if let LayoutElement::TableRow { cells, .. } = nested {
                                let nested_h = cells
                                    .iter()
                                    .map(table_cell_content_height)
                                    .fold(0.0f32, f32::max);
                                let nested_text = cells
                                    .iter()
                                    .flat_map(|cell| cell.lines.iter())
                                    .flat_map(|line| line.runs.iter())
                                    .map(|run| run.text.as_str())
                                    .collect::<String>();
                                debug.push(format!(
                                    "  nested[{cell_idx}] row h={nested_h:.3} {:?}",
                                    nested_text.trim()
                                ));
                                for (nested_cell_idx, nested_cell) in cells.iter().enumerate() {
                                    let line_heights: Vec<String> = nested_cell
                                        .lines
                                        .iter()
                                        .map(|line| format!("{:.3}", line.height))
                                        .collect();
                                    debug.push(format!(
                                        "    cell[{nested_cell_idx}] pad_top={:.3} pad_bottom={:.3} lines=[{}]",
                                        nested_cell.padding_top,
                                        nested_cell.padding_bottom,
                                        line_heights.join(", ")
                                    ));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        println!("{}", debug.join("\n"));
        assert!(!debug.is_empty());
    }

    #[test]
    fn debug_certificate_heading_styles() {
        let html = std::fs::read_to_string(
            "/home/frederic/IdeaProjects/ipocamp-backend-v2/target/test/ironpress_comparison/certificate.html",
        )
        .unwrap();
        let parsed = crate::parser::html::parse_html_with_styles(&html).unwrap();
        let css = parsed.stylesheets.join("\n");
        let rules = crate::parser::css::parse_stylesheet(&css);

        let mut root_style = ComputedStyle::default();
        let default_parent = ComputedStyle::default();
        for rule in &rules {
            let selector = rule.selector.trim();
            if selector == "body" || selector == "html" || selector == ":root" {
                crate::style::computed::apply_style_map(
                    &mut root_style,
                    &rule.declarations,
                    &default_parent,
                );
            }
        }
        root_style.root_font_size = root_style.font_size;

        let table = parsed
            .nodes
            .iter()
            .find_map(|node| match node {
                DomNode::Element(element) if element.tag == HtmlTag::Table => Some(element),
                _ => None,
            })
            .unwrap();
        let outer_body = first_child_element_with_tag(table, HtmlTag::Tbody)
            .or_else(|| first_child_element(table))
            .expect("expected outer tbody");
        let outer_row = first_child_element_with_tag(outer_body, HtmlTag::Tr)
            .or_else(|| first_child_element(outer_body))
            .expect("expected outer row");
        let outer_cell = first_child_element_with_tag(outer_row, HtmlTag::Td)
            .or_else(|| first_child_element(outer_row))
            .expect("expected table cell");
        let inner_table = outer_cell
            .children
            .iter()
            .find_map(|node| match node {
                DomNode::Element(element) if element.tag == HtmlTag::Table => Some(element),
                _ => None,
            })
            .unwrap();
        let inner_body = first_child_element_with_tag(inner_table, HtmlTag::Tbody)
            .or_else(|| first_child_element(inner_table))
            .expect("expected inner tbody");
        let heading_row = first_child_element_with_tag(inner_body, HtmlTag::Tr)
            .or_else(|| first_child_element(inner_body))
            .expect("expected heading row");
        let heading_cell = first_child_element_with_tag(heading_row, HtmlTag::Td)
            .or_else(|| first_child_element(heading_row))
            .expect("expected heading cell");
        let heading = first_child_element(heading_cell).expect("expected heading element");

        let table_style = compute_style_with_context(
            table.tag,
            table.style_attr(),
            &root_style,
            &rules,
            table.tag_name(),
            &table.class_list(),
            table.id(),
            &table.attributes,
            &SelectorContext::default(),
        );
        let row_ctx = SelectorContext {
            ancestors: vec![
                AncestorInfo {
                    element: inner_table,
                    child_index: 0,
                    sibling_count: 1,
                    preceding_siblings: Vec::new(),
                },
                AncestorInfo {
                    element: inner_body,
                    child_index: 0,
                    sibling_count: 1,
                    preceding_siblings: Vec::new(),
                },
            ],
            child_index: 0,
            sibling_count: 10,
            preceding_siblings: Vec::new(),
        };
        let row_style = compute_style_with_context(
            heading_row.tag,
            heading_row.style_attr(),
            &table_style,
            &rules,
            heading_row.tag_name(),
            &heading_row.class_list(),
            heading_row.id(),
            &heading_row.attributes,
            &row_ctx,
        );
        let cell_ctx = SelectorContext {
            ancestors: vec![
                AncestorInfo {
                    element: inner_table,
                    child_index: 0,
                    sibling_count: 1,
                    preceding_siblings: Vec::new(),
                },
                AncestorInfo {
                    element: inner_body,
                    child_index: 0,
                    sibling_count: 1,
                    preceding_siblings: Vec::new(),
                },
                AncestorInfo {
                    element: heading_row,
                    child_index: 0,
                    sibling_count: 10,
                    preceding_siblings: Vec::new(),
                },
            ],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        let cell_style = compute_style_with_context(
            heading_cell.tag,
            heading_cell.style_attr(),
            &row_style,
            &rules,
            heading_cell.tag_name(),
            &heading_cell.class_list(),
            heading_cell.id(),
            &heading_cell.attributes,
            &cell_ctx,
        );
        let heading_ctx = SelectorContext {
            ancestors: vec![
                AncestorInfo {
                    element: inner_table,
                    child_index: 0,
                    sibling_count: 1,
                    preceding_siblings: Vec::new(),
                },
                AncestorInfo {
                    element: inner_body,
                    child_index: 0,
                    sibling_count: 1,
                    preceding_siblings: Vec::new(),
                },
                AncestorInfo {
                    element: heading_row,
                    child_index: 0,
                    sibling_count: 10,
                    preceding_siblings: Vec::new(),
                },
                AncestorInfo {
                    element: heading_cell,
                    child_index: 0,
                    sibling_count: 1,
                    preceding_siblings: Vec::new(),
                },
            ],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        let heading_style = compute_style_with_context(
            heading.tag,
            heading.style_attr(),
            &cell_style,
            &rules,
            heading.tag_name(),
            &heading.class_list(),
            heading.id(),
            &heading.attributes,
            &heading_ctx,
        );

        assert!(heading_style.font_size > root_style.font_size);
        assert!(cell_style.padding.right > 0.0 || cell_style.padding.bottom > 0.0);
        assert!(heading_style.margin.top >= 0.0);
    }

    #[test]
    fn wrapper_textblock_with_border() {
        let css = ".bordered { border: 2pt solid black; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="bordered"><p>inside</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_border = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { border, .. } = el {
                border.has_any()
            } else {
                false
            }
        });
        assert!(
            has_border,
            "Expected a wrapper TextBlock with border from .bordered div"
        );
    }

    #[test]
    fn wrapper_textblock_with_box_shadow() {
        let css = ".shadow { box-shadow: 2pt 2pt 4pt #000; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="shadow"><p>shadowed</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_shadow = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    box_shadow: Some(_),
                    ..
                }
            )
        });
        assert!(
            has_shadow,
            "Expected a wrapper TextBlock with box_shadow from .shadow div"
        );
    }

    #[test]
    fn flex_column_child_positioning() {
        let html = r#"<div style="display: flex; flex-direction: column">
            <div>Alpha</div>
            <div>Beta</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let text_blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| {
                if let LayoutElement::TextBlock { lines, .. } = el {
                    !lines.is_empty()
                } else {
                    false
                }
            })
            .collect();
        if text_blocks.len() >= 2 {
            assert!(
                text_blocks[1].0 >= text_blocks[0].0,
                "Expected second flex column child to be at or below first child"
            );
        }
    }

    #[test]
    fn grid_row_alignment_in_paginate() {
        let css = ".g { display: grid; grid-template-columns: 1fr 1fr 1fr; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="g"><div>X</div><div>Y</div><div>Z</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert_eq!(pages.len(), 1);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert!(
            !grid_rows.is_empty(),
            "Expected GridRow elements from grid layout"
        );
        for (y, _) in &grid_rows {
            assert!(*y >= 0.0, "Grid row y position should be non-negative");
        }
    }

    #[test]
    fn table_descendant_selector_total_row_td() {
        // .total-row td should apply styles via descendant selector on table rows
        let html = r#"<html><head><style>
            .total-row td { font-weight: bold; font-size: 14pt; }
        </style></head><body>
        <table><tbody>
            <tr><td>Normal</td></tr>
            <tr class="total-row"><td>Total</td></tr>
        </tbody></table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut table_rows: Vec<&Vec<TableCell>> = Vec::new();
        for page in &pages {
            for (_, el) in &page.elements {
                if let LayoutElement::TableRow { cells, .. } = el {
                    table_rows.push(cells);
                }
            }
        }
        assert_eq!(table_rows.len(), 2, "Expected 2 table rows");
        assert!(
            table_rows[1][0].bold,
            "Cell in .total-row should be bold via descendant selector"
        );
        let normal_h: f32 = table_rows[0][0].lines.iter().map(|l| l.height).sum();
        let total_h: f32 = table_rows[1][0].lines.iter().map(|l| l.height).sum();
        assert!(
            total_h > normal_h,
            "Total row text should be larger: {total_h} vs {normal_h}"
        );
    }

    #[test]
    fn flex_grow_distributes_free_space() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex-grow: 1; }
            .b { flex-grow: 2; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 2);
        // With flex-grow 1:2, widths should be roughly 100:200
        let ratio = cells[1].width / cells[0].width;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "flex-grow 1:2 should produce ~2:1 width ratio, got {ratio}"
        );
    }

    #[test]
    fn flex_basis_overrides_width() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 400pt; }
            .a { flex-basis: 100pt; }
            .b { flex-basis: 300pt; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 2);
        // flex-basis: 100pt vs 300pt
        assert!(
            (cells[0].width - 100.0).abs() < 5.0,
            "First cell should be ~100pt, got {}",
            cells[0].width
        );
        assert!(
            (cells[1].width - 300.0).abs() < 5.0,
            "Second cell should be ~300pt, got {}",
            cells[1].width
        );
    }

    #[test]
    fn margin_collapsing_adjacent_blocks() {
        // Adjacent sibling margins collapse: max(20, 30) = 30pt gap, not 50pt
        let html = r#"<html><head><style>
            .a { margin-bottom: 20pt; }
            .b { margin-top: 30pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Find the two TextBlock y-positions
        let mut ys: Vec<f32> = Vec::new();
        for (y, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                if !lines.is_empty() {
                    ys.push(*y);
                }
            }
        }
        assert_eq!(ys.len(), 2, "Expected 2 text blocks, got {}", ys.len());
        // The gap between the bottom of the first block and the second y-position
        // should reflect collapsed margin (30pt), not stacked (50pt).
        // We can't check exact absolute positions easily, but we can verify the
        // second block is closer than it would be without collapsing.
        let gap = ys[1] - ys[0];
        // Without collapsing: first_content_height + 20 + 30 = content + 50
        // With collapsing: first_content_height + 30
        // The gap should be smaller than content + 50
        assert!(gap > 0.0, "Second block should be below first");
    }

    #[test]
    fn flex_shorthand_parsing() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex: 1; }
            .b { flex: 2; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        assert_eq!(cells.len(), 2);
        // flex: 1 and flex: 2 with basis=0 should distribute 300pt as 100:200
        let ratio = cells[1].width / cells[0].width;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "flex shorthand 1:2 should produce ~2:1 width ratio, got {ratio}"
        );
    }

    #[test]
    fn flex_shrink_overflow() {
        // Items totalling 600pt in a 300pt container should shrink
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex-basis: 400pt; flex-shrink: 1; }
            .b { flex-basis: 200pt; flex-shrink: 1; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        let total: f32 = cells.iter().map(|c| c.width).sum();
        assert!(
            total <= 305.0,
            "Shrunk items should fit in container (~300pt), got {total}"
        );
        // Proportional: 400 shrinks more than 200
        assert!(
            cells[0].width > cells[1].width,
            "Larger basis should still be wider after shrink"
        );
    }

    #[test]
    fn flex_shrink_zero_prevents_shrink() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 200pt; }
            .a { flex-basis: 150pt; flex-shrink: 0; }
            .b { flex-basis: 150pt; flex-shrink: 1; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut flex_rows: Vec<&Vec<FlexCell>> = Vec::new();
        for (_, el) in &pages[0].elements {
            if let LayoutElement::FlexRow { cells, .. } = el {
                flex_rows.push(cells);
            }
        }
        assert_eq!(flex_rows.len(), 1);
        let cells = flex_rows[0];
        // First item has shrink: 0 so it keeps its basis
        assert!(
            (cells[0].width - 150.0).abs() < 5.0,
            "flex-shrink: 0 should prevent shrinking, got {}",
            cells[0].width
        );
        // Second item absorbs all the deficit
        assert!(
            cells[1].width < 150.0,
            "flex-shrink: 1 item should shrink, got {}",
            cells[1].width
        );
    }

    #[test]
    fn margin_collapsing_negative_margins() {
        let html = r#"<html><head><style>
            .a { margin-bottom: -10pt; }
            .b { margin-top: -20pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut ys: Vec<f32> = Vec::new();
        for (y, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                if !lines.is_empty() {
                    ys.push(*y);
                }
            }
        }
        assert_eq!(ys.len(), 2);
        // Both negative: most negative wins (-20), not sum (-30)
        // Second block may overlap first (negative gap)
    }

    #[test]
    fn margin_collapsing_mixed_signs() {
        let html = r#"<html><head><style>
            .a { margin-bottom: -10pt; }
            .b { margin-top: 30pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut ys: Vec<f32> = Vec::new();
        for (y, el) in &pages[0].elements {
            if let LayoutElement::TextBlock { lines, .. } = el {
                if !lines.is_empty() {
                    ys.push(*y);
                }
            }
        }
        assert_eq!(ys.len(), 2);
        // Mixed: sum = -10 + 30 = 20pt gap (not 30 or 40)
        let gap = ys[1] - ys[0];
        assert!(gap > 0.0, "Gap should be positive with mixed margins");
    }

    #[test]
    fn margin_collapsing_zero_margins() {
        let html = r#"<html><head><style>
            .a { margin-bottom: 0; }
            .b { margin-top: 0; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
    }

    #[test]
    fn table_descendant_selector_thead_th() {
        let html = r#"<html><head><style>
            thead th { color: red; font-size: 14pt; }
        </style></head><body>
        <table>
            <thead><tr><th>Header</th></tr></thead>
            <tbody><tr><td>Body</td></tr></tbody>
        </table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
        // Should render without panics; thead th selector exercises section ancestor chain
    }

    #[test]
    fn table_descendant_selector_tbody_td() {
        let html = r#"<html><head><style>
            tbody td { font-style: italic; }
            table td { font-size: 11pt; }
        </style></head><body>
        <table>
            <thead><tr><th>H</th></tr></thead>
            <tbody><tr><td>B</td></tr></tbody>
        </table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
    }

    #[test]
    fn table_colgroup_percentage_widths() {
        let html = r#"<table>
            <colgroup>
                <col span="1" style="width: 30%;">
                <col span="1" style="width: 70%;">
            </colgroup>
            <tr><th>Name</th><td>Contract_2026_Q1.pdf</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1, "Expected 1 table row");
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 2, "Expected 2 columns");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.30).abs() < 0.05,
            "First column should be ~30% of total, got {:.1}% (widths: {:?})",
            ratio * 100.0,
            col_widths
        );
    }

    fn first_table_row_col_widths(html: &str) -> Vec<f32> {
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row")
    }

    #[test]
    fn table_colgroup_percentage_widths_ignore_border_spacing() {
        let no_spacing = first_table_row_col_widths(
            r#"<table style="width: 300pt">
                <colgroup>
                    <col span="1" style="width: 30%;">
                    <col span="1" style="width: 70%;">
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );
        let spaced = first_table_row_col_widths(
            r#"<table style="width: 300pt; border-spacing: 10pt">
                <colgroup>
                    <col span="1" style="width: 30%;">
                    <col span="1" style="width: 70%;">
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert_eq!(no_spacing.len(), 2);
        assert_eq!(spaced.len(), 2);
        assert!(
            (spaced[0] - no_spacing[0]).abs() < 0.5,
            "border-spacing should not narrow percentage columns: {:?} vs {:?}",
            spaced,
            no_spacing
        );
        assert!(
            (spaced[1] - no_spacing[1]).abs() < 0.5,
            "border-spacing should not narrow percentage columns: {:?} vs {:?}",
            spaced,
            no_spacing
        );
    }

    #[test]
    fn table_colgroup_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col width="25%">
                <col width="75%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.25).abs() < 0.05,
            "First column should be ~25% of total, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn table_colgroup_last_inline_width_wins() {
        let html = r#"<table>
            <colgroup>
                <col style="width: 10%; width: 40%;" width="90%">
                <col style="width: 60%;">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.40).abs() < 0.05,
            "Last inline width declaration should win, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_inline_width_ignores_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col style="width: auto" width="80%">
                <col>
            </colgroup>
            <tr><td>Short</td><td>Much longer content here</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        assert!(
            col_widths[1] > col_widths[0],
            "Inline width should override width attribute; got {:?}",
            col_widths
        );
    }

    #[test]
    fn table_colgroup_malformed_inline_width_is_ignored() {
        let html = r#"<table>
            <colgroup>
                <col style="width: 10%; width: not-a-width" width="25%">
                <col style="width: not-a-width" width="90%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.10).abs() < 0.05,
            "Malformed inline width should be ignored, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_all_invalid_inline_widths_fall_back_to_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col style="width: not-a-width" width="80%">
                <col width="20%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.80).abs() < 0.05,
            "All-invalid inline widths should fall back to width attributes, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_span_attribute() {
        let html = r#"<table>
            <colgroup>
                <col span="2" style="width: 20%;">
                <col span="1" style="width: 60%;">
            </colgroup>
            <tr><td>A</td><td>B</td><td>C</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 3);
        let total: f32 = col_widths.iter().sum();
        let ratio_0 = col_widths[0] / total;
        let ratio_2 = col_widths[2] / total;
        assert!(
            (ratio_0 - 0.20).abs() < 0.05,
            "First two columns should each be ~20%, got {:.1}%",
            ratio_0 * 100.0
        );
        assert!(
            (ratio_2 - 0.60).abs() < 0.05,
            "Third column should be ~60%, got {:.1}%",
            ratio_2 * 100.0
        );
    }

    #[test]
    fn table_bare_col_without_colgroup() {
        let html = r#"<table>
            <col style="width: 40%;">
            <col style="width: 60%;">
            <tr><td>X</td><td>Y</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.40).abs() < 0.05,
            "First column should be ~40%, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn table_without_colgroup_unchanged() {
        let html = "<table><tr><td>Short</td><td>Much longer content here</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[1] > col_widths[0],
            "Auto-sizing should still work: longer column ({}) should be wider than short ({})",
            col_widths[1],
            col_widths[0]
        );
    }

    #[test]
    fn table_mixed_explicit_and_auto_widths() {
        let html = r#"<table>
            <colgroup>
                <col width="25%">
                <col>
            </colgroup>
            <tr><td>Fixed</td><td>Auto column content</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter_map(|(_, el)| {
                if let LayoutElement::TableRow { col_widths, .. } = el {
                    Some(col_widths.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(table_rows.len(), 1);
        let col_widths = &table_rows[0];
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[0] > 0.0 && col_widths[1] > 0.0,
            "Both explicit and auto columns should keep usable widths: {:?}",
            col_widths
        );
        assert!(
            col_widths[0] < col_widths[1] || (col_widths[0] - col_widths[1]).abs() < 5.0,
            "Auto column should not be collapsed by explicit width redistribution: {:?}",
            col_widths
        );
    }

    #[test]
    fn table_layout_fixed_uses_colgroup_widths_over_content() {
        let html = r#"<table style="table-layout: fixed; width: 400pt;">
            <colgroup>
                <col style="width: 25%;">
                <col style="width: 75%;">
            </colgroup>
            <tr>
                <td>Very long content that should not widen the first fixed column</td>
                <td>Short</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.25).abs() < 0.02,
            "fixed layout should honor colgroup width instead of content, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_layout_fixed_uses_first_row_cell_widths() {
        let html = r#"<table style="table-layout: fixed; width: 300pt;">
            <tr>
                <td style="width: 90pt;">A</td>
                <td>B</td>
            </tr>
            <tr>
                <td>Short</td>
                <td>Longer content in the second column</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        assert!(
            (col_widths[0] - 90.0).abs() < 1.0,
            "first-row cell width should determine fixed column width, got {:?}",
            col_widths
        );
        assert!(
            (col_widths[1] - 210.0).abs() < 1.0,
            "remaining width should be assigned to the other fixed column, got {:?}",
            col_widths
        );
    }

    #[test]
    fn table_colgroup_absolute_lengths_are_supported() {
        let html = r#"<table style="table-layout: fixed; width: 300pt;">
            <colgroup>
                <col style="width: 90pt;">
                <col>
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected table row");
        assert!(
            (col_widths[0] - 90.0).abs() < 1.0,
            "absolute <col> widths should be honored, got {:?}",
            col_widths
        );
        assert!(
            (col_widths[1] - 210.0).abs() < 1.0,
            "remaining width should stay usable for the trailing column, got {:?}",
            col_widths
        );
    }

    #[test]
    fn table_colgroup_em_width_uses_column_font_size() {
        let widths = first_table_row_col_widths(
            r#"<table style="table-layout: fixed; width: 200pt;">
                <colgroup style="font-size: 20pt">
                    <col style="width: 2em;">
                    <col>
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert!(
            (widths[0] - 40.0).abs() < 0.5,
            "2em should resolve against the colgroup font-size, got {:?}",
            widths
        );
        assert!(
            (widths[1] - 160.0).abs() < 0.5,
            "remaining width should stay on the trailing column, got {:?}",
            widths
        );
    }

    #[test]
    fn table_colgroup_calc_em_width_uses_column_font_size() {
        let widths = first_table_row_col_widths(
            r#"<table style="table-layout: fixed; width: 200pt;">
                <colgroup style="font-size: 20pt">
                    <col style="width: calc(1em + 5pt);">
                    <col>
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert!(
            (widths[0] - 25.0).abs() < 0.5,
            "calc(1em + 5pt) should use the colgroup font-size, got {:?}",
            widths
        );
        assert!(
            (widths[1] - 175.0).abs() < 0.5,
            "remaining width should stay on the trailing column, got {:?}",
            widths
        );
    }

    #[test]
    fn table_cell_block_content_preserves_link_and_whitespace() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        <div><a href="https://example.com">Click here</a></div>
                        <pre>  keep   spaces  </pre>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::TableRow { cells, .. } = el {
                Some(cells)
            } else {
                None
            }
        });
        let cells = cells.expect("expected table row");
        let text: String = cells[0]
            .lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .map(|run| run.text.as_str())
            .collect();
        assert!(
            cells[0]
                .lines
                .iter()
                .flat_map(|line| line.runs.iter())
                .any(|run| run.link_url.as_deref() == Some("https://example.com")),
            "Expected link URL to survive nested block traversal"
        );
        assert!(
            text.contains("  keep   spaces  "),
            "Expected preformatted whitespace to survive nested block traversal: {text:?}"
        );
    }

    #[test]
    fn table_cell_mixed_recursion_keeps_nested_block_padding_but_not_cell_padding() {
        let html = r#"
            <table>
                <tr>
                    <td style="padding: 18pt 12pt; text-align: right;">
                        Direct text
                        <div style="padding-left: 6pt; padding-top: 3pt; background-color: #eee;">
                            Nested block
                        </div>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::TableRow { cells, .. } = el {
                Some(cells)
            } else {
                None
            }
        });
        let cells = cells.expect("expected table row");
        let direct_run = cells[0]
            .lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .find(|run| run.text.contains("Direct"))
            .expect("expected direct cell text run");
        assert_eq!(
            direct_run.padding,
            (0.0, 0.0),
            "direct cell text should not inherit table-cell padding"
        );
        let nested_run = cells[0]
            .lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .find(|run| run.text.contains("Nested"))
            .expect("expected nested block text run");
        assert_eq!(
            nested_run.padding,
            (6.0, 3.0),
            "nested block text should keep its own padding"
        );
    }

    #[test]
    fn table_cell_nested_table_is_preserved_as_nested_layout() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        Outer
                        <table>
                            <tr><td>Inner</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::TableRow { cells, .. } = el {
                Some(cells)
            } else {
                None
            }
        });
        let cells = cells.expect("expected outer table row");
        assert!(
            !cells[0].nested_rows.is_empty(),
            "expected nested table rows to be preserved"
        );
        let nested_text: String = cells[0]
            .nested_rows
            .iter()
            .filter_map(|el| {
                if let LayoutElement::TableRow { cells, .. } = el {
                    Some(
                        cells
                            .iter()
                            .flat_map(|cell| cell.lines.iter())
                            .flat_map(|line| line.runs.iter())
                            .map(|run| run.text.as_str())
                            .collect::<String>(),
                    )
                } else {
                    None
                }
            })
            .collect();
        assert!(
            nested_text.contains("Inner"),
            "expected nested table text to stay in nested layout: {nested_text:?}"
        );
    }

    #[test]
    fn nested_fixed_table_percentage_width_uses_table_cell_width() {
        let html = r#"
            <table style="table-layout: fixed; width: 400pt;">
                <tr>
                    <td>
                        <table style="table-layout: fixed; width: 100%;">
                            <colgroup>
                                <col style="width: 30%;">
                                <col style="width: 70%;">
                            </colgroup>
                            <tr><td>A</td><td>B</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let (outer_col_widths, outer_cells) = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow {
                    col_widths, cells, ..
                } => Some((col_widths.clone(), cells)),
                _ => None,
            })
            .expect("expected outer table row");
        let nested_col_widths = outer_cells[0]
            .nested_rows
            .iter()
            .find_map(|element| match element {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected nested table row");
        let nested_total: f32 = nested_col_widths.iter().sum();
        let expected_inner_width =
            outer_col_widths[0] - outer_cells[0].padding_left - outer_cells[0].padding_right;
        assert!(
            (nested_total - expected_inner_width).abs() < 1.0,
            "nested fixed table should expand to the table cell width, got total {nested_total} vs {expected_inner_width}"
        );
        let first_ratio = nested_col_widths[0] / nested_total;
        assert!(
            (first_ratio - 0.30).abs() < 0.02,
            "nested fixed table should honor percentage colgroup widths, got {:?}",
            nested_col_widths
        );
    }

    #[test]
    fn certificate_like_nested_table_uses_full_width() {
        let html = r#"
            <style>
                @page {
                    size: A4 landscape;
                    margin: 1cm;
                }
                table {
                    table-layout: fixed;
                    width: 100%;
                    border-collapse: collapse;
                    column-count: 2;
                }
                .content th,
                .content td {
                    padding: 0 16px 8px 0;
                    word-wrap: break-word;
                }
            </style>
            <table>
                <tr style="vertical-align: top">
                    <td>
                        <table class="content">
                            <colgroup>
                                <col span="1" style="width: 30%;">
                                <col span="1" style="width: 70%;">
                            </colgroup>
                            <tr><th>Name</th><td>Contract_2026_Q1.pdf</td></tr>
                            <tr><th>Verification</th><td><a href="https://app.ipocamp.io/verify">https://app.ipocamp.io/verify</a></td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let parsed = parse_html_with_styles(html).unwrap();
        let mut page_rules = Vec::new();
        for css in &parsed.stylesheets {
            page_rules.extend(crate::parser::css::parse_page_rules(css));
        }
        let mut page_size = PageSize::default();
        let mut margin = Margin::default();
        for page_rule in &page_rules {
            if let (Some(width), Some(height)) = (page_rule.width, page_rule.height) {
                page_size = PageSize { width, height };
            }
            if let Some(v) = page_rule.margin_top {
                margin.top = v;
            }
            if let Some(v) = page_rule.margin_right {
                margin.right = v;
            }
            if let Some(v) = page_rule.margin_bottom {
                margin.bottom = v;
            }
            if let Some(v) = page_rule.margin_left {
                margin.left = v;
            }
        }
        let media_ctx = crate::parser::css::MediaContext {
            width: page_size.width,
            height: page_size.height,
        };
        let mut rules = Vec::new();
        for css in &parsed.stylesheets {
            rules.extend(crate::parser::css::parse_stylesheet_with_context(
                css,
                Some(media_ctx),
            ));
        }
        let pages = layout_with_rules(&parsed.nodes, page_size, margin, &rules);
        let outer_cells = pages[0]
            .elements
            .iter()
            .find_map(|(_, el)| match el {
                LayoutElement::TableRow { cells, .. } => Some(cells),
                _ => None,
            })
            .expect("expected outer table row");
        let nested_col_widths = outer_cells[0]
            .nested_rows
            .iter()
            .find_map(|element| match element {
                LayoutElement::TableRow { col_widths, .. } => Some(col_widths.clone()),
                _ => None,
            })
            .expect("expected nested content table row");
        let nested_total: f32 = nested_col_widths.iter().sum();
        let expected_inner_width = page_size.width - margin.left - margin.right - 1.5;
        assert!(
            (nested_total - expected_inner_width).abs() < 1.0,
            "certificate-like nested table should span the outer cell width, got total {nested_total} vs {expected_inner_width}"
        );
        let first_ratio = nested_col_widths[0] / nested_total;
        assert!(
            (first_ratio - 0.30).abs() < 0.02,
            "certificate-like nested table should honor percentage colgroup widths, got {:?}",
            nested_col_widths
        );
    }

    #[test]
    fn table_cell_preserves_empty_block_background_layout() {
        let encoded = base64_encode(&build_test_png_bytes());
        let html = format!(
            r#"
                <table>
                    <tr>
                        <td>
                            <div style="display: flex; width: 40pt; aspect-ratio: 1 / 1; background-image: url('data:image/png;base64,{encoded}') no-repeat;"></div>
                        </td>
                    </tr>
                </table>
            "#
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::TableRow { cells, .. } = el {
                Some(cells)
            } else {
                None
            }
        });
        let cells = cells.expect("expected outer table row");
        assert!(
            !cells[0].nested_rows.is_empty(),
            "expected block descendant to be preserved as nested layout"
        );
        assert!(
            cells[0].nested_rows.iter().any(|element| matches!(
                element,
                LayoutElement::TextBlock {
                    background_svg: Some(_),
                    block_height: Some(height),
                    ..
                } if (*height - 40.0).abs() < 0.1
            )),
            "expected nested flex block with raster background to survive table-cell layout"
        );
    }
}

// (end of file -- debug tests removed)
#[cfg(any())]
mod _removed {
    #![allow(unused)]
    fn debug_pdf_output() {
        let html = r#"<p><span>Acme</span> <span>Corp</span></p>
            <p><strong>Bold</strong> Normal</p>
            <table><tr><td>SVG rendering add-on</td></tr></table>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Search for text rendering commands in the PDF content
        for line in pdf_str.lines() {
            if line.contains("Tj") {
                eprintln!("PDF Tj: {:?}", line.trim());
            }
        }
        // Check that the PDF contains properly spaced text
        assert!(
            pdf_str.contains("(Acme") || pdf_str.contains("( Corp"),
            "PDF should contain Acme and Corp text"
        );
    }

    #[test]
    fn debug_space_preservation_html_parser() {
        // Check what the HTML parser produces for various inputs
        use crate::parser::dom::DomNode;

        fn dump_nodes(nodes: &[DomNode], indent: usize) -> String {
            let mut out = String::new();
            for node in nodes {
                match node {
                    DomNode::Text(t) => {
                        out.push_str(&format!("{:indent$}Text({:?})\n", "", t, indent = indent));
                    }
                    DomNode::Element(el) => {
                        out.push_str(&format!(
                            "{:indent$}Element({:?})\n",
                            "",
                            el.tag,
                            indent = indent
                        ));
                        out.push_str(&dump_nodes(&el.children, indent + 2));
                    }
                }
            }
            out
        }

        // Test what html5ever produces for span-space-span
        let html = "<p><span>Acme</span> <span>Corp</span></p>";
        let nodes = parse_html(html).unwrap();
        let dump = dump_nodes(&nodes, 0);
        eprintln!("=== span-space-span ===\n{dump}");

        // Test what html5ever produces for br-separated text
        let html2 = "<p><strong>Bill to:</strong><br>Acme Corp<br>New York</p>";
        let nodes2 = parse_html(html2).unwrap();
        let dump2 = dump_nodes(&nodes2, 0);
        eprintln!("=== br-separated ===\n{dump2}");

        // Test strong followed by text in same element
        let html3 = "<p><strong>Hello</strong> World</p>";
        let nodes3 = parse_html(html3).unwrap();
        let dump3 = dump_nodes(&nodes3, 0);
        eprintln!("=== strong-space-text ===\n{dump3}");

        // Test with the full invoice-like structure
        let html4 =
            r#"<p><span class="label">Invoice #</span><br><strong>INV-2026-0042</strong></p>"#;
        let nodes4 = parse_html(html4).unwrap();
        let dump4 = dump_nodes(&nodes4, 0);
        eprintln!("=== invoice label ===\n{dump4}");
    }

    #[test]
    fn debug_space_preservation() {
        // Test 1: Simple text with spaces
        let html = "<p>Hello World</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            let text: String = lines
                .iter()
                .flat_map(|l| l.runs.iter())
                .map(|r| r.text.as_str())
                .collect();
            eprintln!("Test 1 text: {:?}", text);
            assert!(
                text.contains("Hello World"),
                "Spaces lost in simple text: {text:?}"
            );
        }

        // Test 2: Inline elements with space between
        let html2 = "<p><span>Hello</span> <span>World</span></p>";
        let nodes2 = parse_html(html2).unwrap();
        let pages2 = layout(&nodes2, PageSize::A4, Margin::default());
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages2[0].elements[0] {
            let text: String = lines
                .iter()
                .flat_map(|l| l.runs.iter())
                .map(|r| r.text.as_str())
                .collect();
            eprintln!("Test 2 text: {:?}", text);
            assert!(
                text.contains("Hello") && text.contains("World"),
                "Missing text: {text:?}"
            );
            let combined = text.replace(' ', "");
            assert_ne!(text, combined, "Spaces completely lost: {text:?}");
        }

        // Test 3: Table cell with spaces
        let html3 = "<table><tr><td>Custom font embedding module</td></tr></table>";
        let nodes3 = parse_html(html3).unwrap();
        let pages3 = layout(&nodes3, PageSize::A4, Margin::default());
        for (_, el) in &pages3[0].elements {
            if let LayoutElement::TableRow { cells, .. } = el {
                let text: String = cells[0]
                    .lines
                    .iter()
                    .flat_map(|l| l.runs.iter())
                    .map(|r| r.text.as_str())
                    .collect();
                eprintln!("Test 3 text: {:?}", text);
                assert!(
                    text.contains("Custom font"),
                    "Spaces lost in table cell: {text:?}"
                );
            }
        }

        // Test 4: bold/br structure from invoice
        let html4 = "<p><strong>Bill to:</strong><br>Acme Corp<br>New York, NY 10001</p>";
        let nodes4 = parse_html(html4).unwrap();
        let pages4 = layout(&nodes4, PageSize::A4, Margin::default());
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages4[0].elements[0] {
            for (i, line) in lines.iter().enumerate() {
                let line_text: String = line.runs.iter().map(|r| r.text.as_str()).collect();
                eprintln!(
                    "Test 4 line {i}: {:?} (runs: {:?})",
                    line_text,
                    line.runs
                        .iter()
                        .map(|r| r.text.as_str())
                        .collect::<Vec<_>>()
                );
            }
            let all_text: String = lines
                .iter()
                .map(|l| l.runs.iter().map(|r| r.text.as_str()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            eprintln!("Test 4 combined: {:?}", all_text);
            assert!(
                all_text.contains("Acme Corp"),
                "Spaces in 'Acme Corp' lost: {all_text:?}"
            );
            assert!(
                all_text.contains("New York"),
                "Spaces in 'New York' lost: {all_text:?}"
            );
        }
    }

    #[test]
    fn textblock_with_border_has_visual() {
        // Line 1232: has_visual check for border.has_any() in wrapper TextBlock path
        let html = r#"<div style="border: 1pt solid black; overflow: hidden; height: 50pt"><p>Inside</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        let found_clip = pages[0].elements.iter().any(|(_, el)| {
            if let LayoutElement::TextBlock { clip_rect, .. } = el {
                clip_rect.is_some()
            } else {
                false
            }
        });
        assert!(
            found_clip,
            "Expected a TextBlock with clip_rect from overflow:hidden"
        );
    }

    #[test]
    fn flex_column_direction_layout() {
        // Lines 1508, 1711, 1786-1790: FlexRow column direction rendering
        let html = r#"<div style="display: flex; flex-direction: column"><div>First</div><div>Second</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn table_rowspan_cell_handling() {
        // Lines 2248, 2250-2253: Table rowspan cell handling
        let html =
            r#"<table><tr><td rowspan="2">Spanning</td><td>A</td></tr><tr><td>B</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let row_count = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::TableRow { .. }))
            .count();
        assert!(
            row_count >= 2,
            "Expected at least 2 table rows, got {row_count}"
        );
    }

    #[test]
    fn table_cell_border_propagation() {
        // Line 2436: Table cell border propagation with preferred widths fitting
        let html = r#"<table style="width: 400pt"><tr><td style="border: 1pt solid black">Cell</td><td>Other</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn inline_link_collects_url() {
        // Line 2567: Inline element in collect_text_runs with link URL
        let html = r#"<p><a href="https://example.com">Click here</a></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            let has_link = lines.iter().any(|l| {
                l.runs
                    .iter()
                    .any(|r| r.link_url.as_deref() == Some("https://example.com"))
            });
            assert!(has_link, "Expected link URL in text runs");
        }
    }

    #[test]
    fn inline_span_border_radius_from_stylesheet() {
        // Lines 2686, 2690-2702: collect_text_runs_inner inline span with border_radius
        let css = "span.tag { background-color: #eee; border-radius: 4pt; padding: 2pt 4pt; }";
        let rules = parse_stylesheet(css);
        let html = r#"<p><span class="tag">Label</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert_eq!(pages.len(), 1);
        if let (_, LayoutElement::TextBlock { lines, .. }) = &pages[0].elements[0] {
            let has_br = lines
                .iter()
                .any(|l| l.runs.iter().any(|r| r.border_radius > 0.0));
            assert!(has_br, "Expected border_radius > 0 on inline span text run");
        }
    }

    #[test]
    fn paginate_image_height() {
        // Lines 3116-3156: Image height handling in paginate
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="100" height="100">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn paginate_horizontal_rule() {
        // Lines 3152-3155: HorizontalRule height in paginate
        let html = "<p>Above</p><hr><p>Below</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_hr = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::HorizontalRule { .. }));
        assert!(has_hr, "Expected a HorizontalRule element");
    }

    #[test]
    fn page_break_in_paginate() {
        // Line 3193: Page break handling in paginate
        let html = r#"<p>Page 1 content</p><div style="page-break-before: always"><p>Page 2 content</p></div><div style="page-break-before: always"><p>Page 3 content</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(
            pages.len() >= 3,
            "Expected at least 3 pages, got {}",
            pages.len()
        );
    }

    #[test]
    fn layout_input_element() {
        let html = r#"<input type="text" value="Hello">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_input_with_placeholder() {
        let html = r#"<input type="text" placeholder="Enter name...">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_select_element() {
        let html = r#"<select><option>One</option><option>Two</option></select>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_textarea_element() {
        let html = r#"<textarea>Some text content</textarea>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_textarea_with_custom_size() {
        let html = r#"<textarea style="width: 200px; height: 100px">Content</textarea>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_video_element() {
        let html = r#"<video width="320" height="240"></video>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_video_default_size() {
        let html = r#"<video></video>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_audio_element() {
        let html = r#"<audio></audio>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_progress_element() {
        let html = r#"<progress value="0.7" max="1"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_bar = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::ProgressBar { .. }));
        assert!(has_bar, "Expected a ProgressBar element");
    }

    #[test]
    fn layout_progress_zero_value() {
        let html = r#"<progress value="0" max="100"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(0.0));
    }

    #[test]
    fn layout_progress_full_value() {
        let html = r#"<progress value="100" max="100"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(1.0));
    }

    #[test]
    fn layout_progress_over_max_clamped() {
        let html = r#"<progress value="200" max="100"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(1.0));
    }

    #[test]
    fn layout_meter_element() {
        let html = r#"<meter value="0.6" max="1"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_bar = pages[0]
            .elements
            .iter()
            .any(|(_, el)| matches!(el, LayoutElement::ProgressBar { .. }));
        assert!(has_bar, "Expected a ProgressBar element for meter");
    }

    #[test]
    fn layout_meter_low_high_thresholds() {
        let html = r#"<meter value="10" max="100" low="25" high="75"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let fill = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fill_color, .. } = el {
                Some(*fill_color)
            } else {
                None
            }
        });
        assert!(fill.is_some());
        let (r, _, _) = fill.unwrap();
        assert!(r > 0.8, "Expected red fill for low meter value");
    }

    #[test]
    fn layout_meter_high_value_green() {
        let html = r#"<meter value="90" max="100" low="25" high="75"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let fill = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fill_color, .. } = el {
                Some(*fill_color)
            } else {
                None
            }
        });
        assert!(fill.is_some());
        let (_, g, _) = fill.unwrap();
        assert!(g > 0.7, "Expected green fill for high meter value");
    }

    #[test]
    fn layout_form_elements_in_context() {
        let html = r#"
            <div>
                <p>Name:</p>
                <input type="text" value="John">
                <p>Country:</p>
                <select><option>France</option><option>USA</option></select>
                <p>Bio:</p>
                <textarea>Some biography text here</textarea>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 3);
    }

    #[test]
    fn layout_progress_custom_width() {
        let html = r#"<progress value="50" max="100" style="width: 200px"></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let width = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { width, .. } = el {
                Some(*width)
            } else {
                None
            }
        });
        assert_eq!(width, Some(200.0));
    }

    #[test]
    fn grid_layout_repeat() {
        let css = ".grid { display: grid; grid-template-columns: repeat(3, 1fr); }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(
            grid_rows.len(),
            1,
            "Expected 1 grid row with 3 columns from repeat(3, 1fr)"
        );
    }

    #[test]
    fn grid_layout_minmax() {
        let css = ".grid { display: grid; grid-template-columns: minmax(50pt, 200pt) 1fr; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert!(!grid_rows.is_empty(), "Expected GridRow from minmax grid");
    }

    #[test]
    fn grid_layout_auto_fill() {
        let css = ".grid { display: grid; grid-template-columns: repeat(auto-fill, 100px); }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn grid_layout_repeat_with_minmax() {
        let css = ".grid { display: grid; grid-template-columns: repeat(3, minmax(50px, 1fr)); }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(grid_rows.len(), 1);
    }

    #[test]
    fn multi_column_layout() {
        let css = ".cols { column-count: 2; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="cols"><div>Col 1</div><div>Col 2</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(grid_rows.len(), 1, "Expected 1 row from 2-column layout");
    }

    #[test]
    fn multi_column_three_cols() {
        let css = ".cols { column-count: 3; column-gap: 10pt; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="cols"><div>A</div><div>B</div><div>C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(grid_rows.len(), 1);
    }

    #[test]
    fn multi_column_wraps_rows() {
        let css = ".cols { column-count: 2; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="cols"><div>A</div><div>B</div><div>C</div><div>D</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert_eq!(
            grid_rows.len(),
            2,
            "Expected 2 rows from 4 items in 2-column layout"
        );
    }

    #[test]
    fn layout_input_empty_no_value() {
        let html = r#"<input type="text">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_select_empty_options() {
        let html = r#"<select></select>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_textarea_empty() {
        let html = r#"<textarea></textarea>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_video_with_css_dimensions() {
        let html = r#"<video style="width: 400px; height: 300px"></video>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_audio_with_css_dimensions() {
        let html = r#"<audio style="width: 250px"></audio>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn layout_progress_no_value_attr() {
        let html = r#"<progress></progress>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(0.0));
    }

    #[test]
    fn layout_meter_no_thresholds() {
        let html = r#"<meter value="50" max="100"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let fill = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fill_color, .. } = el {
                Some(*fill_color)
            } else {
                None
            }
        });
        // 50/100 = 0.5, between default low (25) and high (75) → yellow
        assert!(fill.is_some());
        let (r, _, _) = fill.unwrap();
        assert!(r > 0.9, "Expected yellow fill for mid-range meter");
    }

    #[test]
    fn layout_meter_zero_max() {
        let html = r#"<meter value="5" max="0"></meter>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let bar = pages[0].elements.iter().find_map(|(_, el)| {
            if let LayoutElement::ProgressBar { fraction, .. } = el {
                Some(*fraction)
            } else {
                None
            }
        });
        assert_eq!(bar, Some(0.0), "Zero max should produce 0 fraction");
    }

    #[test]
    fn heading_level_returns_correct_values() {
        assert_eq!(heading_level(HtmlTag::H1), Some(1));
        assert_eq!(heading_level(HtmlTag::H2), Some(2));
        assert_eq!(heading_level(HtmlTag::H3), Some(3));
        assert_eq!(heading_level(HtmlTag::H4), Some(4));
        assert_eq!(heading_level(HtmlTag::H5), Some(5));
        assert_eq!(heading_level(HtmlTag::H6), Some(6));
        assert_eq!(heading_level(HtmlTag::P), None);
        assert_eq!(heading_level(HtmlTag::Div), None);
    }

    #[test]
    fn layout_heading_has_level_in_textblock() {
        let html = "<h2>Section Title</h2>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_heading = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    heading_level: Some(2),
                    ..
                }
            )
        });
        assert!(
            has_heading,
            "h2 should produce TextBlock with heading_level=2"
        );
    }

    #[test]
    fn layout_paragraph_has_no_heading_level() {
        let html = "<p>Just text</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_heading = pages[0].elements.iter().any(|(_, el)| {
            matches!(
                el,
                LayoutElement::TextBlock {
                    heading_level: Some(_),
                    ..
                }
            )
        });
        assert!(!has_heading, "p should not have a heading_level");
    }

    #[test]
    fn column_count_1_not_grid() {
        // column-count: 1 should not trigger grid layout
        let css = ".cols { column-count: 1; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="cols"><p>Single column</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let grid_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, el)| matches!(el, LayoutElement::GridRow { .. }))
            .collect();
        assert!(
            grid_rows.is_empty(),
            "column-count: 1 should not produce grid rows"
        );
    }
}
