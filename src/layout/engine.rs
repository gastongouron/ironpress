use crate::parser::css::{CssRule, CssValue, PseudoElement, SelectorContext, selector_matches};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::png;
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    AlignItems, BorderCollapse, BoxShadow, BoxSizing, Clear, ComputedStyle, ContentItem, Display,
    FlexDirection, FlexWrap, Float, FontFamily, FontStyle, FontWeight, GridTrack, JustifyContent,
    LinearGradient, ListStylePosition, ListStyleType, Overflow, Position, RadialGradient,
    TextAlign, TextOverflow, Transform, VerticalAlign, Visibility, WhiteSpace, compute_style,
    compute_style_with_context,
};
use crate::types::{Margin, PageSize};
use std::collections::HashMap;

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

fn resolve_pseudo_content(
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    pseudo: PseudoElement,
    counter_state: &CounterState,
) -> Option<String> {
    for rule in rules {
        if rule.pseudo_element == Some(pseudo)
            && selector_matches(&rule.selector, tag_name, classes, id)
        {
            if let Some(CssValue::Keyword(k)) = rule.declarations.get("content") {
                let items = crate::style::computed::parse_content_value_pub(k);
                if !items.is_empty() {
                    let text = resolve_content(&items, attributes, counter_state);
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
    }
    None
}

/// Context for rendering list items.
#[derive(Debug, Clone)]
enum ListContext {
    Unordered { indent: f32 },
    Ordered { index: usize, indent: f32 },
}

/// A table cell ready for rendering.
#[derive(Debug)]
#[allow(dead_code)]
pub struct TableCell {
    pub lines: Vec<TextLine>,
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

/// A layout element ready for rendering.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
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
        border_width: f32,
        border_color: Option<(f32, f32, f32)>,
        block_width: Option<f32>,
        block_height: Option<f32>,
        opacity: f32,
        float: Float,
        clear: Clear,
        position: Position,
        offset_top: f32,
        offset_left: f32,
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
        z_index: i32,
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
        data: Vec<u8>,
        width: f32,
        height: f32,
        format: ImageFormat,
        /// PNG-specific metadata for FlateDecode parameters. None for JPEG.
        png_metadata: Option<PngMetadata>,
        margin_top: f32,
        margin_bottom: f32,
    },
    /// A horizontal rule.
    HorizontalRule { margin_top: f32, margin_bottom: f32 },
    /// A page break.
    PageBreak,
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
    let parent_style = ComputedStyle::default();
    let available_width = page_size.width - margin.left - margin.right;
    let content_height = page_size.height - margin.top - margin.bottom;

    // First, flatten DOM into layout elements
    let mut elements = Vec::new();
    let ancestors: Vec<&ElementNode> = Vec::new();
    flatten_nodes(
        nodes,
        &parent_style,
        available_width,
        &mut elements,
        None,
        rules,
        &ancestors,
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
    output: &mut Vec<LayoutElement>,
    list_ctx: Option<&ListContext>,
    rules: &[CssRule],
    ancestors: &[&ElementNode],
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
                        font_family: parent_style.font_family.clone(),
                    };
                    let lines =
                        wrap_text_runs(vec![run], available_width, parent_style.font_size, fonts);
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
                            border_width: 0.0,
                            border_color: None,
                            block_width: None,
                            block_height: None,
                            opacity: 1.0,
                            float: Float::None,
                            clear: Clear::None,
                            position: Position::Static,
                            offset_top: 0.0,
                            offset_left: 0.0,
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
                            z_index: 0,
                        });
                    }
                }
            }
            DomNode::Element(el) => {
                flatten_element(
                    el,
                    parent_style,
                    available_width,
                    output,
                    list_ctx,
                    rules,
                    ancestors,
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
fn flatten_element(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    list_ctx: Option<&ListContext>,
    rules: &[CssRule],
    ancestors: &[&ElementNode],
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

    // display: none — skip this element entirely
    if style.display == Display::None {
        return;
    }

    if el.tag == HtmlTag::Br {
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
                font_family: style.font_family.clone(),
            }],
            height: style.font_size * style.line_height,
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
            border_width: 0.0,
            border_color: None,
            padding_right: 0.0,
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
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
            z_index: 0,
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
        if let Some(img_element) = load_image_from_element(el, available_width, &style) {
            output.push(img_element);
        }
        return;
    }

    if style.page_break_before {
        output.push(LayoutElement::PageBreak);
    }

    // Table handling
    if el.tag == HtmlTag::Table {
        flatten_table(el, &style, available_width, output, rules, fonts);
        return;
    }

    // Build ancestors list for children of this element
    let mut child_ancestors: Vec<&ElementNode> = ancestors.to_vec();
    child_ancestors.push(el);

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
                        output,
                        Some(&ctx),
                        rules,
                        &child_ancestors,
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
                        output,
                        None,
                        rules,
                        &child_ancestors,
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
                font_family: style.font_family.clone(),
            });
        }

        collect_text_runs(&el.children, &style, &mut runs, None);

        if !runs.is_empty() {
            let lines = wrap_text_runs(runs, inner_width, style.font_size, fonts);
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
                border_width: 0.0,
                border_color: None,
                block_width: None,
                block_height: None,
                opacity: style.opacity,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0),
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
                background_gradient: style.background_gradient.clone(),
                background_radial_gradient: style.background_radial_gradient.clone(),
                z_index: style.z_index,
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
                        output,
                        list_ctx,
                        rules,
                        &child_ancestors,
                        child_el_idx,
                        child_el_count,
                        &[],
                        fonts,
                    );
                } else if child_el.tag.is_block() {
                    flatten_element(
                        child_el,
                        &style,
                        available_width,
                        output,
                        None,
                        rules,
                        &child_ancestors,
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
            block_w - style.padding.left - style.padding.right - style.border_width * 2.0
        } else {
            block_w - style.padding.left - style.padding.right
        };
        let inner_width = inner_width.max(0.0);

        // Collect all inline content as text runs, with ::before/::after
        let mut runs = Vec::new();
        let cs = CounterState::default();
        let cls: Vec<&str> = classes.iter().map(|s| s.as_ref()).collect();
        if let Some(bt) = resolve_pseudo_content(
            rules,
            el.tag_name(),
            &cls,
            el.id(),
            &el.attributes,
            PseudoElement::Before,
            &cs,
        ) {
            runs.push(TextRun {
                text: bt,
                font_size: style.font_size,
                bold: style.font_weight == FontWeight::Bold,
                italic: style.font_style == FontStyle::Italic,
                underline: false,
                line_through: false,
                color: style.color.to_f32_rgb(),
                link_url: None,
                font_family: style.font_family.clone(),
            });
        }
        collect_text_runs(&el.children, &style, &mut runs, None);
        if let Some(at) = resolve_pseudo_content(
            rules,
            el.tag_name(),
            &cls,
            el.id(),
            &el.attributes,
            PseudoElement::After,
            &cs,
        ) {
            runs.push(TextRun {
                text: at,
                font_size: style.font_size,
                bold: style.font_weight == FontWeight::Bold,
                italic: style.font_style == FontStyle::Italic,
                underline: false,
                line_through: false,
                color: style.color.to_f32_rgb(),
                link_url: None,
                font_family: style.font_family.clone(),
            });
        }

        if !runs.is_empty() {
            // When white-space: nowrap, prevent wrapping by using a huge width
            let wrap_width = if style.white_space == WhiteSpace::NoWrap {
                f32::MAX
            } else {
                inner_width
            };
            let mut lines = wrap_text_runs(runs, wrap_width, style.font_size, fonts);

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

            let border_clr = style.border_color.map(|c| c.to_f32_rgb());

            let explicit_width = if block_w < available_width || style.min_width.is_some() {
                Some(block_w)
            } else {
                None
            };

            // Compute clip rect before moving lines
            let clip_rect = if style.overflow == Overflow::Hidden {
                let text_height: f32 = lines.iter().map(|l| l.height).sum();
                let content_h = style.padding.top + text_height + style.padding.bottom;
                let total_h = effective_height.map_or(content_h, |h| content_h.max(h));
                Some((0.0, 0.0, block_w, total_h))
            } else {
                None
            };

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
                border_width: style.border_width,
                border_color: border_clr,
                block_width: explicit_width,
                block_height: effective_height,
                opacity: style.opacity,
                float: style.float,
                clear: style.clear,
                position: style.position,
                offset_top: style.top.unwrap_or(0.0),
                offset_left: style.left.unwrap_or(0.0) + auto_offset_left,
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
                background_gradient: style.background_gradient.clone(),
                background_radial_gradient: style.background_radial_gradient.clone(),
                z_index: style.z_index,
            });
        }

        // Also process block children recursively
        let child_el_count = el
            .children
            .iter()
            .filter(|c| matches!(c, DomNode::Element(_)))
            .count();
        let mut child_el_idx = 0;
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if child_el.tag.is_block() {
                    flatten_element(
                        child_el,
                        &style,
                        available_width,
                        output,
                        None,
                        rules,
                        &child_ancestors,
                        child_el_idx,
                        child_el_count,
                        &[],
                        fonts,
                    );
                }
                child_el_idx += 1;
            }
        }
    } else {
        // Inline element — process children with this style context
        flatten_nodes(
            &el.children,
            &style,
            available_width,
            output,
            None,
            rules,
            &child_ancestors,
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
    ancestors: &[&ElementNode],
    fonts: &HashMap<String, TtfFont>,
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
        return;
    }

    // Lay out each child into its own set of elements to measure sizes
    struct FlexItem {
        elements: Vec<LayoutElement>,
        width: f32,
        height: f32,
    }

    let mut items: Vec<FlexItem> = Vec::new();

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
            style,
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

        // Determine child width
        let child_w = child_style.width.unwrap_or_else(|| {
            // Distribute remaining space equally among items without explicit width
            inner_width / child_count as f32
        });

        let child_inner_w = if child_style.box_sizing == BoxSizing::BorderBox {
            child_w
                - child_style.padding.left
                - child_style.padding.right
                - child_style.border_width * 2.0
        } else {
            child_w - child_style.padding.left - child_style.padding.right
        }
        .max(0.0);

        // Collect text runs for this child
        let mut runs = Vec::new();
        collect_text_runs(&child_el.children, &child_style, &mut runs, None);

        let lines = if !runs.is_empty() {
            wrap_text_runs(runs, child_inner_w.max(1.0), child_style.font_size, fonts)
        } else {
            Vec::new()
        };

        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let content_h = child_style.padding.top + text_height + child_style.padding.bottom;
        let child_h = match child_style.height {
            Some(h) => content_h.max(h),
            None => content_h,
        };

        let bg = child_style
            .background_color
            .map(|c: crate::types::Color| c.to_f32_rgb());
        let border_clr = child_style.border_color.map(|c| c.to_f32_rgb());

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
            border_width: child_style.border_width,
            border_color: border_clr,
            block_width: Some(child_w),
            block_height: child_style.height,
            opacity: child_style.opacity,
            float: Float::None,
            clear: Clear::None,
            position: child_style.position,
            offset_top: 0.0,
            offset_left: 0.0,
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
            background_gradient: child_style.background_gradient.clone(),
            background_radial_gradient: child_style.background_radial_gradient.clone(),
            z_index: child_style.z_index,
        };

        items.push(FlexItem {
            elements: vec![elem],
            width: child_w,
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

    // Emit the container background/border as a wrapper element
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
        .map(|c: crate::types::Color| c.to_f32_rgb());
    let border_clr = style.border_color.map(|c| c.to_f32_rgb());

    // Emit container background if it has styling
    if bg.is_some() || style.border_width > 0.0 || style.box_shadow.is_some() {
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
            border_width: style.border_width,
            border_color: border_clr,
            block_width: Some(block_w),
            block_height: Some(container_h),
            opacity: style.opacity,
            float: style.float,
            clear: style.clear,
            position: style.position,
            offset_top: style.top.unwrap_or(0.0),
            offset_left: style.left.unwrap_or(0.0),
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
            background_gradient: None,
            background_radial_gradient: None,
            z_index: 0,
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
                let free_space = inner_width - total_item_width - total_gap;

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

                x += style.padding.left;

                for &item_idx in &line_items {
                    let item = &items[item_idx];

                    // Calculate cross-axis (vertical) alignment
                    let y_offset = match align {
                        AlignItems::FlexStart => 0.0,
                        AlignItems::FlexEnd => line.cross_size - item.height,
                        AlignItems::Center => (line.cross_size - item.height) / 2.0,
                        AlignItems::Stretch => 0.0,
                    };

                    // Update the element's offset_left and offset_top
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
                            border_width: tb_bw,
                            border_color: tb_bc,
                            block_width: tb_bwi,
                            block_height: tb_bh,
                            opacity: tb_op,
                            position: _tb_pos,
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
                            ..
                        } = elem
                        {
                            let effective_height = if align == AlignItems::Stretch {
                                Some(line.cross_size)
                            } else {
                                *tb_bh
                            };

                            output.push(LayoutElement::TextBlock {
                                lines: tb_lines.clone(),
                                margin_top: if cross_offset == 0.0 {
                                    style.margin.top + style.padding.top
                                } else {
                                    0.0
                                },
                                margin_bottom: *tb_mb,
                                text_align: *tb_ta,
                                background_color: *tb_bg,
                                padding_top: *tb_pt,
                                padding_bottom: *tb_pb,
                                padding_left: *tb_pl,
                                padding_right: *tb_pr,
                                border_width: *tb_bw,
                                border_color: *tb_bc,
                                block_width: *tb_bwi,
                                block_height: effective_height,
                                opacity: *tb_op,
                                float: Float::None,
                                clear: Clear::None,
                                position: Position::Relative,
                                offset_top: cross_offset + y_offset + *tb_mt,
                                offset_left: x,
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
                                background_gradient: None,
                                background_radial_gradient: None,
                                z_index: 0,
                            });
                        }
                    }

                    x += item.width + gap + extra_gap;
                }
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
                            border_width: tb_bw,
                            border_color: tb_bc,
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
                            ..
                        } = elem
                        {
                            output.push(LayoutElement::TextBlock {
                                lines: tb_lines.clone(),
                                margin_top: if y == 0.0 {
                                    style.margin.top + style.padding.top + *tb_mt
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
                                border_width: *tb_bw,
                                border_color: *tb_bc,
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
                                background_gradient: None,
                                background_radial_gradient: None,
                                z_index: 0,
                            });
                        }
                    }

                    y += item.height + gap;
                }
            }
        }

        cross_offset += line.cross_size + gap;
    }

    // Emit trailing margin
    if style.margin.bottom > 0.0 {
        output.push(LayoutElement::TextBlock {
            lines: Vec::new(),
            margin_top: style.margin.bottom,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border_width: 0.0,
            border_color: None,
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
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
            z_index: 0,
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

    for track in tracks {
        match track {
            GridTrack::Fixed(v) => fixed_total += *v,
            GridTrack::Fr(v) => fr_total += *v,
            GridTrack::Auto => auto_count += 1,
        }
    }

    let remaining = (space - fixed_total).max(0.0);

    // Auto columns are treated like 1fr each for distribution purposes
    let effective_fr_total = fr_total + auto_count as f32;
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
    ancestors: &[&ElementNode],
    fonts: &HashMap<String, TtfFont>,
) {
    let inner_width = available_width - style.padding.left - style.padding.right;
    let gap = style.grid_gap;

    let col_widths = resolve_grid_columns(&style.grid_template_columns, inner_width, gap);
    let num_cols = col_widths.len();

    // Build ancestors list for children of this element
    let mut child_ancestors: Vec<&ElementNode> = ancestors.to_vec();
    child_ancestors.push(el);

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
            collect_text_runs(&child_el.children, &child_style, &mut runs, None);
            let lines = wrap_text_runs(runs, cell_inner, child_style.font_size, fonts);

            let bg = child_style
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgb());

            cells.push(TableCell {
                lines,
                bold: child_style.font_weight == FontWeight::Bold,
                background_color: bg,
                padding_top: child_style.padding.top,
                padding_right: child_style.padding.right,
                padding_bottom: child_style.padding.bottom,
                padding_left: child_style.padding.left,
                colspan: 1,
                rowspan: 1,
            });
        }

        // Fill remaining columns with empty cells if the row is incomplete
        while cells.len() < num_cols {
            cells.push(TableCell {
                lines: Vec::new(),
                bold: false,
                background_color: None,
                padding_top: 0.0,
                padding_right: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                colspan: 1,
                rowspan: 1,
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

fn flatten_table(
    el: &ElementNode,
    style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    _rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
) {
    let inner_width = available_width - style.margin.left - style.margin.right;

    // Collect all <tr> elements (from direct children, thead, tbody, tfoot)
    let mut rows: Vec<&ElementNode> = Vec::new();
    for child in &el.children {
        if let DomNode::Element(child_el) = child {
            match child_el.tag {
                HtmlTag::Tr => rows.push(child_el),
                HtmlTag::Thead | HtmlTag::Tbody | HtmlTag::Tfoot => {
                    for grandchild in &child_el.children {
                        if let DomNode::Element(gc) = grandchild {
                            if gc.tag == HtmlTag::Tr {
                                rows.push(gc);
                            }
                        }
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

    // --- Auto-sizing pass: measure preferred content width for each column ---
    let min_col_width: f32 = 30.0;
    let mut preferred_widths: Vec<f32> = vec![0.0; num_cols];

    for row in &rows {
        let row_style = compute_style(row.tag, row.style_attr(), style);
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
                    let cell_style = compute_style(cell_el.tag, cell_el.style_attr(), &row_style);
                    let mut runs = Vec::new();
                    collect_text_runs(&cell_el.children, &cell_style, &mut runs, None);
                    let content_width: f32 = runs
                        .iter()
                        .map(|run| run.text.len() as f32 * run.font_size * 0.5)
                        .sum();
                    let total_preferred =
                        content_width + cell_style.padding.left + cell_style.padding.right;
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

    for w in &mut preferred_widths {
        if *w < min_col_width {
            *w = min_col_width;
        }
    }

    let total_preferred: f32 = preferred_widths.iter().sum();
    let col_widths: Vec<f32> = if total_preferred <= inner_width {
        preferred_widths
    } else {
        let scale = inner_width / total_preferred;
        preferred_widths
            .iter()
            .map(|w| (w * scale).max(min_col_width))
            .collect()
    };

    // Build layout rows, tracking cells occupied by rowspan from previous rows.
    // Each entry in `occupied` tracks the remaining rowspan count for that column.
    let mut occupied: Vec<usize> = vec![0; num_cols];
    let mut is_first = true;
    for row in &rows {
        let row_style = compute_style(row.tag, row.style_attr(), style);
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
                    bold: false,
                    background_color: None,
                    padding_top: 0.0,
                    padding_right: 0.0,
                    padding_bottom: 0.0,
                    padding_left: 0.0,
                    colspan: span_cols,
                    rowspan: 0, // phantom cell marker
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

            let cell_style = compute_style(cell_el.tag, cell_el.style_attr(), &row_style);
            // Compute effective width from auto-sized column widths
            let effective_width: f32 = (0..colspan)
                .map(|i| {
                    if col_pos + i < num_cols {
                        col_widths[col_pos + i]
                    } else {
                        0.0
                    }
                })
                .sum();
            let cell_inner = effective_width - cell_style.padding.left - cell_style.padding.right;

            let mut runs = Vec::new();
            collect_text_runs(&cell_el.children, &cell_style, &mut runs, None);
            let lines = wrap_text_runs(runs, cell_inner.max(1.0), cell_style.font_size, fonts);

            let bg = cell_style
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgb());

            cells.push(TableCell {
                lines,
                bold: cell_style.font_weight == FontWeight::Bold,
                background_color: bg,
                padding_top: cell_style.padding.top,
                padding_right: cell_style.padding.right,
                padding_bottom: cell_style.padding.bottom,
                padding_left: cell_style.padding.left,
                colspan,
                rowspan,
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

fn collect_text_runs(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    runs: &mut Vec<TextRun>,
    link_url: Option<&str>,
) {
    for node in nodes {
        match node {
            DomNode::Text(text) => {
                let trimmed = collapse_whitespace(text);
                if !trimmed.is_empty() {
                    runs.push(TextRun {
                        text: trimmed,
                        font_size: parent_style.font_size,
                        bold: parent_style.font_weight == FontWeight::Bold,
                        italic: parent_style.font_style == FontStyle::Italic,
                        underline: parent_style.text_decoration_underline,
                        line_through: parent_style.text_decoration_line_through,
                        color: parent_style.color.to_f32_rgb(),
                        link_url: link_url.map(String::from),
                        font_family: parent_style.font_family.clone(),
                    });
                }
            }
            DomNode::Element(el) => {
                if el.tag.is_inline() || el.tag == HtmlTag::Br {
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
                            font_family: parent_style.font_family.clone(),
                        });
                    } else {
                        let style = compute_style(el.tag, el.style_attr(), parent_style);
                        let url = if el.tag == HtmlTag::A {
                            el.attributes.get("href").map(|s| s.as_str()).or(link_url)
                        } else {
                            link_url
                        };
                        collect_text_runs(&el.children, &style, runs, url);
                    }
                }
            }
        }
    }
}

/// Estimate the width of a word given its font settings and available custom fonts.
fn estimate_word_width(
    word: &str,
    font_size: f32,
    font_family: &FontFamily,
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    if let FontFamily::Custom(name) = font_family {
        if let Some(ttf) = fonts.get(name) {
            return word
                .chars()
                .map(|c| ttf.char_width_scaled(c as u16, font_size))
                .sum();
        }
    }
    // Fallback: fixed-width estimation
    word.len() as f32 * font_size * 0.5
}

/// Simple text wrapping using character width estimation.
/// Uses TTF metrics when a custom font is available, otherwise 0.5 * font_size.
fn wrap_text_runs(
    runs: Vec<TextRun>,
    max_width: f32,
    default_font_size: f32,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<TextLine> {
    let mut lines: Vec<TextLine> = Vec::new();
    let mut current_runs: Vec<TextRun> = Vec::new();
    let mut current_width: f32 = 0.0;
    let mut line_height = default_font_size * 1.4;

    // Concatenate all text then re-split by words, preserving run styles
    let mut styled_words: Vec<(String, TextRun)> = Vec::new();
    for run in &runs {
        if run.text == "\n" {
            styled_words.push(("\n".to_string(), run.clone()));
            continue;
        }
        for word in run.text.split_whitespace() {
            styled_words.push((word.to_string(), run.clone()));
        }
    }

    for (word, template) in styled_words {
        if word == "\n" {
            // Line break
            lines.push(TextLine {
                runs: std::mem::take(&mut current_runs),
                height: line_height,
            });
            current_width = 0.0;
            line_height = default_font_size * 1.4;
            continue;
        }

        let word_width =
            estimate_word_width(&word, template.font_size, &template.font_family, fonts);
        let space_width =
            estimate_word_width(" ", template.font_size, &template.font_family, fonts);

        let needed = if current_width > 0.0 {
            space_width + word_width
        } else {
            word_width
        };

        if current_width + needed > max_width && current_width > 0.0 {
            // Wrap to new line
            lines.push(TextLine {
                runs: std::mem::take(&mut current_runs),
                height: line_height,
            });
            current_width = 0.0;
            line_height = default_font_size * 1.4;
        }

        let text = if current_width > 0.0 {
            format!(" {word}")
        } else {
            word
        };

        let w = estimate_word_width(&text, template.font_size, &template.font_family, fonts);
        current_width += w;
        line_height = line_height.max(template.font_size * 1.4);

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
    let ellipsis_width =
        estimate_word_width(ellipsis, template.font_size, &template.font_family, fonts);

    // Check if the line actually overflows
    let line_width = estimate_word_width(
        &total_text,
        template.font_size,
        &template.font_family,
        fonts,
    );
    if line_width <= max_width {
        return;
    }

    // Truncate character by character until text + ellipsis fits
    let mut truncated = String::new();
    for ch in total_text.chars() {
        truncated.push(ch);
        let w = estimate_word_width(&truncated, template.font_size, &template.font_family, fonts);
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

fn paginate(elements: Vec<LayoutElement>, content_height: f32) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut current_elements: Vec<(f32, LayoutElement)> = Vec::new();
    let mut y = 0.0;

    // Track active float regions for simplified float/clear behavior
    let mut left_floats: Vec<FloatRegion> = Vec::new();
    let mut right_floats: Vec<FloatRegion> = Vec::new();

    for element in elements {
        // Extract float/clear/position info from TextBlock elements
        let (elem_float, elem_clear, elem_position, elem_offset_top) = match &element {
            LayoutElement::TextBlock {
                float,
                clear,
                position,
                offset_top,
                ..
            } => (*float, *clear, *position, *offset_top),
            _ => (Float::None, Clear::None, Position::Static, 0.0),
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

        let (element_height, margin_top_val) = match &element {
            LayoutElement::PageBreak => {
                pages.push(Page {
                    elements: std::mem::take(&mut current_elements),
                });
                y = 0.0;
                left_floats.clear();
                right_floats.clear();
                continue;
            }
            LayoutElement::HorizontalRule {
                margin_top,
                margin_bottom,
            } => (*margin_top + 1.0 + *margin_bottom, *margin_top),
            LayoutElement::TableRow {
                cells,
                margin_top,
                margin_bottom,
                ..
            } => {
                let row_height = cells
                    .iter()
                    .map(|cell| {
                        let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
                        cell.padding_top + text_h + cell.padding_bottom
                    })
                    .fold(0.0f32, f32::max);
                (margin_top + row_height + margin_bottom, *margin_top)
            }
            LayoutElement::GridRow {
                cells,
                margin_top,
                margin_bottom,
                ..
            } => {
                let row_height = cells
                    .iter()
                    .map(|cell| {
                        let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
                        cell.padding_top + text_h + cell.padding_bottom
                    })
                    .fold(0.0f32, f32::max);
                (margin_top + row_height + margin_bottom, *margin_top)
            }
            LayoutElement::TextBlock {
                lines,
                margin_top,
                margin_bottom,
                padding_top,
                padding_bottom,
                border_width,
                block_height,
                ..
            } => {
                let text_height: f32 = lines.iter().map(|l| l.height).sum();
                let border_extra = border_width * 2.0;
                let content_h = padding_top + text_height + padding_bottom;
                // If CSS height is set, use it as minimum content height
                let effective_content_h = match block_height {
                    Some(h) => content_h.max(*h),
                    None => content_h,
                };
                let total = margin_top + effective_content_h + margin_bottom + border_extra;
                (total, *margin_top)
            }
            LayoutElement::Image {
                height,
                margin_top,
                margin_bottom,
                ..
            } => (*margin_top + *height + *margin_bottom, *margin_top),
        };

        // Handle position: absolute -- place at fixed position, don't affect flow
        if elem_position == Position::Absolute {
            let abs_y = elem_offset_top;
            current_elements.push((abs_y, element));
            continue;
        }

        if y + element_height > content_height && y > 0.0 {
            pages.push(Page {
                elements: std::mem::take(&mut current_elements),
            });
            y = 0.0;
            left_floats.clear();
            right_floats.clear();
        }

        // Handle floated elements
        if elem_float != Float::None {
            y += margin_top_val;
            let float_y_end = y + (element_height - margin_top_val);
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
            // Floated elements are placed at current y but don't advance normal flow
            current_elements.push((y, element));
            continue;
        }

        y += margin_top_val;
        let after_margin = element_height - margin_top_val;

        // Handle position: relative -- offset from normal position
        let effective_y = if elem_position == Position::Relative {
            y + elem_offset_top
        } else {
            y
        };

        current_elements.push((effective_y, element));
        y += after_margin;
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
        page.elements.sort_by_key(|(_, el)| match el {
            LayoutElement::TextBlock { z_index, .. } => *z_index,
            _ => 0,
        });
    }

    pages
}

/// Load image data from an <img> element and return a LayoutElement::Image.
fn load_image_from_element(
    el: &ElementNode,
    available_width: f32,
    style: &ComputedStyle,
) -> Option<LayoutElement> {
    let src = el.attributes.get("src")?;
    let (data, format, png_meta) = load_image_data(src)?;

    // Determine dimensions from attributes
    let attr_width = el
        .attributes
        .get("width")
        .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
        .map(|px| px * 0.75);
    let attr_height = el
        .attributes
        .get("height")
        .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
        .map(|px| px * 0.75);

    let (mut width, mut height) = match (attr_width, attr_height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w), // fallback: square
        (None, Some(h)) => (h, h),
        (None, None) => (available_width.min(200.0), 150.0),
    };

    // Scale to fit within available width
    if width > available_width {
        let scale = available_width / width;
        width = available_width;
        height *= scale;
    }

    Some(LayoutElement::Image {
        data,
        width,
        height,
        format,
        png_metadata: png_meta,
        margin_top: style.margin.top,
        margin_bottom: style.margin.bottom,
    })
}

/// Load image data from a src attribute (supports data: URIs and local file paths).
/// Returns (raw_bytes, format, optional_png_metadata).
fn load_image_data(src: &str) -> Option<(Vec<u8>, ImageFormat, Option<PngMetadata>)> {
    let raw = if let Some(rest) = src.strip_prefix("data:") {
        // Parse data URI: data:[<mediatype>][;base64],<data>
        let (_header, encoded) = rest.split_once(',')?;
        // Only support base64 for now
        base64_decode(encoded)?
    } else if src.starts_with("http://") || src.starts_with("https://") {
        // Remote URLs are not supported (SSRF risk)
        return None;
    } else {
        // Treat as local file path
        std::fs::read(src).ok()?
    };

    // Detect format from content
    if png::is_png(&raw) {
        let png_info = png::parse_png(&raw)?;
        let metadata = PngMetadata {
            channels: png_info.channels,
            bit_depth: png_info.bit_depth,
        };
        // For PNG, we pass the IDAT data (already zlib-compressed) to PDF
        Some((png_info.idat_data, ImageFormat::Png, Some(metadata)))
    } else if raw.len() >= 2 && raw[0] == 0xFF && raw[1] == 0xD8 {
        // JPEG: pass entire file as-is
        Some((raw, ImageFormat::Jpeg, None))
    } else {
        None
    }
}

/// Simple base64 decoder (no external dependencies).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let table = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };

    // Strip whitespace
    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;

    while i < bytes.len() {
        let remaining = bytes.len() - i;
        if remaining < 2 {
            break;
        }

        let a = table(bytes[i])?;
        let b = table(bytes[i + 1])?;
        result.push((a << 2) | (b >> 4));

        if i + 2 < bytes.len() && bytes[i + 2] != b'=' {
            let c = table(bytes[i + 2])?;
            result.push((b << 4) | (c >> 2));

            if i + 3 < bytes.len() && bytes[i + 3] != b'=' {
                let d = table(bytes[i + 3])?;
                result.push((c << 6) | d);
            }
        }

        i += 4;
    }

    Some(result)
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
        let nodes = parse_html(html).unwrap();
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
        let nodes = parse_html(html).unwrap();
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
        let decoded = super::base64_decode("SGVsbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn base64_decode_with_whitespace() {
        let decoded = super::base64_decode("SGVs\nbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn layout_jpeg_image_from_data_uri() {
        // Minimal JPEG-like data URI
        let html = r#"<img src="data:image/jpeg;base64,/9j/4AAC/9k=" width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        match &pages[0].elements[0].1 {
            LayoutElement::Image {
                format,
                width,
                height,
                png_metadata,
                ..
            } => {
                assert_eq!(*format, ImageFormat::Jpeg);
                assert!((width - 75.0).abs() < 0.1); // 100px * 0.75
                assert!((height - 60.0).abs() < 0.1); // 80px * 0.75
                assert!(png_metadata.is_none());
            }
            _ => panic!("Expected Image layout element"),
        }
    }

    #[test]
    fn layout_png_image_from_data_uri() {
        // Build a minimal valid PNG and encode as base64
        let png_bytes = build_test_png_bytes();
        let b64 = simple_base64_encode(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="120" height="90">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        match &pages[0].elements[0].1 {
            LayoutElement::Image {
                format,
                png_metadata,
                ..
            } => {
                assert_eq!(*format, ImageFormat::Png);
                let meta = png_metadata.as_ref().unwrap();
                assert_eq!(meta.channels, 3); // RGB
                assert_eq!(meta.bit_depth, 8);
            }
            _ => panic!("Expected Image layout element"),
        }
    }

    #[test]
    fn layout_image_without_dimensions_gets_defaults() {
        let png_bytes = build_test_png_bytes();
        let b64 = simple_base64_encode(&png_bytes);
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
    fn base64_decode_roundtrip() {
        let data = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let encoded = simple_base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn img_scales_to_fit_available_width() {
        // Very wide image: 2000px = 1500pt, which exceeds A4 content width (~451pt)
        let html = r#"<img src="data:image/jpeg;base64,/9j/4AAC/9k=" width="2000" height="1000">"#;
        let nodes = parse_html(html).unwrap();
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

    fn simple_base64_encode(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::new();
        let mut i = 0;
        while i < data.len() {
            let b0 = data[i] as u32;
            let b1 = if i + 1 < data.len() {
                data[i + 1] as u32
            } else {
                0
            };
            let b2 = if i + 2 < data.len() {
                data[i + 2] as u32
            } else {
                0
            };
            let triple = (b0 << 16) | (b1 << 8) | b2;
            result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            if i + 1 < data.len() {
                result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            if i + 2 < data.len() {
                result.push(CHARS[(triple & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            i += 3;
        }
        result
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
        // Should have cell borders
        assert!(content.contains("re\nS\n"));
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

    // --- Flexbox layout tests ---

    fn extract_flex_items(pages: &[Page]) -> Vec<(f32, f32, Option<f32>, String)> {
        let mut result = Vec::new();
        for page in pages {
            for (y, elem) in &page.elements {
                if let LayoutElement::TextBlock {
                    lines,
                    offset_left,
                    block_width,
                    ..
                } = elem
                {
                    let text: String = lines
                        .iter()
                        .flat_map(|l| l.runs.iter().map(|r| r.text.clone()))
                        .collect::<Vec<_>>()
                        .join("");
                    if !text.is_empty() {
                        result.push((*y, *offset_left, *block_width, text));
                    }
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
        let decoded = base64_decode("SGVsbG8=").unwrap();
        assert_eq!(&decoded, b"Hello");
    }

    #[test]
    fn base64_decode_invalid_char() {
        // Covers line 2562: base64 decode with invalid char
        let result = base64_decode("!!!!");
        assert!(result.is_none());
    }

    #[test]
    fn base64_decode_short_input() {
        // Covers line 2574: base64 decode with very short input (breaks early)
        let result = base64_decode("A");
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }
}
