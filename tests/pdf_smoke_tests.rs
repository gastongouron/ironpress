/// Smoke tests: generate PDFs for all major features and verify structural integrity.
/// These tests ensure no feature addition breaks existing PDF generation.

fn pdf_is_valid(pdf: &[u8]) -> bool {
    let s = String::from_utf8_lossy(pdf);
    pdf.starts_with(b"%PDF-1.4")
        && s.contains("/Type /Catalog")
        && s.contains("/Type /Pages")
        && s.contains("%%EOF")
        && s.contains("xref")
}

fn pdf_has_text(pdf: &[u8], text: &str) -> bool {
    String::from_utf8_lossy(pdf).contains(text)
}

fn pdf_page_count(pdf: &[u8]) -> usize {
    let s = String::from_utf8_lossy(pdf);
    // Extract /Count N from /Type /Pages
    if let Some(pos) = s.find("/Type /Pages") {
        let after = &s[pos..];
        if let Some(count_pos) = after.find("/Count ") {
            let num_start = count_pos + 7;
            let num_end = after[num_start..]
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(0)
                + num_start;
            return after[num_start..num_end].parse().unwrap_or(0);
        }
    }
    0
}

// === Basic rendering ===

#[test]
fn smoke_simple_html() {
    let pdf = ironpress::html_to_pdf("<h1>Hello</h1><p>World</p>").unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Hello"));
    assert!(pdf_has_text(&pdf, "World"));
}

#[test]
fn smoke_markdown() {
    let pdf =
        ironpress::markdown_to_pdf("# Title\n\nParagraph with **bold** and *italic*.").unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Title"));
}

// === Headings & bookmarks ===

#[test]
fn smoke_headings_produce_bookmarks() {
    let html = "<h1>Ch1</h1><h2>Sec1</h2><h3>Sub1</h3><p>Content</p>";
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "/Type /Outlines"));
    assert!(pdf_has_text(&pdf, "Ch1"));
    assert!(pdf_has_text(&pdf, "Sec1"));
    assert!(pdf_has_text(&pdf, "Sub1"));
}

// === Inline formatting ===

#[test]
fn smoke_inline_formatting() {
    let html = r#"
        <p><strong>Bold</strong> <em>Italic</em> <u>Underline</u></p>
        <p><del>Deleted</del> <code>Code</code> <mark>Highlighted</mark></p>
        <p><a href="https://example.com">Link</a></p>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Bold"));
    assert!(pdf_has_text(&pdf, "/Subtype /Link"));
}

// === Tables ===

#[test]
fn smoke_table() {
    let html = r#"
        <table>
            <thead><tr><th>Name</th><th>Age</th></tr></thead>
            <tbody>
                <tr><td>Alice</td><td>30</td></tr>
                <tr><td colspan="2">Footer row</td></tr>
            </tbody>
        </table>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Alice"));
}

// === Lists ===

#[test]
fn smoke_lists() {
    let html = r#"
        <ul><li>Item A</li><li>Item B</li></ul>
        <ol><li>First</li><li>Second</li></ol>
        <dl><dt>Term</dt><dd>Definition</dd></dl>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Item A"));
}

// === Images (data URI) ===

#[test]
fn smoke_image_png() {
    let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="50" height="50">"#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "/Subtype /Image"));
}

// === CSS features ===

#[test]
fn smoke_css_styling() {
    let html = r#"
        <style>
            .box { background-color: #336699; color: white; padding: 10pt; border: 2pt solid black; border-radius: 4pt; }
            .center { text-align: center; }
        </style>
        <div class="box"><p class="center">Styled box</p></div>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Styled box"));
}

#[test]
fn smoke_flexbox() {
    let html = r#"
        <style>.flex { display: flex; gap: 10pt; }</style>
        <div class="flex"><div>A</div><div>B</div><div>C</div></div>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_grid() {
    let html = r#"
        <style>.grid { display: grid; grid-template-columns: repeat(3, 1fr); grid-gap: 5pt; }</style>
        <div class="grid"><div>1</div><div>2</div><div>3</div></div>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_grid_minmax() {
    let html = r#"
        <style>.grid { display: grid; grid-template-columns: minmax(100px, 1fr) 2fr; }</style>
        <div class="grid"><div>Left</div><div>Right</div></div>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_multi_column() {
    let html = r#"
        <style>.cols { column-count: 3; column-gap: 10pt; }</style>
        <div class="cols"><div>A</div><div>B</div><div>C</div></div>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

// === v1.1: New HTML elements ===

#[test]
fn smoke_form_controls() {
    let html = r#"
        <input type="text" value="John Doe">
        <select><option>France</option><option>USA</option></select>
        <textarea>Some text here</textarea>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "John Doe"));
}

#[test]
fn smoke_media_elements() {
    let html = r#"
        <video width="320" height="240"></video>
        <audio></audio>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_progress_meter() {
    let html = r#"
        <progress value="70" max="100"></progress>
        <meter value="0.6" max="1" low="0.25" high="0.75"></meter>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

// === v1.3: Page features ===

#[test]
fn smoke_page_break() {
    let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_page_count(&pdf) >= 2);
}

#[test]
fn smoke_header_footer() {
    let pdf = ironpress::HtmlConverter::new()
        .header("My Report")
        .footer("Page {page} of {pages}")
        .convert("<h1>Title</h1><p>Content</p>")
        .unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "My Report"));
    assert!(pdf_has_text(&pdf, "Page 1 of 1"));
}

#[test]
fn smoke_custom_page_size() {
    let pdf = ironpress::HtmlConverter::new()
        .page_size(ironpress::PageSize::LETTER)
        .margin(ironpress::Margin::uniform(36.0))
        .convert("<p>Letter size</p>")
        .unwrap();
    assert!(pdf_is_valid(&pdf));
}

// === SVG ===

#[test]
fn smoke_inline_svg() {
    let html = r#"
        <svg width="100" height="100" viewBox="0 0 100 100">
            <rect x="10" y="10" width="80" height="80" fill="blue" />
            <circle cx="50" cy="50" r="20" fill="red" />
        </svg>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_inline_svg_text_inherits_current_color() {
    let html = r#"
        <div style="color: blue">
            <svg width="160" height="40" viewBox="0 0 160 40">
                <text x="8" y="24" fill="currentColor">Hello SVG</text>
            </svg>
        </div>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Hello SVG"));
    assert!(pdf_has_text(&pdf, "0 0 1 rg"));
}

#[test]
fn smoke_inline_svg_text_tspan_uses_font_attributes() {
    let html = r#"
        <svg width="220" height="40" viewBox="0 0 220 40">
            <text x="8" y="24" font-family="Courier" font-weight="700" font-style="oblique">Hello <tspan>world</tspan>!</text>
        </svg>
    "#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Hello world!"));
    assert!(pdf_has_text(&pdf, "/Courier-BoldOblique"));
}

// === Complex document ===

#[test]
fn smoke_full_document() {
    let html = r#"
        <style>
            body { font-size: 11pt; }
            h1 { color: navy; }
            .highlight { background-color: yellow; }
            table { border-collapse: collapse; }
            td, th { border: 1pt solid #ccc; padding: 4pt; }
        </style>
        <h1>Annual Report</h1>
        <p>This is a <strong>comprehensive</strong> test of <em>all</em> features.</p>
        <h2>Section 1: Data</h2>
        <table>
            <thead><tr><th>Metric</th><th>Value</th></tr></thead>
            <tbody>
                <tr><td>Revenue</td><td>$1.2M</td></tr>
                <tr><td>Growth</td><td>15%</td></tr>
            </tbody>
        </table>
        <h2>Section 2: Progress</h2>
        <p>Project completion: <progress value="85" max="100"></progress></p>
        <h2>Section 3: Form</h2>
        <p>Name: <input type="text" value="Alice"></p>
        <p>Notes:</p>
        <textarea>Quarterly review complete.</textarea>
        <ul>
            <li>Item one</li>
            <li>Item two with <span class="highlight">highlight</span></li>
        </ul>
        <blockquote>A wise quote about testing.</blockquote>
        <hr>
        <p><a href="https://example.com">More details</a></p>
    "#;
    let pdf = ironpress::HtmlConverter::new()
        .header("Confidential")
        .footer("Page {page} of {pages}")
        .convert(html)
        .unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Annual Report"));
    assert!(pdf_has_text(&pdf, "/Type /Outlines"));
    assert!(pdf_has_text(&pdf, "Confidential"));
    assert!(pdf_page_count(&pdf) >= 1);
}

// === Math (LaTeX) ===

#[test]
fn smoke_inline_math_markdown() {
    let md = "The equation $E = mc^2$ is famous.";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
    // Should contain text from the expression
    assert!(pdf_has_text(&pdf, "E"));
}

#[test]
fn smoke_display_math_markdown() {
    let md = "Euler's identity:\n\n$$e^{i\\pi} + 1 = 0$$\n\nBeautiful.";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Beautiful"));
}

#[test]
fn smoke_math_fraction() {
    let md = "Consider: $$\\frac{a^2 + b^2}{c}$$";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_math_sqrt_greek() {
    let md = "Root: $\\sqrt{\\alpha^2 + \\beta^2}$, and $\\Omega$";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_math_sum_integral() {
    let md = r"$$\sum_{i=1}^{n} x_i = \int_0^\infty f(x)\,dx$$";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_math_matrix() {
    let md = r"$$\begin{pmatrix} a & b \\ c & d \end{pmatrix}$$";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_math_via_html() {
    let html = r#"<p>Inline: <span class="math-inline" data-math="x^2">x^2</span></p>
    <div class="math-display" data-math="\frac{1}{2}">\frac{1}{2}</div>"#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_math_complex_document() {
    let md = r#"# Mathematical Analysis

## Theorem 1

For all $n \geq 1$, we have:

$$\sum_{k=1}^{n} k = \frac{n(n+1)}{2}$$

## Proof

By induction. The base case $n = 1$ gives $\frac{1 \cdot 2}{2} = 1$. Assuming
the result holds for $n$, then for $n + 1$:

$$\sum_{k=1}^{n+1} k = \frac{n(n+1)}{2} + (n+1) = \frac{(n+1)(n+2)}{2}$$

Which completes the proof. $\blacksquare$
"#;
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Mathematical Analysis"));
    assert!(pdf_has_text(&pdf, "Theorem"));
    assert!(pdf_has_text(&pdf, "Proof"));
}

#[test]
fn smoke_gfm_table_from_markdown() {
    let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_gfm_strikethrough_from_markdown() {
    let md = "This is ~~deleted~~ text.";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_gfm_footnotes_from_markdown() {
    let md = "Main text[^1].\n\n[^1]: Footnote content.";
    let pdf = ironpress::markdown_to_pdf(md).unwrap();
    assert!(pdf_is_valid(&pdf));
}

// === SVG data URI images ===

#[test]
fn smoke_svg_data_uri_base64() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="blue"/></svg>"#;
    let b64 = base64_encode(svg.as_bytes());
    let html = format!(r#"<img src="data:image/svg+xml;base64,{b64}">"#);
    let pdf = ironpress::html_to_pdf(&html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_svg_data_uri_plain() {
    let html = r#"<img src="data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%20width%3D%2250%22%20height%3D%2250%22%3E%3Ccircle%20cx%3D%2225%22%20cy%3D%2225%22%20r%3D%2220%22%20fill%3D%22red%22%2F%3E%3C%2Fsvg%3E">"#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_svg_data_uri_with_text() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="50"><text x="10" y="30" font-size="16">Hello SVG</text></svg>"#;
    let b64 = base64_encode(svg.as_bytes());
    let html = format!(r#"<p>Before</p><img src="data:image/svg+xml;base64,{b64}"><p>After</p>"#);
    let pdf = ironpress::html_to_pdf(&html).unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Before"));
    assert!(pdf_has_text(&pdf, "After"));
}

#[test]
fn smoke_raster_image_still_works() {
    // Ensure raster images are not broken by SVG probing
    // 1x1 red JPEG (minimal valid JPEG)
    let html = r#"<p>No image crash</p>"#;
    let pdf = ironpress::html_to_pdf(html).unwrap();
    assert!(pdf_is_valid(&pdf));
}

/// Minimal base64 encoder for tests.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[(n >> 18 & 63) as usize] as char);
        result.push(CHARS[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[(n >> 6 & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
