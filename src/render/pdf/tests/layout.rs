#[test]
fn fragmented_effect_geometry_uses_one_composite_size_at_each_fragment_origin() {
    use crate::layout::elements::{BoxFragmentSlice, BoxFragmentation, BoxModel};

    let box_model = BoxModel {
        padding: EdgeSizes::new(3.0, 5.0, 7.0, 11.0),
        ..Default::default()
    };
    let (_, continuation) = BoxFragmentSlice::split(100.0, 80.0, &box_model);
    let painting = PaintBoxGeometry::new(
        PdfRect::from_top(10.0, 200.0, 100.0, 80.0),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );
    let fragmented = painting.for_fragment(BoxFragmentation {
        reference_slice: Some(continuation),
        ..Default::default()
    });

    assert_eq!(fragmented.painting().border_box, painting.border_box);
    assert_eq!(
        fragmented.positioning().border_box,
        PdfRect::from_top(10.0, 300.0, 100.0, 180.0)
    );
    assert_eq!(
        fragmented.shape_reference().border_box,
        PdfRect::from_top(10.0, 200.0, 100.0, 180.0)
    );
    assert_eq!(fragmented.shape_reference().padding, box_model.padding);
}

#[test]
fn nested_absolute_without_containing_block_uses_initial_origin() {
    let mut absolute = test_text_block_from_runs(vec![test_text_run("Absolute")]);
    absolute.update_text(|block| {
        block.positioning.scheme = Position::Absolute;
        block.positioning.insets.top = 10.0;
        block.positioning.insets.left = 20.0;
    });

    let elements = [absolute];
    let planned = plan_nested_layout_elements(
        &elements,
        NestedLayoutFrame::new(PdfPoint::new(50.0, 100.0), PdfPoint::new(10.0, 200.0), 80.0),
    );
    assert_eq!(planned.len(), 1);
    assert!((planned[0].origin.x - 30.0).abs() < 0.01);
    assert!((planned[0].origin.y - 190.0).abs() < 0.01);
}

#[test]
fn nested_static_without_containing_block_uses_local_origin() {
    let static_block = test_text_block_from_runs(vec![test_text_run("Static")]);
    let elements = [static_block];
    let planned = plan_nested_layout_elements(
        &elements,
        NestedLayoutFrame::new(PdfPoint::new(50.0, 100.0), PdfPoint::new(10.0, 200.0), 80.0),
    );
    assert_eq!(planned.len(), 1);
    assert!((planned[0].origin.x - 50.0).abs() < 0.01);
    assert!((planned[0].origin.y - 100.0).abs() < 0.01);
}

#[test]
fn table_cell_absolute_pseudo_background_renders_blurred_copy() {
    use crate::parser::css::parse_stylesheet;

    let png_bytes = {
        let image = image::RgbaImage::from_fn(4, 4, |x, y| {
            image::Rgba([(x * 40) as u8, (y * 40) as u8, 180, 255])
        });
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut encoded),
                image::ImageFormat::Png,
            )
            .unwrap();
        encoded
    };
    let b64 = simple_base64_encode_test(&png_bytes);
    let html = format!(
        r#"<html><head><style>
                .image-container {{
                    display: flex;
                    position: relative;
                    width: 40pt;
                    aspect-ratio: 1 / 1;
                    background-image: url('data:image/png;base64,{b64}');
                    background-size: cover;
                    background-repeat: no-repeat;
                }}
                .image-container::after {{
                    content: '';
                    background-image: inherit;
                    background-size: inherit;
                    background-repeat: inherit;
                    width: 100%;
                    height: 100%;
                    display: block;
                    position: absolute;
                    bottom: -10pt;
                    z-index: -1;
                    filter: blur(4px);
                }}
            </style></head><body>
                <table><tr><td><div class="image-container"></div></td></tr></table>
            </body></html>"#
    );
    let result = crate::parser::html::parse_html_with_styles(&html).unwrap();
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
    fn count_element_background_svgs(element: &dyn LayoutElement) -> usize {
        let own_text = element
            .inspect_text(|text| usize::from(text.paint.background.layers.has_image()))
            .unwrap_or_default();
        let own_flex = element
            .inspect_flex(|flex| {
                usize::from(flex.paint.background.layers.has_image())
                    + flex
                        .content
                        .cells
                        .iter()
                        .map(|cell| usize::from(cell.paint.background.layers.has_image()))
                        .sum::<usize>()
            })
            .unwrap_or_default();
        let mut descendants = 0;
        element.visit_children(&mut |child| {
            descendants += count_element_background_svgs(child);
        });
        own_text + own_flex + descendants
    }

    let background_svg_count: usize = pages[0]
        .elements
        .iter()
        .map(|(_, element)| count_element_background_svgs(element))
        .sum();

    assert!(
        background_svg_count >= 2,
        "Expected both the main block and the blurred pseudo-element to survive into layout with raster backgrounds"
    );

    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("/SMask"),
        "Expected the blurred pseudo-background to preserve alpha via a PDF soft mask"
    );
}

fn first_table_cell(html: &str) -> TableCell {
    let nodes = parse_html(html).unwrap();
    layout(&nodes, PageSize::A4, Margin::default())
        .into_iter()
        .flat_map(|page| page.elements.into_iter().map(|(_, element)| element))
        .find_map(|element| {
            element.inspect_table(|row| {
                row.content
                    .cells
                    .iter()
                    .find(|cell| cell.span.rows != 0)
                    .cloned()
            })?
        })
        .expect("table must produce a real cell")
}

#[test]
fn double_table_border_uses_semantic_edge_after_coordinate_perturbation() {
    let side = crate::layout::engine::LayoutBorderSide {
        width: 3.0,
        style: BorderStyle::Double,
        ..Default::default()
    };
    let mut content = String::new();
    let mut states = Vec::new();
    let mut counter = 0;

    paint_table_cell_border_line(
        &mut content,
        &side,
        PhysicalSide::Left,
        10.0,
        30.0,
        10.02,
        5.0,
        &mut states,
        &mut counter,
    );

    assert!(content.contains("8.5 5 0.75 25 re"));
    assert!(content.contains("10.75 5 0.75 25 re"));
    assert!(!content.contains("8.5 29 0.75 0.02 re"));
}

#[test]
fn legacy_table_border_uses_inset_while_authored_css_preserves_solid() {
    let legacy = first_table_cell(r#"<table border="1"><tr><td>x</td></tr></table>"#);
    let css = first_table_cell(
        r#"<table><tr><td style="border:0.75pt solid #eeeeee">x</td></tr></table>"#,
    );
    let css_over_attr = first_table_cell(
        r#"<table border="1"><tr><td style="border:0.75pt solid #eeeeee">x</td></tr></table>"#,
    );
    let border_none =
        first_table_cell(r#"<table border="1"><tr><td style="border:none">x</td></tr></table>"#);

    for authored in [&css, &css_over_attr] {
        assert_eq!(
            authored.layout.box_model.border.top.width,
            legacy.layout.box_model.border.top.width
        );
        assert_eq!(
            authored.layout.box_model.border.top.color,
            legacy.layout.box_model.border.top.color
        );
        assert_eq!(
            authored.layout.box_model.border.top.style,
            BorderStyle::Solid
        );
    }

    assert_eq!(legacy.layout.box_model.border.top.style, BorderStyle::Inset);
    assert_eq!(border_none.layout.box_model.border.top.width, 0.0);
    assert_eq!(
        border_none.layout.box_model.border.top.style,
        BorderStyle::None
    );
}

#[test]
fn text_align_right_in_flex_cell() {
    let html = r#"<div style="display: flex"><div style="width: 200pt; text-align: right">Right</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(pdf_str.contains("Right"), "Should contain the text 'Right'");
    // The text x-position should be offset from left (not at left margin)
    assert!(
        pdf_str.contains("Td") || pdf_str.contains("Tm"),
        "Should have text positioning operator"
    );
}

#[test]
fn text_align_center_in_flex_cell() {
    let html = r#"<div style="display: flex"><div style="width: 200pt; text-align: center">Center</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Center"),
        "Should contain the text 'Center'"
    );
    assert!(
        pdf_str.contains("Td") || pdf_str.contains("Tm"),
        "Should have text positioning operator"
    );
}

#[test]
fn absolute_position_offset() {
    let html = r#"<div style="position: absolute; left: 100pt; top: 50pt">Absolute</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Absolute"),
        "Should contain positioned text"
    );
}

#[test]
fn nested_float_right_aligns_to_containing_block_edge() {
    let html = r#"
        <style>
            @page { size: 400pt 200pt; margin: 0; }
            html, body { margin: 0; }
        </style>
        <div style="width: 300pt; height: 100pt; background: #ff0000">
            <div style="float: right; width: 75pt; height: 50pt; background: #00ff00">
                Floated
            </div>
        </div>
    "#;
    let pdf = crate::HtmlConverter::new()
        .compress(false)
        .convert(html)
        .unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    let containing_block = rect_after_color(&pdf_str, "1 0 0 rg")
        .expect("containing block background should be painted");
    let floated_block =
        rect_after_color(&pdf_str, "0 1 0 rg").expect("floated block background should be painted");

    let containing_right = containing_block.0 + containing_block.2;
    let floated_right = floated_block.0 + floated_block.2;
    assert!(
        (floated_right - containing_right).abs() < 0.01,
        "float:right should align to its containing block's right edge: containing={containing_block:?}, floated={floated_block:?}"
    );
}

#[test]
fn radial_gradient_clipped() {
    let html = r#"<div style="background: radial-gradient(red, blue); border-radius: 10pt; height: 50pt">Radial</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("/ShadingType 3"),
        "Should have radial shading"
    );
    assert!(
        pdf_str.contains("W n"),
        "Should clip radial gradient to border-radius"
    );
}

#[test]
fn opacity_renders_extgstate() {
    let html = r#"<div style="opacity: 0.5">Transparent</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("/ExtGState"),
        "Should have ExtGState for opacity"
    );
    assert!(pdf_str.contains("gs\n"), "Should apply graphics state");
}

#[test]
fn box_shadow_renders() {
    let html = r#"<div style="box-shadow: 2pt 2pt 0 #888888; height: 30pt">Shadow</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // Box shadow renders as a filled rectangle behind the element
    assert!(
        pdf_str.contains("re\nf\n") || pdf_str.contains("f\n"),
        "Should have fill for box shadow"
    );
    assert!(pdf_str.contains("Shadow"), "Should contain the text");
}

// --- Coverage tests for uncovered lines ---

#[test]
fn position_absolute_block_x() {
    // Covers line 93, 128: Position::Absolute uses margin.left + offset_left
    let html =
        r#"<div style="position: absolute; left: 50pt; background-color: cyan">Absolute</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Absolute"),
        "Should render absolute positioned text"
    );
}

#[test]
fn position_relative_block_x() {
    // Covers lines 119-120, 129: Position::Relative block_x calculation
    let html =
        r#"<div style="position: relative; left: 30pt; background-color: lime">Relative</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Relative"),
        "Should render relative positioned text"
    );
}

#[test]
fn float_right_positioning() {
    // Covers line 131: Float::Right block_x = margin.left + available_width - render_w
    let html = r#"<div style="float: right; width: 100pt">Float right</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Float right"),
        "Should render float right text"
    );
}

#[test]
fn per_side_border_rendering() {
    // Four differently-colored solid sides meet at diagonal miters, so each
    // side paints as a filled trapezoid (`rg` fill) rather than a centerline
    // stroke. This keeps adjacent-color corners on the 45° seam (CSS
    // Backgrounds §6.2) instead of leaving a single-color overlap.
    let html = r#"<div style="border-top: 2pt solid red; border-right: 3pt solid green; border-bottom: 1pt solid blue; border-left: 4pt solid black; width: 200pt; height: 50pt">Borders</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // Each side's color is now a fill (`rg`), not a stroke (`RG`).
    assert!(
        pdf_str.contains("1 0 0 rg"),
        "Should have red top border fill"
    );
    assert!(
        pdf_str.contains("0 0 0 rg"),
        "Should have black left border fill"
    );
    // Trapezoid corners: the miter geometry closes each side with `h\nf`.
    assert!(
        pdf_str.contains("h\nf\n"),
        "Per-side miter borders should fill closed trapezoids"
    );
}

#[test]
fn center_align_with_inline_span() {
    // Covers line 487: TextAlign::Center branch in TextBlock with inline padding
    let html = r#"<p style="text-align: center"><span style="background-color: yellow; padding: 4pt">Centered Span</span></p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Centered Span"),
        "Should render centered span text"
    );
    assert!(
        pdf_str.contains("1 1 0 rg"),
        "Should have yellow background fill"
    );
}

#[test]
fn right_align_with_inline_span() {
    // Covers line 491: TextAlign::Right branch in TextBlock with inline padding
    let html = r#"<p style="text-align: right"><span style="background-color: lime; padding: 4pt">Right Span</span></p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Right Span"),
        "Should render right-aligned span text"
    );
}

#[test]
fn letter_spacing_in_text_rendering() {
    // Covers line 519 (letter-spacing sets Tc operator)
    let html = r#"<p style="letter-spacing: 2pt">Spaced out</p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Tc\n"),
        "Letter spacing should produce Tc operator"
    );
    assert!(
        pdf_str.contains("0 Tc\n"),
        "Letter spacing should be reset to 0"
    );
}

#[test]
fn underline_and_strikethrough_rendering() {
    // Covers underline and strikethrough draw lines with font-size-relative thickness
    let html = r#"<p><span style="text-decoration: underline">Under</span> <span style="text-decoration: line-through">Strike</span></p>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // Both underline and strikethrough produce filled decoration rectangles.
    let decoration_count = filled_rect_count(&pdf_str);
    assert!(
        decoration_count >= 2,
        "Should have at least 2 filled decoration rectangles (underline + strikethrough), got {decoration_count}"
    );
}

#[test]
fn table_cell_rowspan_continuation() {
    // Covers lines 667, 669: rowspan > 1 cell rendering
    let html = r#"<table>
            <tr><td rowspan="2">Spanning</td><td>A</td></tr>
            <tr><td>B</td></tr>
        </table>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(pdf_str.contains("Spanning"), "Should render rowspan cell");
    assert!(pdf_str.contains("A"), "Should render first row cell");
    assert!(pdf_str.contains("B"), "Should render second row cell");
}

#[test]
fn table_cell_nested_table_renders_inner_content() {
    let html = r#"
            <table>
                <tr>
                    <td>
                        Outer
                        <table>
                            <tr><td>Inner</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(pdf_str.contains("Outer"), "Should render outer cell text");
    assert!(
        pdf_str.contains("Inner"),
        "Should render nested table cell text"
    );
}

#[test]
fn flexrow_container_gradient() {
    // Covers lines 742, 744, 753, 848-874: FlexRow linear gradient with border-radius
    let html = r#"<div style="display: flex; background: linear-gradient(to right, red, blue); border-radius: 5pt"><div>Gradient Flex</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Gradient Flex"),
        "Should render flex content"
    );
    // Linear gradient produces shading reference
    assert!(
        has_axial_gradient_pattern(&pdf_str),
        "Should paint an axial shading pattern"
    );
}

#[test]
fn flexrow_non_uniform_border() {
    // Covers lines 790, 798, 804-805, 939-969: FlexRow non-uniform per-side border
    let html = r#"<div style="display: flex; border-top: 2pt solid red; border-right: 3pt solid green; border-bottom: 1pt solid blue; border-left: 4pt solid black"><div>Flex Borders</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // The flex item shrinks to content; the words may render as separate
    // text-show operators, so assert each word is present rather than the
    // joined string.
    assert!(
        pdf_str.contains("(Flex)") && pdf_str.contains("(Borders)"),
        "Should render flex content"
    );
    // Non-uniform solid borders produce per-side fill bands.
    assert!(
        pdf_str.contains("1 0 0 rg"),
        "Should have red fill band for top"
    );
}

#[test]
fn flexrow_cell_inline_background_with_border_radius() {
    // Covers lines 852-903, 982-1001: FlexRow cell bg with border-radius and gradient
    let html = r#"<div style="display: flex"><div style="background-color: orange; border-radius: 8pt; width: 100pt">Cell BG</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(pdf_str.contains("Cell BG"), "Should render cell text");
    // Orange background: 1 0.647.. 0 rg — check for the fill command
    assert!(
        pdf_str.contains("rg\n"),
        "Should have fill color for cell background"
    );
}

#[test]
fn flexrow_cell_text_alignment() {
    // Covers lines 918-969, 1084, 1090: FlexRow cell text-align center and right
    let html = r#"<div style="display: flex">
            <div style="width: 200pt; text-align: center">Center</div>
            <div style="width: 200pt; text-align: right">Right</div>
        </div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("Center"),
        "Should render center-aligned text"
    );
    assert!(
        pdf_str.contains("Right"),
        "Should render right-aligned text"
    );
}

#[test]
fn render_cell_text_vertical_centering() {
    // Covers lines 1116-1123: render_cell_text vertical centering with bg + border-radius
    let run = TextRun {
        text: "Centered".to_string(),
        font_size: 14.0,
        background_color: Some(Color::rgb(255, 0, 0)),
        padding: EdgeSizes::axes(4.0, 2.0),
        border_radii: CornerRadii::circular(3.0),
        ..Default::default()
    };
    let cell = CellBox {
        content: crate::layout::cells::CellContent {
            lines: vec![TextLine {
                runs: vec![run],
                height: 16.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }],
            ..Default::default()
        },
        box_model: crate::layout::cells::CellBoxModel {
            content_insets: EdgeSizes::uniform(4.0),
            ..Default::default()
        },
        alignment: crate::layout::cells::CellAlignment {
            inline: TextAlign::Center,
            block: VerticalAlign::Middle,
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
        CellTextPlacement::new(PdfPoint::new(10.0, 200.0), 100.0),
        &mut text_context,
    );
    assert!(content.contains("Centered"), "Should render cell text");
    // Background with border-radius produces rounded rect
    assert!(
        content.contains("1 0 0 rg"),
        "Should have red inline background"
    );
}

/// A cell whose only line content is an atomic inline box carries no text on
/// that line, yet the box is line content (CSS 2.1 §9.2.2): the cell painter
/// must paint it the way the page-level line painter does instead of skipping
/// the line as empty.
#[test]
fn render_cell_text_paints_a_line_holding_only_an_inline_box() {
    let pdf = crate::HtmlConverter::new()
        .convert(
            r#"<table><tr><td><span style="display:inline-block;width:12pt;height:12pt;background:#ff0000"></span></td></tr></table>"#,
        )
        .expect("valid lone inline-block fixture");
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("1 0 0 rg"),
        "a cell holding only an inline-block must paint that box"
    );
}

/// CSS 2.1 Appendix E step 8: a relatively positioned inline-level box paints
/// in the positioned layer, above the in-flow content of its line, whatever
/// its source order. The cell painter defers such boxes exactly as the
/// page-level line painter does.
#[test]
fn render_cell_text_paints_relative_inline_boxes_above_in_flow_siblings() {
    let inline_run = |background: Color, rel_offset_x: f32| TextRun {
        font_size: 12.0,
        inline_box: Some(Box::new(crate::layout::engine::InlineBox {
            width: 20.0,
            height: 10.0,
            paint: crate::layout::engine::InlineBoxPaint {
                background_color: Some(background),
                ..Default::default()
            },
            rel_offset_x,
            ..Default::default()
        })),
        ..Default::default()
    };
    let cell = CellBox {
        content: crate::layout::cells::CellContent {
            lines: vec![TextLine {
                runs: vec![
                    TextRun {
                        text: "x".to_string(),
                        font_size: 12.0,
                        ..Default::default()
                    },
                    // Source order: the offset box first, then an in-flow box
                    // that overlaps its shifted position.
                    inline_run(Color::rgb(255, 0, 0), -6.0),
                    inline_run(Color::rgb(0, 0, 255), 0.0),
                ],
                height: 14.0,
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            }],
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
        CellTextPlacement::new(PdfPoint::new(10.0, 200.0), 100.0),
        &mut text_context,
    );
    let relative = content
        .find("1 0 0 rg")
        .expect("the relatively positioned box is painted");
    let in_flow = content
        .find("0 0 1 rg")
        .expect("the in-flow box is painted");
    assert!(
        in_flow < relative,
        "the relatively positioned box must paint after its in-flow sibling:\n{content}"
    );
}

#[test]
fn coalesce_text_runs_border_radii_comparison() {
    // Runs merge only when their full corner geometry matches.
    let run_a = TextRun {
        text: "Hello ".to_string(),
        background_color: Some(Color::rgb(255, 255, 0)),
        padding: EdgeSizes::axes(2.0, 1.0),
        border_radii: CornerRadii::circular(4.0),
        ..Default::default()
    };
    let run_b = TextRun {
        text: "World".to_string(),
        background_color: Some(Color::rgb(255, 255, 0)),
        padding: EdgeSizes::axes(2.0, 1.0),
        border_radii: CornerRadii::circular(8.0),
        ..Default::default()
    };
    let merged = crate::text::coalesce_text_runs(&[run_a.clone(), run_b.clone()]);
    // Different corner radii should prevent merging.
    assert_eq!(
        merged.len(),
        2,
        "runs with different corner radii should not merge"
    );
    // Identical corner radii should merge.
    let mut run_b_same = run_b;
    run_b_same.border_radii = CornerRadii::circular(4.0);
    let merged2 = crate::text::coalesce_text_runs(&[run_a, run_b_same]);
    assert_eq!(
        merged2.len(),
        1,
        "runs with identical corner radii should merge"
    );
}
