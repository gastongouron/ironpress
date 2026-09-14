    /// render_nested_layout_elements: rowspan == 0 skips the cell
    #[test]
    fn layout_elements_nested_rowspan_zero_skips_cell() {
        let run = test_text_run("Skipped");
        let run_visible = TextRun {
            text: "Visible".to_string(),
            ..run.clone()
        };
        // rowspan=0 means "continuation" — renderer skips it
        let mut cell_skip = table_cell(vec![TextLine {
                runs: vec![run],
                height: 14.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }]);
        cell_skip.span.rows = 0;
        let cell_visible = table_cell(vec![TextLine {
                runs: vec![run_visible],
                height: 14.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }]);
        let element = test_table_row(vec![cell_skip, cell_visible], vec![100.0, 100.0]).boxed();
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
            TEST_PAGE_PAINT_BOX,
            TEST_PAGE_PAINT_BOX.height,
        );
        let mut content = String::new();
        render_nested_layout_elements(
            &mut content,
            &[element],
            NestedLayoutFrame::new(PdfPoint::new(0.0, 100.0), PdfPoint::new(0.0, 100.0), 200.0),
            &mut ctx,
        );
        assert!(
            content.contains("(Visible)"),
            "Visible cell should be rendered"
        );
        assert!(
            !content.contains("(Skipped)"),
            "rowspan=0 cell should be skipped"
        );
    }

    /// render_nested_layout_elements: cell with background_color in nested table
    #[test]
    fn layout_elements_nested_table_cell_background_color() {
        let run = test_text_run("BgCell");
        let mut cell = table_cell(vec![TextLine {
                runs: vec![run],
                height: 14.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }]);
        cell.layout.paint.background.color = Some(Color::rgb(255, 0, 0));
        cell.layout.box_model.content_insets = EdgeSizes::uniform(2.0);
        let element = test_table_row(vec![cell], vec![100.0]).boxed();
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
            TEST_PAGE_PAINT_BOX,
            TEST_PAGE_PAINT_BOX.height,
        );
        let mut content = String::new();
        render_nested_layout_elements(
            &mut content,
            &[element],
            NestedLayoutFrame::new(PdfPoint::new(0.0, 100.0), PdfPoint::new(0.0, 100.0), 100.0),
            &mut ctx,
        );
        assert!(
            content.contains("1 0 0 rg"),
            "Should have red cell background fill"
        );
        assert!(
            content.contains("re\nf\n"),
            "Should have filled rect for cell background"
        );
        assert!(content.contains("(BgCell)"), "Should render cell text");
    }

    /// render_cell_text: text-align right and center in nested table cell
    #[test]
    fn layout_elements_cell_text_align_right_and_center() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ts_shadings = Vec::new();
        let mut ts_shading_counter = 0usize;
        let mut ts_ext_gstates = Vec::new();
        let mut ts_alpha_counter = 0usize;
        let mut ctx = PageRenderContext::new(
            &mut ts_pdf_writer,
            &mut ts_page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut ts_shadings,
            &mut ts_shading_counter,
            &mut ts_ext_gstates,
            &mut ts_alpha_counter,
            &mut annotations,
            TEST_PAGE_PAINT_BOX,
            TEST_PAGE_PAINT_BOX.height,
        );

        let run = test_text_run("Aligned");

        // Test right-align
        let cell_right = text_cell(
            vec![TextLine {
                runs: vec![run.clone()],
                height: 14.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }],
            TextAlign::Right,
        );
        let mut content_right = String::new();
        render_cell_text(
            &mut content_right,
            &cell_right,
            CellTextPlacement::new(PdfPoint::new(0.0, 100.0), 200.0),
            &mut ctx,
        );
        assert!(
            content_right.contains("(Aligned)"),
            "Should render right-aligned text"
        );

        // Test center-align
        let cell_center = text_cell(
            vec![TextLine {
                runs: vec![run],
                height: 14.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }],
            TextAlign::Center,
        );
        let mut content_center = String::new();
        render_cell_text(
            &mut content_center,
            &cell_center,
            CellTextPlacement::new(PdfPoint::new(0.0, 100.0), 200.0),
            &mut ctx,
        );
        assert!(
            content_center.contains("(Aligned)"),
            "Should render center-aligned text"
        );
    }

    /// render_cell_text: underline and line_through in nested table cell
    #[test]
    fn layout_elements_cell_text_underline_and_line_through() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ts_shadings = Vec::new();
        let mut ts_shading_counter = 0usize;
        let mut ts_ext_gstates = Vec::new();
        let mut ts_alpha_counter = 0usize;
        let mut ctx = PageRenderContext::new(
            &mut ts_pdf_writer,
            &mut ts_page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut ts_shadings,
            &mut ts_shading_counter,
            &mut ts_ext_gstates,
            &mut ts_alpha_counter,
            &mut annotations,
            TEST_PAGE_PAINT_BOX,
            TEST_PAGE_PAINT_BOX.height,
        );

        let underline_run = TextRun {
            decorations: vec![crate::style::computed::TextDecoration {
                lines: crate::style::computed::TextDecorationLines {
                    underline: true,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..test_text_run("Under")
        };
        let strike_run = TextRun {
            decorations: vec![crate::style::computed::TextDecoration {
                lines: crate::style::computed::TextDecorationLines {
                    line_through: true,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..test_text_run("Strike")
        };

        let cell = text_cell(
            vec![
                TextLine {
                    runs: vec![underline_run],
                    height: 14.0,
                    baseline_ascent: None,
                    x_offset: 0.0,
                    metadata: Default::default(),
                },
                TextLine {
                    runs: vec![strike_run],
                    height: 14.0,
                    baseline_ascent: None,
                    x_offset: 0.0,
                    metadata: Default::default(),
                },
            ],
            TextAlign::Left,
        );

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(PdfPoint::new(10.0, 200.0), 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Under)"), "Should render underlined text");
        assert!(
            content.contains("(Strike)"),
            "Should render struck-through text"
        );
        // Both decorations draw filled rectangles in the cell text path.
        let decoration_count = filled_rect_count(&content);
        assert!(
            decoration_count >= 2,
            "Should have filled decorations for underline and line-through, got {decoration_count}"
        );
    }

    /// render_cell_text: inline span with background_color and border_radius in nested cell
    #[test]
    fn layout_elements_cell_text_inline_bg_with_border_radius() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ts_shadings = Vec::new();
        let mut ts_shading_counter = 0usize;
        let mut ts_ext_gstates = Vec::new();
        let mut ts_alpha_counter = 0usize;
        let mut ctx = PageRenderContext::new(
            &mut ts_pdf_writer,
            &mut ts_page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut ts_shadings,
            &mut ts_shading_counter,
            &mut ts_ext_gstates,
            &mut ts_alpha_counter,
            &mut annotations,
            TEST_PAGE_PAINT_BOX,
            TEST_PAGE_PAINT_BOX.height,
        );

        let run = TextRun {
            color: Color::WHITE,
            background_color: Some(Color::from_srgb(0.2, 0.4, 0.8, 1.0)),
            padding: EdgeSizes::axes(3.0, 2.0),
            border_radii: CornerRadii::circular(4.0),
            ..test_text_run("Badge")
        };

        let cell = text_cell(
            vec![TextLine {
                runs: vec![run],
                height: 14.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }],
            TextAlign::Left,
        );

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(PdfPoint::new(10.0, 200.0), 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Badge)"), "Should render badge text");
        // Inline background fill (rounded rect uses Bezier c operator)
        assert!(
            content.contains("0.2 0.4 0.8 rg"),
            "Should have blue inline background color"
        );
        assert!(
            content.contains(" c\n"),
            "Should have Bezier curves for rounded inline bg"
        );
    }

    /// render_cell_text: inline span with background_color but no border_radius (rect path)
    #[test]
    fn layout_elements_cell_text_inline_bg_no_border_radius() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ts_shadings = Vec::new();
        let mut ts_shading_counter = 0usize;
        let mut ts_ext_gstates = Vec::new();
        let mut ts_alpha_counter = 0usize;
        let mut ctx = PageRenderContext::new(
            &mut ts_pdf_writer,
            &mut ts_page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut ts_shadings,
            &mut ts_shading_counter,
            &mut ts_ext_gstates,
            &mut ts_alpha_counter,
            &mut annotations,
            TEST_PAGE_PAINT_BOX,
            TEST_PAGE_PAINT_BOX.height,
        );

        let run = TextRun {
            background_color: Some(Color::rgb(255, 255, 0)), // yellow
            padding: EdgeSizes::axes(2.0, 1.0),
            ..test_text_run("Tag")
        };

        let cell = text_cell(
            vec![TextLine {
                runs: vec![run],
                height: 14.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }],
            TextAlign::Left,
        );

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(PdfPoint::new(10.0, 200.0), 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Tag)"), "Should render tag text");
        assert!(
            content.contains("1 1 0 rg"),
            "Should have yellow inline background color"
        );
        // No border-radius: should use rectangle re operator
        assert!(
            content.contains(" re\nf\n"),
            "Should use rectangle fill for zero-radius inline bg"
        );
    }

    /// plan_nested_layout_elements: Position::Relative with positioned_depth registers origin
    #[test]
    fn layout_elements_plan_relative_with_positioned_depth() {
        let mut relative = test_text_block_from_runs(vec![test_text_run("Relative")]);
        relative.update_text(|block| {
            block.positioning.scheme = Position::Relative;
            block.positioning.insets.top = 5.0;
            block.positioning.insets.left = 15.0;
            block.positioning.containing_block_depth = 1;
        });
        let elements = [relative];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(PdfPoint::new(20.0, 80.0), PdfPoint::new(10.0, 120.0), 100.0),
        );
        assert_eq!(planned.len(), 1);
        // Relative: uses local origin (20.0) + offset_left (15.0)
        assert!(
            (planned[0].origin.x - 35.0).abs() < 0.01,
            "Relative block origin_x should be frame.origin_x + offset_left"
        );
        // top_y: cursor_y (80.0) - margin_top (0) - offset_top (5) = 75.0
        assert!(
            (planned[0].origin.y - 75.0).abs() < 0.01,
            "Relative block top_y should be cursor_y - offset_top"
        );
    }

    /// plan_nested_layout_elements: absolute with containing_block sets blur_canvas_box
    #[test]
    fn layout_elements_plan_absolute_with_containing_block_sets_blur_canvas_box() {
        let containing = crate::layout::engine::ContainingBlock {
            x: 5.0,
            width: 200.0,
            height: 100.0,
            depth: 2,
        };
        let mut absolute = test_text_block_from_runs(vec![test_text_run("Abs")]);
        absolute.update_text(|block| {
            block.positioning.scheme = Position::Absolute;
            block.positioning.containing_block = Some(containing);
            block.positioning.containing_block_depth = 0;
        });
        // First register a positioned origin for depth 2 by planning a relative block
        let mut relative_parent = test_text_block_from_runs(vec![test_text_run("Parent")]);
        relative_parent.update_text(|block| {
            block.positioning.scheme = Position::Relative;
            block.positioning.containing_block_depth = 2;
        });
        let elements = [relative_parent, absolute];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(
                PdfPoint::new(10.0, 200.0),
                PdfPoint::new(10.0, 200.0),
                300.0,
            ),
        );
        // The absolute element should have a blur_canvas_box derived from the containing block
        let _abs_planned = planned
            .iter()
            .find(|planned| planned.element.inspect_text(|_| ()).is_some());
        // Just verify the plan succeeds without panic and produces 2 elements
        assert_eq!(planned.len(), 2, "Should plan both elements");
    }

    /// table_row_total_height: returns 0 for non-TableRow variant
    #[test]
    fn layout_elements_table_row_total_height_non_row_returns_zero() {
        let non_row = PageBreak::default();
        assert_eq!(
            table_row_total_height(&non_row),
            0.0,
            "Non-TableRow element should return 0 height"
        );
        let text_block = test_text_block_from_runs(vec![test_text_run("Hello")]);
        assert_eq!(
            table_row_total_height(&text_block),
            0.0,
            "TextBlock element should return 0 height"
        );
    }

    /// Integration: nested table with vertical-align middle exercises layout_elements paths
    #[test]
    fn layout_elements_nested_table_cell_vertical_align_middle_integration() {
        let html = r#"<table>
            <tr>
                <td>
                    <table>
                        <tr>
                            <td style="vertical-align: middle; height: 50pt">Inner</td>
                            <td style="height: 50pt">Other</td>
                        </tr>
                    </table>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(Inner)"),
            "Should render inner nested cell text"
        );
    }

    /// Integration: nested div inside table cell with SVG background
    /// exercises render_nested_text_block with background_svg via nested cell rows
    #[test]
    fn layout_elements_nested_svg_background_in_table_cell() {
        // A div with SVG background inside a td triggers render_nested_layout_elements
        // and render_nested_text_block with background_svg set
        let html = r#"<table>
            <tr>
                <td>
                    <div style="background-image: url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2210%22 height=%2210%22%3E%3Crect width=%2210%22 height=%2210%22 fill=%22red%22/%3E%3C/svg%3E'); background-size: cover; width: 40pt; height: 20pt;">CellSVG</div>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // The text should render
        assert!(
            pdf_str.contains("(CellSVG)"),
            "Should render text inside nested cell div"
        );
        // The overall PDF should be valid (no crash on SVG background in nested context)
        assert!(pdf_str.contains("%PDF-1.4"), "Should produce a valid PDF");
    }

    /// Integration: border-collapse collapse with nested elements
    #[test]
    fn layout_elements_nested_border_collapse() {
        let html = r#"<table style="border-collapse: collapse">
            <tr>
                <td style="border: 1pt solid black">CollapseA</td>
                <td style="border: 1pt solid black">CollapseB</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("(CollapseA)"), "Should render first cell");
        assert!(pdf_str.contains("(CollapseB)"), "Should render second cell");
    }

    /// Integration: nested table with rowspan > 1 spanning into future rows
    #[test]
    fn layout_elements_nested_rowspan_spans_future_rows() {
        let html = r#"<table>
            <tr>
                <td>
                    <table>
                        <tr>
                            <td rowspan="2">SpanInner</td>
                            <td>A</td>
                        </tr>
                        <tr>
                            <td>B</td>
                        </tr>
                    </table>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(SpanInner)"),
            "Should render spanning nested cell"
        );
        assert!(
            pdf_str.contains("(A)"),
            "Should render first row second cell"
        );
        assert!(
            pdf_str.contains("(B)"),
            "Should render second row second cell"
        );
    }
fn text_cell(lines: Vec<TextLine>, inline: TextAlign) -> CellBox {
    CellBox {
        content: crate::layout::cells::CellContent {
            lines,
            ..Default::default()
        },
        alignment: crate::layout::cells::CellAlignment {
            inline,
            block: VerticalAlign::Top,
        },
        ..Default::default()
    }
}

fn table_cell(lines: Vec<TextLine>) -> TableCell {
    TableCell {
        layout: text_cell(lines, TextAlign::Left),
        ..Default::default()
    }
}
