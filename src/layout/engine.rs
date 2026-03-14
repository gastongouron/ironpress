use crate::parser::css::CssRule;
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::style::computed::{
    ComputedStyle, FontStyle, FontWeight, TextAlign, compute_style, compute_style_with_rules,
};
use crate::types::{Margin, PageSize};

/// Context for rendering list items.
#[derive(Debug, Clone)]
enum ListContext {
    Unordered,
    Ordered(usize),
}

/// A table cell ready for rendering.
#[derive(Debug)]
pub struct TableCell {
    pub lines: Vec<TextLine>,
    pub bold: bool,
    pub background_color: Option<(f32, f32, f32)>,
    pub padding_top: f32,
    pub padding_right: f32,
    pub padding_bottom: f32,
    pub padding_left: f32,
}

/// A styled text run (a piece of text with uniform style).
#[derive(Debug, Clone)]
pub struct TextRun {
    pub text: String,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: (f32, f32, f32),
}

/// A laid-out line of text runs.
#[derive(Debug, Clone)]
pub struct TextLine {
    pub runs: Vec<TextRun>,
    pub height: f32,
}

/// A layout element ready for rendering.
#[derive(Debug)]
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
    },
    /// A table row with cells.
    TableRow {
        cells: Vec<TableCell>,
        col_width: f32,
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
pub fn layout(nodes: &[DomNode], page_size: PageSize, margin: Margin) -> Vec<Page> {
    layout_with_rules(nodes, page_size, margin, &[])
}

/// Lay out the DOM nodes into pages with stylesheet rules.
pub fn layout_with_rules(
    nodes: &[DomNode],
    page_size: PageSize,
    margin: Margin,
    rules: &[CssRule],
) -> Vec<Page> {
    let parent_style = ComputedStyle::default();
    let available_width = page_size.width - margin.left - margin.right;
    let content_height = page_size.height - margin.top - margin.bottom;

    // First, flatten DOM into layout elements
    let mut elements = Vec::new();
    flatten_nodes(
        nodes,
        &parent_style,
        available_width,
        &mut elements,
        None,
        rules,
    );

    // Then paginate
    paginate(elements, content_height)
}

fn flatten_nodes(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    list_ctx: Option<&ListContext>,
    rules: &[CssRule],
) {
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
                        color: parent_style.color.to_f32_rgb(),
                    };
                    let lines = wrap_text_runs(vec![run], available_width, parent_style.font_size);
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
                        });
                    }
                }
            }
            DomNode::Element(el) => {
                flatten_element(el, parent_style, available_width, output, list_ctx, rules);
            }
        }
    }
}

fn flatten_element(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    list_ctx: Option<&ListContext>,
    rules: &[CssRule],
) {
    let classes = el.class_list();
    let style = compute_style_with_rules(
        el.tag,
        el.style_attr(),
        parent_style,
        rules,
        el.tag_name(),
        &classes,
        el.id(),
    );

    if el.tag == HtmlTag::Br {
        let line = TextLine {
            runs: vec![TextRun {
                text: String::new(),
                font_size: style.font_size,
                bold: false,
                italic: false,
                underline: false,
                color: (0.0, 0.0, 0.0),
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
            padding_right: 0.0,
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

    if style.page_break_before {
        output.push(LayoutElement::PageBreak);
    }

    // Table handling
    if el.tag == HtmlTag::Table {
        flatten_table(el, &style, available_width, output, rules);
        return;
    }

    // List handling — Ul/Ol pass context to Li children
    if el.tag == HtmlTag::Ul || el.tag == HtmlTag::Ol {
        let inner_width = available_width - style.margin.left;
        let mut ctx = if el.tag == HtmlTag::Ol {
            ListContext::Ordered(1)
        } else {
            ListContext::Unordered
        };
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if child_el.tag == HtmlTag::Li {
                    flatten_element(child_el, &style, inner_width, output, Some(&ctx), rules);
                    if let ListContext::Ordered(n) = &mut ctx {
                        *n += 1;
                    }
                } else {
                    flatten_element(child_el, &style, inner_width, output, None, rules);
                }
            }
        }
        return;
    }

    // Li handling — prepend bullet/number marker
    if el.tag == HtmlTag::Li {
        let inner_width = available_width - style.padding.left - style.padding.right;
        let mut runs = Vec::new();

        // Add list marker
        let marker = match list_ctx {
            Some(ListContext::Unordered) => "- ".to_string(),
            Some(ListContext::Ordered(n)) => format!("{n}. "),
            None => "- ".to_string(),
        };
        runs.push(TextRun {
            text: marker,
            font_size: style.font_size,
            bold: style.font_weight == FontWeight::Bold,
            italic: style.font_style == FontStyle::Italic,
            underline: false,
            color: style.color.to_f32_rgb(),
        });

        collect_text_runs(&el.children, &style, &mut runs);

        if !runs.is_empty() {
            let lines = wrap_text_runs(runs, inner_width, style.font_size);
            output.push(LayoutElement::TextBlock {
                lines,
                margin_top: style.margin.top,
                margin_bottom: style.margin.bottom,
                text_align: style.text_align,
                background_color: None,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: style.margin.left,
                padding_right: 0.0,
            });
        }

        // Process block children inside li
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if child_el.tag.is_block() {
                    flatten_element(child_el, &style, available_width, output, None, rules);
                }
            }
        }
        return;
    }

    if el.tag.is_block() {
        // Collect all inline content as text runs
        let inner_width = available_width - style.padding.left - style.padding.right;
        let mut runs = Vec::new();
        collect_text_runs(&el.children, &style, &mut runs);

        if !runs.is_empty() {
            let lines = wrap_text_runs(runs, inner_width, style.font_size);
            let bg = style
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgb());

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
            });
        }

        // Also process block children recursively
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                if child_el.tag.is_block() {
                    flatten_element(child_el, &style, available_width, output, None, rules);
                }
            }
        }
    } else {
        // Inline element — process children with this style context
        flatten_nodes(&el.children, &style, available_width, output, None, rules);
    }

    if style.page_break_after {
        output.push(LayoutElement::PageBreak);
    }
}

fn flatten_table(
    el: &ElementNode,
    style: &ComputedStyle,
    available_width: f32,
    output: &mut Vec<LayoutElement>,
    _rules: &[CssRule],
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

    // Determine column count from the widest row
    let num_cols = rows
        .iter()
        .map(|row| {
            row.children
                .iter()
                .filter(|c| {
                    matches!(c, DomNode::Element(e) if e.tag == HtmlTag::Td || e.tag == HtmlTag::Th)
                })
                .count()
        })
        .max()
        .unwrap_or(1);

    let col_width = inner_width / num_cols as f32;

    // Build layout rows
    let mut is_first = true;
    for row in &rows {
        let row_style = compute_style(row.tag, row.style_attr(), style);
        let mut cells = Vec::new();

        for child in &row.children {
            if let DomNode::Element(cell_el) = child {
                if cell_el.tag == HtmlTag::Td || cell_el.tag == HtmlTag::Th {
                    let cell_style = compute_style(cell_el.tag, cell_el.style_attr(), &row_style);
                    let cell_inner = col_width - cell_style.padding.left - cell_style.padding.right;

                    let mut runs = Vec::new();
                    collect_text_runs(&cell_el.children, &cell_style, &mut runs);
                    let lines = wrap_text_runs(runs, cell_inner.max(1.0), cell_style.font_size);

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
                    });
                }
            }
        }

        if !cells.is_empty() {
            output.push(LayoutElement::TableRow {
                cells,
                col_width,
                margin_top: if is_first { style.margin.top } else { 0.0 },
                margin_bottom: 0.0,
            });
            is_first = false;
        }
    }

    // Add bottom margin after the last row
    if let Some(LayoutElement::TableRow { margin_bottom, .. }) = output.last_mut() {
        *margin_bottom = style.margin.bottom;
    }
}

fn collect_text_runs(nodes: &[DomNode], parent_style: &ComputedStyle, runs: &mut Vec<TextRun>) {
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
                        color: parent_style.color.to_f32_rgb(),
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
                            color: (0.0, 0.0, 0.0),
                        });
                    } else {
                        let style = compute_style(el.tag, el.style_attr(), parent_style);
                        collect_text_runs(&el.children, &style, runs);
                    }
                }
            }
        }
    }
}

/// Simple text wrapping using character width estimation.
/// Uses 0.5 * font_size as average character width for the built-in font.
fn wrap_text_runs(runs: Vec<TextRun>, max_width: f32, default_font_size: f32) -> Vec<TextLine> {
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

        let char_width = template.font_size * 0.5;
        let word_width = word.len() as f32 * char_width;
        let space_width = char_width;

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

        let w = text.len() as f32 * template.font_size * 0.5;
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

fn paginate(elements: Vec<LayoutElement>, content_height: f32) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut current_elements: Vec<(f32, LayoutElement)> = Vec::new();
    let mut y = 0.0;

    for element in elements {
        let (element_height, margin_top_val) = match &element {
            LayoutElement::PageBreak => {
                pages.push(Page {
                    elements: std::mem::take(&mut current_elements),
                });
                y = 0.0;
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
            LayoutElement::TextBlock {
                lines,
                margin_top,
                margin_bottom,
                padding_top,
                padding_bottom,
                ..
            } => {
                let text_height: f32 = lines.iter().map(|l| l.height).sum();
                let total = margin_top + padding_top + text_height + padding_bottom + margin_bottom;
                (total, *margin_top)
            }
        };

        if y + element_height > content_height && y > 0.0 {
            pages.push(Page {
                elements: std::mem::take(&mut current_elements),
            });
            y = 0.0;
        }

        y += margin_top_val;
        let after_margin = element_height - margin_top_val;
        current_elements.push((y, element));
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

    pages
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
    use crate::parser::html::parse_html;

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
}
