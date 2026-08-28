    #[test]
    fn render_cell_text_with_empty_line_and_empty_run() {
        // Covers lines 718, 724: empty line text skipped, empty run skipped
        let empty_run = test_text_run("");
        let non_empty_run = test_text_run("Hello");
        let cell = CellBox {
            content: crate::layout::cells::CellContent {
                lines: vec![
                    TextLine {
                        runs: vec![empty_run.clone()],
                        height: 14.0,
                        baseline_ascent: None,
                        x_offset: 0.0,
                        metadata: Default::default(),
                    },
                    TextLine {
                        runs: vec![empty_run.clone(), non_empty_run],
                        height: 14.0,
                        baseline_ascent: None,
                        x_offset: 0.0,
                        metadata: Default::default(),
                    },
                ],
                ..Default::default()
            },
            box_model: crate::layout::cells::CellBoxModel {
                content_insets: EdgeSizes::uniform(2.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut content = String::new();
        let fonts = HashMap::new();
        let mut annotations = Vec::new();
        let prepared_fonts = PreparedCustomFonts::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ts_shadings = Vec::new();
        let mut ts_shading_counter = 0usize;
        let mut ts_ext_gstates = Vec::new();
        let mut ts_alpha_counter = 0usize;
        let mut text_context = PageRenderContext::new(
            &mut ts_pdf_writer,
            &mut ts_page_images,
            &fonts,
            &prepared_fonts,
            &mut ts_shadings,
            &mut ts_shading_counter,
            &mut ts_ext_gstates,
            &mut ts_alpha_counter,
            &mut annotations,
            TEST_PAGE_PAINT_BOX,
            TEST_PAGE_PAINT_BOX.height,
        );
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(PdfPoint::new(0.0, 100.0), 50.0),
            &mut text_context,
        );
        assert!(content.contains("Hello"));
    }

    #[test]
    fn text_block_empty_run_skipped() {
        // Covers line 401: empty text run within a text block line is skipped
        let page = test_page(vec![(
            0.0,
            test_text_block_from_runs(vec![test_text_run(""), test_text_run("Data")]),
        )]);
        let pdf = render_pdf(&[page], PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Data"));
    }

    #[test]
    fn page_break_element_renders() {
        // Covers line 677: PageBreak empty match arm
        let page = test_page(vec![
            (
                0.0,
                test_text_block_from_runs(vec![test_text_run("Before")]),
            ),
            (20.0, PageBreak::default().boxed()),
        ]);
        let pdf = render_pdf(&[page], PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Before"));
    }

    #[test]
    fn font_name_for_run_custom_bold_italic() {
        // Covers lines 761-763: Custom font bold+italic fallback names
        let run_bi = TextRun {
            bold: true,
            font_style: crate::style::computed::FontStyle::Italic,
            font_family: FontFamily::Custom("MyFont".to_string()),
            ..test_text_run("test")
        };
        assert_eq!(font_name_for_run(&run_bi), "Helvetica-BoldOblique");

        let run_b = TextRun {
            bold: true,
            font_family: FontFamily::Custom("MyFont".to_string()),
            ..test_text_run("test")
        };
        assert_eq!(font_name_for_run(&run_b), "Helvetica-Bold");

        let run_i = TextRun {
            font_style: crate::style::computed::FontStyle::Italic,
            font_family: FontFamily::Custom("MyFont".to_string()),
            ..test_text_run("test")
        };
        assert_eq!(font_name_for_run(&run_i), "Helvetica-Oblique");
    }

    #[test]
    fn render_radial_gradient_uses_shading() {
        use crate::types::Color;
        let mut content = String::new();
        let mut shadings = Vec::new();
        let mut counter = 0usize;
        let gradient = RadialGradient {
            ramp: gradient_ramp(
                [
                    gradient_stop(0.0, Color::rgba8(255, 0, 0, 255)),
                    gradient_stop(1.0, Color::rgba8(0, 0, 255, 255)),
                ],
                false,
            ),
            center: crate::style::computed::RadialPoint::default(),
            shape: RadialShape::Circle,
            extent: RadialExtent::FarthestCorner,
            radius: None,
            radii: None,
            layer_box: crate::style::computed::GradientLayerBox::default(),
        };
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        render_radial_gradient(
            &mut content,
            &gradient,
            LayerPaintArea::single(PdfRect::new(0.0, 0.0, 1.0, 1.0)),
            &mut shadings,
            &mut counter,
            &mut pdf_writer,
            &mut page_images,
        );
        assert!(!content.is_empty());
        assert!(content.contains("/SH0 sh"));
        assert_eq!(shadings.len(), 1);
        assert_eq!(shadings[0].kind, PdfShadingKind::Radial);
    }

    #[test]
    fn utf8_to_winansi_ascii() {
        let input = "Hello, World! 123";
        let result = utf8_to_winansi(input);
        assert_eq!(result, input.as_bytes());
    }

    #[test]
    fn utf8_to_winansi_em_dash() {
        // "hello — world" contains U+2014 em dash which should become 0x97
        let input = "hello \u{2014} world";
        let result = utf8_to_winansi(input);
        let expected: Vec<u8> = vec![
            b'h', b'e', b'l', b'l', b'o', b' ', 0x97, b' ', b'w', b'o', b'r', b'l', b'd',
        ];
        assert_eq!(result, expected);
    }

    #[test]
    fn utf8_to_winansi_quotes() {
        // Left/right single and double curly quotes
        let input = "\u{2018}hello\u{2019} \u{201C}world\u{201D}";
        let result = utf8_to_winansi(input);
        assert_eq!(result[0], 0x91); // left single quote
        assert_eq!(result[6], 0x92); // right single quote
        assert_eq!(result[8], 0x93); // left double quote
        assert_eq!(result[14], 0x94); // right double quote
    }

    #[test]
    fn utf8_to_winansi_latin1() {
        // e-acute (U+00E9), n-tilde (U+00F1), u-diaeresis (U+00FC)
        let input = "\u{00E9}\u{00F1}\u{00FC}";
        let result = utf8_to_winansi(input);
        assert_eq!(result, vec![0xE9, 0xF1, 0xFC]);
    }

    #[test]
    fn utf8_to_winansi_unknown() {
        // Chinese character and emoji should be replaced with '?'
        let input = "\u{4E16}\u{1F600}";
        let result = utf8_to_winansi(input);
        assert_eq!(result, vec![b'?', b'?']);
    }

    #[test]
    fn utf8_to_winansi_en_dash_bullet_ellipsis_euro_trademark() {
        assert_eq!(utf8_to_winansi("\u{2013}"), vec![0x96]); // en dash
        assert_eq!(utf8_to_winansi("\u{2022}"), vec![0x95]); // bullet
        assert_eq!(utf8_to_winansi("\u{2026}"), vec![0x85]); // ellipsis
        assert_eq!(utf8_to_winansi("\u{20AC}"), vec![0x80]); // euro
        assert_eq!(utf8_to_winansi("\u{2122}"), vec![0x99]); // trademark
    }

    #[test]
    fn encode_pdf_text_special_chars() {
        assert_eq!(encode_pdf_text("hello"), "hello");
        assert_eq!(encode_pdf_text("(test)"), "\\(test\\)");
        assert_eq!(encode_pdf_text("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn encode_pdf_text_em_dash() {
        let encoded = encode_pdf_text("hello \u{2014} world");
        // 0x97 = 151 decimal = 227 octal; em dash should be \227
        assert_eq!(encoded, "hello \\227 world");
    }

    #[test]
    fn encode_pdf_text_em_dash_in_pdf_bytes() {
        // Verify that rendering em dash produces correct octal escape in PDF
        // and does NOT produce UTF-8 bytes or mojibake
        let html = "<p>hello \u{2014} world</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // The PDF content stream should contain the octal escape \227
        assert!(
            pdf_str.contains("\\227"),
            "PDF should contain octal escape \\227 for em dash"
        );

        // The raw UTF-8 bytes for em dash (0xE2 0x80 0x94) should NOT appear
        let has_utf8_em_dash = pdf.windows(3).any(|w| w == [0xE2, 0x80, 0x94]);
        assert!(
            !has_utf8_em_dash,
            "PDF should not contain raw UTF-8 bytes for em dash"
        );

        // The mojibake pattern should not appear
        let has_mojibake = pdf.windows(2).any(|w| w == [0xC3, 0xA2]);
        assert!(!has_mojibake, "PDF should not contain mojibake bytes");
    }

    #[test]
    fn integration_em_dash_no_mojibake_in_pdf() {
        // Render HTML with em dash and verify the raw UTF-8 mojibake bytes
        // "\xC3\xA2\xC2\x80\xC2\x94" (the UTF-8 encoding of U+2014 read as
        // latin1) do NOT appear in the output.
        let html = "<p>hello \u{2014} world</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();

        // The mojibake sequence for em dash in UTF-8 misinterpreted as latin1
        // is bytes [0xC3, 0xA2]. This must NOT appear in the PDF.
        let has_mojibake = pdf.windows(2).any(|w| w == [0xC3, 0xA2]);
        assert!(
            !has_mojibake,
            "PDF output contains UTF-8 mojibake for em dash"
        );

        // The octal escape sequence \227 (for byte 0x97) should appear in the PDF
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("\\227"),
            "PDF output should contain octal escape \\227 for WinAnsi em dash"
        );
    }

    #[test]
    fn total_row_bold_from_descendant_selector() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            .total-row td { font-weight: bold; font-size: 12pt; }
        </style></head><body>
        <table>
            <tr><td>Item</td><td>$100</td></tr>
            <tr class="total-row"><td>Total</td><td>$100</td></tr>
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
        // The total row cells inherit the UA-default serif family, so the bold
        // descendant selector resolves to Times-Bold at 12pt.
        assert!(
            pdf_str.contains("/Times-Bold 12 Tf"),
            "Total row should use Times-Bold at 12pt, PDF content:\n{}",
            pdf_str
                .lines()
                .filter(|l| l.contains("Times"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn table_cell_em_dash_encoded_correctly() {
        let html = r#"<table><tr><td>HTML/CSS to PDF conversion — Enterprise</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Em dash in table cell should be encoded as octal \227
        assert!(
            pdf_str.contains("\\227"),
            "Table cell em dash should be encoded as \\227"
        );
        // No raw UTF-8 bytes for em dash
        let has_utf8_em_dash = pdf.windows(3).any(|w| w == [0xE2, 0x80, 0x94]);
        assert!(
            !has_utf8_em_dash,
            "Table cell should not contain raw UTF-8 em dash bytes"
        );
    }
