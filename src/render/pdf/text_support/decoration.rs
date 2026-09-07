use super::*;

use crate::render::text_decoration::overline_lift;
use crate::render::text_decoration::whitespace_insets as decoration_ws_insets;

mod stroke;

pub(in crate::render::pdf) use stroke::*;

/// Paint data for one horizontal text run's propagated CSS decorations.
///
/// Every PDF text path uses this type so nested containers, flex cells, table
/// cells, and top-level text cannot silently acquire different decoration
/// capabilities. Its phases encode CSS Text Decoration's stacking order:
/// shadows, underline/overline, glyphs, then line-through.
pub(in crate::render::pdf) struct HorizontalRunDecorations<'a> {
    run: &'a TextRun,
    custom_fonts: &'a dyn crate::font_registry::FontRegistry,
    origin: f32,
    start: f32,
    end: f32,
    baseline: f32,
    previous: Option<&'a TextRun>,
}

/// Geometry and serialization space for one horizontal line paint.
#[derive(Clone, Copy)]
pub(in crate::render::pdf) struct HorizontalLinePaint {
    pub origin: PdfPoint,
    pub line_ascender: f32,
    pub justification_word_spacing: f32,
    pub text_space: PdfContentSpace,
}

/// Paint a complete horizontal line in CSS Text Decoration stacking order.
///
/// Every caller gets the same vector sequence: decoration shadows, glyph
/// shadows, underlines/overlines, glyphs, then line-through and emphasis.
#[allow(clippy::too_many_arguments)]
pub(in crate::render::pdf) fn paint_horizontal_line_text(
    content: &mut String,
    runs: &[TextRun],
    paint: HorizontalLinePaint,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
    prepared_custom_fonts: &PreparedCustomFonts,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    let parent_font_size = crate::layout::text::line_primary_font_size(runs);
    let mut x = paint.origin.x;
    let mut decorations = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        if let Some(advance) = run.atomic_inline_advance() {
            x += advance;
            continue;
        }
        if run.text.is_empty() {
            continue;
        }
        let width = estimate_run_width_with_fonts(run, custom_fonts);
        let previous = runs[..index]
            .iter()
            .rev()
            .find(|previous| previous.inline_box.is_none() && !previous.text.is_empty());
        let baseline = paint.origin.y
            + run_vertical_align_shift(run, parent_font_size)
            + text_emphasis_baseline_shift(run)
            + drop_cap_baseline_shift(run, paint.line_ascender, custom_fonts);
        decorations.push((
            HorizontalRunDecorations::new(run, x, width, baseline, custom_fonts)
                .continuing_after(previous),
            x,
        ));
        x += width;
    }

    for (decoration, _) in &decorations {
        decoration.paint_shadows(content);
    }
    x = paint.origin.x;
    for run in runs {
        if let Some(advance) = run.atomic_inline_advance() {
            x += advance;
            continue;
        }
        if run.text.is_empty() {
            continue;
        }
        let run_y =
            paint.origin.y + drop_cap_baseline_shift(run, paint.line_ascender, custom_fonts);
        render_run_text_shadows_in_space(
            content,
            run,
            x,
            run_y,
            parent_font_size,
            custom_fonts,
            prepared_custom_fonts,
            paint.justification_word_spacing,
            pdf_writer,
            page_images,
            paint.text_space,
        );
        x += estimate_run_width_with_fonts(run, custom_fonts);
    }
    for (decoration, _) in &decorations {
        decoration.paint_below_text(content);
    }
    render_line_glyphs_without_shadows_in_space(
        content,
        runs,
        paint.origin.x,
        paint.origin.y,
        custom_fonts,
        prepared_custom_fonts,
        paint.justification_word_spacing,
        paint.line_ascender,
        pdf_writer,
        page_images,
        paint.text_space,
    );
    for (decoration, _) in &decorations {
        decoration.paint_above_text(content);
    }
    for (decoration, x) in &decorations {
        if decoration_is_emphasis(decoration.run) {
            render_text_emphasis_marks(
                content,
                decoration.run,
                TextEmphasisPlacement {
                    origin: PdfPoint::new(*x, paint.origin.y),
                    color: decoration.run.metadata.emphasis.color,
                },
                custom_fonts,
                prepared_custom_fonts,
                pdf_writer,
                page_images,
            );
        }
    }
}

impl<'a> HorizontalRunDecorations<'a> {
    pub(in crate::render::pdf) fn new(
        run: &'a TextRun,
        start: f32,
        width: f32,
        baseline: f32,
        custom_fonts: &'a dyn crate::font_registry::FontRegistry,
    ) -> Self {
        let (leading, trailing) = decoration_ws_insets(run, custom_fonts);
        Self {
            run,
            custom_fonts,
            origin: start,
            start: start + leading,
            end: start + width - trailing,
            baseline,
            previous: None,
        }
    }

    /// Join a decoration across a styled-run boundary when either side paints
    /// the same line kind. Whitespace still trims the trailing edge of this run;
    /// only its leading inset is removed.
    pub(in crate::render::pdf) fn continuing_after(
        mut self,
        previous: Option<&'a TextRun>,
    ) -> Self {
        self.previous = previous;
        self
    }

    pub(in crate::render::pdf) fn paint_shadows(&self, content: &mut String) {
        if !self.has_lines() {
            return;
        }
        for shadow in self.run.text_shadow.iter().rev() {
            if shadow.blur > 0.0 {
                continue;
            }
            self.paint_layer(
                content,
                Some(shadow.color.to_f32_rgb()),
                shadow.offset_x,
                -shadow.offset_y,
                DecorationPaintPhase::Shadow,
            );
        }
    }

    pub(in crate::render::pdf) fn paint_below_text(&self, content: &mut String) {
        if !self.has_lines() {
            return;
        }
        self.paint_layer(content, None, 0.0, 0.0, DecorationPaintPhase::BelowText);
    }

    pub(in crate::render::pdf) fn paint_above_text(&self, content: &mut String) {
        if !self
            .run
            .decorations
            .iter()
            .any(|decoration| decoration.lines.line_through)
        {
            return;
        }
        self.paint_layer(content, None, 0.0, 0.0, DecorationPaintPhase::AboveText);
    }

    /// Paint one horizontal run through the shared decoration and glyph path.
    /// Layout and annotation remain with the caller; this owns the invariant
    /// ordering shared by every horizontal text context.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::render::pdf) fn paint_text(
        &self,
        content: &mut String,
        parent_font_size: f32,
        prepared_custom_fonts: &PreparedCustomFonts,
        word_spacing: f32,
        pdf_writer: &mut PdfWriter,
        page_images: &mut Vec<ImageRef>,
    ) -> f32 {
        self.paint_shadows(content);
        render_run_text_shadows(
            content,
            self.run,
            self.origin,
            self.baseline,
            parent_font_size,
            self.custom_fonts,
            prepared_custom_fonts,
            word_spacing,
            pdf_writer,
            page_images,
        );
        self.paint_below_text(content);
        let advance = render_run_glyphs_without_shadows(
            content,
            self.run,
            self.origin,
            self.baseline,
            parent_font_size,
            self.custom_fonts,
            prepared_custom_fonts,
            word_spacing,
            pdf_writer,
            page_images,
        );
        self.paint_above_text(content);
        advance
    }

    fn has_lines(&self) -> bool {
        !self.run.decorations.is_empty()
    }

    fn paint_layer(
        &self,
        content: &mut String,
        color_override: Option<(f32, f32, f32)>,
        inline_offset: f32,
        block_offset: f32,
        phase: DecorationPaintPhase,
    ) {
        let end = self.end + inline_offset;
        for (origin_index, decoration) in self.run.decorations.iter().enumerate() {
            let color = color_override
                .unwrap_or_else(|| decoration.resolved_color(self.run.color).to_f32_rgb());
            let start = self.decoration_start(origin_index) + inline_offset;
            if decoration.lines.underline && phase.paints_below_text() {
                let axis = underline_center_y(self.run, decoration, self.baseline) - self.baseline;
                self.paint_stroke(
                    content,
                    decoration,
                    origin_index,
                    DecorationStroke::new(
                        color,
                        DecorationLine::Underline,
                        start,
                        end,
                        self.baseline + axis + block_offset,
                        axis,
                    ),
                );
            }
            if decoration.lines.line_through && phase.paints_above_text() {
                let axis = self.run.font_size * 0.3;
                self.paint_stroke(
                    content,
                    decoration,
                    origin_index,
                    DecorationStroke::new(
                        color,
                        DecorationLine::LineThrough,
                        start,
                        end,
                        self.baseline + axis + block_offset,
                        axis,
                    ),
                );
            }
            if decoration.lines.overline && phase.paints_below_text() {
                let (ascender_ratio, _) = crate::fonts::font_metrics_ratios(
                    self.run.css_font_family(),
                    self.run.bold,
                    self.run.font_style.is_slanted(),
                    self.custom_fonts,
                );
                let axis = ascender_ratio * self.run.font_size + overline_lift(self.run);
                self.paint_stroke(
                    content,
                    decoration,
                    origin_index,
                    DecorationStroke::new(
                        color,
                        DecorationLine::Overline,
                        start,
                        end,
                        self.baseline + axis + block_offset,
                        axis,
                    ),
                );
            }
        }
    }

    fn decoration_start(&self, origin_index: usize) -> f32 {
        if self.previous.is_some_and(|previous| {
            previous.decorations.get(origin_index) == self.run.decorations.get(origin_index)
        }) {
            self.origin
        } else {
            self.start
        }
    }

    fn paint_stroke(
        &self,
        content: &mut String,
        decoration: &crate::style::computed::TextDecoration,
        origin_index: usize,
        stroke: DecorationStroke,
    ) {
        let mut skip_intervals = crate::render::text_decoration::ink_skip_intervals(
            self.run,
            decoration,
            stroke.line,
            stroke.axis_from_baseline,
            self.custom_fonts,
        )
        .into_iter()
        .map(|interval| interval.translated(self.origin))
        .collect::<Vec<_>>();
        if let Some(previous_end) = self.previous_ink_skip_end(
            decoration,
            origin_index,
            stroke.line,
            stroke.axis_from_baseline,
        ) {
            skip_intervals.push(crate::render::text_decoration::InlineInterval::new(
                stroke.span.start,
                previous_end,
            ));
        }
        for segment in crate::render::text_decoration::visible_segments(stroke.span, skip_intervals)
        {
            push_decoration_stroke(
                content,
                self.run,
                decoration,
                DecorationStroke {
                    span: segment,
                    ..stroke
                },
            );
        }
    }

    fn previous_ink_skip_end(
        &self,
        decoration: &crate::style::computed::TextDecoration,
        origin_index: usize,
        line: DecorationLine,
        axis_from_baseline: f32,
    ) -> Option<f32> {
        let previous = self.previous?;
        let previous_decoration = previous.decorations.get(origin_index)?;
        if previous_decoration != decoration {
            return None;
        }
        let previous_width = estimate_run_width_with_fonts(previous, self.custom_fonts);
        let previous_origin = self.origin - previous_width;
        crate::render::text_decoration::ink_skip_intervals(
            previous,
            previous_decoration,
            line,
            axis_from_baseline,
            self.custom_fonts,
        )
        .last()
        .map(|skip| previous_origin + skip.end)
        .filter(|skip_end| *skip_end > self.origin)
    }
}

#[derive(Clone, Copy)]
enum DecorationPaintPhase {
    Shadow,
    BelowText,
    AboveText,
}

impl DecorationPaintPhase {
    const fn paints_below_text(self) -> bool {
        matches!(self, Self::Shadow | Self::BelowText)
    }

    const fn paints_above_text(self) -> bool {
        matches!(self, Self::Shadow | Self::AboveText)
    }
}
