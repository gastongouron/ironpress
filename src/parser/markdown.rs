/// Convert a Markdown string to HTML.
///
/// Supports: headings, bold, italic, bold+italic, inline code, code blocks,
/// links, images, unordered/ordered lists, blockquotes, horizontal rules,
/// and paragraphs.
pub fn markdown_to_html(md: &str) -> String {
    let mut html = String::new();
    let mut in_ul = false;
    let mut in_ol = false;
    let mut in_code_block = false;
    let mut code_block = String::new();
    let mut paragraph = String::new();

    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Fenced code blocks
        if line.trim_start().starts_with("```") {
            if in_code_block {
                html.push_str("<pre><code>");
                html.push_str(&escape_html(&code_block));
                html.push_str("</code></pre>\n");
                code_block.clear();
                in_code_block = false;
            } else {
                flush_paragraph(&mut paragraph, &mut html);
                close_list(&mut in_ul, &mut in_ol, &mut html);
                in_code_block = true;
            }
            i += 1;
            continue;
        }

        if in_code_block {
            if !code_block.is_empty() {
                code_block.push('\n');
            }
            code_block.push_str(line);
            i += 1;
            continue;
        }

        let trimmed = line.trim();

        // Empty line — flush paragraph
        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut html);
            close_list(&mut in_ul, &mut in_ol, &mut html);
            i += 1;
            continue;
        }

        // Horizontal rule
        if is_horizontal_rule(trimmed) {
            flush_paragraph(&mut paragraph, &mut html);
            close_list(&mut in_ul, &mut in_ol, &mut html);
            html.push_str("<hr>\n");
            i += 1;
            continue;
        }

        // Headings
        if let Some((level, text)) = parse_heading(trimmed) {
            flush_paragraph(&mut paragraph, &mut html);
            close_list(&mut in_ul, &mut in_ol, &mut html);
            html.push_str(&format!("<h{level}>{}</h{level}>\n", inline_format(text)));
            i += 1;
            continue;
        }

        // Blockquote
        if let Some(text) = trimmed
            .strip_prefix("> ")
            .or_else(|| if trimmed == ">" { Some("") } else { None })
        {
            flush_paragraph(&mut paragraph, &mut html);
            close_list(&mut in_ul, &mut in_ol, &mut html);
            // Collect consecutive blockquote lines
            let mut bq = String::from(text);
            while i + 1 < lines.len() {
                let next = lines[i + 1].trim();
                if let Some(cont) = next.strip_prefix("> ") {
                    bq.push(' ');
                    bq.push_str(cont);
                    i += 1;
                } else if next == ">" {
                    bq.push(' ');
                    i += 1;
                } else {
                    break;
                }
            }
            html.push_str(&format!(
                "<blockquote><p>{}</p></blockquote>\n",
                inline_format(&bq)
            ));
            i += 1;
            continue;
        }

        // Unordered list
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            flush_paragraph(&mut paragraph, &mut html);
            if in_ol {
                html.push_str("</ol>\n");
                in_ol = false;
            }
            if !in_ul {
                html.push_str("<ul>\n");
                in_ul = true;
            }
            html.push_str(&format!("<li>{}</li>\n", inline_format(&trimmed[2..])));
            i += 1;
            continue;
        }

        // Ordered list
        if let Some(text) = parse_ordered_item(trimmed) {
            flush_paragraph(&mut paragraph, &mut html);
            if in_ul {
                html.push_str("</ul>\n");
                in_ul = false;
            }
            if !in_ol {
                html.push_str("<ol>\n");
                in_ol = true;
            }
            html.push_str(&format!("<li>{}</li>\n", inline_format(text)));
            i += 1;
            continue;
        }

        // Regular text — accumulate into paragraph
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(trimmed);
        i += 1;
    }

    // Flush remaining state
    if in_code_block {
        html.push_str("<pre><code>");
        html.push_str(&escape_html(&code_block));
        html.push_str("</code></pre>\n");
    }
    flush_paragraph(&mut paragraph, &mut html);
    close_list(&mut in_ul, &mut in_ol, &mut html);

    html
}

fn flush_paragraph(paragraph: &mut String, html: &mut String) {
    if !paragraph.is_empty() {
        html.push_str(&format!("<p>{}</p>\n", inline_format(paragraph)));
        paragraph.clear();
    }
}

fn close_list(in_ul: &mut bool, in_ol: &mut bool, html: &mut String) {
    if *in_ul {
        html.push_str("</ul>\n");
        *in_ul = false;
    }
    if *in_ol {
        html.push_str("</ol>\n");
        *in_ol = false;
    }
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let bytes = line.as_bytes();
    let mut level = 0u8;
    while (level as usize) < bytes.len() && bytes[level as usize] == b'#' {
        level += 1;
    }
    if level == 0 || level > 6 {
        return None;
    }
    if (level as usize) < bytes.len() && bytes[level as usize] == b' ' {
        Some((level, line[level as usize + 1..].trim()))
    } else {
        None
    }
}

fn parse_ordered_item(line: &str) -> Option<&str> {
    let dot_pos = line.find(". ")?;
    let prefix = &line[..dot_pos];
    if prefix.chars().all(|c| c.is_ascii_digit()) && !prefix.is_empty() {
        Some(line[dot_pos + 2..].trim())
    } else {
        None
    }
}

fn is_horizontal_rule(line: &str) -> bool {
    let chars: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.len() < 3 {
        return false;
    }
    let first = chars[0];
    (first == '-' || first == '*' || first == '_') && chars.iter().all(|&c| c == first)
}

/// Format inline markdown: bold, italic, code, links, images.
fn inline_format(text: &str) -> String {
    let text = format_code_spans(text);
    let text = format_images(&text);
    let text = format_links(&text);
    format_bold_italic(&text)
}

fn format_code_spans(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find('`') {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + 1..];
        if let Some(end) = after.find('`') {
            result.push_str("<code>");
            result.push_str(&escape_html(&after[..end]));
            result.push_str("</code>");
            remaining = &after[end + 1..];
        } else {
            result.push('`');
            remaining = after;
        }
    }
    result.push_str(remaining);
    result
}

fn format_images(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("![") {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + 2..];
        if let Some(close_bracket) = after.find("](") {
            let alt = &after[..close_bracket];
            let url_part = &after[close_bracket + 2..];
            if let Some(close_paren) = url_part.find(')') {
                let src = &url_part[..close_paren];
                result.push_str(&format!(
                    "<img src=\"{}\" alt=\"{}\">",
                    escape_html(src),
                    escape_html(alt)
                ));
                remaining = &url_part[close_paren + 1..];
                continue;
            }
        }
        result.push_str("![");
        remaining = after;
    }
    result.push_str(remaining);
    result
}

fn format_links(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find('[') {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + 1..];
        if let Some(close_bracket) = after.find("](") {
            let label = &after[..close_bracket];
            let url_part = &after[close_bracket + 2..];
            if let Some(close_paren) = url_part.find(')') {
                let href = &url_part[..close_paren];
                result.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape_html(href),
                    escape_html(label)
                ));
                remaining = &url_part[close_paren + 1..];
                continue;
            }
        }
        result.push('[');
        remaining = after;
    }
    result.push_str(remaining);
    result
}

fn format_bold_italic(text: &str) -> String {
    // Process ***bold italic***, **bold**, *italic* using a character scan
    let mut result = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '*' || chars[i] == '_' {
            let marker = chars[i];
            let mut count = 0;
            while i + count < chars.len() && chars[i + count] == marker {
                count += 1;
            }

            if count >= 3 {
                if let Some(end) = find_closing_marker(&chars, i + count, marker, 3) {
                    let inner: String = chars[i + 3..end].iter().collect();
                    result.push_str(&format!("<strong><em>{inner}</em></strong>"));
                    i = end + 3;
                    continue;
                }
            }
            if count >= 2 {
                if let Some(end) = find_closing_marker(&chars, i + 2, marker, 2) {
                    let inner: String = chars[i + 2..end].iter().collect();
                    result.push_str(&format!("<strong>{inner}</strong>"));
                    i = end + 2;
                    continue;
                }
            }
            if count >= 1 {
                if let Some(end) = find_closing_marker(&chars, i + 1, marker, 1) {
                    let inner: String = chars[i + 1..end].iter().collect();
                    result.push_str(&format!("<em>{inner}</em>"));
                    i = end + 1;
                    continue;
                }
            }

            // No matching closer — output literally
            for _ in 0..count {
                result.push(marker);
            }
            i += count;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

fn find_closing_marker(chars: &[char], start: usize, marker: char, count: usize) -> Option<usize> {
    let mut i = start;
    while i + count <= chars.len() {
        if chars[i] == marker {
            let mut n = 0;
            while i + n < chars.len() && chars[i + n] == marker {
                n += 1;
            }
            if n >= count {
                return Some(i);
            }
            i += n;
        } else {
            i += 1;
        }
    }
    None
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings() {
        assert_eq!(markdown_to_html("# Hello"), "<h1>Hello</h1>\n");
        assert_eq!(markdown_to_html("## World"), "<h2>World</h2>\n");
        assert_eq!(markdown_to_html("### Three"), "<h3>Three</h3>\n");
        assert_eq!(markdown_to_html("###### Six"), "<h6>Six</h6>\n");
    }

    #[test]
    fn paragraphs() {
        assert_eq!(markdown_to_html("Hello world"), "<p>Hello world</p>\n");
        assert_eq!(
            markdown_to_html("Line one\nstill same paragraph"),
            "<p>Line one still same paragraph</p>\n"
        );
        assert_eq!(
            markdown_to_html("Para one\n\nPara two"),
            "<p>Para one</p>\n<p>Para two</p>\n"
        );
    }

    #[test]
    fn bold_italic() {
        assert_eq!(
            markdown_to_html("**bold**"),
            "<p><strong>bold</strong></p>\n"
        );
        assert_eq!(markdown_to_html("*italic*"), "<p><em>italic</em></p>\n");
        assert_eq!(
            markdown_to_html("***both***"),
            "<p><strong><em>both</em></strong></p>\n"
        );
    }

    #[test]
    fn inline_code() {
        assert_eq!(
            markdown_to_html("Use `foo()` here"),
            "<p>Use <code>foo()</code> here</p>\n"
        );
    }

    #[test]
    fn code_block() {
        let md = "```\nfn main() {\n    println!(\"hi\");\n}\n```";
        let html = markdown_to_html(md);
        assert!(html.contains("<pre><code>"));
        assert!(html.contains("fn main()"));
        assert!(html.contains("</code></pre>"));
    }

    #[test]
    fn unordered_list() {
        let md = "- one\n- two\n- three";
        let html = markdown_to_html(md);
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>one</li>"));
        assert!(html.contains("<li>two</li>"));
        assert!(html.contains("<li>three</li>"));
        assert!(html.contains("</ul>"));
    }

    #[test]
    fn ordered_list() {
        let md = "1. first\n2. second\n3. third";
        let html = markdown_to_html(md);
        assert!(html.contains("<ol>"));
        assert!(html.contains("<li>first</li>"));
        assert!(html.contains("<li>second</li>"));
        assert!(html.contains("</ol>"));
    }

    #[test]
    fn links() {
        assert_eq!(
            markdown_to_html("[click](https://example.com)"),
            "<p><a href=\"https://example.com\">click</a></p>\n"
        );
    }

    #[test]
    fn images() {
        assert_eq!(
            markdown_to_html("![alt](img.png)"),
            "<p><img src=\"img.png\" alt=\"alt\"></p>\n"
        );
    }

    #[test]
    fn blockquote() {
        let html = markdown_to_html("> Some wise words");
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("Some wise words"));
        assert!(html.contains("</blockquote>"));
    }

    #[test]
    fn horizontal_rule() {
        assert_eq!(markdown_to_html("---"), "<hr>\n");
        assert_eq!(markdown_to_html("***"), "<hr>\n");
        assert_eq!(markdown_to_html("___"), "<hr>\n");
    }

    #[test]
    fn mixed_content() {
        let md = "# Title\n\nSome **bold** text.\n\n- item 1\n- item 2\n\n---\n\n> quote";
        let html = markdown_to_html(md);
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<ul>"));
        assert!(html.contains("<hr>"));
        assert!(html.contains("<blockquote>"));
    }

    #[test]
    fn html_escaping() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
    }

    #[test]
    fn unclosed_code_block() {
        let md = "```\nsome code";
        let html = markdown_to_html(md);
        assert!(html.contains("<pre><code>"));
        assert!(html.contains("some code"));
    }

    #[test]
    fn list_with_formatting() {
        let md = "- **bold item**\n- *italic item*";
        let html = markdown_to_html(md);
        assert!(html.contains("<strong>bold item</strong>"));
        assert!(html.contains("<em>italic item</em>"));
    }

    #[test]
    fn multiline_blockquote() {
        let md = "> line one\n> line two";
        let html = markdown_to_html(md);
        assert!(html.contains("line one line two"));
    }

    #[test]
    fn heading_not_without_space() {
        // "#hello" without space should be a paragraph, not a heading
        let html = markdown_to_html("#hello");
        assert!(html.contains("<p>#hello</p>"));
    }

    #[test]
    fn star_and_plus_list_markers() {
        let md = "* star\n+ plus";
        let html = markdown_to_html(md);
        assert!(html.contains("<li>star</li>"));
        assert!(html.contains("<li>plus</li>"));
    }

    #[test]
    fn underscore_bold_italic() {
        assert_eq!(
            markdown_to_html("__bold__"),
            "<p><strong>bold</strong></p>\n"
        );
        assert_eq!(markdown_to_html("_italic_"), "<p><em>italic</em></p>\n");
    }

    #[test]
    fn blockquote_empty_continuation() {
        // Lines 89-91: blockquote continuation with bare ">"
        let md = "> line one\n>\n> line two";
        let html = markdown_to_html(md);
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("line one"));
        assert!(html.contains("line two"));
    }

    #[test]
    fn ol_closes_ul_when_switching() {
        // Lines 108-109: switching from ul to ol closes the ul
        let md = "- bullet\n\n1. numbered";
        let html = markdown_to_html(md);
        assert!(html.contains("</ul>"));
        assert!(html.contains("<ol>"));
        assert!(html.contains("<li>bullet</li>"));
        assert!(html.contains("<li>numbered</li>"));
    }

    #[test]
    fn ul_closes_ol_when_switching() {
        // Lines 124-125: switching from ol to ul closes the ol
        let md = "1. numbered\n\n- bullet";
        let html = markdown_to_html(md);
        assert!(html.contains("</ol>"));
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>numbered</li>"));
        assert!(html.contains("<li>bullet</li>"));
    }

    #[test]
    fn parse_ordered_item_no_match() {
        // Line 196: non-digit prefix returns None
        assert!(parse_ordered_item("abc. text").is_none());
        assert!(parse_ordered_item("no dot here").is_none());
    }

    #[test]
    fn horizontal_rule_too_short() {
        // Lines 202-203: less than 3 chars returns false
        assert!(!is_horizontal_rule("--"));
        assert!(!is_horizontal_rule("*"));
        assert!(!is_horizontal_rule(""));
    }

    #[test]
    fn horizontal_rule_mixed_chars() {
        // Line 206: mixed chars do not form a rule
        assert!(!is_horizontal_rule("-*-"));
        assert!(!is_horizontal_rule("--*"));
    }

    #[test]
    fn unclosed_backtick_inline() {
        // Lines 230-231: unclosed backtick in inline code
        let html = markdown_to_html("Use `foo here");
        assert!(html.contains("`foo"));
    }

    #[test]
    fn broken_image_syntax() {
        // Lines 259-260: malformed image syntax falls through
        let html = markdown_to_html("![alt](");
        assert!(html.contains("!["));
    }

    #[test]
    fn broken_link_syntax() {
        // Lines 287-288: malformed link syntax falls through
        let html = markdown_to_html("[label](");
        assert!(html.contains("["));
    }

    #[test]
    fn unmatched_markers_output_literally() {
        // Lines 334-337: no closing marker, markers are output literally
        let html = markdown_to_html("trailing ***");
        assert!(html.contains("<p>"));
        assert!(html.contains("trailing"));
        // The *** at end has no closing match, output literally
        let html2 = markdown_to_html("end *");
        assert!(html2.contains("*"));
    }

    #[test]
    fn find_closing_marker_skip_short_run() {
        // Lines 358, 363: closing marker finder skips runs shorter than needed
        // Triple marker needs 3 closing stars; a single star mid-text is skipped
        let html = markdown_to_html("***bold*italic***");
        // Should still produce some output
        assert!(html.contains("<p>"));
    }

    #[test]
    fn underscore_bold_italic_combined() {
        // Exercises ___triple___ underscores
        assert_eq!(
            markdown_to_html("___both___"),
            "<p><strong><em>both</em></strong></p>\n"
        );
    }

    #[test]
    fn ol_then_ul_direct_switch() {
        // Lines 108-109, 124-125: direct switch without blank line between lists
        let md = "1. first\n- bullet";
        let html = markdown_to_html(md);
        assert!(html.contains("<li>first</li>"));
        assert!(html.contains("<li>bullet</li>"));
    }

    #[test]
    fn ul_then_ol_direct_switch() {
        let md = "- bullet\n1. first";
        let html = markdown_to_html(md);
        assert!(html.contains("<li>bullet</li>"));
        assert!(html.contains("<li>first</li>"));
    }
}
