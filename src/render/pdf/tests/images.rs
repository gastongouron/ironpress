    #[test]
    fn render_image_contains_xobject() {
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="100" height="80">"#);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /DCTDecode"),
            "JPEG image should use DCTDecode filter"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    fn render_image_xobject_uses_source_pixel_dimensions() {
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="120" height="90">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Width 1 /Height 1"),
            "image XObject should use source pixel dimensions, not CSS box dimensions"
        );
    }

    #[test]
    fn fragmented_raster_reuses_one_complete_source_image() {
        let source = rgb_png_data_uri(8, 20);
        let html = format!(
            r#"<style>
                @page {{ size: 16px 8px; margin: 0 }}
                * {{ margin: 0; box-sizing: border-box }}
                img {{ display: block; width: 8px; height: 20px }}
            </style><img alt="" src="{source}">"#,
        );
        let pdf = crate::HtmlConverter::new()
            .compress(false)
            .sanitize(false)
            .convert(&html)
            .expect("fragmented raster should render");
        let content = String::from_utf8_lossy(&pdf);
        let image_draws = content
            .lines()
            .filter(|line| line.starts_with("/Im") && line.ends_with(" Do"))
            .count();

        assert_eq!(
            content.matches("/Subtype /Image").count(),
            1,
            "every fragment must reference one document-level image XObject"
        );
        assert!(
            content.contains("/Width 8 /Height 20"),
            "fragmentation must preserve the complete source raster"
        );
        assert_eq!(
            image_draws, 3,
            "the complete source must be clipped and translated once per page"
        );
    }

    #[test]
    fn nested_filtered_replaced_output_paints_once_across_parent_phases() {
        let pdf = crate::HtmlConverter::new()
            .compress(false)
            .sanitize(false)
            .convert(
                r#"<style>
                    * { margin: 0; box-sizing: border-box }
                    .outer { width: 200px; height: 140px; background: #b0bec5 }
                    .filtered { width: 200px; height: 140px; background: #c62828;
                                filter: opacity(0.5) }
                </style>
                <div><div class="outer"><div class="filtered"></div></div></div>"#,
            )
            .expect("filtered nested box should render");
        let content = String::from_utf8_lossy(&pdf);

        let image_draws = content
            .lines()
            .filter(|line| line.starts_with("/Im") && line.ends_with(" Do"))
            .count();
        assert_eq!(
            image_draws,
            1,
            "the atomic filtered image must appear only in the parent's contents phase\n{content}",
        );
    }

    #[test]
    fn stylesheet_image_rendering_controls_pdf_image_sampling() {
        const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";
        let pixelated = format!(
            r#"<style>img {{ image-rendering: pixelated }}</style><img src="{PIXEL}" width="2" height="2">"#,
        );
        let smooth = format!(
            r#"<style>img {{ image-rendering: smooth }}</style><img src="{PIXEL}" width="2" height="2">"#,
        );

        let render = |html: &str| crate::HtmlConverter::new()
            .compress(false)
            .convert(html)
            .expect("test PDF should render");
        let pixelated_pdf = render(&pixelated);
        let smooth_pdf = render(&smooth);

        assert!(
            String::from_utf8_lossy(&pixelated_pdf).contains("/Interpolate false"),
            "pixelated source images must preserve their final sample grid"
        );
        assert!(
            String::from_utf8_lossy(&smooth_pdf).contains("/Interpolate true"),
            "smooth source images must request PDF interpolation"
        );
    }

    #[test]
    fn image_cache_keeps_distinct_sampling_modes() {
        let source = rgb_png_data_uri(2, 2);
        let html = format!(
            r#"<style>
                img {{ width: 4px; height: 4px }}
                .pixelated {{ image-rendering: pixelated }}
                .smooth {{ image-rendering: smooth }}
            </style>
            <img class="pixelated" alt="" src="{source}">
            <img class="smooth" alt="" src="{source}">"#,
        );
        let pdf = crate::HtmlConverter::new()
            .compress(false)
            .sanitize(false)
            .convert(&html)
            .expect("sampling variants should render");
        let content = String::from_utf8_lossy(&pdf);

        assert_eq!(
            content.matches("/Subtype /Image").count(),
            2,
            "sampling modes with different PDF behavior must not share an XObject"
        );
        assert!(content.contains("/Interpolate false"));
        assert!(content.contains("/Interpolate true"));
    }

    #[test]
    fn integral_pixelated_cover_embeds_only_visible_source_pixels() {
        let source = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAECAIAAAA8r+mnAAAAIElEQVR42mM4IScHR3I9NnDEgFNCzu0EHN1ZJQJHOCUAni4lgeO2HLIAAAAASUVORK5CYII=";
        let html = format!(
            r#"<style>
                @page {{ size: 384px 224px; margin: 0 }}
                * {{ margin: 0; box-sizing: border-box }}
                img {{
                    display: block;
                    width: 160px;
                    height: 160px;
                    object-fit: cover;
                    image-rendering: pixelated;
                }}
            </style><img alt="" src="{source}">"#,
        );
        let pdf = crate::HtmlConverter::new()
            .compress(false)
            .sanitize(false)
            .convert(&html)
            .expect("pixelated cover should render");
        let content = String::from_utf8_lossy(&pdf);

        assert_eq!(content.matches("/Subtype /Image").count(), 1);
        assert!(
            content.contains("/Width 4 /Height 4"),
            "the exact visible source crop should remain at source resolution"
        );
        assert!(!content.contains("/Width 320 /Height 160"));
        assert!(content.contains("/Interpolate false"));
    }

    #[test]
    fn render_no_image_no_xobject() {
        let html = "<p>No images here</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/XObject"),
            "PDF without images should not contain /XObject"
        );
    }

    #[test]
    fn ordinary_css_borders_remain_vector_pdf_content() {
        let html = r#"
            <div style="width:96px;height:24px;border-style:solid dashed dotted double;
                        border-width:3px 5px 7px 9px;border-color:#e11 #1a1 #11e #a51;
                        border-radius:16px 8px 13px 5px">a</div>
            <div style="width:96px;height:24px;border-style:groove ridge inset outset;
                        border-width:8px;border-color:#56789a;border-radius:11px">b</div>
        "#;
        let pdf = crate::HtmlConverter::new()
            .compress(false)
            .convert(html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert!(
            !content.contains("/Subtype /Image"),
            "ordinary CSS borders must be emitted as PDF paths, never image XObjects"
        );
        assert!(
            content.contains("re\n") || content.contains(" c\n"),
            "ordinary CSS borders should emit vector path operators"
        );
    }

    #[test]
    fn render_dashed_border_emits_dash_pattern() {
        let html = r#"<div style="border: 2px dashed black; width: 100pt">Dashed</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Dashed borders are now painted as filled dash segments so adjacent
        // sides can meet cleanly at corners.
        let segment_count = content.matches(" re\n").count();
        assert!(
            segment_count >= 8,
            "Dashed border should paint filled dash segments. Got: {}",
            &content[..content.len().min(2000)]
        );
        assert!(
            content.contains("(Dashed)"),
            "Dashed border test should still render the element text"
        );
    }

    #[test]
    fn render_dotted_border_emits_dash_pattern() {
        let html = r#"<div style="border: 2px dotted red; width: 100pt">Dotted</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Dotted borders are now painted as filled circle paths, avoiding viewer
        // differences in round-cap dash rendering.
        let curve_count = content.matches(" c\n").count();
        assert!(
            curve_count >= 16,
            "Dotted border should paint filled circular dot paths. Got: {}",
            &content[..content.len().min(2000)]
        );
        assert!(
            content.contains("(Dotted)"),
            "Dotted border test should still render the element text"
        );
    }

    #[test]
    fn render_solid_border_no_dash_pattern() {
        let html = r#"<div style="border: 2px solid black; width: 100pt">Solid</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Solid borders should NOT set any dash pattern (no `[...] 0 d` and no
        // round-cap toggle).
        assert!(
            !content.contains("0 d\n") && !content.contains("1 J\n"),
            "Solid border should not emit dash patterns"
        );
    }

    #[test]
    fn border_style_parsed_from_shorthand() {
        use crate::parser::dom::HtmlTag;
        use crate::style::computed::BorderStyle;
        use crate::style::computed::ComputedStyle;
        let parent = ComputedStyle::default();
        let style = crate::style::computed::compute_style(
            HtmlTag::Div,
            Some("border: 2px dashed red"),
            &parent,
        );
        assert_eq!(style.border.top.style, BorderStyle::Dashed);
        assert_eq!(style.border.right.style, BorderStyle::Dashed);
        assert_eq!(style.border.bottom.style, BorderStyle::Dashed);
        assert_eq!(style.border.left.style, BorderStyle::Dashed);
    }

    #[test]
    fn render_times_roman_font_family() {
        let html = r#"<p style="font-family: serif">Serif text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Roman"),
            "PDF should use Times-Roman for serif font-family"
        );
    }

    #[test]
    fn render_times_bold_italic() {
        let html =
            r#"<p style="font-family: serif"><strong><em>Bold Italic Serif</em></strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-BoldItalic"),
            "PDF should use Times-BoldItalic for bold italic serif"
        );
    }

    #[test]
    fn render_times_bold() {
        let html = r#"<p style="font-family: times"><strong>Bold Serif</strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Bold"),
            "PDF should use Times-Bold for bold serif"
        );
    }

    #[test]
    fn render_times_italic() {
        let html = r#"<p style="font-family: serif"><em>Italic Serif</em></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Italic"),
            "PDF should use Times-Italic for italic serif"
        );
    }

    #[test]
    fn render_courier_font_family() {
        let html = r#"<p style="font-family: monospace">Monospace text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier ") || content.contains("/Courier\n"),
            "PDF should use Courier for monospace font-family"
        );
    }

    #[test]
    fn render_courier_bold_italic() {
        let html =
            r#"<p style="font-family: courier"><strong><em>Bold Italic Mono</em></strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-BoldOblique"),
            "PDF should use Courier-BoldOblique for bold italic monospace"
        );
    }

    #[test]
    fn render_courier_bold() {
        let html = r#"<p style="font-family: monospace"><strong>Bold Mono</strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-Bold"),
            "PDF should use Courier-Bold for bold monospace"
        );
    }

    #[test]
    fn render_courier_oblique() {
        let html = r#"<p style="font-family: courier"><em>Italic Mono</em></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-Oblique"),
            "PDF should use Courier-Oblique for italic monospace"
        );
    }

    #[test]
    fn render_font_family_via_stylesheet() {
        let html = r#"
            <html>
            <head><style>p { font-family: serif }</style></head>
            <body><p>Styled serif</p></body>
            </html>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Roman"),
            "Stylesheet font-family should produce Times-Roman"
        );
    }

    #[test]
    fn render_jpeg_image_contains_xobject() {
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="100" height="80">"#);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /DCTDecode"),
            "JPEG image should use DCTDecode filter"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    #[ignore] // TODO: Container renderer doesn't render background images yet
    fn render_jpeg_background_uses_decoded_image_xobject() {
        use image::ImageEncoder;

        let mut jpeg_bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg_bytes)
            .write_image(
                &[255u8, 128, 0, 0, 128, 255, 0, 0, 0, 255, 255, 255],
                2,
                2,
                image::ExtendedColorType::Rgb8,
            )
            .expect("jpeg encoding should succeed");
        let jpeg_b64 = simple_base64_encode_test(&jpeg_bytes);
        let html = format!(
            r#"
            <div style="
                width: 100pt;
                height: 100pt;
                background-image: url(data:image/jpeg;base64,{jpeg_b64});
                background-repeat: no-repeat;
                background-size: 100pt 100pt;
            "></div>
        "#,
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert_eq!(content.matches("/Subtype /Image").count(), 1);
        assert!(
            content.contains("/Filter /FlateDecode"),
            "decoded JPEG backgrounds should use a Flate image XObject"
        );
        assert!(
            !content.contains("/Filter /DCTDecode"),
            "decoded JPEG backgrounds should not passthrough raw JPEG bytes"
        );
    }

    #[test]
    fn render_png_image_contains_flatedecode() {
        // Build a minimal valid PNG as base64 data URI
        let png_bytes = build_minimal_test_png();
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="100" height="100">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with PNG image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /FlateDecode"),
            "PNG image should use FlateDecode filter"
        );
        assert!(
            content.contains("/Predictor 15"),
            "PNG image should have Predictor 15 in DecodeParms"
        );
        assert!(
            content.contains("/Colors 3"),
            "RGB PNG should have Colors 3"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    fn render_opaque_png_auto_resize_uses_target_dpi() {
        let src = rgb_png_data_uri(96, 96);
        let html = format!(r#"<img src="{src}" style="width:24pt;height:24pt">"#);
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .image_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert!(
            content.contains("/Width 24 /Height 24"),
            "opaque PNG should be resized to target image DPI before embedding"
        );
        assert!(
            !content.contains("/Width 96 /Height 96"),
            "opaque PNG should not embed the full source dimensions when downscaling"
        );
        assert!(
            content.contains("/Filter /FlateDecode"),
            "small resized PNG should stay on the lossless PNG/Flate path"
        );
    }

    #[test]
    fn render_color_filtered_png_retains_configured_filter_dpi() {
        let src = rgb_png_data_uri(96, 96);
        let html =
            format!(r#"<img src="{src}" style="width:24pt;height:24pt;filter:brightness(90%)">"#);
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .image_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        let image_headers: Vec<_> = content
            .lines()
            .filter(|line| line.contains("/Subtype /Image"))
            .collect();

        assert!(
            content.contains("/Width 100 /Height 100"),
            "24pt at the default 300-DPI filter resolution is exactly 100 device samples; \
             embedded image headers: {image_headers:?}"
        );
        assert!(
            !content.contains("/Width 96 /Height 96"),
            "the renderer-owned filter surface must not fall back to source-image resolution"
        );
        assert!(
            !content.contains("/Width 24 /Height 24"),
            "image DPI must not downsample a renderer-owned filter surface"
        );
    }

    #[test]
    fn render_group_filter_keeps_its_filter_resolution() {
        let html = r#"
            <style>
                @page { size: 48pt 48pt; margin: 0 }
                div { width: 24pt; height: 24pt; background: #8e24aa; filter: saturate(200%) }
            </style>
            <div></div>
        "#;
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .image_dpi(72.0)
            .filter_dpi(300.0)
            .convert(html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert!(
            content.contains("/Width 100 /Height 100"),
            "a group filter should retain its configured 300-DPI raster"
        );
        assert!(
            !content.contains("/Width 24 /Height 24"),
            "the source-image DPI must not downsample a renderer-owned filter raster"
        );
    }

    #[test]
    fn render_drop_shadow_image_honors_filter_dpi() {
        let src = rgb_png_data_uri(16, 16);
        let html = format!(
            r#"
            <style>
              img {{
                display: block;
                width: 72pt;
                height: 72pt;
                filter: drop-shadow(0 0 0 rgba(0,0,0,.5));
              }}
            </style>
            <img src="{src}">
            "#
        );
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .filter_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Width 74 /Height 74"),
            "the filter-DPI surface should retain only its required sampling border"
        );
        assert!(
            !content.contains("/Width 152 /Height 152"),
            "drop-shadow image raster should not use the old hardcoded 150+ DPI surface"
        );
        assert!(
            !content.contains("/Width 302 /Height 302"),
            "drop-shadow image raster should not use the old hardcoded 300 DPI surface"
        );
    }

    #[test]
    fn render_drop_shadow_uses_one_configured_filter_surface() {
        let src = rgb_png_data_uri(192, 192);
        let html = format!(
            r#"
            <style>
              img {{
                display: block;
                width: 72pt;
                height: 72pt;
                filter: drop-shadow(0 0 0 rgba(0,0,0,.5));
              }}
            </style>
            <img src="{src}">
            "#
        );
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .image_dpi(150.0)
            .filter_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert!(
            content.contains("/Width 74 /Height 74"),
            "source and shadow should share one finite filter-DPI surface"
        );
        assert!(
            !content.contains("/Width 192 /Height 192"),
            "the intrinsic source should not be embedded beside the filter surface"
        );
        assert!(
            !content.contains("/Width 150 /Height 150"),
            "a split image-DPI source surface would misalign with the shadow"
        );
    }

    #[test]
    fn render_drop_shadow_rasterizes_a_low_dpi_source_at_filter_dpi() {
        let src = rgb_png_data_uri(16, 16);
        let html = format!(
            r#"
            <style>
              img {{
                display: block;
                width: 72pt;
                height: 72pt;
                filter: drop-shadow(0 0 0 rgba(0,0,0,.5));
              }}
            </style>
            <img src="{src}">
            "#
        );
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .image_dpi(300.0)
            .filter_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert!(
            content.contains("/Width 74 /Height 74"),
            "the finite filtered composite should use filter DPI"
        );
        assert!(
            !content.contains("/Width 16 /Height 16"),
            "the intrinsic source should not be embedded separately"
        );
    }

    #[test]
    fn render_pseudo_blur_background_paints_below_relative_image_z_index() {
        let src = rgb_png_data_uri(96, 96);
        let html = format!(
            r#"
            <style>
              .wrap {{
                position: relative;
                width: 72pt;
                height: 72pt;
              }}
              .wrap::before {{
                content: "";
                position: absolute;
                left: 0;
                top: 0;
                width: 72pt;
                height: 72pt;
                background-image: url('{src}');
                background-size: cover;
                background-repeat: no-repeat;
                filter: blur(4pt);
                z-index: 0;
              }}
              .wrap img {{
                display: block;
                position: relative;
                z-index: 1;
                width: 72pt;
                height: 72pt;
              }}
            </style>
            <div class="wrap"><img src="{src}"></div>
            "#
        );
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .compress(false)
            .image_dpi(72.0)
            .filter_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // 72px source + 3σ padding on both sides; blur(4pt) is σ=4px at 72 DPI.
        let blur_name = image_name_for_dimensions(&content, 96, 96)
            .expect("blurred pseudo background should retain its full 3-sigma tail");
        let image_name = image_name_for_dimensions(&content, 72, 72)
            .expect("foreground source image should use image DPI");
        let blur_draw = content
            .find(&format!("/{blur_name} Do"))
            .expect("blurred pseudo background should be drawn");
        let image_draw = content
            .find(&format!("/{image_name} Do"))
            .expect("foreground image should be drawn");

        assert!(
            blur_draw < image_draw,
            "pseudo blur background must paint before the relative z-indexed image"
        );
    }

    #[test]
    fn render_reordered_relative_image_preserves_following_flow_top() {
        let src = rgb_png_data_uri(96, 96);
        let html = format!(
            r#"
            <style>
              .wrap {{
                position: relative;
                width: 72pt;
              }}
              .wrap::before {{
                content: "";
                position: absolute;
                left: 0;
                top: 0;
                width: 72pt;
                height: 72pt;
                background: rgba(0,0,0,.35);
                filter: blur(2pt);
                z-index: 0;
              }}
              .wrap img {{
                display: block;
                position: relative;
                z-index: 1;
                width: 72pt;
                height: 72pt;
              }}
              .after {{
                margin: 0;
                width: 72pt;
                height: 12pt;
                background: rgb(255,0,0);
              }}
            </style>
            <div class="wrap"><img src="{src}"><div class="after"></div></div>
            "#
        );
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .compress(false)
            .image_dpi(72.0)
            .filter_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        let image_name = image_name_for_dimensions(&content, 72, 72)
            .expect("foreground source image should use image DPI");
        let image_bottom = image_draw_bottom_y(&content, &image_name)
            .expect("foreground source image draw should have a placement matrix");
        let (_, after_bottom, _, after_height) = rect_after_color(&content, "1 0 0 rg")
            .expect("following block should paint as a red rectangle");

        assert!(
            after_bottom + after_height <= image_bottom + 0.1,
            "following in-flow block must stay below the image slot; after_bottom={after_bottom}, after_height={after_height}, image_bottom={image_bottom}"
        );
    }

    #[test]
    fn render_pseudo_blur_background_paints_below_relative_svg_z_index() {
        let bg = rgb_png_data_uri(96, 96);
        let svg = svg_data_uri(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96">
                <rect x="8" y="8" width="80" height="80" fill="#336699"/>
            </svg>"##,
        );
        let html = format!(
            r#"
            <style>
              .wrap {{
                position: relative;
                width: 72pt;
                height: 72pt;
              }}
              .wrap::before {{
                content: "";
                position: absolute;
                left: 0;
                top: 0;
                width: 72pt;
                height: 72pt;
                background-image: url('{bg}');
                background-size: cover;
                background-repeat: no-repeat;
                filter: blur(4pt);
                z-index: 0;
              }}
              .wrap img {{
                display: block;
                position: relative;
                z-index: 1;
                width: 72pt;
                height: 72pt;
              }}
            </style>
            <div class="wrap"><img src="{svg}"></div>
            "#
        );
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .compress(false)
            .image_dpi(72.0)
            .filter_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // 72px source + 3σ padding on both sides; blur(4pt) is σ=4px at 72 DPI.
        let blur_name = image_name_for_dimensions(&content, 96, 96)
            .expect("blurred pseudo background should retain its full 3-sigma tail");
        let blur_draw = content
            .find(&format!("/{blur_name} Do"))
            .expect("blurred pseudo background should be drawn");
        let svg_rect = content
            .find("8 8 80 80 re")
            .expect("foreground SVG should remain vector PDF geometry");

        assert!(
            blur_draw < svg_rect,
            "pseudo blur background must paint before the relative z-indexed SVG image"
        );
    }

    #[test]
    fn render_svg_embedded_png_uses_image_dpi_without_rasterizing_svg_vector_content() {
        let embedded = rgb_png_data_uri(96, 96);
        let svg = svg_data_uri(&format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="72" height="72" viewBox="0 0 72 72">
                <rect x="2" y="2" width="8" height="8" fill="#00ff00"/>
                <image href="{embedded}" x="0" y="0" width="72" height="72" preserveAspectRatio="none"/>
            </svg>"##
        ));
        let html = format!(r#"<img src="{svg}" style="width:72pt;height:72pt">"#);
        let pdf = crate::HtmlConverter::new()
            .sanitize(false)
            .compress(false)
            .image_dpi(72.0)
            .convert(&html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert!(
            content.contains("2 2 8 8 re"),
            "SVG vector geometry should remain vector PDF content"
        );
        assert!(
            content.contains("/Width 72 /Height 72"),
            "embedded raster inside SVG should be downsampled using image DPI"
        );
        assert!(
            !content.contains("/Width 96 /Height 96"),
            "embedded raster inside SVG should not bypass image-DPI optimization"
        );
    }

    #[test]
    fn render_png_grayscale_image() {
        let png_bytes = build_test_png_with_color_type(0); // Grayscale
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="50" height="50">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Filter /FlateDecode"));
        assert!(content.contains("/ColorSpace /DeviceGray"));
        assert!(content.contains("/Colors 1"));
    }

    /// Regression (public API): a protocol-relative `//abs/path` must not escape
    /// the authorized root. Unix-only, where a leading `//` collapses to an
    /// absolute path — the exploit that let it be read as a local file.
    #[cfg(unix)]
    #[test]
    fn protocol_relative_reference_cannot_escape_resource_root() {
        let root = tempfile::tempdir().expect("authorized root");
        let outside = tempfile::tempdir().expect("outside directory");
        let secret = outside.path().join("secret.png");
        std::fs::write(&secret, build_minimal_test_png()).expect("secret fixture");

        // Positive control, so the negative assertion can't pass vacuously.
        std::fs::write(root.path().join("ok.png"), build_minimal_test_png())
            .expect("in-root fixture");
        let allowed = crate::HtmlConverter::new()
            .compress(false)
            .sanitize(false)
            .resource_root(root.path())
            .convert(r#"<img src="ok.png" width="10" height="10">"#)
            .expect("in-root image converts");
        assert!(
            String::from_utf8_lossy(&allowed).contains("/Subtype /Image"),
            "control: an authorized in-root image should embed"
        );

        let reference = format!("/{}", secret.display()); // //<abs path>, e.g. //tmp/xxx/secret.png
        let html = format!(r#"<img src="{reference}" width="10" height="10">"#);
        let escaped = crate::HtmlConverter::new()
            .compress(false)
            .sanitize(false)
            .resource_root(root.path())
            .convert(&html)
            .expect("conversion still succeeds, just without the outside file");
        assert!(
            !String::from_utf8_lossy(&escaped).contains("/Subtype /Image"),
            "a protocol-relative reference must not read a file outside the resource root"
        );
    }

    /// Build a minimal valid PNG (1x1 RGB, 8-bit).
    fn build_minimal_test_png() -> Vec<u8> {
        build_test_png_with_color_type(2) // RGB
    }

    fn rgb_png_data_uri(width: u32, height: u32) -> String {
        let img = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([
                ((x * 3 + y) % 256) as u8,
                ((x + y * 5) % 256) as u8,
                ((x * 7 + y * 11) % 256) as u8,
            ])
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        format!("data:image/png;base64,{}", simple_base64_encode_test(&png))
    }

    fn svg_data_uri(markup: &str) -> String {
        format!(
            "data:image/svg+xml;base64,{}",
            simple_base64_encode_test(markup.as_bytes())
        )
    }

    fn image_name_for_dimensions(content: &str, width: u32, height: u32) -> Option<String> {
        let dimensions = format!("/Width {width} /Height {height}");
        let mut search_from = 0;
        while let Some(relative_pos) = content[search_from..].find(&dimensions) {
            let dimensions_pos = search_from + relative_pos;
            let before = &content[..dimensions_pos];
            let marker = " 0 obj\n";
            let marker_pos = before.rfind(marker)?;
            let id_start = before[..marker_pos].rfind('\n').map_or(0, |pos| pos + 1);
            let id = before[id_start..marker_pos].trim().parse::<usize>().ok()?;
            let name = format!("Im{id}");
            if content.contains(&format!("/{name} Do")) {
                return Some(name);
            }
            search_from = dimensions_pos + dimensions.len();
        }
        None
    }

    fn image_draw_bottom_y(content: &str, image_name: &str) -> Option<f32> {
        let draw_pos = content.find(&format!("/{image_name} Do"))?;
        let before = &content[..draw_pos];
        let cm_pos = before.rfind(" cm\n")?;
        let line_start = before[..cm_pos].rfind('\n').map_or(0, |pos| pos + 1);
        let nums: Vec<f32> = before[line_start..cm_pos]
            .split_whitespace()
            .filter_map(|part| part.parse::<f32>().ok())
            .collect();
        nums.get(5).copied()
    }

    fn rect_after_color(content: &str, color_operator: &str) -> Option<(f32, f32, f32, f32)> {
        let color_pos = content.find(color_operator)?;
        let after_color = &content[color_pos + color_operator.len()..];
        let re_pos = after_color.find(" re\n")?;
        let before_re = &after_color[..re_pos];
        let line_start = before_re.rfind('\n').map_or(0, |pos| pos + 1);
        let nums: Vec<f32> = before_re[line_start..]
            .split_whitespace()
            .filter_map(|part| part.parse::<f32>().ok())
            .collect();
        Some((*nums.first()?, *nums.get(1)?, *nums.get(2)?, *nums.get(3)?))
    }

    fn build_test_png_with_color_type(color_type: u8) -> Vec<u8> {
        let mut png = Vec::new();
        // PNG signature
        png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        // IHDR chunk (13 bytes data)
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes()); // width
        ihdr.extend_from_slice(&1u32.to_be_bytes()); // height
        ihdr.push(8); // bit depth
        ihdr.push(color_type);
        ihdr.push(0); // compression
        ihdr.push(0); // filter
        ihdr.push(0); // interlace
        append_png_chunk(&mut png, b"IHDR", &ihdr);
        // IDAT chunk with dummy zlib-compressed data
        let idat = [
            0x78, 0x01, 0x62, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
        ];
        append_png_chunk(&mut png, b"IDAT", &idat);
        // IEND
        append_png_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn append_png_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(chunk_type);
        buf.extend_from_slice(data);
        buf.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
    }

    fn simple_base64_encode_test(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::new();
        let mut i = 0;
        while i < data.len() {
            let b0 = data[i] as u32;
            let b1 = if i + 1 < data.len() {
                data[i + 1] as u32
            } else {
                0
            };
            let b2 = if i + 2 < data.len() {
                data[i + 2] as u32
            } else {
                0
            };
            let triple = (b0 << 16) | (b1 << 8) | b2;
            result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            if i + 1 < data.len() {
                result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            if i + 2 < data.len() {
                result.push(CHARS[(triple & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            i += 3;
        }
        result
    }

    fn write_test_png_file(name: &str, bytes: &[u8]) -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ironpress-{name}-{}-{nonce}.png",
            std::process::id()
        ));
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }
