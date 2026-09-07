use super::*;

/// Rasterize one blurred text shadow and register its image object.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_text_shadow_blur(
    content: &mut String,
    run: &TextRun,
    origin_x_pt: f32,
    baseline_y_pt: f32,
    blur_pt: f32,
    color: (f32, f32, f32, f32),
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> bool {
    let (_, font) = match crate::text::resolve_custom_font(
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        custom_fonts,
    ) {
        Some(f) => f,
        None => return false,
    };
    let shaped = match crate::text::shape_text_run(run, custom_fonts) {
        Some(s) if !s.glyphs.is_empty() => s,
        _ => return false,
    };
    let embolden_pt = run
        .synthetic_bold_stroke_width(custom_fonts)
        .unwrap_or_default();
    let raster =
        match crate::render::blur::rasterize_run_alpha(crate::render::blur::GlyphRasterRequest {
            font,
            font_size: font.adjusted_font_size(run.font_size),
            glyphs: &shaped.glyphs,
            style: crate::render::blur::GlyphRasterStyle {
                embolden: embolden_pt,
                shear: run.synthetic_italic_shear(custom_fonts).unwrap_or_default(),
            },
            origin: crate::render::blur::GlyphBaselineOrigin::pdf(origin_x_pt, baseline_y_pt),
            dpi: pdf_writer.opts.raster_quality.filter_dpi,
        }) {
            Some(r) => r,
            None => return false,
        };
    let (mask_w, mask_h) = (raster.mask.width(), raster.mask.height());
    let (blurred, pad) = match crate::render::blur::blur_shadow_alpha_mask(
        &raster.mask,
        blur_pt,
        color,
        pdf_writer.opts.raster_quality.filter_dpi,
    ) {
        Some(b) => b,
        None => return false,
    };

    let px_per_pt =
        crate::render::raster_scale::RasterScale::at_dpi(pdf_writer.opts.raster_quality.filter_dpi)
            .pixels_per_point();
    let buf_w_px = (mask_w + 2 * pad) as f32;
    let buf_h_px = (mask_h + 2 * pad) as f32;
    let w_pt = buf_w_px / px_per_pt;
    let h_pt = buf_h_px / px_per_pt;

    // Text origin inside the blurred buffer (device px from top-left).
    let bx = raster.placement.baseline_in_mask.x + pad as f32;
    let by = raster.placement.baseline_in_mask.y + pad as f32;

    // Place the buffer so its text-origin pixel lands at the shadow PDF origin.
    let ix = origin_x_pt - bx / px_per_pt;
    let iy = baseline_y_pt - h_pt + by / px_per_pt;

    let img_obj_id = pdf_writer.add_image_object(
        &blurred.asset.data,
        blurred.asset.source_width,
        blurred.asset.source_height,
        blurred.asset.format,
        blurred.asset.png_metadata.as_ref(),
    );
    let img_name = format!("Im{img_obj_id}");
    content.push_str(&format!(
        "q\n{w_pt} 0 0 {h_pt} {ix} {iy} cm\n/{img_name} Do\nQ\n"
    ));
    page_images.push(ImageRef {
        name: img_name,
        obj_id: img_obj_id,
    });
    true
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_run_glyphs(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> f32 {
    render_run_glyph_layers_in_space(
        content,
        run,
        x,
        text_y,
        parent_font_size,
        custom_fonts,
        prepared_custom_fonts,
        word_spacing,
        pdf_writer,
        page_images,
        PdfContentSpace::Points,
        TextShadowPaint::Include,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_run_glyphs_without_shadows(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> f32 {
    render_run_glyph_layers_in_space(
        content,
        run,
        x,
        text_y,
        parent_font_size,
        custom_fonts,
        prepared_custom_fonts,
        word_spacing,
        pdf_writer,
        page_images,
        PdfContentSpace::Points,
        TextShadowPaint::Skip,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_run_text_shadows(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    render_run_text_shadows_in_space(
        content,
        run,
        x,
        text_y,
        parent_font_size,
        custom_fonts,
        prepared_custom_fonts,
        word_spacing,
        pdf_writer,
        page_images,
        PdfContentSpace::Points,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_run_text_shadows_in_space(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    text_space: PdfContentSpace,
) {
    let baseline = run_paint_baseline(run, text_y, parent_font_size);
    paint_run_text_shadows_at_baseline(
        content,
        run,
        x,
        baseline,
        parent_font_size,
        custom_fonts,
        prepared_custom_fonts,
        word_spacing,
        pdf_writer,
        page_images,
        text_space,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TextShadowPaint {
    Include,
    Skip,
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::collapsible_if)]
pub(super) fn render_run_glyph_layers_in_space(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    justification_word_spacing: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    text_space: PdfContentSpace,
    shadow_paint: TextShadowPaint,
) -> f32 {
    let (r, g, b) = run.color.to_f32_rgb();
    let letter_spacing = text_run_letter_spacing(run);
    let word_spacing = run.metadata.spacing.word + justification_word_spacing;
    // css2 §10.8.1: `vertical-align: super`/`sub` paint a text run with its
    // baseline raised/lowered by a fraction of the parent (line) font size. This
    // only moves the painted glyphs vertically; the horizontal advance (the
    // returned width) is unchanged, so callers position the next run normally.
    let text_y = run_paint_baseline(run, text_y, parent_font_size);

    // CSS `text-shadow` (css-text-decor-3 §3): paint the glyphs again behind the
    // real text, once per shadow (back-to-front: the last listed shadow is
    // drawn first / furthest back). Each shadow is offset by (offset_x right,
    // offset_y down) in the shadow's colour. PDF Y grows upward, so a positive
    // CSS offset-y subtracts from `text_y`.
    //
    // When `blur > 0`, the shadow is a true gaussian (σ = blur/2): rasterize the
    // run's glyph outlines into an alpha mask, blur+tint it (reusing
    // `render::blur`), and embed as an image XObject — matching Chrome's soft
    // halo. When `blur == 0` (or rasterization is unavailable), paint a sharp
    // offset vector copy. Decorations and nested shadows are cleared on the
    // shadow run to avoid double-painting.
    if shadow_paint == TextShadowPaint::Include {
        paint_run_text_shadows_at_baseline(
            content,
            run,
            x,
            text_y,
            parent_font_size,
            custom_fonts,
            prepared_custom_fonts,
            justification_word_spacing,
            pdf_writer,
            page_images,
            text_space,
        );
    }

    // For runs with mixed scripts (e.g. "Chinese: 你好世界"), split into
    // segments and render each with the appropriate font: primary font for
    // characters it covers, fallback font for the rest.
    if crate::text::needs_unicode_fallback(run, custom_fonts) {
        let segments = crate::text::split_run_by_font_coverage(run, custom_fonts);
        let mut total_width = 0.0f32;
        let mut cur_x = x;
        for (segment_index, (segment_text, use_fallback)) in segments.iter().enumerate() {
            let mut sub_run = run.clone();
            sub_run.text = segment_text.clone();
            sub_run.metadata.boundary = crate::layout::engine::InlineBoundaryAdvance::none();
            // `text_y` already carries this run's vertical-align shift; clear it on
            // the per-segment recursion so the shift is not applied a second time.
            sub_run.vertical_align = VerticalAlign::Baseline;
            sub_run.metadata.emphasis = Default::default();
            if *use_fallback {
                if let Some((fallback_shaped, fallback_key, fallback_font)) =
                    crate::text::shape_with_unicode_fallback(&sub_run, custom_fonts)
                {
                    let w = sub_run
                        .metadata
                        .spacing
                        .add_internal_advance(fallback_shaped.width, &sub_run.text);
                    let font_name = sanitize_pdf_name(fallback_key);
                    let font_size =
                        fallback_font.adjusted_font_size(text_space.length(sub_run.font_size));
                    if let Some(begin) = text_space.begin_operator() {
                        content.push_str(&begin);
                    }
                    content.push_str(&PdfRgb::from((r, g, b)).fill_operator());
                    content.push_str("BT\n");
                    content.push_str(&format!("/{font_name} {font_size} Tf\n"));
                    let prepared_font = prepared_custom_fonts.get(fallback_key);
                    let render = ShapedTextRender::new(
                        PdfPoint::new(cur_x, text_y),
                        sub_run.font_size,
                        fallback_font,
                        &fallback_shaped,
                        prepared_font,
                        text_space,
                    )
                    .with_word_spacing(word_spacing)
                    .with_letter_spacing(letter_spacing);
                    if render.has_complex_offsets() {
                        append_positioned_shaped_text(content, render);
                    } else {
                        append_tj_shaped_text(content, render);
                    }
                    content.push_str("ET\n");
                    if let Some(end) = text_space.end_operator() {
                        content.push_str(end);
                    }
                    cur_x += w;
                    total_width += w;
                }
            } else {
                let w = render_run_glyph_layers_in_space(
                    content,
                    &sub_run,
                    cur_x,
                    text_y,
                    parent_font_size,
                    custom_fonts,
                    prepared_custom_fonts,
                    justification_word_spacing,
                    pdf_writer,
                    page_images,
                    text_space,
                    TextShadowPaint::Skip,
                );
                cur_x += w;
                total_width += w;
            }
            if segment_index + 1 < segments.len() {
                cur_x += letter_spacing;
                total_width += letter_spacing;
            }
        }
        return run.inline_advance(total_width);
    }

    let shaped = crate::text::shape_text_run(run, custom_fonts);
    let run_width = shaped.as_ref().map_or_else(
        || estimate_run_width_with_fonts(run, custom_fonts),
        |shaped| run.text_advance(shaped.width, &run.text),
    );
    let custom_font = crate::text::resolve_custom_font(
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        custom_fonts,
    );
    let font_name = resolve_font_name(run, custom_font, shaped.as_ref(), custom_fonts);
    let font_size = custom_font.map_or(text_space.length(run.font_size), |(_, font)| {
        font.adjusted_font_size(text_space.length(run.font_size))
    });

    if let Some(begin) = text_space.begin_operator() {
        content.push_str(&begin);
    }
    content.push_str(&PdfRgb::from((r, g, b)).fill_operator());
    content.push_str("BT\n");
    content.push_str(&format!("/{font_name} {font_size} Tf\n"));

    // Synthetic (faux) bold for custom-font runs that have no genuine bold face:
    // stroke each glyph outline (text render mode 2 = fill+stroke) with a thin
    // line so the stems thicken, mirroring browser algorithmic bold (CSS Fonts 4
    // §2.3). The stroke colour matches the fill so the glyph stays one colour.
    let prepared_font_name = custom_font
        .map(|(resolved_name, _)| prepared_font_name_for_run(resolved_name, run, custom_fonts));
    let prepared_font = prepared_font_name
        .as_deref()
        .and_then(|name| prepared_custom_fonts.get(name));
    let synthetic_bold_width = run
        .synthetic_bold_stroke_width(custom_fonts)
        .filter(|_| prepared_font.is_none_or(|font| !font.embeds_synthetic_weight()));
    if let Some(stroke_width) = synthetic_bold_width {
        content.push_str(&PdfRgb::from((r, g, b)).stroke_operator());
        content.push_str(&format!(
            "{} w\n",
            format_pdf_number(text_space.length(stroke_width))
        ));
        content.push_str("2 Tr\n");
    }
    // Synthetic (faux) italic when an italic request resolved to an upright face
    // (CSS Fonts 4 §2.4 `font-synthesis: style`): apply the requested oblique
    // angle as a text-matrix shear. The shear pivots on the baseline (no x-shift
    // at y=0) and does not change advances, so positioning is unaffected.
    let shear = run.synthetic_italic_shear(custom_fonts).unwrap_or_default();
    if let (Some((_, font)), Some(shaped)) = (custom_font, shaped.as_ref()) {
        let render = ShapedTextRender::new(
            PdfPoint::new(x, text_y),
            run.font_size,
            font,
            shaped,
            prepared_font,
            text_space,
        )
        .with_word_spacing(word_spacing)
        .with_letter_spacing(letter_spacing)
        .with_shear(shear);
        if render.has_complex_offsets() {
            append_positioned_shaped_text(content, render);
        } else {
            append_tj_shaped_text(content, render);
        }
    } else {
        if letter_spacing != 0.0 {
            content.push_str(&format!("{} Tc\n", text_space.length(letter_spacing)));
        }
        if word_spacing != 0.0 {
            content.push_str(&format!("{} Tw\n", text_space.length(word_spacing)));
        }
        let encoded = encode_pdf_text(&run.text);
        if text_space.is_page_css() {
            let origin = text_space.point(PdfPoint::new(x, text_y));
            content.push_str(&format!(
                "1 0 0 {} {} {} Tm\n",
                format_pdf_number(text_space.y_axis()),
                format_pdf_number(origin.x),
                format_pdf_number(origin.y),
            ));
        } else {
            content.push_str(&format!(
                "{} {} Td\n",
                format_pdf_number(x),
                format_pdf_number(text_y),
            ));
        }
        content.push_str(&format!("({encoded}) Tj\n"));
        if letter_spacing != 0.0 {
            content.push_str("0 Tc\n");
        }
        if word_spacing != 0.0 {
            content.push_str("0 Tw\n");
        }
    }

    // Restore the default fill-only render mode so the faux-bold stroke does not
    // leak into subsequent runs (Tr is a persistent text-state parameter).
    if synthetic_bold_width.is_some() {
        content.push_str("0 Tr\n");
    }

    content.push_str("ET\n");
    if let Some(end) = text_space.end_operator() {
        content.push_str(end);
    }
    run_width
}

fn run_paint_baseline(run: &TextRun, text_y: f32, parent_font_size: f32) -> f32 {
    text_y + run_vertical_align_shift(run, parent_font_size) + text_emphasis_baseline_shift(run)
}

#[allow(clippy::too_many_arguments)]
fn paint_run_text_shadows_at_baseline(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    justification_word_spacing: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    text_space: PdfContentSpace,
) {
    if !run.text_shadow.is_empty() {
        for shadow in run.text_shadow.iter().rev() {
            let (sr, sg, sb, alpha) = shadow.color.to_f32_rgba();
            if alpha <= 0.0 {
                continue;
            }
            // Try the glyph-alpha raster path first when the shadow has blur and
            // the run is a shapeable custom font (outlines available).
            if shadow.blur > 0.0
                && render_text_shadow_blur(
                    content,
                    run,
                    x + shadow.offset_x,
                    text_y - shadow.offset_y,
                    shadow.blur,
                    (sr, sg, sb, alpha),
                    custom_fonts,
                    pdf_writer,
                    page_images,
                )
            {
                continue;
            }
            let mut shadow_run = run.clone();
            shadow_run.color = shadow.color;
            shadow_run.text_shadow = Vec::new();
            shadow_run.decorations.clear();
            shadow_run.background_color = None;
            shadow_run.link_url = None;
            // `text_y` already includes the vertical-align shift; neutralise it
            // on the recursive call so the shift is not applied twice.
            shadow_run.vertical_align = VerticalAlign::Baseline;
            shadow_run.metadata.emphasis = Default::default();
            render_run_glyph_layers_in_space(
                content,
                &shadow_run,
                x + shadow.offset_x,
                text_y - shadow.offset_y,
                parent_font_size,
                custom_fonts,
                prepared_custom_fonts,
                justification_word_spacing,
                pdf_writer,
                page_images,
                text_space,
                TextShadowPaint::Skip,
            );
        }
    }
}

/// Physical horizontal advance of an already-identified
/// `text-combine-upright` composition.
///
/// CSS Writing Modes treats the composition as one em square for layout. The
/// glyphs keep their normal height and are compressed only along the horizontal
/// axis when their shaped advance exceeds one em.
pub(super) fn text_combine_advance(
    run: &TextRun,
    _custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Option<f32> {
    run.metadata
        .text_combine_upright
        .is_active()
        // CSS Writing Modes §9.1.2 gives every composition a measured 1em
        // square. Its ink may be narrower, but that must not move the square's
        // centre or the following vertical character.
        .then(|| run.inline_advance(run.font_size.max(0.0)))
}

/// Render one horizontal-in-vertical composition, returning its one-em-bounded
/// physical advance. The transform is scoped to this run, so neighbouring
/// glyphs, decorations, and the page coordinate system remain unchanged.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_text_combine_run(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> f32 {
    let raw_advance = crate::text::measure_text_width_with_shaping(
        &run.text,
        run.font_size,
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        run.shaping,
        custom_fonts,
    )
    .map_or_else(
        || run.internal_text_advance(estimate_run_width(run), &run.text),
        |width| run.internal_text_advance(width, &run.text),
    );
    let advance = run.font_size.max(0.0);
    if advance <= f32::EPSILON || raw_advance <= f32::EPSILON {
        return run.inline_advance(advance);
    }
    let scale_x = (advance / raw_advance).min(1.0);
    let painted_advance = raw_advance * scale_x;
    // The measured square is always one em wide. Center its horizontal inline
    // contents whether they needed compression or not.
    let text_x = x + (advance - painted_advance) / 2.0;
    if scale_x < 1.0 {
        // Keep `text_x` fixed after scaling the local text coordinate system:
        // page_x = scale_x * text_x + text_x * (1 - scale_x) = text_x.
        content.push_str("q\n");
        content.push_str(&format!(
            "{} 0 0 1 {} 0 cm\n",
            format_pdf_number(scale_x),
            format_pdf_number(text_x * (1.0 - scale_x)),
        ));
    }
    render_run_glyphs(
        content,
        run,
        text_x,
        text_y,
        parent_font_size,
        custom_fonts,
        prepared_custom_fonts,
        word_spacing,
        pdf_writer,
        page_images,
    );
    if scale_x < 1.0 {
        content.push_str("Q\n");
    }
    run.inline_advance(advance)
}

/// Render all text runs of a line in a single BT/ET block so the PDF viewer
/// advances the text cursor naturally after each Tj, eliminating cumulative
/// positioning errors between runs.
///
/// Falls back to per-run glyph painting when any run requires custom-font
/// shaping (complex glyph positioning).
/// `vertical-align: super`/`sub` raise/lower an atomic inline box's baseline by
/// these fractions of the parent (line) font size. CSS leaves the exact amount
/// to the UA; these match Chromium's measured superscript/subscript offsets.
/// Used both when positioning the box (`render_inline_box`) and when growing the
/// line box to contain it (`line_box_metrics`, `wrap_text_runs`), so the box and
/// the line box that holds it stay consistent.
pub(crate) const SUPER_SHIFT_RATIO: f32 = 0.38;
pub(crate) const SUB_SHIFT_RATIO: f32 = 0.23;

/// The x-height (as a fraction of em) of the parent text a `vertical-align:
/// middle` box aligns its centre against (CSS2 §10.8.1: centre at
/// `baseline + x-height/2`). Read from the largest baseline-aligned text run's
/// font; falls back to 0.5em when the line carries no measurable custom-font
/// text.
pub(super) fn line_primary_x_height_ratio(
    runs: &[TextRun],
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    let pick = runs
        .iter()
        .filter(|r| {
            r.inline_box.is_none()
                && matches!(r.vertical_align, VerticalAlign::Baseline)
                && !r.text.trim().is_empty()
        })
        .max_by(|a, b| {
            a.font_size
                .partial_cmp(&b.font_size)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .or_else(|| runs.iter().find(|r| r.inline_box.is_none()));
    if let Some(run) = pick
        && let FontFamily::Custom(name) = &run.font_family
        && let Some((_, ttf)) = crate::system_fonts::find_font(
            custom_fonts,
            name,
            run.bold,
            run.font_style.is_slanted(),
        )
    {
        return ttf.x_height_ratio();
    }
    0.5
}
