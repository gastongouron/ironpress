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
    let content = String::from_utf8_lossy(pdf);
    if content.contains(text) {
        return true;
    }
    // CID path: each font has its own ToUnicode CMap. Parse each separately
    // to avoid glyph ID collisions between different fonts.
    let cmap_str: &str = content.as_ref();
    let mut cmaps: Vec<std::collections::HashMap<String, char>> = Vec::new();
    let mut pos = 0;
    while let Some(start) = cmap_str[pos..].find("beginbfchar") {
        let block_start = pos + start + 11;
        let block_end = cmap_str[block_start..]
            .find("endbfchar")
            .map(|e| block_start + e)
            .unwrap_or(cmap_str.len());
        let mut map = std::collections::HashMap::new();
        for line in cmap_str[block_start..block_end].lines() {
            let parts: Vec<&str> = line
                .trim()
                .split(|c: char| c == '<' || c == '>' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .collect();
            if parts.len() >= 2 {
                if let Ok(cp) = u32::from_str_radix(parts[1], 16) {
                    if let Some(ch) = char::from_u32(cp) {
                        map.insert(parts[0].to_uppercase(), ch);
                    }
                }
            }
        }
        if !map.is_empty() {
            cmaps.push(map);
        }
        pos = block_end;
    }
    if cmaps.is_empty() {
        return false;
    }
    let mut decoded = String::new();
    let mut search_pos = 0;
    while let Some(tj_end) = cmap_str[search_pos..].find("] TJ") {
        let tj_end_abs = search_pos + tj_end;
        if let Some(tj_start) = cmap_str[..tj_end_abs].rfind('[') {
            let arr = &cmap_str[tj_start + 1..tj_end_abs];
            let hexes: Vec<String> = {
                let mut v = Vec::new();
                let mut ap = 0;
                while let Some(o) = arr[ap..].find('<') {
                    let oa = ap + o;
                    if let Some(c) = arr[oa..].find('>') {
                        v.push(arr[oa + 1..oa + c].trim().to_uppercase());
                        ap = oa + c + 1;
                    } else {
                        break;
                    }
                }
                v
            };
            for cmap in &cmaps {
                let d: String = hexes
                    .iter()
                    .filter_map(|h| cmap.get(h.as_str()).copied())
                    .collect();
                if !d.is_empty() {
                    decoded.push_str(&d);
                }
            }
        }
        decoded.push(' ');
        search_pos = tj_end_abs + 4;
    }
    decoded.contains(text)
}

fn smoke_html_to_pdf(html: &str) -> Vec<u8> {
    ironpress::HtmlConverter::new()
        .compress(false)
        .convert(html)
        .unwrap()
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
    let pdf = smoke_html_to_pdf("<h1>Hello</h1><p>World</p>");
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
    let pdf = smoke_html_to_pdf(html);
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
    let pdf = smoke_html_to_pdf(html);
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
    let pdf = smoke_html_to_pdf(html);
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
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Item A"));
}

// === Images (data URI) ===

#[test]
fn smoke_image_png() {
    let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="50" height="50">"#;
    let pdf = smoke_html_to_pdf(html);
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
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Styled box"));
}

#[test]
fn smoke_flexbox() {
    let html = r#"
        <style>.flex { display: flex; gap: 10pt; }</style>
        <div class="flex"><div>A</div><div>B</div><div>C</div></div>
    "#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_grid() {
    let html = r#"
        <style>.grid { display: grid; grid-template-columns: repeat(3, 1fr); grid-gap: 5pt; }</style>
        <div class="grid"><div>1</div><div>2</div><div>3</div></div>
    "#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_grid_minmax() {
    let html = r#"
        <style>.grid { display: grid; grid-template-columns: minmax(100px, 1fr) 2fr; }</style>
        <div class="grid"><div>Left</div><div>Right</div></div>
    "#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_multi_column() {
    let html = r#"
        <style>.cols { column-count: 3; column-gap: 10pt; }</style>
        <div class="cols"><div>A</div><div>B</div><div>C</div></div>
    "#;
    let pdf = smoke_html_to_pdf(html);
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
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "John Doe"));
}

#[test]
fn smoke_media_elements() {
    let html = r#"
        <video width="320" height="240"></video>
        <audio></audio>
    "#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
}

#[test]
fn smoke_progress_meter() {
    let html = r#"
        <progress value="70" max="100"></progress>
        <meter value="0.6" max="1" low="0.25" high="0.75"></meter>
    "#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
}

// === v1.3: Page features ===

#[test]
fn smoke_page_break() {
    let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_page_count(&pdf) >= 2);
}

#[test]
fn smoke_header_footer() {
    let pdf = ironpress::HtmlConverter::new()
        .compress(false)
        .header("My Report")
        .footer("Page {page} of {pages}")
        .convert("<h1>Title</h1><p>Content</p>")
        .unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "My Report"));
    assert!(pdf_has_text(&pdf, "Page 1 of 1"));
}

#[test]
fn body_page_name_selects_named_page_margin_boxes() {
    let html = r#"
        <style>
            @page {
                size: 150mm 70mm;
                margin: 14mm;
                @bottom-center { content: "DEFAULT page rule"; }
            }
            @page named {
                @bottom-center { content: "NAMED page rule"; }
            }
            html, body { margin: 0; }
            body { page: named; }
        </style>
        <p>Document body</p>
    "#;

    let pdf = smoke_html_to_pdf(html);

    assert!(pdf_has_text(&pdf, "NAMED page rule"));
    assert!(!pdf_has_text(&pdf, "DEFAULT page rule"));
}

#[test]
fn smoke_custom_page_size() {
    let pdf = ironpress::HtmlConverter::new()
        .compress(false)
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
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
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
        .compress(false)
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

// === SVG background image ===

#[test]
fn smoke_svg_background_image_percent_encoded() {
    let html = r#"<html><head><style>
body { background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='100' height='100'%3E%3Crect width='100' height='100' fill='%23eee'/%3E%3Ccircle cx='50' cy='50' r='30' fill='%23ccc'/%3E%3C/svg%3E"); background-size: cover; }
</style></head><body>
<h1>Background Test</h1>
<p>This page should have an SVG pattern background.</p>
</body></html>"#;
    let pdf = ironpress::HtmlConverter::new()
        .compress(false)
        .sanitize(false)
        .convert(html)
        .unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Background Test"));
    // The SVG should generate PDF drawing operators (rect and circle beziers)
    let content = String::from_utf8_lossy(&pdf);
    // "re" (rectangle) from the SVG rect element
    assert!(content.contains(" re\n"));
}

#[test]
fn smoke_svg_background_image_base64() {
    // SVG: <svg xmlns='http://www.w3.org/2000/svg' width='50' height='50'><rect width='50' height='50' fill='blue'/></svg>
    let html = r#"<html><head><style>
body { background-image: url("data:image/svg+xml;base64,PHN2ZyB4bWxucz0naHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmcnIHdpZHRoPSc1MCcgaGVpZ2h0PSc1MCc+PHJlY3Qgd2lkdGg9JzUwJyBoZWlnaHQ9JzUwJyBmaWxsPSdibHVlJy8+PC9zdmc+"); background-size: cover; }
</style></head><body>
<p>Base64 SVG background</p>
</body></html>"#;
    let pdf = ironpress::HtmlConverter::new()
        .compress(false)
        .sanitize(false)
        .convert(html)
        .unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Base64 SVG background"));
}

#[test]
fn smoke_svg_background_with_sanitizer() {
    // Verify that the sanitizer preserves data: URIs in CSS
    let html = r#"<html><head><style>
body { background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10'%3E%3Crect width='10' height='10' fill='%23ddd'/%3E%3C/svg%3E"); }
</style></head><body>
<p>Sanitized SVG background</p>
</body></html>"#;
    // With sanitizer enabled (default)
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "Sanitized SVG background"));
}

// === CSS filter: blur() ===

#[test]
fn smoke_filter_blur_png_image() {
    let html = r#"
        <style>
        .blurred { filter: blur(5px); }
        </style>
        <img class="blurred" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9AAAAFklEQVQYV2P8z8BQz0BFwMgwasCoAgBGWAkF3c01mQAAAABJRU5ErkJggg==" width="200" height="200" />
        <p>Text below blurred image</p>
    "#;
    let pdf = ironpress::HtmlConverter::new()
        .compress(false)
        .convert(html)
        .unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "/SMask"));
    assert!(pdf_has_text(&pdf, "Text below blurred image"));
}

#[test]
fn smoke_filter_blur_zero_radius_no_blur() {
    let html = r#"
        <style>
        .no-blur { filter: blur(0px); }
        </style>
        <img class="no-blur" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9AAAAFklEQVQYV2P8z8BQz0BFwMgwasCoAgBGWAkF3c01mQAAAABJRU5ErkJggg==" width="100" height="100" />
    "#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "/Filter /FlateDecode"));
}

#[test]
fn smoke_filter_blur_none_keyword() {
    let html = r#"
        <style>
        .clear { filter: none; }
        </style>
        <img class="clear" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9AAAAFklEQVQYV2P8z8BQz0BFwMgwasCoAgBGWAkF3c01mQAAAABJRU5ErkJggg==" width="100" height="100" />
    "#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "/Filter /FlateDecode"));
}

#[test]
fn smoke_filter_blur_inline_style() {
    let html = r#"
        <img style="filter: blur(10px)" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAoAAAAKCAYAAACNMs+9AAAAFklEQVQYV2P8z8BQz0BFwMgwasCoAgBGWAkF3c01mQAAAABJRU5ErkJggg==" width="150" height="150" />
        <p>After inline blur</p>
    "#;
    let pdf = ironpress::HtmlConverter::new()
        .compress(false)
        .convert(html)
        .unwrap();
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "/SMask"));
    assert!(pdf_has_text(&pdf, "After inline blur"));
}

#[test]
fn smoke_filter_blur_text_element_no_crash() {
    let html = r#"
        <style>
        .blurred-text { filter: blur(3px); }
        </style>
        <p class="blurred-text">This text has a blur filter applied</p>
        <p>Normal text</p>
    "#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    assert!(pdf_has_text(&pdf, "/SMask"));
    assert!(pdf_has_text(&pdf, "Normal text"));
}

// === Block-level boxes inside inline boxes (CSS 2.1 § 9.2.1.1) ===

#[test]
fn smoke_block_inside_inline_is_laid_out() {
    let html =
        r#"<div>Lead <span>Inline start<div>Nested block</div>Inline end</span></div><p>After</p>"#;
    let pdf = smoke_html_to_pdf(html);
    assert!(pdf_is_valid(&pdf));
    for text in [
        "Lead",
        "Inline start",
        "Nested block",
        "Inline end",
        "After",
    ] {
        assert!(pdf_has_text(&pdf, text), "missing {text:?}");
    }
}

#[test]
fn smoke_unclosed_inline_keeps_following_blocks() {
    // The parser keeps an unclosed <span> open, so every later block nests
    // inside it.
    let html =
        r#"<center>Lead <span>Open span<div>First block</div><p>Second block</p><h2>Heading</h2>"#;
    let pdf = smoke_html_to_pdf(html);
    for text in ["Open span", "First block", "Second block", "Heading"] {
        assert!(pdf_has_text(&pdf, text), "missing {text:?}");
    }
}

#[test]
fn smoke_css_block_inside_inline_is_laid_out() {
    let html = r#"<style>em.block { display: block }</style>
        <div>Lead <b>Bold<em class="block">Styled block</em>Bold end</b></div>"#;
    let pdf = smoke_html_to_pdf(html);
    for text in ["Styled block", "Bold end"] {
        assert!(pdf_has_text(&pdf, text), "missing {text:?}");
    }
}

#[test]
fn smoke_block_inside_inline_in_table_cell_and_flex_item() {
    let html = r#"
        <table><tr><td>Cell <span>Cell inline<div>Cell block</div></span></td></tr></table>
        <div style="display:flex"><div>Item <span>Item inline<div>Item block</div></span></div></div>"#;
    let pdf = smoke_html_to_pdf(html);
    for text in ["Cell inline", "Cell block", "Item inline", "Item block"] {
        assert!(pdf_has_text(&pdf, text), "missing {text:?}");
    }
}

// === Deeply nested documents ===

#[test]
fn smoke_deeply_nested_content_is_laid_out() {
    // 200 levels: deeper than the default stack would allow without growth,
    // within the sanitizer's nesting limit.
    let depth = 200;
    let html = format!(
        "{}<p>Deepest text</p>{}<p>After nesting</p>",
        "<div style=\"padding-left:1px\">".repeat(depth),
        "</div>".repeat(depth)
    );
    let pdf = smoke_html_to_pdf(&html);
    assert!(pdf_has_text(&pdf, "Deepest text"));
    assert!(pdf_has_text(&pdf, "After nesting"));
}

#[test]
fn smoke_unbreakable_table_row_taller_than_page_terminates() {
    // The single line is taller than a page, so the row cannot be broken:
    // it must overflow one page instead of being split into empty pages
    // forever. A worker thread turns a regression into a failure, not a hang.
    let html = "<table><tr><td><div style=\"line-height:780pt\">Tall line</div></td></tr></table>\
                <p>After table</p>";
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(smoke_html_to_pdf(html));
    });
    let pdf = receiver
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("pagination did not terminate");
    assert!(pdf_has_text(&pdf, "Tall line"));
    assert!(pdf_has_text(&pdf, "After table"));
}

#[test]
fn smoke_split_that_moves_a_box_to_the_next_page_is_kept_below_the_page_top() {
    // Only the empty first child fits below the spacer, so the split leaves a
    // continuation as tall as the box. Lower on a page that is progress: the
    // box continues on the next pages instead of overflowing this one.
    let lines: String = (0..30).map(|i| format!("Line {i}<br>")).collect();
    let html = format!(
        "<style>@page {{ size: 200pt 150pt; margin: 0 }} * {{ margin: 0 }}</style>\
         <div style=\"height:140pt\"></div>\
         <div style=\"background:red\"><div style=\"height:0\"></div><p>{lines}</p></div>"
    );
    let pdf = smoke_html_to_pdf(&html);
    assert!(pdf_page_count(&pdf) >= 4, "{} pages", pdf_page_count(&pdf));
    assert!(pdf_has_text(&pdf, "Line 29"));
}
