//! # ironpress
//!
//! Pure Rust HTML/CSS/Markdown to PDF converter. No browser, no system dependencies.
//!
//! Converts HTML with inline CSS styles into PDF documents using a built-in
//! layout engine. Supports headings, paragraphs, bold/italic text, colors,
//! tables, lists, page breaks, and more.
//!
//! ## Quick start
//!
//! ```
//! use ironpress::html_to_pdf;
//!
//! let pdf_bytes = html_to_pdf("<h1>Hello</h1><p>World</p>").unwrap();
//! assert!(pdf_bytes.starts_with(b"%PDF"));
//! ```
//!
//! ## With options
//!
//! ```
//! use ironpress::{HtmlConverter, PageSize, Margin};
//!
//! let pdf = HtmlConverter::new()
//!     .page_size(PageSize::LETTER)
//!     .margin(Margin::uniform(54.0))
//!     .convert("<h1>Hello</h1>")
//!     .unwrap();
//! ```

pub mod error;
pub mod layout;
pub mod parser;
pub mod render;
pub mod security;
pub mod style;
pub mod types;

pub use error::IronpressError;
pub use types::{Margin, PageSize};

/// Convert an HTML string to PDF bytes using default settings (A4, 1-inch margins).
///
/// The HTML is sanitized before conversion to remove potentially dangerous
/// elements like `<script>`, `<iframe>`, and event handlers.
///
/// # Example
///
/// ```
/// let pdf = ironpress::html_to_pdf("<h1>Title</h1><p>Hello World</p>").unwrap();
/// assert!(pdf.starts_with(b"%PDF"));
/// ```
pub fn html_to_pdf(html: &str) -> Result<Vec<u8>, IronpressError> {
    HtmlConverter::new().convert(html)
}

/// Convert a Markdown string to PDF bytes using default settings (A4, 1-inch margins).
///
/// # Example
///
/// ```
/// let pdf = ironpress::markdown_to_pdf("# Hello\n\nWorld").unwrap();
/// assert!(pdf.starts_with(b"%PDF"));
/// ```
pub fn markdown_to_pdf(md: &str) -> Result<Vec<u8>, IronpressError> {
    let html = parser::markdown::markdown_to_html(md);
    HtmlConverter::new().sanitize(false).convert(&html)
}

/// Convert a Markdown file to a PDF file using default settings.
///
/// # Example
///
/// ```no_run
/// ironpress::convert_markdown_file("input.md", "output.pdf").unwrap();
/// ```
pub fn convert_markdown_file(input: &str, output: &str) -> Result<(), IronpressError> {
    let md = std::fs::read_to_string(input)?;
    let pdf = markdown_to_pdf(&md)?;
    std::fs::write(output, pdf)?;
    Ok(())
}

/// Convert an HTML file to a PDF file using default settings.
///
/// # Example
///
/// ```no_run
/// ironpress::convert_file("input.html", "output.pdf").unwrap();
/// ```
pub fn convert_file(input: &str, output: &str) -> Result<(), IronpressError> {
    let html = std::fs::read_to_string(input)?;
    let pdf = html_to_pdf(&html)?;
    std::fs::write(output, pdf)?;
    Ok(())
}

/// Builder for HTML-to-PDF conversion with custom options.
pub struct HtmlConverter {
    page_size: PageSize,
    margin: Margin,
    sanitize: bool,
}

impl HtmlConverter {
    /// Create a new converter with default settings (A4, 1-inch margins, sanitization enabled).
    pub fn new() -> Self {
        Self {
            page_size: PageSize::default(),
            margin: Margin::default(),
            sanitize: true,
        }
    }

    /// Set the page size.
    pub fn page_size(mut self, size: PageSize) -> Self {
        self.page_size = size;
        self
    }

    /// Set the page margins.
    pub fn margin(mut self, margin: Margin) -> Self {
        self.margin = margin;
        self
    }

    /// Enable or disable HTML sanitization (enabled by default).
    pub fn sanitize(mut self, enabled: bool) -> Self {
        self.sanitize = enabled;
        self
    }

    /// Convert a Markdown string to PDF bytes.
    pub fn convert_markdown(&self, md: &str) -> Result<Vec<u8>, IronpressError> {
        let html = parser::markdown::markdown_to_html(md);
        self.convert(&html)
    }

    /// Convert an HTML string to PDF bytes.
    pub fn convert(&self, html: &str) -> Result<Vec<u8>, IronpressError> {
        // Step 1: Sanitize
        let html = if self.sanitize {
            security::sanitizer::sanitize_html(html)?
        } else {
            html.to_string()
        };

        // Step 2: Parse HTML and extract stylesheets
        let result = parser::html::parse_html_with_styles(&html)?;

        // Step 3: Parse stylesheets
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parser::css::parse_stylesheet(css));
        }

        // Step 4: Layout
        let pages =
            layout::engine::layout_with_rules(&result.nodes, self.page_size, self.margin, &rules);

        // Step 5: Render PDF
        render::pdf::render_pdf(&pages, self.page_size, self.margin)
    }
}

impl Default for HtmlConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_pdf_basic() {
        let pdf = html_to_pdf("<h1>Hello</h1><p>World</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("%%EOF"));
    }

    #[test]
    fn html_to_pdf_with_styles() {
        let html = r#"<h1 style="color: red; text-align: center">Title</h1>
                      <p style="font-size: 14pt">Some text here.</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_formatting() {
        let html = "<p>Normal <strong>bold</strong> <em>italic</em> <u>underline</u></p>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Helvetica-Bold"));
        assert!(content.contains("Helvetica-Oblique"));
    }

    #[test]
    fn html_to_pdf_empty() {
        let pdf = html_to_pdf("").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_sanitizes_script() {
        let html = "<p>Safe</p><script>alert('xss')</script>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(!content.contains("alert"));
        assert!(content.contains("Safe"));
    }

    #[test]
    fn converter_builder() {
        let pdf = HtmlConverter::new()
            .page_size(PageSize::LETTER)
            .margin(Margin::uniform(54.0))
            .convert("<p>Test</p>")
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn converter_no_sanitize() {
        let pdf = HtmlConverter::new()
            .sanitize(false)
            .convert("<p>Test</p>")
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_headings() {
        let html = "<h1>H1</h1><h2>H2</h2><h3>H3</h3><h4>H4</h4><h5>H5</h5><h6>H6</h6>";
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_horizontal_rule() {
        let pdf = html_to_pdf("<p>Above</p><hr><p>Below</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_line_break() {
        let pdf = html_to_pdf("<p>Line one<br>Line two</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn convert_file_roundtrip() {
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_test_input.html");
        let output = dir.join("ironpress_test_output.pdf");
        std::fs::write(&input, "<h1>Test</h1><p>Hello</p>").unwrap();
        convert_file(input.to_str().unwrap(), output.to_str().unwrap()).unwrap();
        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        std::fs::remove_file(&input).ok();
        std::fs::remove_file(&output).ok();
    }

    #[test]
    fn converter_default_impl() {
        let converter = HtmlConverter::default();
        let pdf = converter.convert("<p>Default</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_unordered_list() {
        let html = "<ul><li>Item one</li><li>Item two</li><li>Item three</li></ul>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("-"));
        assert!(content.contains("Item"));
    }

    #[test]
    fn html_to_pdf_ordered_list() {
        let html = "<ol><li>First</li><li>Second</li><li>Third</li></ol>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1."));
        assert!(content.contains("2."));
        assert!(content.contains("3."));
    }

    #[test]
    fn html_to_pdf_table() {
        let html = r#"
            <table>
                <tr><th>Name</th><th>Age</th></tr>
                <tr><td>Alice</td><td>30</td></tr>
                <tr><td>Bob</td><td>25</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Name"));
        assert!(content.contains("Alice"));
        assert!(content.contains("Bob"));
        // Should have cell borders (rectangle stroke)
        assert!(content.contains("re\nS\n"));
    }

    #[test]
    fn html_to_pdf_table_with_sections() {
        let html = r#"
            <table>
                <thead><tr><th>Header</th></tr></thead>
                <tbody><tr><td>Body</td></tr></tbody>
                <tfoot><tr><td>Footer</td></tr></tfoot>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Header"));
        assert!(content.contains("Body"));
        assert!(content.contains("Footer"));
    }

    #[test]
    fn html_to_pdf_with_style_block() {
        let html = r#"
            <html>
            <head><style>p { color: red } .highlight { font-weight: bold }</style></head>
            <body>
                <p>Red text</p>
                <p class="highlight">Bold red text</p>
            </body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 rg")); // red color
        assert!(content.contains("Helvetica-Bold")); // bold from .highlight
    }

    #[test]
    fn html_to_pdf_style_block_in_body() {
        let html = r#"
            <style>h1 { color: blue }</style>
            <h1>Blue Title</h1>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("0 0 1 rg")); // blue color
    }

    #[test]
    fn html_to_pdf_definition_list() {
        let html = "<dl><dt>Term</dt><dd>Definition here</dd></dl>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Term"));
        assert!(content.contains("Definition"));
    }

    #[test]
    fn markdown_to_pdf_basic() {
        let pdf = markdown_to_pdf("# Hello\n\nWorld").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Hello"));
        assert!(content.contains("World"));
    }

    #[test]
    fn markdown_to_pdf_formatting() {
        let pdf = markdown_to_pdf("**bold** and *italic*").unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Helvetica-Bold"));
        assert!(content.contains("Helvetica-Oblique"));
    }

    #[test]
    fn markdown_to_pdf_list() {
        let pdf = markdown_to_pdf("- one\n- two\n- three").unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("one"));
        assert!(content.contains("two"));
    }

    #[test]
    fn markdown_to_pdf_code_block() {
        let md = "# Code\n\n```\nfn main() {}\n```";
        let pdf = markdown_to_pdf(md).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn markdown_to_pdf_full() {
        let md = r#"# Project Title

Some **bold** and *italic* text with `inline code`.

## Features

- Item one
- Item two
- Item three

1. First
2. Second

> A wise quote

---

```
fn main() {
    println!("hello");
}
```

[Link](https://example.com)
"#;
        let pdf = markdown_to_pdf(md).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Project"));
        assert!(content.contains("Title"));
    }

    #[test]
    fn converter_markdown() {
        let pdf = HtmlConverter::new()
            .page_size(PageSize::LETTER)
            .convert_markdown("# Hello")
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_full_document() {
        let html = r#"
            <html>
            <head><title>Test</title></head>
            <body>
                <h1>Document Title</h1>
                <p>This is a <strong>bold</strong> and <em>italic</em> paragraph.</p>
                <hr>
                <p style="color: blue; text-align: center">Centered blue text.</p>
            </body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Document"));
        assert!(content.contains("Title"));
    }
}
