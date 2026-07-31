#[test]
fn linear_gradient_uses_shading() {
    let html = r#"<div style="background: linear-gradient(to bottom, red, blue); height: 50pt">Gradient</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("/ShadingType 2"),
        "Linear gradient should produce ShadingType 2 (axial)"
    );
}

#[test]
fn radial_gradient_uses_shading_in_pdf() {
    let html =
        r#"<div style="background: radial-gradient(red, blue); height: 50pt">Gradient</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let content = String::from_utf8_lossy(&pdf);
    assert!(
        content.contains("/ShadingType 3"),
        "Radial gradient should produce ShadingType 3"
    );
}

#[test]
fn gradient_clipped_to_border_radius() {
    let html = r#"<div style="background: linear-gradient(to bottom, red, blue); border-radius: 10pt; height: 50pt">Clipped</div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        has_axial_gradient_pattern(&pdf_str),
        "Should paint an axial shading pattern"
    );
    assert!(
        pdf_str.contains("W n"),
        "Should have clip operator for border-radius"
    );
}

#[test]
fn svg_background_clipped_to_border_radius() {
    let html = r#"<div style="width: 200pt; height: 80pt; border-radius: 12pt; background: url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%221%22 height=%221%22%3E%3Crect width=%221%22 height=%221%22 fill=%22red%22/%3E%3C/svg%3E') no-repeat"></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains(" c\n"),
        "Rounded clip should use Bezier curves"
    );
    assert!(pdf_str.contains("W n"), "SVG background should be clipped");
}

#[test]
fn svg_background_percent_size_uses_positioning_area() {
    let tree = crate::parser::svg::SvgTree {
        width: 1.0,
        height: 1.0,
        width_attr: None,
        height_attr: None,
        preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
        view_box: None,
        defs: Default::default(),
        children: vec![crate::parser::svg::SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            rx: 0.0,
            ry: 0.0,
            style: crate::parser::svg::SvgStyle {
                fill: crate::parser::svg::SvgPaint::Color(crate::types::Color::rgb(255, 0, 0)),
                ..Default::default()
            },
        }],
        text_ctx: crate::parser::svg::SvgTextContext::default(),
        source_markup: None,
    };
    let mut content = String::new();
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0usize;
    render_svg_background(
        &mut content,
        &tree,
        PdfBackgroundResources::new(
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            None,
        ),
        PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
            SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
            SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
            CornerRadii::ZERO,
            0.0,
            BackgroundSize::Explicit {
                width: 50.0,
                height: Some(25.0),
                width_is_percent: true,
                height_is_percent: true,
            },
            BackgroundPosition::default(),
            BackgroundRepeat::NoRepeat,
        )),
    );
    assert!(
        content.contains("0 0 100 25 re W n"),
        "Expected SVG tile viewport to resolve against the 200pt by 100pt positioning area"
    );
    // Both background-size values are explicit (50% 25%), so the image is
    // scaled to exactly that box, ignoring its intrinsic 1:1 ratio
    // (css-backgrounds-3 §3.9): the 1x1 SVG stretches to the full 100x25 tile.
    assert!(
        content.contains("100 0 0 25 0 0 cm"),
        "Expected explicit two-value background-size to stretch the SVG to the 100pt by 25pt tile"
    );
}

#[test]
fn svg_background_single_percent_size_preserves_aspect_ratio() {
    let tree = crate::parser::svg::SvgTree {
        width: 2.0,
        height: 1.0,
        width_attr: None,
        height_attr: None,
        preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
        view_box: None,
        defs: Default::default(),
        children: vec![crate::parser::svg::SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 1.0,
            rx: 0.0,
            ry: 0.0,
            style: crate::parser::svg::SvgStyle {
                fill: crate::parser::svg::SvgPaint::Color(crate::types::Color::rgb(255, 0, 0)),
                ..Default::default()
            },
        }],
        text_ctx: crate::parser::svg::SvgTextContext::default(),
        source_markup: None,
    };
    let mut content = String::new();
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0usize;
    render_svg_background(
        &mut content,
        &tree,
        PdfBackgroundResources::new(
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            None,
        ),
        PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
            SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
            SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
            CornerRadii::ZERO,
            0.0,
            BackgroundSize::Explicit {
                width: 50.0,
                height: None,
                width_is_percent: true,
                height_is_percent: false,
            },
            BackgroundPosition::default(),
            BackgroundRepeat::NoRepeat,
        )),
    );
    assert!(
        content.contains("50 0 0 50 0 0 cm"),
        "Single-value background-size should preserve intrinsic aspect ratio"
    );
}

#[test]
fn svg_background_uses_outer_clip_box() {
    let tree = crate::parser::svg::SvgTree {
        width: 1.0,
        height: 1.0,
        width_attr: None,
        height_attr: None,
        preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
        view_box: None,
        defs: Default::default(),
        children: vec![crate::parser::svg::SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            rx: 0.0,
            ry: 0.0,
            style: crate::parser::svg::SvgStyle {
                fill: crate::parser::svg::SvgPaint::Color(crate::types::Color::rgb(255, 0, 0)),
                ..Default::default()
            },
        }],
        text_ctx: crate::parser::svg::SvgTextContext::default(),
        source_markup: None,
    };
    let mut content = String::new();
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0usize;
    render_svg_background(
        &mut content,
        &tree,
        PdfBackgroundResources::new(
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            None,
        ),
        PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
            SvgViewportBox::new(20.0, 10.0, 160.0, 80.0),
            SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
            CornerRadii::ZERO,
            0.0,
            BackgroundSize::Auto,
            BackgroundPosition::default(),
            BackgroundRepeat::NoRepeat,
        )),
    );
    assert!(
        content.contains("0 0 200 100 re\nW n"),
        "Clip box should stay on the outer element box, not shrink to the origin box"
    );
}

fn render_fragmented_jpeg_background(
    clip_box: SvgViewportBox,
    repeat: BackgroundRepeat,
) -> (String, Vec<ImageRef>) {
    let tree = crate::layout::images::build_raster_background_tree(TEST_JPEG_DATA_URI)
        .expect("test JPEG should produce a raster background tree");
    let mut content = String::new();
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0usize;
    render_svg_background(
        &mut content,
        &tree,
        PdfBackgroundResources::new(
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            None,
        ),
        PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
            SvgViewportBox::new(0.0, 0.0, 100.0, 100.0),
            clip_box,
            CornerRadii::ZERO,
            0.0,
            BackgroundSize::Explicit {
                width: 20.0,
                height: Some(20.0),
                width_is_percent: false,
                height_is_percent: false,
            },
            BackgroundPosition::default(),
            repeat,
        )),
    );
    (content, page_images)
}

#[test]
fn no_repeat_background_outside_fragment_allocates_no_pdf_resources() {
    let (content, page_images) = render_fragmented_jpeg_background(
        SvgViewportBox::new(0.0, 0.0, 100.0, 50.0),
        BackgroundRepeat::NoRepeat,
    );

    assert!(content.is_empty());
    assert!(page_images.is_empty());
}

#[test]
fn no_repeat_background_intersecting_fragment_retains_its_pdf_resource() {
    let (content, page_images) = render_fragmented_jpeg_background(
        SvgViewportBox::new(0.0, 75.0, 100.0, 25.0),
        BackgroundRepeat::NoRepeat,
    );

    assert!(content.contains(" Do\n"));
    assert_eq!(page_images.len(), 1);
}

#[test]
fn repeated_background_is_not_culled_by_its_first_tile() {
    let (content, page_images) = render_fragmented_jpeg_background(
        SvgViewportBox::new(0.0, 0.0, 100.0, 50.0),
        BackgroundRepeat::Repeat,
    );

    assert!(content.contains(" Do\n"));
    assert!(!page_images.is_empty());
}

#[test]
fn flexrow_with_gradient() {
    let html = r#"<div style="display: flex; background: linear-gradient(to right, red, blue); height: 40pt"><div style="width: 100pt">A</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        pdf_str.contains("/ShadingType 2"),
        "FlexRow with linear-gradient should produce ShadingType 2"
    );
}

#[test]
fn flexrow_cell_background() {
    let html = r#"<div style="display: flex"><div style="width: 100pt; background-color: yellow">Yellow</div><div style="width: 100pt">Plain</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // Yellow = 1 1 0 rg
    assert!(
        pdf_str.contains("1 1 0 rg"),
        "Should have yellow fill color for cell background"
    );
    assert!(
        pdf_str.contains("re\nf\n"),
        "Should have rectangle fill for cell background"
    );
}

#[test]
fn flexrow_cell_border_radius() {
    let html = r#"<div style="display: flex"><div style="width: 100pt; background-color: red; border-radius: 8pt">Round</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // Rounded rect uses Bezier curve commands (c)
    assert!(pdf_str.contains("1 0 0 rg"), "Should have red fill");
    assert!(
        pdf_str.contains(" c\n"),
        "Should have Bezier curve for border-radius"
    );
}

#[test]
fn flexrow_cell_gradient() {
    let html = r#"<div style="display: flex"><div style="width: 150pt; background: linear-gradient(to bottom, green, yellow)">Grad</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    assert!(
        has_axial_gradient_pattern(&pdf_str),
        "Should paint an axial shading pattern for the cell gradient"
    );
    assert!(
        pdf_str.contains("/ShadingType 2"),
        "Cell gradient should use axial shading"
    );
}

#[test]
fn flexrow_border_radius_background() {
    let html = r#"<div style="display: flex; border-radius: 10pt; background-color: #cccccc"><div style="width: 100pt">Rounded</div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // Rounded background uses Bezier curves, not re
    assert!(
        pdf_str.contains(" c\n"),
        "Should have Bezier curves for rounded background"
    );
    assert!(pdf_str.contains("f\n"), "Should have fill command");
}

#[test]
fn inline_span_border_radius() {
    let html = r#"<div style="display: flex"><div style="width: 300pt"><p><span style="background-color: yellow; border-radius: 4pt; padding: 2pt">Tag</span> text</p></div></div>"#;
    let nodes = parse_html(html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);
    // Inline span with border-radius should produce rounded rect path + fill
    assert!(
        pdf_str.contains("1 1 0 rg"),
        "Should have yellow fill for span bg"
    );
}

#[test]
fn root_svg_background_renders_in_pdf() {
    use crate::parser::css::parse_stylesheet;

    let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
    let rules = parse_stylesheet(css);
    let nodes = parse_html("<p>text</p>").unwrap();
    let pages =
        crate::layout::engine::layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);

    assert!(
        pdf_str.contains("1 0 0 rg"),
        "Expected red SVG background fill"
    );
}

#[test]
fn root_svg_background_viewbox_only_renders_in_pdf() {
    use crate::parser::css::parse_stylesheet;

    let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
    let rules = parse_stylesheet(css);
    let nodes = parse_html("<p>text</p>").unwrap();
    let pages =
        crate::layout::engine::layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);

    assert!(
        pdf_str.contains("1 0 0 rg"),
        "Expected viewBox-only SVG background to render"
    );
}

#[test]
fn root_svg_background_with_gradient_registers_shading_resources() {
    use crate::parser::css::parse_stylesheet;

    let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 10'%3E%3Cdefs%3E%3ClinearGradient id='g' x1='0' y1='0' x2='20' y2='0' gradientUnits='userSpaceOnUse'%3E%3Cstop offset='0' stop-color='%23f00'/%3E%3Cstop offset='1' stop-color='%2300f'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect width='20' height='10' fill='url(%23g)'/%3E%3C/svg%3E\"); background-size: cover; }";
    let rules = parse_stylesheet(css);
    let nodes = parse_html("<p>text</p>").unwrap();
    let pages =
        crate::layout::engine::layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);

    assert!(
        pdf_str.contains("/ShadingType 2"),
        "Expected gradient SVG background to emit an axial shading resource"
    );
}

#[test]
fn table_cell_nested_background_block_renders_image_xobject() {
    let _loader = crate::layout::images::trusted_scope();
    let path = write_test_png_file("table-cell-pdf-bg", &build_minimal_test_png());
    let html = format!(
        r#"<table><tr><td><div style="display: flex; width: 40pt; aspect-ratio: 1 / 1; background-image: url('{path}'); background-repeat: no-repeat;"></div></td></tr></table>"#
    );
    let nodes = parse_html(&html).unwrap();
    let pages = layout(&nodes, PageSize::A4, Margin::default());
    let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
    let pdf_str = String::from_utf8_lossy(&pdf);

    assert!(
        pdf_str.contains("BI\n"),
        "Expected nested table-cell background block to emit an inline image"
    );
    assert!(
        pdf_str.contains("EI\n"),
        "Expected nested table-cell background block to terminate the inline image"
    );
}

#[test]
fn nested_text_block_padding_top_offsets_text() {
    let lines = vec![test_text_line(vec![test_text_run("Nested")])];
    let custom_fonts = HashMap::new();
    let prepared_custom_fonts = PreparedCustomFonts::new();
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0usize;
    let mut page_ext_gstates = Vec::new();
    let mut bg_alpha_counter = 0usize;
    let mut annotations = Vec::new();

    let mut without_padding = String::new();
    {
        let mut without_padding_context = PageRenderContext::new(
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
        render_nested_text_block(
            &mut without_padding,
            NestedTextBlock {
                lines: &lines,
                clips: false,
                text_align: TextAlign::Left,
                padding: EdgeSizes::ZERO,
                border: LayoutBorder::default(),
                block_width: Some(80.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radii: CornerRadii::ZERO,
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(PdfPoint::new(10.0, 100.0), PdfPoint::new(10.0, 100.0), 80.0),
            &mut without_padding_context,
        );
    }

    let mut with_padding = String::new();
    let mut with_padding_context = PageRenderContext::new(
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
    render_nested_text_block(
        &mut with_padding,
        NestedTextBlock {
            lines: &lines,
            clips: false,
            text_align: TextAlign::Left,
            padding: EdgeSizes::new(12.0, 0.0, 0.0, 0.0),
            border: LayoutBorder::default(),
            block_width: Some(80.0),
            block_height: None,
            background_color: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            background_clip: BackgroundClip::Border,
            background_blur_canvas_box: None,
            border_radii: CornerRadii::ZERO,
            text_indent: 0.0,
        },
        NestedLayoutFrame::new(PdfPoint::new(10.0, 100.0), PdfPoint::new(10.0, 100.0), 80.0),
        &mut with_padding_context,
    );

    let without_padding_y = first_td_y(&without_padding).unwrap();
    let with_padding_y = first_td_y(&with_padding).unwrap();
    assert!((without_padding_y - with_padding_y - 12.0).abs() < 0.01);
}
