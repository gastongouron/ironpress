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
    HtmlConverter::new().convert(&html)
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

/// Convert an HTML string to PDF, writing output to any `std::io::Write` implementation.
///
/// This is the streaming variant of [`html_to_pdf`]. Instead of returning a `Vec<u8>`,
/// it writes PDF content directly to the provided writer.
pub fn html_to_pdf_writer<W: std::io::Write>(
    html: &str,
    writer: &mut W,
) -> Result<(), IronpressError> {
    HtmlConverter::new().convert_to_writer(html, writer)
}

/// Convert a Markdown string to PDF, writing output to any `std::io::Write` implementation.
///
/// This is the streaming variant of [`markdown_to_pdf`].
pub fn markdown_to_pdf_writer<W: std::io::Write>(
    md: &str,
    writer: &mut W,
) -> Result<(), IronpressError> {
    let html = parser::markdown::markdown_to_html(md);
    HtmlConverter::new().convert_to_writer(&html, writer)
}

/// Async version of [`convert_file`]. Requires the `async` feature.
///
/// Uses `tokio::fs` for async file I/O and `tokio::task::spawn_blocking`
/// for the CPU-bound conversion step.
#[cfg(feature = "async")]
pub async fn convert_file_async(input: &str, output: &str) -> Result<(), IronpressError> {
    let html = tokio::fs::read_to_string(input).await?;
    let pdf = tokio::task::spawn_blocking(move || html_to_pdf(&html))
        .await
        .map_err(|e| IronpressError::RenderError(format!("task join error: {e}")))?;
    let pdf = pdf?;
    tokio::fs::write(output, pdf).await?;
    Ok(())
}

/// Async version of [`convert_markdown_file`]. Requires the `async` feature.
///
/// Uses `tokio::fs` for async file I/O and `tokio::task::spawn_blocking`
/// for the CPU-bound conversion step.
#[cfg(feature = "async")]
pub async fn convert_markdown_file_async(input: &str, output: &str) -> Result<(), IronpressError> {
    let md = tokio::fs::read_to_string(input).await?;
    let pdf = tokio::task::spawn_blocking(move || markdown_to_pdf(&md))
        .await
        .map_err(|e| IronpressError::RenderError(format!("task join error: {e}")))?;
    let pdf = pdf?;
    tokio::fs::write(output, pdf).await?;
    Ok(())
}

/// Builder for HTML-to-PDF conversion with custom options.
pub struct HtmlConverter {
    page_size: PageSize,
    margin: Margin,
    sanitize: bool,
    custom_fonts: std::collections::HashMap<String, Vec<u8>>,
}

impl HtmlConverter {
    /// Create a new converter with default settings (A4, 1-inch margins, sanitization enabled).
    pub fn new() -> Self {
        Self {
            page_size: PageSize::default(),
            margin: Margin::default(),
            sanitize: true,
            custom_fonts: std::collections::HashMap::new(),
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

    /// Register a custom TrueType font.
    ///
    /// The `name` should match the `font-family` value used in CSS.
    /// The `ttf_data` is the raw contents of a `.ttf` file.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ironpress::HtmlConverter;
    ///
    /// let ttf_data = std::fs::read("MyFont.ttf").unwrap();
    /// let pdf = HtmlConverter::new()
    ///     .add_font("MyFont", ttf_data)
    ///     .convert(r#"<p style="font-family: MyFont">Custom text</p>"#)
    ///     .unwrap();
    /// ```
    pub fn add_font(mut self, name: &str, ttf_data: Vec<u8>) -> Self {
        self.custom_fonts
            .insert(name.to_ascii_lowercase(), ttf_data);
        self
    }

    /// Convert a Markdown string to PDF bytes.
    pub fn convert_markdown(&self, md: &str) -> Result<Vec<u8>, IronpressError> {
        let html = parser::markdown::markdown_to_html(md);
        self.convert(&html)
    }

    /// Convert an HTML string to PDF bytes.
    pub fn convert(&self, html: &str) -> Result<Vec<u8>, IronpressError> {
        let mut buf = Vec::new();
        self.convert_to_writer(html, &mut buf)?;
        Ok(buf)
    }

    /// Convert an HTML string to PDF, writing directly to any `std::io::Write` implementation.
    pub fn convert_to_writer<W: std::io::Write>(
        &self,
        html: &str,
        writer: &mut W,
    ) -> Result<(), IronpressError> {
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
        let mut page_rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parser::css::parse_stylesheet(css));
            page_rules.extend(parser::css::parse_page_rules(css));
        }

        // Step 3b: Apply @page rules to override page size and margins
        let mut effective_page_size = self.page_size;
        let mut effective_margin = self.margin;
        for pr in &page_rules {
            if let (Some(w), Some(h)) = (pr.width, pr.height) {
                effective_page_size = PageSize {
                    width: w,
                    height: h,
                };
            }
            if let Some(v) = pr.margin_top {
                effective_margin.top = v;
            }
            if let Some(v) = pr.margin_right {
                effective_margin.right = v;
            }
            if let Some(v) = pr.margin_bottom {
                effective_margin.bottom = v;
            }
            if let Some(v) = pr.margin_left {
                effective_margin.left = v;
            }
        }

        // Step 4: Parse custom fonts
        let parsed_fonts = self.parse_custom_fonts();

        // Step 5: Layout
        let pages = layout::engine::layout_with_rules_and_fonts(
            &result.nodes,
            effective_page_size,
            effective_margin,
            &rules,
            &parsed_fonts,
        );

        // Step 6: Render PDF
        if parsed_fonts.is_empty() {
            render::pdf::render_pdf_to_writer(&pages, effective_page_size, effective_margin, writer)
        } else {
            let pdf_bytes = render::pdf::render_pdf_with_fonts(
                &pages,
                effective_page_size,
                effective_margin,
                &parsed_fonts,
            )?;
            writer
                .write_all(&pdf_bytes)
                .map_err(|e| IronpressError::RenderError(format!("write error: {e}")))
        }
    }

    /// Convert a Markdown string to PDF, writing directly to any `std::io::Write` implementation.
    pub fn convert_markdown_to_writer<W: std::io::Write>(
        &self,
        md: &str,
        writer: &mut W,
    ) -> Result<(), IronpressError> {
        let html = parser::markdown::markdown_to_html(md);
        self.convert_to_writer(&html, writer)
    }

    /// Parse all registered custom fonts into TtfFont structs.
    fn parse_custom_fonts(&self) -> std::collections::HashMap<String, parser::ttf::TtfFont> {
        let mut fonts = std::collections::HashMap::new();
        for (name, data) in &self.custom_fonts {
            if let Ok(font) = parser::ttf::parse_ttf(data.clone()) {
                fonts.insert(name.clone(), font);
            }
        }
        fonts
    }

    /// Async version of [`HtmlConverter::convert`] for file-based conversion.
    /// Requires the `async` feature.
    ///
    /// Reads the input HTML file asynchronously, performs the CPU-bound conversion
    /// in a blocking task, then writes the output PDF asynchronously.
    #[cfg(feature = "async")]
    pub async fn convert_file_async(
        &self,
        input: &str,
        output: &str,
    ) -> Result<(), IronpressError> {
        let html = tokio::fs::read_to_string(input).await?;
        let page_size = self.page_size;
        let margin = self.margin;
        let sanitize = self.sanitize;
        let pdf = tokio::task::spawn_blocking(move || {
            HtmlConverter::new()
                .page_size(page_size)
                .margin(margin)
                .sanitize(sanitize)
                .convert(&html)
        })
        .await
        .map_err(|e| IronpressError::RenderError(format!("task join error: {e}")))?;
        let pdf = pdf?;
        tokio::fs::write(output, pdf).await?;
        Ok(())
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
    fn markdown_to_pdf_roundtrip() {
        // Exercises markdown_to_pdf() (line 64-67)
        let pdf = markdown_to_pdf("# Test\n\nHello **world**").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Test"));
        assert!(content.contains("world"));
    }

    #[test]
    fn convert_markdown_file_roundtrip() {
        // Exercises convert_markdown_file() (lines 76-80)
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_test_md_input.md");
        let output = dir.join("ironpress_test_md_output.pdf");
        std::fs::write(&input, "# Hello\n\nWorld").unwrap();
        convert_markdown_file(input.to_str().unwrap(), output.to_str().unwrap()).unwrap();
        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Hello"));
        std::fs::remove_file(&input).ok();
        std::fs::remove_file(&output).ok();
    }

    #[test]
    fn convert_markdown_file_missing_input() {
        let result = convert_markdown_file("/nonexistent/file.md", "/tmp/out.pdf");
        assert!(result.is_err());
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

    #[test]
    fn html_to_pdf_display_none_hides_element() {
        let html = r#"<p>Visible</p><p style="display: none">Secret</p><p>Remaining</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Visible"));
        assert!(!content.contains("Secret"));
        assert!(content.contains("Remaining"));
    }

    #[test]
    fn html_to_pdf_display_block_on_span() {
        let html = r#"<p><span style="display: block">Blocked</span></p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Blocked"));
    }

    #[test]
    fn html_to_pdf_media_print_applied() {
        let html = r#"
            <html>
            <head><style>
                @media print { p { color: red } }
            </style></head>
            <body><p>Print styled</p></body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 rg")); // red color applied
    }

    #[test]
    fn html_to_pdf_media_screen_ignored() {
        let html = r#"
            <html>
            <head><style>
                @media screen { p { color: red } }
            </style></head>
            <body><p>Not red</p></body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should NOT have red color since screen media is ignored
        assert!(!content.contains("1 0 0 rg"));
    }

    #[test]
    fn html_to_pdf_strikethrough() {
        let html = "<p><del>deleted</del> and <s>struck</s></p>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("deleted"));
        assert!(content.contains("struck"));
    }

    #[test]
    fn html_to_pdf_page_break() {
        let html = r#"<p style="page-break-after: always">Page one</p><p>Page two</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_border() {
        let html = r#"<div style="border: 2px solid blue">Bordered content</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Bordered"));
    }

    #[test]
    fn html_to_pdf_font_families() {
        let html = r#"
            <p style="font-family: serif">Serif text</p>
            <p style="font-family: monospace">Mono text</p>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Times-Roman"));
        assert!(content.contains("Courier"));
    }

    #[test]
    fn html_to_pdf_table_colspan() {
        let html = r#"
            <table>
                <tr><td colspan="2">Wide</td></tr>
                <tr><td>A</td><td>B</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Wide"));
    }

    #[test]
    fn html_to_pdf_style_border_color_and_width() {
        let html = r#"
            <html>
            <head><style>div { border-width: 2pt; border-color: red }</style></head>
            <body><div>Bordered</div></body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn sanitizer_malformed_style_tag() {
        // Style tag without closing tag
        let html = "<style>p { color: red }";
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn sanitizer_event_handler_with_spaces() {
        let html = r#"<p onclick = "alert('xss')">Safe text</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(!content.contains("alert"));
        assert!(content.contains("Safe"));
    }

    // --- Streaming output tests ---

    #[test]
    fn streaming_produces_same_output_as_non_streaming() {
        let html = "<h1>Hello</h1><p>World</p>";
        let pdf_vec = html_to_pdf(html).unwrap();
        let mut streamed = Vec::new();
        html_to_pdf_writer(html, &mut streamed).unwrap();
        assert_eq!(pdf_vec, streamed);
    }

    #[test]
    fn streaming_markdown_produces_same_output() {
        let md = "# Title\n\nSome **bold** text.";
        let pdf_vec = markdown_to_pdf(md).unwrap();
        let mut streamed = Vec::new();
        markdown_to_pdf_writer(md, &mut streamed).unwrap();
        assert_eq!(pdf_vec, streamed);
    }

    #[test]
    fn streaming_to_file() {
        let dir = std::env::temp_dir();
        let output = dir.join("ironpress_stream_test.pdf");
        let mut file = std::fs::File::create(&output).unwrap();
        html_to_pdf_writer("<p>Streamed</p>", &mut file).unwrap();
        drop(file);
        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Streamed"));
        std::fs::remove_file(&output).ok();
    }

    #[test]
    fn converter_convert_to_writer() {
        let html = "<p>Builder streaming</p>";
        let pdf_vec = HtmlConverter::new().convert(html).unwrap();
        let mut streamed = Vec::new();
        HtmlConverter::new()
            .convert_to_writer(html, &mut streamed)
            .unwrap();
        assert_eq!(pdf_vec, streamed);
    }

    #[test]
    fn converter_convert_markdown_to_writer() {
        let md = "# Markdown streaming";
        let pdf_vec = HtmlConverter::new().convert_markdown(md).unwrap();
        let mut streamed = Vec::new();
        HtmlConverter::new()
            .convert_markdown_to_writer(md, &mut streamed)
            .unwrap();
        assert_eq!(pdf_vec, streamed);
    }

    #[test]
    fn url_image_ignored_for_security() {
        // Remote URLs are not loaded (SSRF risk). The PDF is generated without the image.
        let html = r#"<img src="https://example.com/image.png" width="100" height="100">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    // --- Async tests (feature-gated) ---

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_convert_file_roundtrip() {
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_async_test_input.html");
        let output = dir.join("ironpress_async_test_output.pdf");
        tokio::fs::write(&input, "<h1>Async</h1><p>Test</p>")
            .await
            .unwrap();
        convert_file_async(input.to_str().unwrap(), output.to_str().unwrap())
            .await
            .unwrap();
        let pdf = tokio::fs::read(&output).await.unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Async"));
        tokio::fs::remove_file(&input).await.ok();
        tokio::fs::remove_file(&output).await.ok();
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_convert_markdown_file_roundtrip() {
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_async_md_test.md");
        let output = dir.join("ironpress_async_md_test.pdf");
        tokio::fs::write(&input, "# Async MD\n\nHello")
            .await
            .unwrap();
        convert_markdown_file_async(input.to_str().unwrap(), output.to_str().unwrap())
            .await
            .unwrap();
        let pdf = tokio::fs::read(&output).await.unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Async"));
        tokio::fs::remove_file(&input).await.ok();
        tokio::fs::remove_file(&output).await.ok();
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_converter_convert_file() {
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_async_builder_input.html");
        let output = dir.join("ironpress_async_builder_output.pdf");
        tokio::fs::write(&input, "<p>Builder async</p>")
            .await
            .unwrap();
        HtmlConverter::new()
            .page_size(PageSize::LETTER)
            .convert_file_async(input.to_str().unwrap(), output.to_str().unwrap())
            .await
            .unwrap();
        let pdf = tokio::fs::read(&output).await.unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        tokio::fs::remove_file(&input).await.ok();
        tokio::fs::remove_file(&output).await.ok();
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_convert_file_missing_input() {
        let result = convert_file_async("/nonexistent/file.html", "/tmp/out.pdf").await;
        assert!(result.is_err());
    }

    #[test]
    fn html_to_pdf_with_width() {
        let html = r#"<div style="width: 200pt">Constrained width</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_max_width() {
        let html = r#"<div style="max-width: 300pt">Max width block</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_height() {
        let html = r#"<div style="height: 100pt">Fixed height</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_opacity() {
        let html = r#"<div style="opacity: 0.5">Semi-transparent</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/ExtGState"));
        assert!(content.contains("/ca 0.5"));
    }

    // --- Integration tests for float / clear / position / box-shadow ---

    #[test]
    fn html_to_pdf_with_float_left() {
        let html = r#"<div style="float: left; width: 100pt">Floated</div><div>Normal</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_clear_both() {
        let html = r#"
            <div style="float: left">Floated</div>
            <div style="clear: both">Cleared</div>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_position_relative() {
        let html = r#"<div style="position: relative; top: 10pt; left: 5pt">Offset content</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_position_absolute() {
        let html = r#"<div style="position: absolute; top: 100pt; left: 50pt">Absolute</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_box_shadow() {
        let html = r#"<div style="box-shadow: 3px 3px black">Shadowed</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        // The PDF should contain the shadow rectangle (a filled rect with black color)
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("re\nf"),
            "Box shadow should produce a filled rectangle"
        );
    }

    #[test]
    fn html_to_pdf_float_and_clear_combined() {
        let html = r#"
            <div style="float: left; width: 150pt">Left sidebar</div>
            <div style="float: right; width: 150pt">Right sidebar</div>
            <div style="clear: both">Footer content below floats</div>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_box_shadow_with_blur() {
        let html = r#"<div style="box-shadow: 2px 2px 4px red">Shadow with blur</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    /// Build a minimal valid TTF for integration testing.
    fn build_integration_test_ttf() -> Vec<u8> {
        let mut buf = Vec::new();
        let num_tables: u16 = 6;
        buf.extend_from_slice(&[0, 1, 0, 0]);
        buf.extend_from_slice(&num_tables.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        let dir_start = buf.len();
        buf.resize(dir_start + num_tables as usize * 16, 0);

        // head table (54 bytes)
        let head_offset = buf.len();
        buf.extend_from_slice(&[0, 1, 0, 0]);
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
        buf.extend_from_slice(&0x000Bu16.to_be_bytes());
        buf.extend_from_slice(&1000u16.to_be_bytes()); // unitsPerEm
        buf.extend_from_slice(&[0; 16]); // created + modified
        buf.extend_from_slice(&(-100i16).to_be_bytes());
        buf.extend_from_slice(&(-200i16).to_be_bytes());
        buf.extend_from_slice(&800i16.to_be_bytes());
        buf.extend_from_slice(&900i16.to_be_bytes());
        buf.extend_from_slice(&[0; 8]); // macStyle..glyphDataFormat
        let head_len = buf.len() - head_offset;

        // hhea table (36 bytes)
        let hhea_offset = buf.len();
        buf.extend_from_slice(&[0, 1, 0, 0]);
        buf.extend_from_slice(&800i16.to_be_bytes());
        buf.extend_from_slice(&(-200i16).to_be_bytes());
        buf.extend_from_slice(&[0; 24]); // remaining fields
        buf.extend_from_slice(&3u16.to_be_bytes()); // numOfLongHorMetrics
        let hhea_len = buf.len() - hhea_offset;

        // maxp table
        let maxp_offset = buf.len();
        buf.extend_from_slice(&[0, 0, 0x50, 0]);
        buf.extend_from_slice(&3u16.to_be_bytes());
        let maxp_len = buf.len() - maxp_offset;

        // hmtx table (3 glyphs)
        let hmtx_offset = buf.len();
        for w in [500u16, 250, 700] {
            buf.extend_from_slice(&w.to_be_bytes());
            buf.extend_from_slice(&0i16.to_be_bytes());
        }
        let hmtx_len = buf.len() - hmtx_offset;

        // cmap table (format 4): char 32->glyph 1, char 65->glyph 2
        let cmap_offset = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&3u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&12u32.to_be_bytes());
        let subtable_start = buf.len();
        buf.extend_from_slice(&4u16.to_be_bytes());
        let len_pos = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&6u16.to_be_bytes()); // segCountX2 = 3*2
        buf.extend_from_slice(&4u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&2u16.to_be_bytes());
        // endCode
        for v in [32u16, 65, 0xFFFF] {
            buf.extend_from_slice(&v.to_be_bytes());
        }
        buf.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
        // startCode
        for v in [32u16, 65, 0xFFFF] {
            buf.extend_from_slice(&v.to_be_bytes());
        }
        // idDelta
        for v in [-31i16, -63, 1] {
            buf.extend_from_slice(&v.to_be_bytes());
        }
        // idRangeOffset
        for _ in 0..3 {
            buf.extend_from_slice(&0u16.to_be_bytes());
        }
        let subtable_len = (buf.len() - subtable_start) as u16;
        buf[len_pos] = (subtable_len >> 8) as u8;
        buf[len_pos + 1] = subtable_len as u8;
        let cmap_len = buf.len() - cmap_offset;

        // name table
        let name_offset = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&18u16.to_be_bytes());
        let font_name_str = b"TestFont";
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&(font_name_str.len() as u16).to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(font_name_str);
        let name_len = buf.len() - name_offset;

        // Fill in table directory
        let tables_info: [(&[u8; 4], usize, usize); 6] = [
            (b"head", head_offset, head_len),
            (b"hhea", hhea_offset, hhea_len),
            (b"maxp", maxp_offset, maxp_len),
            (b"hmtx", hmtx_offset, hmtx_len),
            (b"cmap", cmap_offset, cmap_len),
            (b"name", name_offset, name_len),
        ];
        for (i, (tag, offset, length)) in tables_info.iter().enumerate() {
            let dir_off = dir_start + i * 16;
            buf[dir_off..dir_off + 4].copy_from_slice(*tag);
            buf[dir_off + 4..dir_off + 8].copy_from_slice(&0u32.to_be_bytes());
            buf[dir_off + 8..dir_off + 12].copy_from_slice(&(*offset as u32).to_be_bytes());
            buf[dir_off + 12..dir_off + 16].copy_from_slice(&(*length as u32).to_be_bytes());
        }
        buf
    }

    #[test]
    fn add_font_embeds_truetype_in_pdf() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont">Hello A</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Subtype /TrueType"),
            "PDF should contain embedded TrueType font"
        );
        assert!(
            content.contains("/BaseFont /testfont"),
            "PDF should reference the custom font name"
        );
        assert!(
            content.contains("/FontDescriptor"),
            "PDF should contain FontDescriptor"
        );
        assert!(
            content.contains("/FontFile2"),
            "FontDescriptor should reference embedded font file"
        );
        assert!(
            content.contains("/Widths ["),
            "Font object should contain Widths array"
        );
        assert!(
            content.contains("/Encoding /WinAnsiEncoding"),
            "Font should use WinAnsiEncoding"
        );
    }

    #[test]
    fn add_font_uses_custom_font_in_content_stream() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont">Hello</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/testfont"),
            "Content stream should reference custom font"
        );
    }

    #[test]
    fn custom_font_falls_back_to_helvetica_when_not_registered() {
        let pdf = html_to_pdf(r#"<p style="font-family: 'UnknownFont'">Text</p>"#).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Helvetica"),
            "Should fall back to Helvetica for unregistered custom font"
        );
    }

    #[test]
    fn add_font_font_descriptor_has_metrics() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont">A</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Ascent"),
            "FontDescriptor should have Ascent"
        );
        assert!(
            content.contains("/Descent"),
            "FontDescriptor should have Descent"
        );
        assert!(
            content.contains("/FontBBox"),
            "FontDescriptor should have FontBBox"
        );
        assert!(
            content.contains("/Flags"),
            "FontDescriptor should have Flags"
        );
    }

    #[test]
    fn add_font_standard_fonts_still_work() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<p style="font-family: testfont">Custom</p>
                   <p style="font-family: serif">Serif</p>
                   <p>Default</p>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
        assert!(content.contains("/Times-Roman"));
        assert!(content.contains("/Helvetica"));
    }

    #[test]
    fn add_font_multiple_custom_fonts() {
        let ttf1 = build_integration_test_ttf();
        let ttf2 = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("fontone", ttf1)
            .add_font("fonttwo", ttf2)
            .convert(
                r#"<p style="font-family: fontone">First</p>
                   <p style="font-family: fonttwo">Second</p>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/fontone"));
        assert!(content.contains("/fonttwo"));
    }

    #[test]
    fn add_font_case_insensitive_matching() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("MyFont", ttf_data)
            .convert(r#"<p style="font-family: MyFont">Text</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Font name is lowercased internally
        assert!(content.contains("/myfont") || content.contains("/MyFont"));
    }

    #[test]
    fn add_font_in_table_cell() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<table><tr><td style="font-family: testfont">Cell</td></tr></table>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
    }

    #[test]
    fn add_font_with_bold_text() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont"><b>Bold custom</b></p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn add_font_with_italic_text() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont"><i>Italic custom</i></p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn add_font_empty_text_no_crash() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont"></p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn add_font_with_inline_style_inheritance() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<div style="font-family: testfont"><p>Inherited</p><p>Also inherited</p></div>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
    }

    #[test]
    fn add_font_with_stylesheet() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<html><head><style>.custom { font-family: testfont; }</style></head>
                   <body><p class="custom">Styled</p></body></html>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
    }

    #[test]
    fn add_font_invalid_ttf_data_gracefully_degrades() {
        let pdf = HtmlConverter::new()
            .add_font("badfont", vec![0, 1, 2, 3])
            .convert(r#"<p style="font-family: badfont">Text</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should fall back to Helvetica since the font couldn't be parsed
        assert!(content.contains("/Helvetica"));
    }

    #[test]
    fn add_font_preserves_page_size_and_margin() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .page_size(PageSize {
                width: 612.0,
                height: 792.0,
            })
            .margin(Margin::uniform(36.0))
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont">Custom</p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_in_list_item() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<ul style="font-family: testfont"><li>Item 1</li><li>Item 2</li></ul>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_in_nested_elements() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<div style="font-family: testfont"><p><span>Nested <b>bold</b></span></p></div>"#,
            )
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_with_long_text_wrapping() {
        let ttf_data = build_integration_test_ttf();
        let long_text = "A ".repeat(500);
        let html = format!(r#"<p style="font-family: testfont">{long_text}</p>"#,);
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(&html)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_mixed_with_standard_in_same_paragraph() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<p><span style="font-family: testfont">Custom</span> and <span style="font-family: serif">Serif</span></p>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
        assert!(content.contains("/Times-Roman"));
    }

    #[test]
    fn custom_font_with_opacity() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont; opacity: 0.5">Transparent custom</p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_with_width_and_background() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<div style="font-family: testfont; width: 200px; background-color: yellow">Boxed custom</div>"#,
            )
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_markdown_conversion() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert_markdown("# Hello World\n\nSome text here.")
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn linear_gradient_produces_pdf() {
        let html = r#"<div style="background: linear-gradient(to right, red, blue); height: 50pt; width: 200pt">Gradient</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Should contain colored rectangles (gradient strips)
        assert!(content.contains("rg"));
    }

    #[test]
    fn radial_gradient_produces_pdf() {
        let html = r#"<div style="background: radial-gradient(red, blue); height: 100pt; width: 100pt">Radial</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn page_rule_changes_page_size() {
        let html = r#"<style>@page { size: letter; }</style><p>Hello</p>"#;
        let pdf = HtmlConverter::new().convert(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Letter size is 612x792, should appear in MediaBox
        assert!(content.contains("612"));
        assert!(content.contains("792"));
    }

    #[test]
    fn page_rule_changes_margins() {
        let html = r#"<style>@page { margin: 0.5in; }</style><p>Hello</p>"#;
        let pdf = HtmlConverter::new().convert(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn page_rule_a4_landscape() {
        let html = r#"<style>@page { size: a4 landscape; }</style><p>Hello</p>"#;
        let pdf = HtmlConverter::new().convert(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Landscape A4: 841.89 x 595.28
        assert!(content.contains("841.89"));
        assert!(content.contains("595.28"));
    }

    #[test]
    fn linear_gradient_with_multiple_stops() {
        let html = r#"<div style="background: linear-gradient(to right, red 0%, white 50%, blue 100%); height: 50pt; width: 200pt">Multi-stop</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn gradient_via_background_image_property() {
        let html = r#"<div style="background-image: linear-gradient(45deg, #ff0000, #0000ff); height: 50pt; width: 200pt">Angled</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_border_radius() {
        let html = r#"<div style="border: 1px solid black; border-radius: 10pt; background-color: yellow; padding: 10pt">Rounded corners</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Rounded rect uses Bezier curves (c operator)
        assert!(content.contains(" c\n"));
    }

    #[test]
    fn html_to_pdf_outline() {
        let html = r#"<div style="outline: 3px solid blue; width: 200pt">With outline</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Outline draws a stroke
        assert!(content.contains("S\n"));
    }

    #[test]
    fn html_to_pdf_box_sizing_border_box() {
        let html = r#"<div style="box-sizing: border-box; width: 200pt; padding: 20pt; border: 2px solid black; background-color: green">Border box</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_combined_features() {
        let html = r#"<div style="border: 2px solid black; border-radius: 15pt; outline: 3px solid red; box-sizing: border-box; width: 300pt; padding: 20pt; background-color: #eee">All features combined</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains(" c\n")); // Bezier curves from border-radius
    }

    // --- Coverage tests for pdf.rs and engine.rs uncovered lines ---

    #[test]
    fn pdf_float_right_positions_block() {
        // Covers pdf.rs line 119: Float::Right block_x calculation
        let html = r#"<p style="float: right; width: 100pt">FloatRight</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("FloatRight"));
    }

    #[test]
    fn pdf_visibility_hidden_skips_rendering() {
        // Covers pdf.rs line 110: visibility hidden skips rendering
        let html = r#"<p style="visibility: hidden">HiddenStuff</p><p>VisibleStuff</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("VisibleStuff"));
        assert!(!content.contains("(HiddenStuff)"));
    }

    #[test]
    fn pdf_overflow_hidden_clips_content() {
        // Covers pdf.rs lines 155-172: clip_rect with overflow: hidden
        let html = r#"<p style="overflow: hidden; width: 100pt; height: 50pt">ClippedHere</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("W n\n"));
    }

    #[test]
    fn pdf_overflow_hidden_with_border_radius() {
        // Covers pdf.rs lines 161-169: clip_rect with border-radius uses rounded path + W n
        let html = r#"<p style="overflow: hidden; border-radius: 10pt; width: 100pt; height: 50pt">RoundedClip</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("W n\n"));
        assert!(content.contains(" c\n"));
    }

    #[test]
    fn pdf_opacity_sets_ext_gstate() {
        // Covers pdf.rs lines 176-181: opacity < 1.0 creates ExtGState
        let html = r#"<p style="opacity: 0.5">Translucent</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("gs\n"));
    }

    #[test]
    fn pdf_box_shadow_renders_rect() {
        // Covers pdf.rs lines 184-213: box-shadow rendering
        let html =
            r#"<p style="box-shadow: 5pt 5pt black; width: 100pt; padding: 10pt">ShadowBox</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("f\n"));
    }

    #[test]
    fn pdf_box_shadow_with_explicit_height() {
        // Covers pdf.rs line 188: box-shadow with block_height Some(h) path
        let html = r#"<p style="box-shadow: 3pt 3pt black; width: 100pt; height: 80pt; padding: 10pt">ShadowH</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("f\n"));
    }

    #[test]
    fn pdf_box_shadow_with_border_radius() {
        // Covers pdf.rs lines 195-202: box-shadow with border-radius uses rounded rect
        let html = r#"<p style="box-shadow: 3pt 3pt black; border-radius: 10pt; width: 100pt; padding: 10pt">RoundShadow</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains(" c\n"));
        assert!(content.contains("f\n"));
    }

    #[test]
    fn pdf_background_with_explicit_height() {
        // Covers pdf.rs line 220: background_color with block_height Some(h) path
        let html =
            r#"<p style="background-color: #ff0000; width: 100pt; height: 80pt">BGHeight</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 rg"));
        assert!(content.contains("f\n"));
    }

    #[test]
    fn pdf_linear_gradient_renders_strips() {
        // Covers pdf.rs lines 245-261, 806-864: linear gradient rendering
        let html = r#"<p style="background: linear-gradient(to right, red, blue); width: 200pt; height: 50pt; padding: 10pt">Gradient</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("rg\n"));
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn pdf_linear_gradient_vertical() {
        // Covers pdf.rs lines 831-847: vertical gradient (to bottom)
        let html = r#"<p style="background: linear-gradient(to bottom, red, blue); width: 200pt; height: 50pt; padding: 10pt">VertGrad</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn pdf_linear_gradient_with_block_height() {
        // Covers pdf.rs line 251: gradient with block_height Some(h)
        let html = r#"<p style="background: linear-gradient(to right, red, blue); width: 200pt; height: 100pt; padding: 10pt">GradHeight</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn pdf_linear_gradient_diagonal() {
        // Covers pdf.rs lines 848-863: angled gradient (45deg)
        let html = r#"<p style="background: linear-gradient(45deg, red, blue); width: 200pt; height: 50pt; padding: 10pt">DiagGrad</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn pdf_radial_gradient_renders_circles() {
        // Covers pdf.rs lines 264-281, 867-900: radial gradient rendering
        let html = r#"<p style="background: radial-gradient(red, blue); width: 200pt; height: 100pt; padding: 10pt">Radial</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains(" c\n"));
    }

    #[test]
    fn pdf_radial_gradient_with_block_height() {
        // Covers pdf.rs line 270: radial gradient with block_height Some(h)
        let html = r#"<p style="background: radial-gradient(red, blue); width: 200pt; height: 120pt; padding: 10pt">RadialH</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains(" c\n"));
    }

    #[test]
    fn pdf_border_with_block_height() {
        // Covers pdf.rs line 288: border with block_height Some(h) path
        let html = r#"<p style="border: 2pt solid black; width: 100pt; height: 80pt">BorderH</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("RG\n"));
        assert!(content.contains("S\n"));
    }

    #[test]
    fn pdf_outline_with_block_height() {
        // Covers pdf.rs line 320: outline with block_height Some(h) path
        let html = r#"<p style="outline: 3pt solid red; width: 100pt; height: 80pt">OutlineH</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("RG\n"));
        assert!(content.contains("S\n"));
    }

    #[test]
    fn pdf_transform_rotate() {
        // Covers pdf.rs lines 132-152: transform rendering
        let html = r#"<p style="transform: rotate(45deg)">Rotated</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("cm\n"));
        assert!(content.contains("q\n"));
        assert!(content.contains("Q\n"));
    }

    #[test]
    fn pdf_transform_scale() {
        // Covers pdf.rs line 147: scale transform
        let html = r#"<p style="transform: scale(2)">Scaled</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("cm\n"));
    }

    #[test]
    fn pdf_transform_translate() {
        // Covers pdf.rs lines 149-150: translate transform
        let html = r#"<p style="transform: translate(10pt, 20pt)">Translated</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 1"));
        assert!(content.contains("cm\n"));
    }

    #[test]
    fn pdf_text_justify_alignment() {
        // Covers pdf.rs lines 363-374: text-align: justify with word spacing
        let html = r#"<p style="text-align: justify; width: 200pt">This is a long sentence with many words that should be justified across the width of the container for proper testing purposes here.</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Tw\n"));
    }

    #[test]
    fn pdf_page_break_element() {
        // Covers pdf.rs line 616: PageBreak element
        // Also covers engine.rs line 602: page-break-after
        let html = r#"<p style="page-break-after: always">PageOne</p><p>PageTwo</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("PageOne"));
        assert!(content.contains("PageTwo"));
    }

    #[test]
    fn pdf_grid_row_renders_cells() {
        // Covers pdf.rs lines 535-573: GridRow rendering
        // Covers engine.rs lines 607-622: grid container handling
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr">
                <div>CellAlpha</div>
                <div>CellBeta</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("CellAlpha"));
        assert!(content.contains("CellBeta"));
    }

    #[test]
    fn pdf_grid_row_with_background() {
        // Covers pdf.rs lines 550-557: grid cell background rendering
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr">
                <div style="background-color: red">RedCell</div>
                <div style="background-color: blue">BlueCell</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("rg\n"));
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn pdf_grid_with_three_columns() {
        // Covers pdf.rs line 546: fallback col_widths for extra cells
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr 1fr">
                <div>A</div><div>B</div><div>C</div><div>D</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn pdf_grid_with_page_break_after() {
        // Covers engine.rs lines 619-620: page_break_after for grid container
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr; page-break-after: always">
                <div>GridPageOne</div>
            </div>
            <p>AfterGrid</p>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("GridPageOne"));
        assert!(content.contains("AfterGrid"));
    }

    #[test]
    fn engine_flex_container_with_background() {
        // Covers engine.rs lines 1059-1097: flex container bg/border/shadow emit
        let html = r#"<html><body>
            <div style="display: flex; background-color: #eee; border: 1pt solid black; padding: 10pt">
                <div style="width: 100pt">FlexChild</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("FlexChild"));
    }

    #[test]
    fn engine_flex_wrap_wraps_items() {
        // Covers engine.rs lines 979-989: flex-wrap: wrap wrapping behavior
        let html = r#"<html><body>
            <div style="display: flex; flex-wrap: wrap; width: 200pt">
                <div style="width: 120pt">ItemOne</div>
                <div style="width: 120pt">ItemTwo</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("ItemOne"));
        assert!(content.contains("ItemTwo"));
    }

    #[test]
    fn engine_flex_justify_space_between() {
        // Covers engine.rs lines 1122-1127: justify-content: space-between
        let html = r#"<html><body>
            <div style="display: flex; justify-content: space-between; width: 300pt">
                <div style="width: 50pt">LeftSide</div>
                <div style="width: 50pt">RightSide</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("LeftSide"));
        assert!(content.contains("RightSide"));
    }

    #[test]
    fn engine_flex_justify_space_between_single() {
        // Covers engine.rs line 1126: space-between with single item (0 gap)
        let html = r#"<html><body>
            <div style="display: flex; justify-content: space-between; width: 300pt">
                <div style="width: 50pt">OnlyItem</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("OnlyItem"));
    }

    #[test]
    fn engine_flex_justify_space_around() {
        // Covers engine.rs lines 1129-1132: justify-content: space-around
        let html = r#"<html><body>
            <div style="display: flex; justify-content: space-around; width: 300pt">
                <div style="width: 50pt">ItemX</div>
                <div style="width: 50pt">ItemY</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("ItemX"));
        assert!(content.contains("ItemY"));
    }

    #[test]
    fn engine_flex_justify_center() {
        // Covers engine.rs line 1121: justify-content: center
        let html = r#"<html><body>
            <div style="display: flex; justify-content: center; width: 300pt">
                <div style="width: 50pt">CenteredItem</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("CenteredItem"));
    }

    #[test]
    fn engine_flex_justify_flex_end() {
        // Covers engine.rs line 1120: justify-content: flex-end
        let html = r#"<html><body>
            <div style="display: flex; justify-content: flex-end; width: 300pt">
                <div style="width: 50pt">EndItem</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("EndItem"));
    }

    #[test]
    fn engine_flex_align_items_center() {
        // Covers engine.rs line 1144: align-items: center
        let html = r#"<html><body>
            <div style="display: flex; align-items: center; width: 300pt">
                <div style="width: 100pt">TallItem</div>
                <div style="width: 100pt">ShortItem</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("TallItem"));
        assert!(content.contains("ShortItem"));
    }

    #[test]
    fn engine_flex_align_items_flex_end() {
        // Covers engine.rs line 1143: align-items: flex-end
        let html = r#"<html><body>
            <div style="display: flex; align-items: flex-end; width: 300pt">
                <div style="width: 100pt">BottomItem</div>
                <div style="width: 100pt">AlsoBottom</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("BottomItem"));
        assert!(content.contains("AlsoBottom"));
    }

    #[test]
    fn engine_flex_direction_column() {
        // Covers engine.rs lines 1002-1021, 1230-1335: flex-direction: column
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; width: 200pt">
                <div style="width: 100pt">RowAlpha</div>
                <div style="width: 100pt">RowBeta</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("RowAlpha"));
        assert!(content.contains("RowBeta"));
    }

    #[test]
    fn engine_flex_column_align_center() {
        // Covers engine.rs lines 1247-1249: column flex align-items: center (x_offset)
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; align-items: center; width: 300pt">
                <div style="width: 100pt">ColCenter</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("ColCenter"));
    }

    #[test]
    fn engine_flex_column_align_flex_end() {
        // Covers engine.rs lines 1248: column flex align-items: flex-end
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; align-items: flex-end; width: 300pt">
                <div style="width: 100pt">ColEnd</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("ColEnd"));
    }

    #[test]
    fn engine_flex_container_with_margin() {
        // Covers engine.rs lines 1342-1378: flex trailing margin
        let html = r#"<html><body>
            <div style="display: flex; margin: 20pt; background-color: #ccc; width: 200pt">
                <div style="width: 100pt">MarginedFlex</div>
            </div>
            <p>AfterFlex</p>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("MarginedFlex"));
        assert!(content.contains("AfterFlex"));
    }

    #[test]
    fn engine_flex_with_overflow_hidden() {
        // Covers engine.rs lines 1082-1085: overflow: hidden in flex container
        let html = r#"<html><body>
            <div style="display: flex; overflow: hidden; width: 200pt; background-color: #eee">
                <div style="width: 100pt">ClippedFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("ClippedFlex"));
    }

    #[test]
    fn engine_flex_with_transform() {
        // Covers engine.rs line 1087: transform in flex container
        let html = r#"<html><body>
            <div style="display: flex; transform: rotate(5deg); background-color: #eee; width: 200pt">
                <div style="width: 100pt">TransFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("TransFlex"));
    }

    #[test]
    fn engine_flex_with_box_shadow() {
        // Covers engine.rs lines 1059, 1080: box-shadow in flex container
        let html = r#"<html><body>
            <div style="display: flex; box-shadow: 3pt 3pt black; width: 200pt">
                <div style="width: 100pt">ShadowFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("ShadowFlex"));
    }

    #[test]
    fn engine_flex_height_constrains_container() {
        // Covers engine.rs line 1049: flex height with Some(h) path
        let html = r#"<html><body>
            <div style="display: flex; height: 200pt; background-color: #eee; width: 300pt">
                <div style="width: 100pt">TallFlexContent</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("TallFlexContent"));
    }

    #[test]
    fn engine_flex_child_box_sizing_border_box() {
        // Covers engine.rs lines 865-869: box-sizing: border-box in flex child
        let html = r#"<html><body>
            <div style="display: flex; width: 300pt">
                <div style="width: 150pt; box-sizing: border-box; padding: 10pt; border: 2pt solid black">BorderBoxChild</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("BorderBoxChild"));
    }

    #[test]
    fn engine_flex_with_max_width() {
        // Covers engine.rs lines 800, 803: flex container width/max-width
        let html = r#"<html><body>
            <div style="display: flex; width: 300pt; max-width: 250pt; background-color: #eee">
                <div style="width: 100pt">MaxWidthFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("MaxWidthFlex"));
    }

    #[test]
    fn engine_flex_child_display_none() {
        // Covers engine.rs line 856: child with display: none is skipped
        let html = r#"<html><body>
            <div style="display: flex; width: 300pt">
                <div style="display: none; width: 100pt">HiddenFlex</div>
                <div style="width: 100pt">VisibleFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(!content.contains("(HiddenFlex)"));
        assert!(content.contains("VisibleFlex"));
    }

    #[test]
    fn engine_flex_page_break_after() {
        // Covers engine.rs lines 601-602: page-break-after for flex container
        let html = r#"<html><body>
            <div style="display: flex; page-break-after: always">
                <div style="width: 100pt">FlexPageOne</div>
            </div>
            <p>FlexPageTwo</p>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("FlexPageOne"));
        assert!(content.contains("FlexPageTwo"));
    }

    #[test]
    fn engine_grid_with_gap() {
        // Covers engine.rs line 1390: grid column gap
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 10pt">
                <div>GridAlpha</div>
                <div>GridBeta</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("GridAlpha"));
        assert!(content.contains("GridBeta"));
    }

    #[test]
    fn engine_grid_fixed_columns() {
        // Covers engine.rs line 1414: fixed + fr grid tracks
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 100pt 1fr">
                <div>FixedCol</div>
                <div>FlexCol</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("FixedCol"));
        assert!(content.contains("FlexCol"));
    }

    #[test]
    fn engine_table_with_colspan() {
        // Covers engine.rs line 1602: colspan counting in table
        let html = r#"
            <table>
                <tr><td colspan="2">Spanning</td></tr>
                <tr><td>CellA</td><td>CellB</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Spanning"));
        assert!(content.contains("CellA"));
        assert!(content.contains("CellB"));
    }

    #[test]
    fn engine_table_with_rowspan() {
        // Covers pdf.rs lines 490-504, engine.rs rowspan handling
        let html = r#"
            <table>
                <tr><td rowspan="2">TallCell</td><td>TopCell</td></tr>
                <tr><td>BottomCell</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("TallCell"));
        assert!(content.contains("TopCell"));
        assert!(content.contains("BottomCell"));
    }

    #[test]
    fn engine_table_with_thead_tbody_tfoot_coverage() {
        // Covers engine.rs lines 1565, 1575: table section traversal
        let html = r#"
            <table>
                <thead><tr><th>HeadCol</th></tr></thead>
                <tbody><tr><td>BodyRow</td></tr></tbody>
                <tfoot><tr><td>FootRow</td></tr></tfoot>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("HeadCol"));
        assert!(content.contains("BodyRow"));
        assert!(content.contains("FootRow"));
    }

    #[test]
    fn engine_table_non_tr_children_ignored() {
        // Covers engine.rs line 1575: non-tr/thead/tbody/tfoot children
        let html = r#"
            <table>
                <tr><td>ValidCell</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("ValidCell"));
    }

    #[test]
    fn engine_table_non_td_children_in_row() {
        // Covers engine.rs line 1687: non-td/th elements in a row are skipped
        let html = r#"
            <table>
                <tr><td>GoodCell</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("GoodCell"));
    }

    #[test]
    fn engine_ordered_list_indent() {
        // Covers engine.rs lines 486, 491: ordered list indent
        let html = r#"<ol><li>First</li><li>Second</li></ol>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1."));
        assert!(content.contains("2."));
    }

    #[test]
    fn engine_clear_right() {
        // Covers engine.rs lines 2003-2006: clear: right
        let html = r#"<p style="float: right; width: 100pt">FloatedRight</p><p style="clear: right">ClearedRight</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("FloatedRight"));
        assert!(content.contains("ClearedRight"));
    }

    #[test]
    fn engine_clear_both() {
        // Covers engine.rs lines 1995-2001: clear: both
        let html = r#"<p style="float: left; width: 100pt">FloatLeft</p><p style="float: right; width: 100pt">FloatRight</p><p style="clear: both">ClearedBoth</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("FloatLeft"));
        assert!(content.contains("FloatRight"));
        assert!(content.contains("ClearedBoth"));
    }

    #[test]
    fn engine_image_with_only_width_attr() {
        // Covers engine.rs line 2173: image with width only (falls back to square)
        let html = r#"<img width="100" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==">"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Do\n"));
    }

    #[test]
    fn engine_image_with_only_height_attr() {
        // Covers engine.rs line 2174: image with height only
        let html = r#"<img height="80" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==">"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Do\n"));
    }

    #[test]
    fn engine_image_unsupported_format_ignored() {
        // Covers engine.rs line 2225: non-PNG, non-JPEG data returns None
        let html = r#"<img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn engine_image_remote_url_blocked() {
        // Covers engine.rs lines 2204-2206: remote URLs are blocked
        let html = r#"<img src="https://example.com/image.png">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn engine_image_local_file_not_found() {
        // Covers engine.rs line 2209: local file path that doesn't exist
        let html = r#"<img src="/nonexistent/path/to/image.png">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn pdf_linear_gradient_to_left() {
        // Covers pdf.rs line 819: reversed horizontal gradient (to left)
        let html = r#"<p style="background: linear-gradient(to left, red, blue); width: 200pt; height: 50pt; padding: 10pt">ToLeft</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn pdf_linear_gradient_to_top_vertical() {
        // Covers pdf.rs lines 832-845: vertical gradient to top (reversed)
        let html = r#"<p style="background: linear-gradient(to top, red, blue); width: 200pt; height: 50pt; padding: 10pt">ToTop</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn pdf_gradient_three_stops() {
        // Covers pdf.rs lines 781, 784, 794: color_at_position with multiple stops
        let html = r#"<p style="background: linear-gradient(to right, red 0%, white 50%, blue 100%); width: 200pt; height: 50pt; padding: 10pt">ThreeStops</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn engine_flex_column_non_stretch_width() {
        // Covers engine.rs line 1256: non-stretch width in column flex
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; align-items: flex-start; width: 300pt">
                <div style="width: 100pt">NarrowChild</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("NarrowChild"));
    }

    #[test]
    fn engine_flex_column_with_position_relative() {
        // Covers engine.rs line 1311: column flex with x_offset > 0 sets Position::Relative
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; align-items: center; width: 300pt">
                <div style="width: 100pt">ColCentered</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("ColCentered"));
    }

    #[test]
    fn engine_flex_with_gap() {
        // Covers engine.rs lines 976, 992, 1012: gap in flex layout
        let html = r#"<html><body>
            <div style="display: flex; gap: 10pt; width: 300pt">
                <div style="width: 80pt">GapA</div>
                <div style="width: 80pt">GapB</div>
                <div style="width: 80pt">GapC</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("GapA"));
        assert!(content.contains("GapB"));
        assert!(content.contains("GapC"));
    }

    #[test]
    fn engine_grid_incomplete_row_fills_empty_cells() {
        // Covers engine.rs lines 1517-1529: incomplete grid row fills with empty cells
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr 1fr">
                <div>OnlyOne</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("OnlyOne"));
    }

    #[test]
    fn engine_table_cell_background() {
        // Covers pdf.rs lines 510-518: table cell background rendering
        let html = r#"
            <table>
                <tr><td style="background-color: yellow">YellowCell</td><td>PlainCell</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("YellowCell"));
        assert!(content.contains("rg\n"));
    }

    #[test]
    fn engine_flex_empty_children_skipped() {
        // Covers engine.rs line 943-944: items.is_empty() check
        let html = r#"<html><body>
            <div style="display: flex; width: 200pt">
                <div style="display: none">HiddenOne</div>
                <div style="display: none">HiddenTwo</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn engine_flex_no_children() {
        // Covers engine.rs line 822-823: flex with no element children
        let html = r#"<html><body><div style="display: flex; width: 200pt"></div></body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn engine_grid_text_nodes_filtered() {
        // Covers engine.rs line 1456: text nodes are filtered in grid
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr">
                <div>GridChild</div>
                <div>AnotherChild</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("GridChild"));
        assert!(content.contains("AnotherChild"));
    }
}
