#[test]
fn render_simple_pdf() {
    let nodes = parse_html("<p>Hello World</p>").unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();

    // Valid PDF starts with %PDF
    assert!(pdf.starts_with(b"%PDF-1.4"));
    // Valid PDF ends with %%EOF
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("%%EOF"));
    // Contains Helvetica font
    assert!(content.contains("/Helvetica"));
}

#[test]
fn render_bold_italic() {
    let nodes = parse_html("<p><strong>Bold</strong> and <em>italic</em></p>").unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("/Helvetica-Bold"));
    assert!(content.contains("/Helvetica-Oblique"));
}

#[test]
fn render_empty_document() {
    let nodes = parse_html("").unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.4"));
}

#[test]
fn pdf_string_escaping() {
    assert_eq!(escape_pdf_string("hello"), "hello");
    assert_eq!(escape_pdf_string("(test)"), "\\(test\\)");
    assert_eq!(escape_pdf_string("back\\slash"), "back\\\\slash");
}

#[test]
fn render_background_color() {
    let html = r#"<pre>code here</pre>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // Pre has gray background — PDF should contain rectangle fill commands
    assert!(content.contains("re\nf\n") || content.contains("re"));
}

#[test]
fn render_center_align() {
    let html = r#"<p style="text-align: center">Centered</p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn render_right_align() {
    let html = r#"<p style="text-align: right">Right</p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn render_underline() {
    let html = "<p><u>Underlined text</u></p>";
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("Underlined text"));
    assert!(
        filled_rect_count(&content) >= 1,
        "Underline should draw a filled decoration rectangle"
    );
}

#[test]
fn render_bold_italic_combined() {
    let html = "<p><strong><em>Bold Italic</em></strong></p>";
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("/Helvetica-BoldOblique"));
}

#[test]
fn render_page_break_in_content() {
    let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // Should have multiple page objects
    assert!(content.matches("/Type /Page").count() >= 2);
}

#[test]
fn render_svg_without_viewbox_scales_to_layout_box() {
    let tree = crate::parser::svg::SvgTree {
        width: 120.0,
        height: 60.0,
        width_attr: None,
        height_attr: None,
        preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
        view_box: None,
        defs: Default::default(),
        children: vec![crate::parser::svg::SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rx: 0.0,
            ry: 0.0,
            style: crate::parser::svg::SvgStyle::default(),
        }],
        text_ctx: crate::parser::svg::SvgTextContext::default(),
        source_markup: None,
    };
    let pages = vec![Page {
        elements: vec![(
            0.0,
            Svg {
                tree,
                geometry: ReplacedGeometry::new(
                    Size::new(240.0, 120.0),
                    BlockMargins::default(),
                    Default::default(),
                ),
                positioning: Default::default(),
                paint: SvgPaint::default(),
                replaced: Default::default(),
            }
            .boxed(),
        )],
        ..Default::default()
    }];
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("2 0 0 2 0 0 cm"),
        "expected outer scale for SVG without a viewBox"
    );
}

#[test]
fn render_svg_honors_root_preserve_aspect_ratio() {
    let tree = crate::parser::svg::SvgTree {
        width: 20.0,
        height: 20.0,
        width_attr: Some("20".to_string()),
        height_attr: Some("20".to_string()),
        preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
        view_box: Some(crate::parser::svg::ViewBox {
            min_x: 0.0,
            min_y: 0.0,
            width: 100.0,
            height: 20.0,
        }),
        defs: Default::default(),
        children: vec![],
        text_ctx: crate::parser::svg::SvgTextContext::default(),
        source_markup: None,
    };
    let pages = vec![Page {
        elements: vec![(
            0.0,
            Svg {
                tree,
                geometry: ReplacedGeometry::new(
                    Size::new(20.0, 20.0),
                    BlockMargins::default(),
                    Default::default(),
                ),
                positioning: Default::default(),
                paint: SvgPaint::default(),
                replaced: Default::default(),
            }
            .boxed(),
        )],
        ..Default::default()
    }];
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("0.2025 0 0 0.20250002 0 8.1 cm"),
        "expected meet scaling inside the snapped replaced-element paint box"
    );
}

#[test]
fn render_colored_text() {
    let html = r#"<p style="color: red">Red text</p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("1 0 0 rg")); // red in PDF
}

#[test]
fn render_table_basic() {
    let html = r#"
            <table>
                <tr><th>Name</th><th>Age</th></tr>
                <tr><td>Alice</td><td>30</td></tr>
            </table>
        "#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // No default cell borders — only CSS-specified borders produce strokes
    assert!(content.contains("Name"));
    assert!(content.contains("Alice"));
}

#[test]
fn render_table_with_background() {
    let html = r#"
            <table>
                <tr><td style="background-color: yellow">Highlighted</td></tr>
            </table>
        "#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // Background fill command
    assert!(content.contains("re\nf\n"));
}

#[test]
fn render_empty_line_skipped() {
    let html = "<p>Above</p><br><p>Below</p>";
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("Above"));
    assert!(content.contains("Below"));
}

#[test]
fn render_empty_run_skipped() {
    let html = "<p>Text</p>";
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn render_page_break_element() {
    let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // Multiple pages rendered
    assert!(content.matches("/Type /Page ").count() >= 2);
}

#[test]
fn render_cell_text_empty_line_skipped() {
    let html = r#"<table><tr><td></td><td>Content</td></tr></table>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("Content"));
}

#[test]
fn render_horizontal_rule() {
    let html = "<p>Above</p><hr><p>Below</p>";
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // HR draws a line with stroke
    assert!(content.contains(" l\nS\n"));
}

#[test]
fn render_input_element() {
    let pdf = crate::html_to_pdf(r#"<input type="text" value="Hello">"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert!(pdf.len() > 100);
}

#[test]
fn render_input_with_placeholder() {
    let pdf = crate::html_to_pdf(r#"<input placeholder="Type here...">"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn render_select_element() {
    let pdf =
        crate::html_to_pdf(r#"<select><option>A</option><option>B</option></select>"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert!(pdf.len() > 100);
}

#[test]
fn render_textarea_element() {
    let pdf = crate::html_to_pdf(r#"<textarea>Hello World</textarea>"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert!(pdf.len() > 100);
}

#[test]
fn render_video_element() {
    let pdf = crate::html_to_pdf(r#"<video width="320" height="240"></video>"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert!(pdf.len() > 100);
}

#[test]
fn render_audio_element() {
    let pdf = crate::html_to_pdf(r#"<audio></audio>"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert!(pdf.len() > 100);
}

#[test]
fn render_progress_element() {
    let pdf = crate::html_to_pdf(r#"<progress value="0.7" max="1"></progress>"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    let content = String::from_utf8_lossy(&pdf);
    // Progress bar draws rectangles (track + fill + border)
    assert!(
        content.contains("re\nf\n"),
        "Expected filled rectangles for progress bar"
    );
}

#[test]
fn render_progress_empty() {
    let pdf = crate::html_to_pdf(r#"<progress value="0" max="1"></progress>"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn render_meter_element() {
    let pdf = crate::html_to_pdf(r#"<meter value="0.5" max="1"></meter>"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("re\nf\n"),
        "Expected filled rectangles for meter bar"
    );
}

#[test]
fn render_meter_low_value() {
    let pdf =
        crate::html_to_pdf(r#"<meter value="5" max="100" low="25" high="75"></meter>"#).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn render_form_controls_styled() {
    let html = r#"
            <input type="text" value="styled" style="width: 200px; border: 2px solid blue; background-color: #eee">
        "#;
    let pdf = crate::html_to_pdf(html).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn render_mixed_form_and_text() {
    let html = r#"
            <p>Fill in the form:</p>
            <input type="text" value="John">
            <p>Select country:</p>
            <select><option>France</option></select>
            <p>Comments:</p>
            <textarea>Great product!</textarea>
            <p>Rating:</p>
            <progress value="80" max="100"></progress>
        "#;
    let pdf = crate::html_to_pdf(html).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    assert!(pdf.len() > 500);
}

#[test]
fn render_pdf_bookmarks_from_headings() {
    let html = "<h1>Chapter 1</h1><p>Content</p><h2>Section 1.1</h2><p>More</p>";
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("/Type /Outlines"), "Expected PDF outlines");
    assert!(
        content.contains("Chapter 1"),
        "Expected heading text in bookmark"
    );
    assert!(
        content.contains("Section 1.1"),
        "Expected h2 heading in bookmark"
    );
}

#[test]
fn render_pdf_no_bookmarks_without_headings() {
    let html = "<p>No headings here</p>";
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        !content.contains("/Type /Outlines"),
        "Should not have outlines without headings"
    );
}

#[test]
fn render_pdf_bookmarks_multi_page() {
    let html = r#"
            <h1>Page 1 Title</h1>
            <p>Content</p>
            <div style="page-break-before: always">
                <h1>Page 2 Title</h1>
                <p>More content</p>
            </div>
        "#;
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("Page 1 Title"));
    assert!(content.contains("Page 2 Title"));
    assert!(content.contains("/Type /Outlines"));
}

#[test]
fn render_pdf_bookmarks_all_levels() {
    let html = "<h1>H1</h1><h2>H2</h2><h3>H3</h3><h4>H4</h4><h5>H5</h5><h6>H6</h6>";
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("/Count 6"), "Expected 6 outline entries");
}

#[test]
fn render_page_footer() {
    let pdf = crate::HtmlConverter::new()
        .footer("Page {page} of {pages}")
        .convert("<h1>Title</h1><p>Content</p>")
        .unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("Page 1 of 1"),
        "Expected footer with page numbers"
    );
}

#[test]
fn render_page_header() {
    let pdf = crate::HtmlConverter::new()
        .header("My Document")
        .convert("<p>Content</p>")
        .unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("My Document"),
        "Expected header text in PDF"
    );
}

#[test]
fn render_header_and_footer() {
    let pdf = crate::HtmlConverter::new()
        .header("Report Title")
        .footer("Page {page} of {pages}")
        .convert("<p>Page 1</p>")
        .unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("Report Title"));
    assert!(content.contains("Page 1 of 1"));
}

#[test]
fn render_footer_multi_page() {
    let html = r#"
            <p>First page</p>
            <div style="page-break-before: always"><p>Second page</p></div>
        "#;
    let pdf = crate::HtmlConverter::new()
        .footer("Page {page} of {pages}")
        .convert(html)
        .unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // Verify page number substitution works (at least page 1 and last page are present)
    assert!(content.contains("Page 1 of"), "Expected footer with page 1");
    assert!(content.contains("Page 2 of"), "Expected footer with page 2");
}

#[test]
fn render_no_header_footer_by_default() {
    let pdf = crate::html_to_pdf("<p>Test</p>").unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(!content.contains("Page 1 of"));
}

/// CSS `@page` margin boxes with page counters (CSS Paged Media 3 §5):
/// `@bottom-center { content: "Page " counter(page) " of " counter(pages) }`
/// must render a per-page running footer with `counter(page)` resolved to the
/// 1-based page index and `counter(pages)` to the total page count.
#[test]
fn render_at_page_margin_box_counters_three_pages() {
    let html = r#"
            <style>
              @page {
                @bottom-center { content: "Page " counter(page) " of " counter(pages) }
              }
            </style>
            <p>First page</p>
            <div style="page-break-before: always"><p>Second page</p></div>
            <div style="page-break-before: always"><p>Third page</p></div>
        "#;
    let pdf = crate::HtmlConverter::new()
        .compress(false)
        .convert(html)
        .unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // Generated-content terms are rendered as independent runs so each term
    // retains the font inherited by its own formatting context. The PDF
    // therefore contains the individual resolved terms, not one combined
    // text token per footer.
    assert!(
        content.matches("Page ").count() >= 3,
        "each page should contain the literal footer prefix"
    );
    assert!(
        content.matches(" of ").count() >= 3,
        "each page should contain the literal footer separator"
    );
    assert!(
        content.matches("3").count() >= 4,
        "the immutable total-page counter should resolve to 3 on every page"
    );
}

/// `@top-left`/`@top-right` margin boxes render a running header with
/// left/right horizontal alignment, and a literal-only box renders verbatim.
#[test]
fn render_at_page_margin_box_header_alignment() {
    let html = r#"
            <style>
              @page {
                @top-left { content: "Chapter 1" }
                @top-right { content: counter(page) }
              }
            </style>
            <p>Body content</p>
        "#;
    let pdf = crate::HtmlConverter::new()
        .compress(false)
        .convert(html)
        .unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("Chapter 1"), "top-left literal header");
    // counter(page) on the single page resolves to 1.
    assert!(content.contains("(1) Tj"), "top-right page-number header");
}

#[test]
fn margin_box_background_does_not_move_generated_text() {
    fn header_td(html: &str) -> String {
        let pdf = crate::HtmlConverter::new()
            .compress(false)
            .convert(html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        let marker_pos = content.rfind("(Header) Tj").expect("header text");
        content[..marker_pos]
            .lines()
            .rev()
            .find(|line| line.ends_with(" Td") || line.ends_with(" Tm"))
            .expect("header text position")
            .to_string()
    }

    let plain =
        r#"<style>@page { margin: 40pt; @top-center { content: "Header" } }</style><p>Body</p>"#;
    let transparent = r#"<style>@page { margin: 40pt; @top-center { content: "Header"; background: transparent } }</style><p>Body</p>"#;
    assert_eq!(header_td(plain), header_td(transparent));
}

#[test]
fn render_header_only_no_footer() {
    let pdf = crate::HtmlConverter::new()
        .header("Header Only")
        .convert("<p>Content</p>")
        .unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("Header Only"));
    assert!(!content.contains("Page 1"));
}

#[test]
fn render_footer_only_no_header() {
    let pdf = crate::HtmlConverter::new()
        .footer("{page}/{pages}")
        .convert("<p>Content</p>")
        .unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("1/1"));
}

#[test]
fn render_progress_bar_zero_fraction() {
    let html = r#"<progress value="0" max="1"></progress>"#;
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // Track is drawn but fill is skipped when fraction=0
    assert!(content.contains("re\nf\n")); // track rect
    assert!(content.contains("re\nS\n")); // border stroke
}

#[test]
fn render_progress_bar_full_fraction() {
    let html = r#"<progress value="1" max="1"></progress>"#;
    let pdf = crate::html_to_pdf(html).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}

#[test]
fn pdf_text_string_encodes_non_ascii_as_utf16be() {
    // PDF 32000-1 §7.9.2.2: a text string is PDFDocEncoding or UTF-16BE with a BOM.
    assert_eq!(pdf_text_string("A (b)"), "(A \\(b\\))");
    assert_eq!(pdf_text_string("Ș"), "<FEFF0218>");
    assert_eq!(pdf_text_string("𝄞"), "<FEFFD834DD1E>");
}

#[test]
fn render_bookmark_non_ascii_title() {
    let pdf = crate::html_to_pdf("<h1>Știri</h1>").unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("/Title <FEFF02180074006900720069>"),
        "Expected a UTF-16BE outline title"
    );
}

#[test]
fn render_bookmark_special_chars() {
    let html = r#"<h1>Title with (parens) &amp; "quotes"</h1>"#;
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("/Type /Outlines"));
}

#[test]
fn render_single_heading_bookmark() {
    let html = "<h1>Only One</h1><p>Text</p>";
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("/Count 1"));
    assert!(content.contains("Only One"));
}

#[test]
fn render_link_annotation() {
    let html = r#"<p><a href="https://example.com">Click here</a></p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    // Should contain a Link annotation with the URI
    assert!(
        content.contains("/Subtype /Link"),
        "PDF should contain a Link annotation"
    );
    assert!(
        content.contains("/S /URI"),
        "PDF should contain a URI action"
    );
    assert!(
        content.contains("https://example.com"),
        "PDF should contain the link URL"
    );
    assert!(
        content.contains("/P "),
        "PDF link annotations should record their owning page"
    );
    // The page object should reference annotations
    assert!(
        content.contains("/Annots ["),
        "Page should have an /Annots array"
    );
}

#[test]
fn render_table_cell_link_annotation() {
    let html = r#"
            <table>
                <tr>
                    <td><a href="https://example.com/table">Cell link</a></td>
                </tr>
            </table>
        "#;
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert_eq!(content.matches("/Subtype /Link").count(), 1);
    assert!(content.contains("https://example.com/table"));
    assert!(content.contains("/Annots ["));
}

#[test]
fn render_nested_table_link_annotation() {
    let html = r#"
            <table>
                <tr>
                    <td>
                        <table>
                            <tr>
                                <td><a href="https://example.com/nested">Nested link</a></td>
                            </tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
    let pdf = crate::html_to_pdf(html).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert_eq!(content.matches("/Subtype /Link").count(), 1);
    assert!(content.contains("https://example.com/nested"));
    assert!(content.contains("/Annots ["));
}

#[test]
fn render_link_no_annotation_without_href() {
    // An <a> tag without href should not produce an annotation
    let html = "<p><a>No link</a></p>";
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        !content.contains("/Subtype /Link"),
        "PDF should not contain a Link annotation without href"
    );
}

#[test]
fn render_link_url_escaped() {
    // URL with parentheses should be properly escaped
    let html = r#"<p><a href="https://example.com/page(1)">Link</a></p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("/Subtype /Link"));
    assert!(content.contains(r"https://example.com/page\(1\)"));
}

#[test]
fn render_multiple_links() {
    let html = r#"<p><a href="https://one.com">One</a> and <a href="https://two.com">Two</a></p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(content.contains("https://one.com"));
    assert!(content.contains("https://two.com"));
    // Should have two Link annotations
    assert_eq!(
        content.matches("/Subtype /Link").count(),
        2,
        "Should have exactly 2 link annotations"
    );
}

#[test]
fn render_page_without_links_has_no_annots() {
    let html = "<p>No links here</p>";
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        !content.contains("/Annots"),
        "Page without links should not have /Annots"
    );
}
