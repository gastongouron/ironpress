use super::*;

/// Cross-axis geometry of a vertical line.
///
/// `over_edge` is the right edge in `vertical-rl` and the left edge in
/// `vertical-lr`. Keeping its direction alongside the coordinate prevents a
/// fallback glyph's font-derived central baseline from being reflected twice.
#[derive(Clone, Copy)]
pub(super) struct VerticalLineCrossAxis {
    over_edge: f32,
    advances_right: bool,
}

impl VerticalLineCrossAxis {
    pub(super) const fn from_content_edges(
        content_left: f32,
        content_right: f32,
        vertical_lr: bool,
    ) -> Self {
        Self {
            over_edge: if vertical_lr {
                content_left
            } else {
                content_right
            },
            advances_right: vertical_lr,
        }
    }

    fn central_baseline_x(self, distance_from_over: f32) -> f32 {
        if self.advances_right {
            self.over_edge + distance_from_over
        } else {
            self.over_edge - distance_from_over
        }
    }
}

/// The origins used by upright vertical text.
///
/// Horizontal `text-combine-upright` occupies a physical one-em square whose
/// left edge is `composition_origin.x`. Ordinary glyphs instead use the
/// font's vertical baseline. Keeping both named avoids treating a horizontal
/// composition origin as a vertical one.
#[derive(Clone, Copy)]
pub(super) struct UprightLinePosition {
    composition_origin: PdfPoint,
    cross_axis: VerticalLineCrossAxis,
}

impl UprightLinePosition {
    pub(super) const fn new(
        composition_origin: PdfPoint,
        cross_axis: VerticalLineCrossAxis,
    ) -> Self {
        Self {
            composition_origin,
            cross_axis,
        }
    }

    fn vertical_baseline_x(
        self,
        run: &TextRun,
        custom_fonts: &dyn crate::font_registry::FontRegistry,
    ) -> f32 {
        let fallback_distance = run.font_size.max(0.0) * 0.5;
        let distance_from_over = crate::text::contains_cjk_vertical_text(&run.text)
            .then(|| crate::text::upright_vertical_font_metrics(run, custom_fonts))
            .flatten()
            .map_or(fallback_distance, |metrics| {
                metrics.central_baseline_from_over_ratio() * run.font_size.max(0.0)
            });
        self.cross_axis.central_baseline_x(distance_from_over)
    }
}

/// Paint an atomic inline box (`display: inline-block`) inside a line of text.
///
/// `box_x` is the left edge of the box in PDF coordinates; `baseline_y` is the
/// text baseline of the enclosing line; `line_top_y`/`line_bottom_y` bound the
/// line box. The box is positioned vertically per its `vertical_align`, then its
/// background, border, and pre-wrapped inner text are drawn.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_inline_box(
    content: &mut String,
    inline: &crate::layout::engine::InlineBox,
    box_x: f32,
    baseline_y: f32,
    _page_height: f32,
    line_top_y: f32,
    line_bottom_y: f32,
    line_text_top_y: f32,
    line_text_bottom_y: f32,
    line_font_size: f32,
    line_height: f32,
    parent_x_height_ratio: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
    page_shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    let h = inline.height;
    // The box's own baseline as a distance from its TOP edge: for an
    // inline-block with in-flow line content this is its last line box's
    // baseline (CSS2 §10.8.1); with no content baseline the box's bottom margin
    // edge is its baseline, so the ascent equals the full height.
    let box_ascent = inline.baseline_ascent.unwrap_or(h);
    // Bottom edge of the box (PDF, y-up) for each vertical-align mode. The box
    // baseline sits at `box_top - box_ascent = box_bottom + h - box_ascent`;
    // aligning it to a target line baseline gives
    // `box_bottom = target_baseline - h + box_ascent`.
    let align_baseline = |target: f32| target - h + box_ascent;
    let box_bottom = match inline.vertical_align {
        VerticalAlign::Top => line_top_y - h,
        VerticalAlign::Bottom => line_bottom_y,
        // text-top: box top aligns to the parent's text content-area top
        // (parent baseline + parent ascent), which is below the line-box top by
        // the strut's half-leading (css2 §10.8.1).
        VerticalAlign::TextTop => line_text_top_y - h,
        // text-bottom: box bottom aligns to the parent's text content-area
        // bottom (parent baseline - parent descent).
        VerticalAlign::TextBottom => line_text_bottom_y,
        // Middle: box centre aligns roughly to the parent's mid-x-height, i.e.
        // a quarter-em above the baseline.
        // Middle: align the box centre to the parent's mid-x-height (CSS2
        // §10.8.1: baseline + x-height/2), read from the parent font — not a flat
        // 0.25em (which assumes x-height == 0.5em).
        VerticalAlign::Middle => {
            baseline_y + line_font_size * parent_x_height_ratio * 0.5 - h / 2.0
        }
        // Sub/super shift the box's baseline below/above the line baseline by a
        // fraction of the parent font size (css-inline-3; CSS2 §10.8.1). The
        // fractions match Chromium's measured subscript/superscript offsets.
        VerticalAlign::Sub => align_baseline(baseline_y - line_font_size * SUB_SHIFT_RATIO),
        VerticalAlign::Super => align_baseline(baseline_y + line_font_size * SUPER_SHIFT_RATIO),
        VerticalAlign::Length(v) => align_baseline(baseline_y + v),
        VerticalAlign::Percent(p) => align_baseline(baseline_y + line_height * p),
        // Baseline: align the box's baseline to the line baseline.
        VerticalAlign::Baseline => align_baseline(baseline_y),
    };

    // CSS `position: relative` shifts the painted box (and its inner content)
    // without changing its in-flow slot: x right, y down (PDF y is up, so the
    // downward shift subtracts from y).
    let box_x = box_x + inline.rel_offset_x;
    let box_bottom = box_bottom - inline.rel_offset_y;
    let geometry = LayoutBoxGeometry::from_layout(
        PdfRect::new(box_x, box_bottom, inline.width, h),
        &inline.paint.border,
        inline.padding,
        inline.paint.border_image.as_ref(),
    );
    let box_geometry = pdf_writer.resolve_box_geometry(geometry);
    let paint_geometry = box_geometry.painting();
    let fragment_geometry = box_geometry.fragment(Default::default());
    let background_box =
        paint_geometry.background_clip_box(BackgroundClip::Border, inline.paint.border_radii);

    // Background fill.
    if let Some(background) = inline.paint.background_color {
        paint_solid_background(
            content,
            background,
            background_box,
            page_ext_gstates,
            bg_alpha_counter,
        );
    }

    // Replaced-element image (pseudo `content: url(...)`): fill the content box
    // (inside the border) with the decoded raster, scaled to the box size.
    if let Some(image) = &inline.image {
        let content_box = paint_geometry.content_box();
        let img_obj_id = pdf_writer.add_image_object(
            &image.data,
            image.source_width,
            image.source_height,
            image.format,
            image.png_metadata.as_ref(),
        );
        let img_name = format!("Im{img_obj_id}");
        content.push_str(&format!(
            "q\n{width} 0 0 {height} {left} {bottom} cm\n/{img_name} Do\nQ\n",
            width = content_box.width,
            height = content_box.height,
            left = content_box.left,
            bottom = content_box.bottom,
        ));
        page_images.push(ImageRef {
            name: img_name,
            obj_id: img_obj_id,
        });
    }

    // Border (drawn inside the border box, matching border-box sizing).
    paint_box_decoration(
        content,
        fragment_geometry,
        &inline.paint.border,
        inline.paint.border_radii,
        inline.paint.border_image.as_ref(),
        BorderPaintResources {
            shadings: page_shadings,
            shading_counter,
            page_ext_gstates,
            alpha_counter: bg_alpha_counter,
            pdf_writer,
            page_images,
        },
    );
    if let Some(stroke) = inline.paint.centered_stroke {
        paint_centered_inline_stroke(
            content,
            paint_geometry.rounded_border_box(inline.paint.border_radii),
            stroke,
            page_ext_gstates,
            bg_alpha_counter,
        );
    }

    // Inner text lines, laid out from the content-box top downward.
    let content_box = geometry.content_box();
    let mut baseline_cursor =
        TextBaselineCursor::new(content_box.top(), pdf_writer.page_content_transform);
    for line in &inline.lines {
        let metrics = line_box_metrics(line, custom_fonts);
        let inner_y = baseline_cursor.next_horizontal(metrics);
        let merged = crate::text::coalesce_text_runs(&line.runs);
        let parent_font_size = crate::layout::text::line_primary_font_size(&merged);
        let line_ascender = line_text_top(line, custom_fonts);
        let mut x = content_box.left;
        for (run_index, run) in merged.iter().enumerate() {
            if let Some(advance) = run.atomic_inline_advance() {
                x += advance;
                continue;
            }
            if run.text.is_empty() {
                continue;
            }
            let run_y = inner_y + drop_cap_baseline_shift(run, line_ascender, custom_fonts);
            let run_width = estimate_run_width_with_fonts(run, custom_fonts);
            let previous = merged[..run_index]
                .iter()
                .rev()
                .find(|previous| previous.inline_box.is_none() && !previous.text.is_empty());
            let decoration = HorizontalRunDecorations::new(run, x, run_width, run_y, custom_fonts)
                .continuing_after(previous);
            x += decoration.paint_text(
                content,
                parent_font_size,
                prepared_custom_fonts,
                0.0,
                pdf_writer,
                page_images,
            );
        }
    }
}

fn paint_centered_inline_stroke(
    content: &mut String,
    contour: RoundedRect,
    stroke: crate::layout::engine::CenteredStroke,
    page_ext_gstates: &mut Vec<(String, f32)>,
    alpha_counter: &mut usize,
) {
    let side = stroke.side();
    let alpha = begin_border_alpha(content, page_ext_gstates, alpha_counter, side.color.alpha());
    content.push_str(&PdfRgb::from(side.color).stroke_operator());
    content.push_str("0 J\n0 j\n4 M\n");
    content.push_str(&format!("{} w\n", side.width));
    content.push_str(&contour.path_or_rect());
    content.push_str("S\n");
    end_border_alpha(content, alpha);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_line_glyphs_without_shadows_in_space(
    content: &mut String,
    runs: &[TextRun],
    start_x: f32,
    y: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    line_ascender: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    text_space: PdfContentSpace,
) {
    // Keep text runs plus any atomic inline boxes (empty text but real advance).
    let non_empty: Vec<&TextRun> = runs
        .iter()
        .filter(|r| !r.text.is_empty() || r.inline_box.is_some())
        .collect();
    if non_empty.is_empty() {
        return;
    }

    // A `vertical-align: super`/`sub` run is shifted by a fraction of the parent
    // (surrounding) font size; resolve it once for the whole line.
    let parent_font_size = crate::layout::text::line_primary_font_size(runs);

    // Every font family, inline box, and effect advances through the same
    // explicit run path. Besides keeping fallback and custom-font behavior
    // homogeneous, this avoids PDF `Tc` adding an unowned trailing advance
    // when a standard-font run ends.
    let mut x = start_x;
    for run in &non_empty {
        // Inline boxes are painted in Phase 1; here they only advance.
        if let Some(advance) = run.atomic_inline_advance() {
            x += advance;
            continue;
        }
        // A floated `::first-letter` drop cap is lowered so its glyph top
        // sits on the line's text top (css-pseudo-4 §2.2).
        let run_y = y + drop_cap_baseline_shift(run, line_ascender, custom_fonts);
        x += render_run_glyph_layers_in_space(
            content,
            run,
            x,
            run_y,
            parent_font_size,
            custom_fonts,
            prepared_custom_fonts,
            word_spacing,
            pdf_writer,
            page_images,
            text_space,
            TextShadowPaint::Skip,
        );
    }
}

/// Render an upright vertical line. A retained `text-combine-upright` marker
/// denotes one atomic horizontal composition; all other runs use the ordinary
/// text path unchanged.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_upright_vertical_line_text(
    content: &mut String,
    runs: &[TextRun],
    position: UprightLinePosition,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    line_ascender: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    let parent_font_size = crate::layout::text::line_primary_font_size(runs);
    let mut x = position.composition_origin.x;
    for run in runs
        .iter()
        .filter(|run| !run.text.is_empty() || run.inline_box.is_some())
    {
        if let Some(advance) = run.atomic_inline_advance() {
            x += advance;
            continue;
        }
        let run_y = position.composition_origin.y
            + drop_cap_baseline_shift(run, line_ascender, custom_fonts);
        let advance = if run.metadata.text_combine_upright.is_active() {
            render_text_combine_run(
                content,
                run,
                x,
                run_y,
                parent_font_size,
                custom_fonts,
                prepared_custom_fonts,
                word_spacing,
                pdf_writer,
                page_images,
            )
        } else {
            render_upright_vertical_run(
                content,
                run,
                position.vertical_baseline_x(run, custom_fonts),
                run_y,
                parent_font_size,
                custom_fonts,
                prepared_custom_fonts,
            )
            .unwrap_or_else(|| {
                render_run_glyphs(
                    content,
                    run,
                    x,
                    run_y,
                    parent_font_size,
                    custom_fonts,
                    prepared_custom_fonts,
                    word_spacing,
                    pdf_writer,
                    page_images,
                )
            })
        };
        x += advance;
    }
}

/// Paint a non-combined upright run with the font's vertical origin metrics.
///
/// The legacy horizontal path remains the safe fallback for effects it owns
/// (shadows and synthetic faces). Basic upright CJK uses this path, which is
/// enough to preserve the actual `vmtx`/`VORG` origin instead of centring by a
/// line-height heuristic.
fn render_upright_vertical_run(
    content: &mut String,
    run: &TextRun,
    vertical_baseline_x: f32,
    horizontal_baseline_y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
) -> Option<f32> {
    if !crate::text::contains_cjk_vertical_text(&run.text)
        || !run.text_shadow.is_empty()
        || run.synthetic_bold_stroke_width(custom_fonts).is_some()
        || (matches!(run.font_family, FontFamily::Custom(_))
            && crate::system_fonts::needs_faux_italic(
                custom_fonts,
                run.font_family.name(),
                run.bold,
                run.font_style.is_slanted(),
            ))
    {
        return None;
    }

    let shaped = crate::text::shape_upright_vertical_run(run, custom_fonts)?;
    let vertical_origin_offset = shaped.shaped.glyphs.first()?.y_offset;
    let glyph_baseline_y = horizontal_baseline_y
        + run_vertical_align_shift(run, parent_font_size)
        + text_emphasis_baseline_shift(run);
    // `horizontal_baseline_y` is the old renderer's physical glyph baseline.
    // The TTB shaper reports the glyph origin relative to the vertical
    // top-centre baseline, so invert that offset to preserve the line's block
    // position while using its real cross-axis vertical origin.
    let origin = PdfPoint::new(
        vertical_baseline_x,
        glyph_baseline_y - vertical_origin_offset,
    );

    content.push_str(&PdfRgb::from(run.color).fill_operator());
    content.push_str("BT\n");
    let font_size = custom_fonts
        .get(shaped.font_key)
        .map_or(run.font_size, |font| font.adjusted_font_size(run.font_size));
    content.push_str(&format!(
        "/{} {} Tf\n",
        sanitize_pdf_name(shaped.font_key),
        font_size
    ));
    append_positioned_vertical_shaped_text(
        content,
        origin,
        &shaped.shaped,
        prepared_custom_fonts.get(shaped.font_key),
    );
    content.push_str("ET\n");
    Some(run.inline_advance(run.font_size.max(0.0)))
}

pub(super) fn vertical_mixed_upright_run(run: &TextRun) -> bool {
    run.inline_box.is_none()
        && !run.text.is_empty()
        && run.text.chars().any(is_cjk_codepoint)
        && run
            .text
            .chars()
            .all(|ch| ch.is_whitespace() || is_cjk_codepoint(ch))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_vertical_mixed_upright_run(
    content: &mut String,
    run: &TextRun,
    x: f32,
    y: f32,
    parent_font_size: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    vertical_e: f32,
    vertical_f: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> f32 {
    let run_width = estimate_run_width_with_fonts(run, custom_fonts);
    let page_x = y + vertical_e - (run.font_size - run_width).max(0.0) * 0.5;
    let page_y = vertical_f - x - run.font_size * 0.75;
    content.push_str("q\n");
    content.push_str(&format!("0 1 -1 0 {vertical_f} {} cm\n", -vertical_e));
    let width = render_run_glyphs(
        content,
        run,
        page_x,
        page_y,
        parent_font_size,
        custom_fonts,
        prepared_custom_fonts,
        word_spacing,
        pdf_writer,
        page_images,
    );
    content.push_str("Q\n");
    width
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_vertical_mixed_line_text(
    content: &mut String,
    runs: &[TextRun],
    start_x: f32,
    y: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    line_ascender: f32,
    vertical_e: f32,
    vertical_f: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    let parent_font_size = crate::layout::text::line_primary_font_size(runs);
    let mut x = start_x;
    for run in runs
        .iter()
        .filter(|r| !r.text.is_empty() || r.inline_box.is_some())
    {
        if let Some(advance) = run.atomic_inline_advance() {
            x += advance;
            continue;
        }
        let run_y = y + drop_cap_baseline_shift(run, line_ascender, custom_fonts);
        let run_width = if vertical_mixed_upright_run(run) {
            render_vertical_mixed_upright_run(
                content,
                run,
                x,
                run_y,
                parent_font_size,
                custom_fonts,
                prepared_custom_fonts,
                word_spacing,
                vertical_e,
                vertical_f,
                pdf_writer,
                page_images,
            )
        } else {
            render_run_glyphs(
                content,
                run,
                x,
                run_y,
                parent_font_size,
                custom_fonts,
                prepared_custom_fonts,
                word_spacing,
                pdf_writer,
                page_images,
            )
        };
        x += run_width;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn push_line_text_clip(
    content: &mut String,
    runs: &[TextRun],
    start_x: f32,
    y: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    line_ascender: f32,
) -> bool {
    let non_empty: Vec<&TextRun> = runs
        .iter()
        .filter(|r| !r.text.is_empty() || r.inline_box.is_some())
        .collect();
    if non_empty.is_empty() {
        return false;
    }

    let parent_font_size = crate::layout::text::line_primary_font_size(runs);
    let has_inline_box = non_empty.iter().any(|r| r.inline_box.is_some());
    let has_drop_cap = non_empty.iter().any(|run| is_drop_cap_run(run));
    let all_standard = !has_inline_box
        && !has_drop_cap
        && non_empty.iter().all(|run| {
            crate::text::resolve_custom_font(
                &run.font_family,
                run.bold,
                run.font_style.is_slanted(),
                custom_fonts,
            )
            .is_none()
                && crate::text::shape_with_unicode_fallback(run, custom_fonts).is_none()
        });

    content.push_str("BT\n7 Tr\n");
    if all_standard {
        let mut cur_baseline = y;
        let mut first = true;
        for run in &non_empty {
            let font_name = resolve_font_name(run, None, None, custom_fonts);
            content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));
            let target_baseline = y
                + run_vertical_align_shift(run, parent_font_size)
                + drop_cap_baseline_shift(run, line_ascender, custom_fonts);
            if first {
                content.push_str(&format!(
                    "{} {} Td\n",
                    format_pdf_number(start_x),
                    format_pdf_number(target_baseline),
                ));
                cur_baseline = target_baseline;
                first = false;
            } else if (target_baseline - cur_baseline).abs() > f32::EPSILON {
                content.push_str(&format!(
                    "0 {} Td\n",
                    format_pdf_number(target_baseline - cur_baseline),
                ));
                cur_baseline = target_baseline;
            }
            let encoded = encode_pdf_text(&run.text);
            content.push_str(&format!("({encoded}) Tj\n"));
        }
    } else {
        let mut x = start_x;
        for run in &non_empty {
            if let Some(advance) = run.atomic_inline_advance() {
                x += advance;
                continue;
            }
            if run.text.is_empty() {
                continue;
            }
            let run_y = y
                + run_vertical_align_shift(run, parent_font_size)
                + drop_cap_baseline_shift(run, line_ascender, custom_fonts);
            let shaped = crate::text::shape_text_run(run, custom_fonts);
            let run_width = shaped.as_ref().map_or_else(
                || estimate_run_width_with_fonts(run, custom_fonts),
                |shaped| shaped.width,
            );
            let custom_font = crate::text::resolve_custom_font(
                &run.font_family,
                run.bold,
                run.font_style.is_slanted(),
                custom_fonts,
            );
            let font_name = resolve_font_name(run, custom_font, shaped.as_ref(), custom_fonts);
            let font_size = custom_font.map_or(run.font_size, |(_, font)| {
                font.adjusted_font_size(run.font_size)
            });
            content.push_str(&format!("/{font_name} {font_size} Tf\n"));
            let prepared_font_name = custom_font.map(|(resolved_name, _)| {
                prepared_font_name_for_run(resolved_name, run, custom_fonts)
            });
            let prepared_font = prepared_font_name
                .as_deref()
                .and_then(|name| prepared_custom_fonts.get(name));
            let synthetic_bold_width = run
                .synthetic_bold_stroke_width(custom_fonts)
                .filter(|_| prepared_font.is_none_or(|font| !font.embeds_synthetic_weight()));
            let shear = run.synthetic_italic_shear(custom_fonts).unwrap_or_default();
            if let (Some((_, font)), Some(shaped)) = (custom_font, shaped.as_ref()) {
                // Text render mode 7 clips to fills only, so expand a synthetic
                // weight by repeating the outline around half its stroke width.
                let embolden = synthetic_bold_width.unwrap_or_default() / 2.0;
                let offsets = [
                    (0.0, 0.0),
                    (-embolden, 0.0),
                    (embolden, 0.0),
                    (0.0, embolden),
                    (0.0, -embolden),
                    (-embolden, embolden),
                    (embolden, embolden),
                    (0.0, embolden * 2.0),
                    (-embolden * 1.5, embolden),
                ];
                let offset_count = synthetic_bold_width.map_or(1, |_| offsets.len());
                for (dx, dy) in offsets.iter().take(offset_count) {
                    let render = ShapedTextRender::new(
                        PdfPoint::new(x + dx, run_y + dy),
                        run.font_size,
                        font,
                        shaped,
                        prepared_font,
                        PdfContentSpace::Points,
                    )
                    .with_word_spacing(word_spacing)
                    .with_shear(shear);
                    if render.has_complex_offsets() {
                        append_positioned_shaped_text(content, render);
                    } else {
                        append_tj_shaped_text(content, render);
                    }
                }
            } else {
                let encoded = encode_pdf_text(&run.text);
                content.push_str(&format!(
                    "1 0 0 1 {} {} Tm\n",
                    format_pdf_number(x),
                    format_pdf_number(run_y),
                ));
                content.push_str(&format!("({encoded}) Tj\n"));
            }
            x += run_width;
        }
    }
    content.push_str("ET\n");
    true
}
