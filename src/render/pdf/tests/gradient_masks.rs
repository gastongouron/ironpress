#[test]
fn build_shading_function_two_stops() {
    let stops = PdfGradientStops::unit([(0.0, (1.0, 0.0, 0.0)), (1.0, (0.0, 0.0, 1.0))]).unwrap();
    let result = build_shading_function(&stops);
    assert!(result.contains("/FunctionType 2"));
    assert!(result.contains("/C0 [1 0 0]"));
    assert!(result.contains("/C1 [0 0 1]"));
}

#[test]
fn build_shading_function_three_stops() {
    let stops = PdfGradientStops::unit([
        (0.0, (1.0, 0.0, 0.0)),
        (0.5, (0.0, 1.0, 0.0)),
        (1.0, (0.0, 0.0, 1.0)),
    ])
    .unwrap();
    let result = build_shading_function(&stops);
    assert!(result.contains("/FunctionType 3"));
    assert!(result.contains("/Bounds [0.5]"));
    assert!(result.contains("/Encode [0 1 0 1]"));
}

#[test]
fn gradient_raster_size_preserves_requested_dpi_without_an_edge_cap() {
    assert_eq!(
        gradient_raster_dimensions(72.0, 36.0, 96.0),
        Some(RasterDimensions {
            width: 96,
            height: 48,
        }),
    );
    assert_eq!(
        gradient_raster_dimensions(3_000.0, 1_600.0, 96.0),
        Some(RasterDimensions {
            width: 4_000,
            height: 2_133,
        }),
    );
    assert_eq!(
        gradient_raster_dimensions(72.0, 36.0, f32::NAN),
        Some(RasterDimensions {
            width: 1,
            height: 1,
        }),
    );
    assert!(gradient_raster_dimensions(f32::MAX, 1.0, 96.0).is_none());
}

#[test]
fn tiny_positive_mask_geometry_keeps_its_exact_sampling_scale() {
    let grid = MaskRasterGrid::new(
        RasterDimensions {
            width: 10,
            height: 1,
        },
        5e-7,
        5e-7,
    )
    .unwrap();
    let source = MaskSource::Radial(RadialGradient {
        ramp: gradient_ramp(
            [
                gradient_stop(0.0, crate::types::Color::rgba8(0, 0, 0, 0)),
                gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 0, 255)),
            ],
            false,
        ),
        center: crate::style::computed::RadialPoint::new(
            crate::style::computed::RadialPos::Fraction(0.0),
            crate::style::computed::RadialPos::Fraction(0.5),
        ),
        shape: RadialShape::Circle,
        extent: RadialExtent::FarthestCorner,
        radius: Some(2.5e-7),
        radii: None,
        layer_box: Default::default(),
    });

    let coverage = rasterize_mask_coverage(&source, MaskMode::Alpha, grid.full_window()).unwrap();

    assert_eq!(
        coverage,
        vec![26, 77, 128, 179, 230, 255, 255, 255, 255, 255]
    );
}

#[test]
fn gradient_mask_windows_match_one_global_sampling_grid() {
    let grid = MaskRasterGrid::new(
        RasterDimensions {
            width: 4_097,
            height: 3,
        },
        768.1875,
        0.5625,
    )
    .unwrap();
    let source = MaskSource::Conic(test_conic_gradient(
        [
            gradient_stop(0.0, crate::types::Color::rgba8(0, 0, 0, 0)),
            gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 0, 255)),
        ],
        false,
    ));
    let full = rasterize_mask_coverage(&source, MaskMode::Alpha, grid.full_window()).unwrap();
    let tiled = rasterize_mask_grid_by_tiles(grid, MAX_RASTER_TILE_EDGE, |window| {
        rasterize_mask_coverage(&source, MaskMode::Alpha, window)
    })
    .unwrap();

    assert_eq!(tiled, full);
}

#[test]
fn default_repeated_linear_mask_uses_the_full_origin_box_as_its_tile() {
    let grid = MaskRasterGrid::new(
        RasterDimensions {
            width: 750,
            height: 500,
        },
        180.0,
        120.0,
    )
    .unwrap();
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(0.0, 0.0, 180.0, 120.0),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );
    let layer = MaskLayer {
        source: MaskLayerSource::Linear(LinearGradient {
            angle: 90.0,
            ramp: gradient_ramp(
                [
                    gradient_stop(0.0, crate::types::Color::rgba8(0, 0, 0, 255)),
                    gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 0, 0)),
                ],
                false,
            ),
            layer_box: Default::default(),
        }),
        mode: MaskMode::Alpha,
        layer_box: Default::default(),
        origin: crate::style::computed::ShapeBox::Border,
        clip: crate::style::computed::ShapeBox::Border,
        composite: MaskComposite::Add,
    };

    let coverage = rasterize_mask_layer(
        &layer,
        grid.full_window(),
        geometry,
        &crate::parser::svg::SvgDefs::default(),
    )
    .unwrap();
    let row = 250 * 750;

    assert_eq!(coverage[row], 255);
    assert!(coverage[row + 375].abs_diff(127) <= 1);
    assert_eq!(coverage[row + 749], 0);
}

#[test]
fn svg_mask_windows_keep_global_geometry_at_tile_boundaries() {
    let grid = MaskRasterGrid::new(
        RasterDimensions {
            width: 4_097,
            height: 3,
        },
        768.1875,
        0.5625,
    )
    .unwrap();
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4097" height="3" viewBox="0 0 4097 3"><rect x="2047.25" y="0" width="1.5" height="3" fill="white"/></svg>"#;
    let full = rasterize_svg_mask_coverage(svg, MaskMode::Alpha, grid.full_window()).unwrap();
    let tiled = rasterize_mask_grid_by_tiles(grid, MAX_RASTER_TILE_EDGE, |window| {
        rasterize_svg_mask_coverage(svg, MaskMode::Alpha, window)
    })
    .unwrap();

    assert_eq!(tiled, full);
    assert!(full[2_047] > 0);
    assert!(full[2_048] > 0);
}

#[test]
fn oversized_mask_layer_is_windowed_without_truncating_its_source_tile() {
    let grid = MaskRasterGrid::new(
        RasterDimensions {
            width: 4_097,
            height: 10,
        },
        409.7,
        1.0,
    )
    .unwrap();
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(0.0, 0.0, grid.width_pt, grid.height_pt),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );
    let layer = MaskLayer {
            source: MaskLayerSource::Svg(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="white"/></svg>"#.to_vec(),
            ),
            mode: MaskMode::Alpha,
            layer_box: crate::style::computed::GradientLayerBox {
                size: Some(BackgroundSize::Explicit {
                    width: 250.0,
                    height: Some(1.0),
                    width_is_percent: false,
                    height_is_percent: false,
                }),
                position: Some(BackgroundPosition {
                    x: 150.0,
                    y: 0.0,
                    x_is_percent: false,
                    y_is_percent: false,
                }),
                repeat: Some(BackgroundRepeat::NoRepeat),
                ..Default::default()
            },
            origin: crate::style::computed::ShapeBox::Border,
            clip: crate::style::computed::ShapeBox::Border,
            composite: MaskComposite::Add,
        };
    let layers = [layer];
    let defs = crate::parser::svg::SvgDefs::default();
    let full = rasterize_mask_layers(&layers, grid.full_window(), geometry, &defs).unwrap();
    let tiled = rasterize_mask_grid_by_tiles(grid, MAX_RASTER_TILE_EDGE, |window| {
        rasterize_mask_layers(&layers, window, geometry, &defs)
    })
    .unwrap();
    let source_width = grid.dimensions_for_points(250.0, 1.0).unwrap().width;
    let destination_x = (150.0 * grid.scale_x()).round() as usize;
    let source_end = destination_x + source_width as usize;
    let row = 5 * grid.pixels.width as usize;

    assert!(source_width > MAX_RASTER_TILE_EDGE);
    assert_eq!(tiled, full);
    assert_eq!(full[row + destination_x - 1], 0);
    assert_eq!(full[row + destination_x], 255);
    assert_eq!(full[row + source_end - 1], 255);
    assert_eq!(full[row + source_end], 0);
}

#[test]
fn subpixel_repeating_mask_work_is_bounded_by_the_output_window() {
    let grid = MaskRasterGrid::new(
        RasterDimensions {
            width: 128,
            height: 1,
        },
        128.0,
        1.0,
    )
    .unwrap();
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(0.0, 0.0, grid.width_pt, grid.height_pt),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );
    let layer = MaskLayer {
        source: MaskLayerSource::Linear(LinearGradient {
            angle: 90.0,
            ramp: gradient_ramp(
                [
                    gradient_stop(0.0, crate::types::Color::rgba8(0, 0, 0, 255)),
                    gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 0, 255)),
                ],
                false,
            ),
            layer_box: Default::default(),
        }),
        mode: MaskMode::Alpha,
        layer_box: crate::style::computed::GradientLayerBox {
            size: Some(BackgroundSize::Explicit {
                width: 1e-9,
                height: Some(1.0),
                width_is_percent: false,
                height_is_percent: false,
            }),
            position: Some(BackgroundPosition {
                x: 1e30,
                x_is_percent: false,
                ..Default::default()
            }),
            repeat: Some(BackgroundRepeat::RepeatX),
            ..Default::default()
        },
        origin: crate::style::computed::ShapeBox::Border,
        clip: crate::style::computed::ShapeBox::Border,
        composite: MaskComposite::Add,
    };

    let coverage = rasterize_mask_layer(
        &layer,
        grid.full_window(),
        geometry,
        &crate::parser::svg::SvgDefs::default(),
    )
    .unwrap();

    assert_eq!(coverage, vec![255; 128]);
}

#[test]
fn subpixel_repeating_gradient_uses_one_pdf_pattern_cell() {
    let gradient = LinearGradient {
        angle: 90.0,
        ramp: gradient_ramp(
            [
                gradient_stop(0.0, crate::types::Color::rgba8(255, 0, 0, 255)),
                gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 255, 255)),
            ],
            false,
        ),
        layer_box: crate::style::computed::GradientLayerBox {
            size: Some(BackgroundSize::Explicit {
                width: 1e-9,
                height: Some(10.0),
                width_is_percent: false,
                height_is_percent: false,
            }),
            position: Some(BackgroundPosition {
                x: 1e30,
                x_is_percent: false,
                ..Default::default()
            }),
            repeat: Some(BackgroundRepeat::RepeatX),
            ..Default::default()
        },
    };
    let mut content = String::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0;
    let mut writer = PdfWriter::new();
    let resolved = gradient_layer_pattern(
        &gradient.layer_box,
        LayerPaintArea::single(PdfRect::new(0.0, 0.0, 100.0, 10.0)),
    )
    .unwrap();
    let pattern_geometry = resolved
        .pdf_pattern(PdfRect::new(0.0, 0.0, 1e-9, 10.0))
        .unwrap();
    assert!(
        pattern_geometry.transform.translation.x > -1e-9
            && pattern_geometry.transform.translation.x < 100.0,
        "{pattern_geometry:?}"
    );
    render_linear_gradient(
        &mut content,
        &gradient,
        GradientBackdrop::default(),
        LayerPaintArea::single(PdfRect::new(0.0, 0.0, 100.0, 10.0)),
        &mut shadings,
        &mut shading_counter,
        &mut writer,
        &mut Vec::new(),
    );

    assert_eq!(writer.tiling_patterns.len(), 1);
    assert_eq!(shadings.len(), 1);
    assert_eq!(content.matches(" Do\n").count(), 1);
    assert!(content.len() < 256);
    let entry = &writer.tiling_patterns[0];
    let pattern = &writer.objects[entry.pattern_id - 1];
    let &PdfTilingPatternTarget::Form { object_id: form_id } = &entry.target else {
        assert!(false, "repeating gradients must use a local pattern form");
        return;
    };
    let form = &writer.objects[form_id - 1];
    assert!(pattern.contains("/PatternType 1"));
    assert!(pattern.contains("/XStep 0.000000001"));
    assert!(form.contains(&format!("/Pattern << /Cell {} 0 R >>", entry.pattern_id)));
    assert!(!pattern.contains("/Pattern <<"));
}

#[test]
fn local_form_resources_are_exact_and_acyclic() {
    let mut writer = PdfWriter::new();
    let pattern = PdfTilingPattern {
        bbox: PdfRect::new(0.0, 0.0, 10.0, 10.0),
        paint_box: PdfRect::new(0.0, 0.0, 80.0, 40.0),
        step: PdfVector::new(10.0, 10.0),
        transform: PdfMatrix::translate(PdfPoint::new(3.0, 4.0)),
        ..Default::default()
    };
    let form = writer
        .add_tiling_pattern("1 0 0 rg\n0 0 10 10 re f\n".to_owned(), pattern)
        .unwrap();
    let pattern_id = writer.tiling_patterns[0].pattern_id;
    let &PdfTilingPatternTarget::Form { object_id: form_id } = &writer.tiling_patterns[0].target
    else {
        assert!(false, "add_tiling_pattern must create a local form");
        return;
    };
    let group = writer.add_transparency_group_form(
        format!("q\n1 0 0 1 0 0 cm\n/{name} Do\nQ\n", name = form.name),
        PdfRect::new(0.0, 0.0, 80.0, 40.0),
    );
    let group_id = group.obj_id;
    let mut content = String::from("q\n0 -1 1 0 20 90 cm\n");
    content.push_str(&format!("1 0 0 1 5 7 cm\n/{} Do\nQ\n", group.name));
    writer.add_page(
        100.0,
        100.0,
        PagePaintStreams::document_only(&content),
        Vec::new(),
        vec![form, group],
        Vec::new(),
        Vec::new(),
    );

    let mut pdf = Vec::new();
    writer.finish_to_writer(&mut pdf, &[]).unwrap();
    let pdf = String::from_utf8_lossy(&pdf);
    let object = |id| {
        let start = pdf.find(&format!("{id} 0 obj\n")).unwrap();
        let end = start + pdf[start..].find("endobj").unwrap();
        &pdf[start..end]
    };
    let pattern_object = object(pattern_id);
    let form_object = object(form_id);
    let group_object = object(group_id);

    assert!(pattern_object.contains("/Resources <<  >>"));
    assert!(!pattern_object.contains("/Pattern <<"));
    assert!(!pattern_object.contains(&format!("{form_id} 0 R")));
    assert!(form_object.contains(&format!("/Pattern << /Cell {pattern_id} 0 R >>")));
    assert!(group_object.contains(&format!("/XObject << /Fm{form_id} {form_id} 0 R >>")));
    assert!(group_object.contains("/BBox [0 0 80 40]"));
    assert!(!group_object.contains("4096"));
    assert!(!group_object.contains(&format!("{group_id} 0 R")));
    assert_eq!(pdf.matches("/Pattern <<").count(), 1);
    assert!(content.contains(&format!("/Fm{group_id} Do")));
}

#[test]
fn alpha_conic_mask_uses_a_vector_luminosity_shading() {
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(12.0, 24.0, 150.0, 150.0),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );
    let source = MaskSource::Layers(vec![MaskLayer {
        source: MaskLayerSource::Conic(test_conic_gradient(
            [
                gradient_stop(0.0, crate::types::Color::rgba8(0, 0, 0, 255)),
                gradient_stop(0.25, crate::types::Color::rgba8(0, 0, 0, 255)),
                gradient_stop(0.25, crate::types::Color::rgba8(0, 0, 0, 0)),
                gradient_stop(0.5, crate::types::Color::rgba8(0, 0, 0, 0)),
            ],
            true,
        )),
        mode: MaskMode::MatchSource,
        layer_box: Default::default(),
        origin: crate::style::computed::ShapeBox::Border,
        clip: crate::style::computed::ShapeBox::Border,
        composite: MaskComposite::Add,
    }]);
    let mut writer = PdfWriter::new();
    let state = writer
        .add_mask_soft_mask(&source, MaskMode::MatchSource, geometry)
        .unwrap();

    assert!(
        writer
            .objects
            .iter()
            .all(|object| !object.contains("/Subtype /Image"))
    );
    assert_eq!(writer.conic_shadings.len(), 1);
    let form_id = writer.soft_mask_gstates[0].form_id;
    let stream = std::str::from_utf8(&writer.binary_objects[&form_id]).unwrap();
    assert!(stream.contains("/ShConic0 sh"));

    writer.add_page(
        200.0,
        200.0,
        PagePaintStreams::document_only(&format!(
            "q\n/{state} gs\n0 0 200 200 re f\nQ\n"
        )),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut pdf = Vec::new();
    writer.finish_to_writer(&mut pdf, &[]).unwrap();
    let pdf = String::from_utf8_lossy(&pdf);
    let start = pdf.find(&format!("{form_id} 0 obj\n")).unwrap();
    let form = &pdf[start..start + pdf[start..].find("endobj").unwrap()];
    assert!(form.contains("/Shading << /ShConic0 "));
    assert!(pdf.contains("/SMask << /Type /Mask /S /Luminosity"));
}

#[test]
fn alpha_repeating_radial_mask_uses_a_vector_luminosity_function() {
    let ramp = gradient_ramp(
        [
            gradient_stop(0.0, crate::types::Color::rgba8(255, 0, 0, 255)),
            gradient_stop(0.5, crate::types::Color::rgba8(255, 0, 0, 255)),
            gradient_stop(0.5, crate::types::Color::rgba8(0, 0, 255, 0)),
            gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 255, 0)),
        ],
        true,
    );
    let function = radial_alpha_function_gradient(&ramp, 24.0).unwrap();
    let calculator = function.calculator().unwrap();
    assert_eq!(function.period(), 1.0);
    assert!(calculator.contains("truncate sub"));
    assert!(calculator.contains("pop 1"));
    assert!(calculator.contains("pop 0"));

    let source = MaskSource::Layers(vec![MaskLayer {
        source: MaskLayerSource::Radial(RadialGradient {
            ramp,
            center: crate::style::computed::RadialPoint::default(),
            shape: RadialShape::Circle,
            extent: RadialExtent::FarthestCorner,
            radius: None,
            radii: None,
            layer_box: Default::default(),
        }),
        mode: MaskMode::MatchSource,
        layer_box: Default::default(),
        origin: crate::style::computed::ShapeBox::Border,
        clip: crate::style::computed::ShapeBox::Border,
        composite: MaskComposite::Add,
    }]);
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(0.0, 0.0, 150.0, 150.0),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );
    let mut writer = PdfWriter::new();
    let state = writer
        .add_mask_soft_mask(&source, MaskMode::MatchSource, geometry)
        .unwrap();

    assert!(state.starts_with("GSmask"));
    assert_eq!(writer.pdf_patterns.len(), 1);
    assert!(
        writer
            .objects
            .iter()
            .any(|object| object.contains("/FunctionType 4"))
    );
    assert!(
        writer
            .objects
            .iter()
            .any(|object| object.contains("/PatternType 2") && object.contains("/ShadingType 1"))
    );
    assert!(
        writer
            .objects
            .iter()
            .all(|object| !object.contains("/Subtype /Image"))
    );
}

#[test]
fn full_size_radial_add_layers_are_eligible_for_vector_alpha_composition() {
    let parent = crate::style::computed::ComputedStyle::default();
    let style = crate::style::computed::compute_style(
        crate::parser::dom::HtmlTag::Div,
        Some(
            "mask-image: radial-gradient(circle 38px at 55px 70px, #000 0 38px, transparent 39px), radial-gradient(circle 38px at 145px 70px, #000 0 38px, transparent 39px); mask-size: 100% 100%, 100% 100%; mask-repeat: no-repeat, no-repeat; mask-composite: add; -webkit-mask-image: radial-gradient(circle 38px at 145px 70px, #000 0 38px, transparent 39px), radial-gradient(circle 38px at 55px 70px, #000 0 38px, transparent 39px); -webkit-mask-size: 100% 100%, 100% 100%; -webkit-mask-repeat: no-repeat, no-repeat; -webkit-mask-composite: source-over;",
        ),
        &parent,
    );
    let Some(MaskSource::Layers(layers)) = style.mask_image else {
        panic!("expected the two computed mask layers");
    };
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(0.0, 0.0, 150.0, 105.0),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].composite, MaskComposite::Add);
    assert_eq!(layers[1].composite, MaskComposite::Add);
    for layer in &layers {
        let paint = MaskLayerPaint::resolve(layer, geometry).unwrap();
        assert_eq!(paint.tile, geometry.border_box);
        assert_eq!(paint.clip, geometry.border_box);
    }

    let mut writer = PdfWriter::new();
    assert!(
        writer
            .add_mask_soft_mask(&MaskSource::Layers(layers), MaskMode::MatchSource, geometry)
            .is_some()
    );
    assert!(
        writer
            .objects
            .iter()
            .all(|object| !object.contains("/Subtype /Image"))
    );
}

#[test]
fn opaque_linear_exclude_radial_uses_an_inverse_vector_mask() {
    let parent = crate::style::computed::ComputedStyle::default();
    let style = crate::style::computed::compute_style(
        crate::parser::dom::HtmlTag::Div,
        Some(
            "mask-image: linear-gradient(#000, #000), radial-gradient(circle 44px at 50% 50%, #000 0 44px, transparent 45px); mask-size: 100% 100%, 100% 100%; mask-repeat: no-repeat, no-repeat; mask-composite: exclude;",
        ),
        &parent,
    );
    let Some(MaskSource::Layers(layers)) = style.mask_image else {
        panic!("expected the two computed mask layers");
    };
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(0.0, 0.0, 150.0, 105.0),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );

    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].composite, MaskComposite::Exclude);
    let mut writer = PdfWriter::new();
    assert!(writer
        .add_mask_soft_mask(&MaskSource::Layers(layers), MaskMode::MatchSource, geometry)
        .is_some());
    assert_eq!(writer.pdf_patterns.len(), 1);
    assert!(writer
        .objects
        .iter()
        .all(|object| !object.contains("/Subtype /Image") && !object.contains("/BM /Exclusion")));
}

#[test]
fn oversized_luminance_conic_mask_emits_lossless_noninterpolated_devicegray_tiles() {
    let width = 5_000.0 * 0.75 / 4.0;
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(12.0, 24.0, width, 0.75),
        EdgeSizes::ZERO,
        EdgeSizes::ZERO,
    );
    let source = MaskSource::Conic(test_conic_gradient(
        [
            gradient_stop(0.0, crate::types::Color::rgba8(0, 0, 0, 0)),
            gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 0, 255)),
        ],
        false,
    ));
    let mut writer = PdfWriter {
        opts: RenderOpts {
            raster_quality: crate::style::raster_quality::RasterQuality {
                mask_dpi: 384.0,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };

    writer
        .add_mask_soft_mask(&source, MaskMode::Luminance, geometry)
        .unwrap();

    let images: Vec<_> = writer
        .objects
        .iter()
        .filter(|object| object.contains("/Subtype /Image"))
        .collect();
    assert_eq!(images.len(), 3);
    assert!(images[0].contains("/Width 2048 /Height 4"));
    assert!(images[1].contains("/Width 2048 /Height 4"));
    assert!(images[2].contains("/Width 904 /Height 4"));
    for image in images {
        assert!(image.contains("/ColorSpace /DeviceGray"));
        assert!(image.contains("/Filter /FlateDecode"));
        assert!(!image.contains("/DCTDecode"));
        assert!(!image.contains("/Interpolate"));
    }
    let form_id = writer.soft_mask_gstates[0].form_id;
    let form = &writer.objects[form_id - 1];
    assert_eq!(form.matches(" 0 R ").count(), 3);
    let stream = std::str::from_utf8(&writer.binary_objects[&form_id]).unwrap();
    assert_eq!(stream.matches(" Do\n").count(), 3);
}

#[test]
fn opaque_large_gradient_tile_embeds_direct_lossless_rgb() {
    let dimensions = RasterDimensions {
        width: 256,
        height: 256,
    };
    let mut content = String::new();
    let mut writer = PdfWriter::new();
    let mut page_images = Vec::new();
    draw_tiled_gradient_raster(
        &mut content,
        &mut writer,
        &mut page_images,
        dimensions,
        PdfRect::new(0.0, 0.0, 72.0, 72.0),
        |x, y| {
            let value = x.wrapping_mul(73).wrapping_add(y.wrapping_mul(151)) as u8;
            image::Rgba([value, value.wrapping_mul(43), value ^ y as u8, 255])
        },
    );

    assert_eq!(page_images.len(), 1);
    let header = &writer.objects[page_images[0].obj_id - 1];
    assert!(header.contains("/Filter /FlateDecode"));
    assert!(!header.contains("/Filter /DCTDecode"));
    assert!(!header.contains("/SMask"));
    assert!(!header.contains("/Predictor"));
}

#[test]
fn native_selector_is_exact_for_hard_and_adjacent_stops() {
    let boundary = 0.5_f32;
    let hard = [
        gradient_stop(0.0, crate::types::Color::rgba8(255, 0, 0, 255)),
        gradient_stop(boundary, crate::types::Color::rgba8(255, 0, 0, 255)),
        gradient_stop(boundary, crate::types::Color::rgba8(0, 0, 255, 255)),
        gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 255, 255)),
    ];
    let hard_stops = native_pdf_gradient_stops(&gradient_ramp(hard, false), 1.0).unwrap();
    assert!(build_shading_function(&hard_stops).contains("/Bounds [0.5 0.5]"));
    let hard_linear = native_pdf_linear_gradient(&gradient_ramp(hard, false), 1.0).unwrap();
    assert!(build_shading_function(&hard_linear.stops).contains("/Bounds [0.5 0.5]"));

    let adjacent = [
        hard[0],
        hard[1],
        gradient_stop(
            f32::from_bits(boundary.to_bits() + 1),
            crate::types::Color::rgba8(0, 0, 255, 255),
        ),
        hard[3],
    ];
    assert!(native_pdf_gradient_stops(&gradient_ramp(adjacent, false), 1.0).is_some());

    let out_of_range = [
        gradient_stop(-0.5, crate::types::Color::rgba8(255, 0, 0, 255)),
        gradient_stop(1.5, crate::types::Color::rgba8(0, 0, 255, 255)),
    ];
    assert!(native_pdf_gradient_stops(&gradient_ramp(out_of_range, false), 1.0).is_some());
}

#[test]
fn native_linear_gradient_preserves_authored_out_of_range_span() {
    let gradient = native_pdf_linear_gradient(
        &gradient_ramp(
            [
                gradient_stop(-0.5, crate::types::Color::rgba8(255, 0, 0, 255)),
                gradient_stop(1.5, crate::types::Color::rgba8(0, 0, 255, 255)),
            ],
            false,
        ),
        1.0,
    )
    .unwrap();

    assert_eq!(gradient.span.start, -0.5);
    assert_eq!(gradient.span.end, 1.5);
    assert_eq!(gradient.stops.stops().len(), 2);
}

#[test]
fn hard_stop_function_keeps_incoming_color_at_the_boundary() {
    let ramp = gradient_ramp(
        [
            gradient_stop(0.0, crate::types::Color::rgb(255, 0, 0)),
            gradient_stop(0.5, crate::types::Color::rgb(255, 0, 0)),
            gradient_stop(0.5, crate::types::Color::rgb(0, 0, 255)),
            gradient_stop(1.0, crate::types::Color::rgb(0, 0, 255)),
        ],
        false,
    );
    let red = linear_function_gradient(&ramp, 100.0)
        .unwrap()
        .selector(0)
        .unwrap();

    assert!(red.contains("dup 0.5 le {pop 1}"));
}

#[test]
fn hint_function_preserves_legacy_srgb_blend_precision() {
    let mut first = gradient_stop(0.0, crate::types::Color::rgb(229, 57, 53));
    first.hint_after = Some(crate::style::computed::GradientPosition::fraction(0.75));
    let ramp = gradient_ramp(
        [
            first,
            gradient_stop(1.0, crate::types::Color::rgb(30, 136, 229)),
        ],
        false,
    );
    let resolved = ramp.resolve(1.0).unwrap();
    let stops = PdfFunctionStopSequence::from_resolved(&resolved).unwrap();

    assert!(
        stops
            .selector(0)
            .unwrap()
            .contains("dup 0.40384617 le {0 sub -0.21742316 mul 0.898 add}")
    );
    assert!(
        stops
            .selector(1)
            .unwrap()
            .contains("dup 0.40384617 le {0 sub 0.08631365 mul 0.2235 add}")
    );
    assert!(
        stops
            .selector(2)
            .unwrap()
            .contains("dup 0.40384617 le {0 sub 0.19229373 mul 0.2078 add}")
    );
}

#[test]
fn native_selector_rejects_oklab_and_accepts_explicit_srgb() {
    use crate::style::computed::{
        GradientColor, GradientColorProvenance, GradientInterpolation, GradientPosition,
        GradientStop,
    };
    let modern = |position, color| {
        GradientStop::new(
            GradientColor::new(color, GradientColorProvenance::Modern),
            Some(GradientPosition::fraction(position)),
        )
    };
    let stops = [
        modern(0.0, crate::types::Color::rgb(255, 0, 0)),
        modern(1.0, crate::types::Color::rgb(0, 0, 255)),
    ];
    let oklab = GradientRamp {
        stops: stops.into(),
        interpolation: GradientInterpolation::Oklab,
        ..Default::default()
    };
    assert!(native_pdf_gradient_stops(&oklab, 1.0).is_none());

    let srgb = GradientRamp {
        stops: stops.into(),
        interpolation: GradientInterpolation::Srgb,
        ..Default::default()
    };
    assert!(native_pdf_gradient_stops(&srgb, 1.0).is_some());
}

#[test]
fn border_image_gradient_uses_one_source_form_across_eight_slices() {
    let gradient = LinearGradient {
        angle: 90.0,
        ramp: gradient_ramp(
            [
                gradient_stop(0.0, crate::types::Color::rgb(255, 0, 0)),
                gradient_stop(0.5, crate::types::Color::rgb(0, 128, 0)),
                gradient_stop(1.0, crate::types::Color::rgb(0, 0, 255)),
            ],
            false,
        ),
        layer_box: GradientLayerBox::default(),
    };
    let border_image = BorderImagePaint {
        source: BorderImageSource::LinearGradient(gradient),
        geometry: BorderImage {
                slices: BorderImageSlices::uniform(BorderImageSliceValue::Number(1.0)),
                ..Default::default()
        },
    };
    let geometry = PaintBoxGeometry::new(
        PdfRect::new(10.0, 20.0, 100.0, 40.0),
        EdgeSizes::new(3.0, 5.0, 7.0, 11.0),
        EdgeSizes::ZERO,
    );
    let mut content = String::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0;
    let mut writer = PdfWriter::new();
    let mut images = Vec::new();

    render_border_image(
        &mut content,
        &border_image,
        geometry,
        BorderImageShadings {
            entries: &mut shadings,
            counter: &mut shading_counter,
        },
        &mut Vec::new(),
        &mut writer,
        &mut images,
    );

    assert_eq!(content.matches("W n\n").count(), 8);
    assert_eq!(content.matches(" Do\n").count(), 8);
    assert_eq!(shadings.len(), 1);
    assert_eq!(shadings[0].stops.stops().len(), 3);
    let [form] = images.as_slice() else {
        panic!("expected one shared border-image source form");
    };
    assert_eq!(writer.local_forms.len(), 1);
    assert_eq!(writer.local_forms[0].form_id, form.obj_id);
    assert!(String::from_utf8_lossy(&writer.binary_objects[&form.obj_id]).contains(" sh\n"));
}

#[test]
fn exact_hard_stop_uses_single_linear_raster_path() {
    let boundary = 0.5_f32;
    let gradient = LinearGradient {
        angle: 90.0,
        ramp: gradient_ramp(
            [
                gradient_stop(0.0, crate::types::Color::rgba8(255, 0, 0, 255)),
                gradient_stop(boundary, crate::types::Color::rgba8(255, 0, 0, 255)),
                gradient_stop(boundary, crate::types::Color::rgba8(0, 0, 255, 255)),
                gradient_stop(1.0, crate::types::Color::rgba8(0, 0, 255, 255)),
            ],
            false,
        ),
        layer_box: Default::default(),
    };
    let mut content = String::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0;
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();
    render_linear_gradient(
        &mut content,
        &gradient,
        GradientBackdrop::default(),
        LayerPaintArea::single(PdfRect::new(0.0, 0.0, 10.0, 10.0)),
        &mut shadings,
        &mut shading_counter,
        &mut pdf_writer,
        &mut page_images,
    );
    assert_eq!(shadings.len(), 1);
    assert!(page_images.is_empty());
    assert!(content.contains("/SH0 sh\n"));
}

#[test]
fn alpha_linear_gradient_over_opaque_solid_uses_one_vector_shading() {
    let gradient = LinearGradient {
        angle: 135.0,
        ramp: gradient_ramp(
            [
                gradient_stop(0.0, crate::types::Color::rgba8(255, 209, 102, 209)),
                gradient_stop(1.0, crate::types::Color::rgba8(6, 214, 160, 107)),
            ],
            false,
        ),
        layer_box: Default::default(),
    };
    let backdrop = GradientBackdrop::isolated_linear_layer(
        Some(crate::types::Color::rgb(231, 245, 255)),
        false,
        crate::style::computed::BlendMode::Normal,
    );
    let mut content = String::new();
    let mut shadings = Vec::new();
    let mut shading_counter = 0;
    let mut pdf_writer = PdfWriter::new();
    let mut page_images = Vec::new();

    render_linear_gradient(
        &mut content,
        &gradient,
        backdrop,
        LayerPaintArea::single(PdfRect::new(0.0, 0.0, 126.0, 68.0)),
        &mut shadings,
        &mut shading_counter,
        &mut pdf_writer,
        &mut page_images,
    );

    assert_eq!(shadings.len(), 1);
    assert!(page_images.is_empty());
    assert!(pdf_writer.binary_objects.is_empty());
    assert!(content.contains("/SH0 sh\n"));
}
