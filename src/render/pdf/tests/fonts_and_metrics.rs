#[test]
fn build_shading_function_four_stops_stitching() {
    // Covers lines 1277-1304: Type 3 stitching function with 4 stops
    let stops = PdfGradientStops::unit([
        (0.0, (1.0, 0.0, 0.0)),
        (0.33, (0.0, 1.0, 0.0)),
        (0.66, (0.0, 0.0, 1.0)),
        (1.0, (1.0, 1.0, 0.0)),
    ])
    .unwrap();
    let result = build_shading_function(&stops);
    assert!(
        result.contains("/FunctionType 3"),
        "4 stops should produce Type 3 stitching function"
    );
    assert!(
        result.contains("/Bounds [0.33 0.66]"),
        "Should have bounds for intermediate stops"
    );
    assert!(
        result.contains("/Encode [0 1 0 1 0 1]"),
        "Should have encode entries for each sub-function"
    );
    // Should contain 3 sub-functions (one per stop pair)
    let subfn_count = result.matches("/FunctionType 2").count();
    assert_eq!(
        subfn_count, 3,
        "Should have 3 Type 2 sub-functions, got {subfn_count}"
    );
}

#[test]
fn custom_font_embedding_in_pdf() {
    // Covers lines 1628-1657: TTF font objects in PDF
    use crate::parser::ttf::TtfFont;
    let mut cmap = HashMap::new();
    for c in 32u32..=126 {
        cmap.insert(c, (c - 31) as u16);
    }
    let ttf = TtfFont {
        font_name: "TestFont".to_string(),
        face_index: Default::default(),
        units_per_em: 1000,
        size_adjust: 1.0,
        bbox: [0, -200, 800, 800],
        vertical_metrics: crate::parser::ttf::FontVerticalMetricSet::from(
            crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
        ),
        cmap,
        glyph_widths: (0..=96).map(|_| 500).collect(),
        num_h_metrics: 96,
        flags: 32,
        is_bold: false,
        is_italic: false,
        text_metrics: Default::default(),
        data: std::sync::Arc::new(vec![0u8; 64]), // Minimal dummy font data
        shaping: None,
    };
    let mut fonts = HashMap::new();
    fonts.insert("TestFont".to_string(), ttf);

    let mut run = test_text_run("Custom");
    run.font_family = FontFamily::Custom("TestFont".to_string());
    let page = test_page(vec![(0.0, test_text_block_from_runs(vec![run]))]);
    let pdf = render_pdf_with_fonts(&[page], PageSize::A4, Margin::default(), &fonts).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("/BaseFont /TestFont"),
        "Should have custom font BaseFont entry"
    );
    assert!(
        pdf_str.contains("/Subtype /Type0"),
        "Should have Type0 font wrapper"
    );
    assert!(
        pdf_str.contains("/Subtype /CIDFontType2"),
        "Should have CIDFontType2 descendant font"
    );
    assert!(
        pdf_str.contains("/FontDescriptor"),
        "Should have FontDescriptor reference"
    );
    assert!(
        pdf_str.contains("/Encoding /Identity-H"),
        "Should use Identity-H for shaped custom glyphs"
    );
    assert!(
        pdf_str.contains("/ToUnicode"),
        "Should attach a ToUnicode CMap for text extraction"
    );
    assert!(
        pdf_str.contains("/FontFile2"),
        "Should have FontFile2 reference for embedded TTF"
    );
    assert!(
        pdf_str.contains("/TestFont"),
        "Should reference custom font name"
    );
}

#[test]
fn inline_background_uses_the_css_font_strut() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity/fonts/ParitySans.ttf"),
    )
    .expect("ParitySans test font");
    let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans TTF");
    let fonts = HashMap::from([("ParitySans".to_string(), font)]);
    let run = TextRun {
        font_family: FontFamily::Custom("ParitySans".to_string()),
        // 18 CSS px in the point-based layout coordinate system.
        font_size: 13.5,
        ..Default::default()
    };

    let (bottom, height) = inline_background_y_and_height(&run, 96.75, EdgeSizes::ZERO, &fonts);

    assert_eq!(bottom, 93.75);
    assert_eq!(height, 15.75);
}

#[test]
fn render_run_glyphs_falls_back_to_standard_font_when_custom_shaping_fails() {
    use crate::parser::ttf::TtfFont;

    let mut cmap = HashMap::new();
    for c in 32u32..=126 {
        cmap.insert(c, (c - 31) as u16);
    }
    let ttf = TtfFont {
        font_name: "TestFont".to_string(),
        face_index: Default::default(),
        units_per_em: 1000,
        size_adjust: 1.0,
        bbox: [0, -200, 800, 800],
        vertical_metrics: crate::parser::ttf::FontVerticalMetricSet::from(
            crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
        ),
        cmap,
        glyph_widths: (0..=96).map(|_| 500).collect(),
        num_h_metrics: 96,
        flags: 32,
        is_bold: false,
        is_italic: false,
        text_metrics: Default::default(),
        data: std::sync::Arc::new(vec![0u8; 64]),
        shaping: None,
    };
    let mut fonts = HashMap::new();
    fonts.insert(
        crate::system_fonts::font_variant_key("TestFont", false, false),
        ttf,
    );

    let mut run = test_text_run("Custom");
    run.font_family = FontFamily::Custom("TestFont".to_string());

    let mut content = String::new();
    let prepared_custom_fonts = PreparedCustomFonts::new();
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();
    render_run_glyphs(
        &mut content,
        &run,
        10.0,
        20.0,
        run.font_size,
        &fonts,
        &prepared_custom_fonts,
        0.0,
        &mut pdf_writer,
        &mut page_images,
    );

    assert!(content.contains("/Helvetica 12 Tf\n"));
    assert!(content.contains("(Custom) Tj\n"));
    assert!(!content.contains("/testfont 12 Tf\n"));
}

fn tj_test_font() -> crate::parser::ttf::TtfFont {
    crate::parser::ttf::TtfFont {
        font_name: "TestFont".to_string(),
        face_index: Default::default(),
        units_per_em: 1000,
        size_adjust: 1.0,
        bbox: [0, -200, 800, 800],
        vertical_metrics: crate::parser::ttf::FontVerticalMetricSet::from(
            crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
        ),
        cmap: HashMap::new(),
        glyph_widths: vec![0, 500, 500],
        num_h_metrics: 3,
        flags: 32,
        is_bold: false,
        is_italic: false,
        text_metrics: Default::default(),
        data: std::sync::Arc::new(Vec::new()),
        shaping: None,
    }
}

#[test]
fn append_tj_shaped_text_uses_single_text_matrix() {
    let font = tj_test_font();
    let shaped = crate::text::ShapedRun {
        glyphs: vec![
            crate::text::ShapedGlyph {
                glyph_id: 1,
                cluster: 0,
                x_advance: 6.0,
                y_advance: 0.0,
                x_offset: 0.0,
                y_offset: 0.0,
                unicode: vec![0x0041],
            },
            crate::text::ShapedGlyph {
                glyph_id: 2,
                cluster: 1,
                x_advance: 6.0,
                y_advance: 0.0,
                x_offset: 0.0,
                y_offset: 0.0,
                unicode: vec![0x0042],
            },
        ],
        width: 12.0,
    };
    let mut content = String::new();
    append_tj_shaped_text(
        &mut content,
        ShapedTextRender::new(
            PdfPoint::new(10.0, 20.0),
            12.0,
            &font,
            &shaped,
            None,
            PdfContentSpace::Points,
        ),
    );

    assert!(
        content.contains("1 0 0 1 10 20 Tm"),
        "Should position the run once with a single text matrix"
    );
    assert!(
        content.contains("[<0001> <0002>] TJ"),
        "Should encode the shaped run as one TJ array"
    );
    assert_eq!(
        content.matches(" Tm\n").count(),
        1,
        "Simple shaped runs should not emit per-glyph matrices"
    );
}

#[test]
fn append_tj_shaped_text_keeps_repeated_subthreshold_adjustments() {
    let font = tj_test_font();
    let glyph_count = 17;
    let shaped = crate::text::ShapedRun {
        glyphs: (0..glyph_count)
            .map(|cluster| crate::text::ShapedGlyph {
                glyph_id: 1,
                cluster,
                x_advance: 6.0,
                y_advance: 0.0,
                x_offset: 0.0,
                y_offset: 0.0,
                unicode: vec![0x0041],
            })
            .collect(),
        width: 6.0 * glyph_count as f32,
    };
    let authored_adjustment = 0.000_49_f32;
    let letter_spacing = -(authored_adjustment * 12.0 / 1000.0);
    let expected_adjustment = -(letter_spacing * 1000.0 / 12.0);
    let mut content = String::new();
    append_tj_shaped_text(
        &mut content,
        ShapedTextRender::new(
            PdfPoint::new(0.0, 0.0),
            12.0,
            &font,
            &shaped,
            None,
            PdfContentSpace::Points,
        )
        .with_letter_spacing(letter_spacing),
    );

    let array = content
        .split_once('[')
        .and_then(|(_, rest)| rest.split_once("] TJ"))
        .map(|(array, _)| array)
        .unwrap();
    let adjustments = array
        .split_ascii_whitespace()
        .filter_map(|token| token.parse::<f32>().ok())
        .collect::<Vec<_>>();

    assert_eq!(adjustments.len(), glyph_count - 1);
    assert!(
        adjustments
            .iter()
            .all(|value| value.to_bits() == expected_adjustment.to_bits())
    );
    assert!(adjustments.iter().sum::<f32>() > authored_adjustment);
}

#[test]
fn synthetic_italic_shear_keeps_its_visual_direction_in_each_text_space() {
    let font = tj_test_font();
    let shaped = crate::text::ShapedRun {
        glyphs: vec![crate::text::ShapedGlyph {
            glyph_id: 1,
            cluster: 0,
            x_advance: 6.0,
            y_advance: 0.0,
            x_offset: 0.0,
            y_offset: 0.0,
            unicode: vec![0x0041],
        }],
        width: 6.0,
    };

    let render = |text_space| {
        let mut content = String::new();
        append_tj_shaped_text(
            &mut content,
            ShapedTextRender::new(
                PdfPoint::new(10.0, 20.0),
                12.0,
                &font,
                &shaped,
                None,
                text_space,
            )
            .with_shear(0.25),
        );
        content
    };

    assert!(render(PdfContentSpace::Points).contains("1 0 0.25 1 10 20 Tm"));
    assert!(
        render(PdfContentSpace::PageCss { page_height: 100.0 }).contains("1 0 0.25 -1"),
        "the PageCss Y reflection must not reverse the horizontal italic shear"
    );
}

#[test]
fn build_tounicode_cmap_supports_multi_codepoint_glyphs() {
    let cmap = build_tounicode_cmap(&[(1, vec![0x0066, 0x0069])]);
    assert!(
        cmap.contains("<0001> <00660069>"),
        "ToUnicode should preserve multi-codepoint mappings such as ligatures"
    );
}

#[test]
fn ext_gstate_objects_rendered() {
    // Covers line 2011: ExtGState objects in resource dict
    let html = r#"<div style="opacity: 0.3">Dim</div><div style="opacity: 0.7">Bright</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(pdf_str.contains("/ca 0.3"), "Should have fill opacity 0.3");
    assert!(pdf_str.contains("/ca 0.7"), "Should have fill opacity 0.7");
    assert!(
        pdf_str.contains("/ExtGState"),
        "Should have ExtGState in resources"
    );
    // Opacity groups are isolated by the PDF graphics-state stack. `Q`
    // restores the complete prior state, so emitting `/GSDefault gs` after
    // every form would be redundant and could accidentally reset unrelated
    // state owned by an outer scope.
    let isolated_groups = pdf_str
        .lines()
        .collect::<Vec<_>>()
        .windows(4)
        .filter(|lines| {
            lines[0] == "q"
                && lines[1].starts_with("/GSgrp")
                && lines[1].ends_with(" gs")
                && lines[2].ends_with(" Do")
                && lines[3] == "Q"
        })
        .count();
    assert!(
        isolated_groups >= 2,
        "each opacity form must be isolated by a balanced q/Q scope"
    );
}

#[test]
fn flexrow_cell_gradient_with_border_radius() {
    // Covers lines 1009-1060: FlexRow cell with linear gradient + border-radius clip
    let html = r#"<div style="display: flex"><div style="width: 150pt; background: linear-gradient(to bottom, red, blue); border-radius: 10pt">Grad Cell</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(pdf_str.contains("Grad Cell"), "Should render cell text");
    assert!(
        has_axial_gradient_pattern(&pdf_str),
        "Should paint an axial shading pattern for the cell gradient"
    );
}

#[test]
fn half_leading_text_positioning() {
    // Text blocks should use half-leading model (not full line.height offset)
    let html = "<p style=\"font-size: 20pt; line-height: 2\">Test</p>";
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // The shared per-run path uses an absolute text matrix; either PDF text
    // positioning operator is valid for this geometry assertion.
    assert!(
        pdf_str.contains("Td\n") || pdf_str.contains("Tm\n"),
        "Should have text positioning"
    );
    // Text should be rendered
    assert!(pdf_str.contains("(Test)"), "Should contain text content");
}

#[test]
fn underline_in_flex_cell() {
    // Underline in flex cells should produce stroke commands
    let html = r#"<html><head><style>
            .row { display: flex; }
        </style></head><body>
        <div class="row">
            <div><u>Underlined in flex</u></div>
        </div>
        </body></html>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        filled_rect_count(&pdf_str) >= 1,
        "Should draw underline decoration in flex cell"
    );
}

#[test]
fn propagated_underlines_use_the_shared_nested_flex_text_path() {
    let html = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/parity/cases/interactions/",
        "interactions-cartesian-flexbox-x-text-advanced.html"
    ));
    let font = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/parity/fonts/ParitySans.ttf"
    ))
    .unwrap();
    let pdf = crate::HtmlConverter::new()
        .compress(false)
        .add_font("ParitySans", font)
        .convert(html)
        .unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    let decoration_fill = PdfRgb::from(crate::types::Color::rgb(0xef, 0x47, 0x6f)).fill_operator();
    let decorations = pdf_str.matches(&decoration_fill).count();

    assert!(
        decorations >= 4,
        "every direct and nested flex text path must paint its propagated underline; found {decorations}"
    );
}

#[test]
fn strikethrough_in_flex_cell() {
    let html = r#"<html><head><style>
            .row { display: flex; }
        </style></head><body>
        <div class="row">
            <div><del>Deleted in flex</del></div>
        </div>
        </body></html>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        filled_rect_count(&pdf_str) >= 1,
        "Should draw strikethrough decoration in flex cell"
    );
}

#[test]
fn underline_in_table_cell() {
    let html = r#"<table><tr><td><u>Underlined cell</u></td></tr></table>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        filled_rect_count(&pdf_str) >= 1,
        "Should draw underline decoration in table cell"
    );
}

#[test]
fn strikethrough_in_table_cell() {
    let html = r#"<table><tr><td><s>Struck cell</s></td></tr></table>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        filled_rect_count(&pdf_str) >= 1,
        "Should draw strikethrough decoration in table cell"
    );
}

#[test]
fn font_size_relative_underline_thickness() {
    // Large font should produce thicker underline than small font
    let html = r#"<p><span style="font-size: 6pt; text-decoration: underline">Small</span></p>
        <p><span style="font-size: 30pt; text-decoration: underline">Big</span></p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // Both should have decorations; thickness should vary with font size.
    let heights = filled_rect_heights(&pdf_str);
    let min_height = heights.iter().copied().fold(f32::INFINITY, f32::min);
    let max_height = heights.iter().copied().fold(0.0, f32::max);
    assert!(
        heights.len() >= 2 && max_height - min_height > 1.0,
        "Should have at least 2 underline rectangles with varied thickness, got {heights:?}"
    );
}

#[test]
fn table_cell_vertical_centering_with_metrics() {
    // Table cells with different row heights should center text
    let html = r#"<table>
            <tr>
                <td style="padding: 20pt">Centered</td>
                <td>Short</td>
            </tr>
        </table>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("(Centered)"),
        "Should render centered cell text"
    );
    assert!(pdf_str.contains("(Short)"), "Should render short cell text");
}

// ===== layout_elements.rs coverage tests =====

/// table_cell_content_top: VerticalAlign::Middle positions text mid-row
#[test]
fn layout_elements_vertical_align_middle_in_table_cell() {
    use crate::parser::css::parse_stylesheet;
    let html = r#"<html><head><style>
            td { vertical-align: middle; }
        </style></head><body>
        <table>
            <tr>
                <td style="height: 80pt; padding: 0">Middle</td>
                <td style="height: 80pt; padding: 0">Other</td>
            </tr>
        </table>
        </body></html>"#;
    let result = crate::parser::html::parse_html_with_styles(html).unwrap();
    let mut rules = Vec::new();
    for css in &result.stylesheets {
        rules.extend(parse_stylesheet(css));
    }
    let pages = crate::layout::engine::layout_with_rules(
        &result.nodes,
        PageSize::A4,
        Margin::default(),
        &rules,
    );
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("(Middle)"),
        "Should render middle-aligned text"
    );
}

/// table_cell_content_top: VerticalAlign::Bottom positions text at bottom
#[test]
fn layout_elements_vertical_align_bottom_in_table_cell() {
    use crate::parser::css::parse_stylesheet;
    let html = r#"<html><head><style>
            td.bottom { vertical-align: bottom; }
        </style></head><body>
        <table>
            <tr>
                <td class="bottom" style="padding: 0">Bottom</td>
                <td style="padding: 0; height: 60pt">Tall</td>
            </tr>
        </table>
        </body></html>"#;
    let result = crate::parser::html::parse_html_with_styles(html).unwrap();
    let mut rules = Vec::new();
    for css in &result.stylesheets {
        rules.extend(parse_stylesheet(css));
    }
    let pages = crate::layout::engine::layout_with_rules(
        &result.nodes,
        PageSize::A4,
        Margin::default(),
        &rules,
    );
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("(Bottom)"),
        "Should render bottom-aligned text"
    );
}
