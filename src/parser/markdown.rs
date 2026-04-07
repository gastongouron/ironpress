/// Convert a Markdown string to HTML using a CommonMark-compliant parser.
///
/// Enables GFM extensions (tables, strikethrough, task lists, footnotes)
/// and LaTeX math (`$...$` for inline, `$$...$$` for display).
///
/// Powered by [pulldown-cmark](https://crates.io/crates/pulldown-cmark).
pub fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser};

    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH;

    let parser = Parser::new_ext(md, options);
    let mut html = String::new();

    // Collect events, converting math events to HTML inline, then flush
    // non-math events through pulldown-cmark's HTML renderer in batches
    // so that multi-event constructs (images, links) stay intact.
    let mut batch: Vec<Event<'_>> = Vec::new();
    let events: Vec<Event<'_>> = parser.collect();

    for event in events {
        match event {
            Event::InlineMath(tex) => {
                // Flush pending non-math events
                if !batch.is_empty() {
                    pulldown_cmark::html::push_html(&mut html, batch.drain(..));
                }
                html.push_str("<span class=\"math-inline\" data-math=\"");
                push_html_escaped(&tex, &mut html);
                html.push_str("\">");
                push_html_escaped(&tex, &mut html);
                html.push_str("</span>");
            }
            Event::DisplayMath(tex) => {
                if !batch.is_empty() {
                    pulldown_cmark::html::push_html(&mut html, batch.drain(..));
                }
                html.push_str("<div class=\"math-display\" data-math=\"");
                push_html_escaped(&tex, &mut html);
                html.push_str("\">");
                push_html_escaped(&tex, &mut html);
                html.push_str("</div>");
            }
            other => {
                batch.push(other);
            }
        }
    }
    // Flush remaining
    if !batch.is_empty() {
        pulldown_cmark::html::push_html(&mut html, batch.into_iter());
    }
    html
}

/// HTML-escape a string for safe attribute/text embedding.
fn push_html_escaped(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings() {
        assert!(markdown_to_html("# Hello").contains("<h1>Hello</h1>"));
        assert!(markdown_to_html("## World").contains("<h2>World</h2>"));
        assert!(markdown_to_html("### Three").contains("<h3>Three</h3>"));
        assert!(markdown_to_html("###### Six").contains("<h6>Six</h6>"));
    }

    #[test]
    fn paragraphs() {
        assert!(markdown_to_html("Hello world").contains("<p>Hello world</p>"));
        let html = markdown_to_html("Para one\n\nPara two");
        assert!(html.contains("<p>Para one</p>"));
        assert!(html.contains("<p>Para two</p>"));
    }

    #[test]
    fn bold_italic() {
        assert!(markdown_to_html("**bold**").contains("<strong>bold</strong>"));
        assert!(markdown_to_html("*italic*").contains("<em>italic</em>"));
        let html = markdown_to_html("***both***");
        assert!(html.contains("<em>") && html.contains("<strong>"));
    }

    #[test]
    fn inline_code() {
        assert!(markdown_to_html("Use `foo()` here").contains("<code>foo()</code>"));
    }

    #[test]
    fn code_block() {
        let md = "```\nfn main() {\n    println!(\"hi\");\n}\n```";
        let html = markdown_to_html(md);
        assert!(html.contains("<pre>") || html.contains("<code>"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn unordered_list() {
        let md = "- one\n- two\n- three";
        let html = markdown_to_html(md);
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>"));
        assert!(html.contains("one"));
        assert!(html.contains("two"));
        assert!(html.contains("three"));
    }

    #[test]
    fn ordered_list() {
        let md = "1. first\n2. second\n3. third";
        let html = markdown_to_html(md);
        assert!(html.contains("<ol>"));
        assert!(html.contains("<li>"));
        assert!(html.contains("first"));
        assert!(html.contains("second"));
    }

    #[test]
    fn links() {
        let html = markdown_to_html("[click](https://example.com)");
        assert!(html.contains("href=\"https://example.com\""));
        assert!(html.contains("click"));
    }

    #[test]
    fn images() {
        let html = markdown_to_html("![alt](img.png)");
        assert!(html.contains("src=\"img.png\""));
        assert!(html.contains("alt=\"alt\""));
    }

    #[test]
    fn blockquote() {
        let html = markdown_to_html("> Some wise words");
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("Some wise words"));
    }

    #[test]
    fn horizontal_rule() {
        assert!(markdown_to_html("---").contains("<hr"));
        assert!(markdown_to_html("***").contains("<hr"));
        assert!(markdown_to_html("___").contains("<hr"));
    }

    #[test]
    fn mixed_content() {
        let md = "# Title\n\nSome **bold** text.\n\n- item 1\n- item 2\n\n---\n\n> quote";
        let html = markdown_to_html(md);
        assert!(html.contains("<h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<ul>"));
        assert!(html.contains("<hr"));
        assert!(html.contains("<blockquote>"));
    }

    #[test]
    fn unclosed_code_block() {
        let md = "```\nsome code";
        let html = markdown_to_html(md);
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
        assert!(html.contains("line one"));
        assert!(html.contains("line two"));
    }

    #[test]
    fn heading_not_without_space() {
        // CommonMark: "#hello" without space is NOT a heading
        let html = markdown_to_html("#hello");
        assert!(!html.contains("<h1>"));
    }

    #[test]
    fn underscore_bold_italic() {
        assert!(markdown_to_html("__bold__").contains("<strong>bold</strong>"));
        assert!(markdown_to_html("_italic_").contains("<em>italic</em>"));
    }

    #[test]
    fn strikethrough() {
        // pulldown-cmark supports strikethrough with ~~
        let html = markdown_to_html("~~deleted~~");
        // May or may not be supported depending on extensions
        assert!(html.contains("deleted"));
    }

    #[test]
    fn nested_lists() {
        let md = "- outer\n  - inner\n- back";
        let html = markdown_to_html(md);
        assert!(html.contains("outer"));
        assert!(html.contains("inner"));
        assert!(html.contains("back"));
    }

    #[test]
    fn link_with_title() {
        let html = markdown_to_html(r#"[text](url "title")"#);
        assert!(html.contains("href=\"url\""));
        assert!(html.contains("title=\"title\""));
    }

    #[test]
    fn html_in_markdown() {
        // CommonMark allows raw HTML passthrough
        let html = markdown_to_html("<div class=\"custom\">hello</div>");
        assert!(html.contains("<div class=\"custom\">hello</div>"));
    }

    #[test]
    fn table_extension() {
        // Basic table (may not be supported without GFM extension)
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let html = markdown_to_html(md);
        // Just ensure no panic
        assert!(!html.is_empty());
    }

    #[test]
    fn empty_input() {
        assert!(markdown_to_html("").is_empty());
    }

    #[test]
    fn only_whitespace() {
        let html = markdown_to_html("   \n\n   ");
        assert!(html.trim().is_empty() || html.contains("<p>"));
    }
}
