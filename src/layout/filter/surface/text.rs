//! Text and decoration painting into a filter `SourceGraphic`.

use crate::layout::engine::{FlexCell, TextLine, TextRun};
use crate::style::computed::{AlignItems, TextAlign};
use crate::types::{Color, Point, Size};

use super::canvas::SurfaceRect;
use super::painter::SourcePainter;

#[derive(Clone, Copy)]
struct DecorationSpan {
    start: f32,
    width: f32,
    baseline: f32,
    offset: Point,
}

#[derive(Clone, Copy)]
enum DecorationPhase {
    All,
    BelowText,
    AboveText,
}

impl DecorationPhase {
    const fn paints_below_text(self) -> bool {
        matches!(self, Self::All | Self::BelowText)
    }

    const fn paints_above_text(self) -> bool {
        matches!(self, Self::All | Self::AboveText)
    }
}

impl SourcePainter<'_> {
    pub(super) fn paint_text_lines(
        &mut self,
        lines: &[TextLine],
        content: SurfaceRect,
        alignment: TextAlign,
        indent: f32,
    ) -> Option<()> {
        let mut baseline_cursor = crate::render::blur::RasterBaselineCursor::new(
            content.origin.y,
            self.space.css_pixel_grid_origin.y,
        );
        let mut line_top = content.origin.y;
        for (line_index, line) in lines.iter().enumerate() {
            let baseline_ascent = line_baseline_ascent(line, self.fonts);
            let baseline = baseline_cursor.next(crate::render::blur::RasterBaselineAdvance::new(
                baseline_ascent,
                (line.height - baseline_ascent).max(0.0),
            ));
            let runs = merged_runs(&line.runs);
            let parent_font_size = crate::layout::text::line_primary_font_size(&runs);
            let line_width = runs
                .iter()
                .map(|run| run_width(run, self.fonts))
                .sum::<Option<f32>>()?;
            let first_indent = if line_index == 0 { indent } else { 0.0 };
            let line_x = match alignment {
                TextAlign::Right => {
                    content.origin.x
                        + first_indent
                        + (content.size.width - first_indent - line_width).max(0.0)
                }
                TextAlign::Center => {
                    content.origin.x
                        + first_indent
                        + (content.size.width - first_indent - line_width).max(0.0) / 2.0
                }
                _ => content.origin.x + first_indent,
            } + line.x_offset;
            self.canvas.include_paint_bounds(SurfaceRect::new(
                Point::new(line_x, line_top),
                Size::new(line_width, line.height),
            ));
            self.paint_text_runs(&runs, parent_font_size, line_x, baseline)?;
            line_top += line.height;
        }
        Some(())
    }

    fn paint_text_runs(
        &mut self,
        runs: &[TextRun],
        parent_font_size: f32,
        mut run_x: f32,
        baseline: f32,
    ) -> Option<()> {
        for run in runs {
            if run.inline_box.is_some() || run.background_color.is_some() || run.text.is_empty() {
                return None;
            }
            let (_, font) = crate::text::resolve_custom_font(
                &run.font_family,
                run.bold,
                run.font_style.is_slanted(),
                self.fonts,
            )?;
            let mut shaped = crate::text::shape_text_run(run, self.fonts)?;
            apply_authored_spacing(run, &mut shaped.glyphs);
            let run_baseline = baseline - run.glyph_baseline_shift(parent_font_size);
            let run_origin =
                crate::render::blur::GlyphBaselineOrigin::top_down(run_x, run_baseline);
            let raster = crate::render::blur::rasterize_run_alpha(
                crate::render::blur::GlyphRasterRequest {
                    font,
                    font_size: font.adjusted_font_size(run.font_size),
                    glyphs: &shaped.glyphs,
                    style: crate::render::blur::GlyphRasterStyle {
                        embolden: run
                            .synthetic_bold_stroke_width(self.fonts)
                            .unwrap_or_default(),
                        shear: run.synthetic_italic_shear(self.fonts).unwrap_or_default(),
                    },
                    origin: run_origin,
                    dpi: self.filter_dpi,
                },
            )?;
            let advance = run_width(run, self.fonts)?;
            let span = DecorationSpan {
                start: run_x,
                width: advance,
                baseline: run_baseline,
                offset: Point::ORIGIN,
            };
            self.paint_text_shadows(run, &raster, span)?;
            self.paint_run_decorations(run, span, None, DecorationPhase::BelowText)?;
            self.canvas
                .composite_mask(&raster.mask, raster.placement.mask_origin, run.color);
            self.canvas.include_paint_bounds(
                raster.paint_bounds_at(run_origin, self.canvas.pixels_per_point)?,
            );
            self.paint_run_decorations(run, span, None, DecorationPhase::AboveText)?;
            run_x += advance;
        }
        Some(())
    }

    fn paint_text_shadows(
        &mut self,
        run: &TextRun,
        raster: &crate::render::blur::GlyphRaster,
        span: DecorationSpan,
    ) -> Option<()> {
        for shadow in run.text_shadow.iter().rev() {
            if shadow.blur > 0.0 {
                return None;
            }
            let offset = Point::new(shadow.offset_x, shadow.offset_y);
            let shadow_span = DecorationSpan { offset, ..span };
            let shadow_origin = crate::render::blur::GlyphBaselineOrigin::top_down(
                span.start + offset.x,
                span.baseline + offset.y,
            );
            self.paint_run_decorations(run, shadow_span, Some(shadow.color), DecorationPhase::All)?;
            self.canvas.composite_mask(
                &raster.mask,
                raster
                    .placement
                    .mask_origin_at(shadow_origin, self.canvas.pixels_per_point)?,
                shadow.color,
            );
            self.canvas.include_paint_bounds(
                raster.paint_bounds_at(shadow_origin, self.canvas.pixels_per_point)?,
            );
        }
        Some(())
    }

    fn paint_run_decorations(
        &mut self,
        run: &TextRun,
        span: DecorationSpan,
        color_override: Option<Color>,
        phase: DecorationPhase,
    ) -> Option<()> {
        if run.decorations.is_empty() {
            return Some(());
        }
        if run.decorations.iter().any(|decoration| {
            decoration.style != crate::style::computed::TextDecorationStyle::Solid
        }) {
            return None;
        }
        let (leading, trailing) =
            crate::render::text_decoration::whitespace_insets(run, self.fonts);
        let start = span.start + leading + span.offset.x;
        let width = (span.width - leading - trailing).max(0.0);
        for decoration in &run.decorations {
            let color = color_override.unwrap_or_else(|| decoration.resolved_color(run.color));
            let thickness = crate::render::text_decoration::thickness(run, decoration);
            let mut paint_line = |line, center_y: f32| {
                let axis_from_baseline = span.baseline + span.offset.y - center_y;
                let exclusions = crate::render::text_decoration::ink_skip_intervals(
                    run,
                    decoration,
                    line,
                    axis_from_baseline,
                    self.fonts,
                )
                .into_iter()
                .map(|interval| interval.translated(span.start + span.offset.x));
                for segment in crate::render::text_decoration::visible_segments(
                    crate::render::text_decoration::InlineInterval::new(start, start + width),
                    exclusions,
                ) {
                    self.canvas.fill(
                        SurfaceRect::new(
                            Point::new(segment.start, center_y - thickness / 2.0),
                            Size::new(segment.end - segment.start, thickness),
                        ),
                        color,
                    );
                }
            };
            if decoration.lines.underline && phase.paints_below_text() {
                paint_line(
                    crate::render::text_decoration::DecorationLine::Underline,
                    span.baseline
                        + crate::render::text_decoration::underline_distance_from_baseline(
                            run, decoration,
                        )
                        + span.offset.y,
                );
            }
            if decoration.lines.line_through && phase.paints_above_text() {
                paint_line(
                    crate::render::text_decoration::DecorationLine::LineThrough,
                    span.baseline - run.font_size * 0.3 + span.offset.y,
                );
            }
            if decoration.lines.overline && phase.paints_below_text() {
                let (ascender_ratio, _) = crate::fonts::font_metrics_ratios(
                    run.css_font_family(),
                    run.bold,
                    run.font_style.is_slanted(),
                    self.fonts,
                );
                paint_line(
                    crate::render::text_decoration::DecorationLine::Overline,
                    span.baseline
                        - ascender_ratio * run.font_size
                        - crate::render::text_decoration::overline_lift(run)
                        + span.offset.y,
                );
            }
        }
        Some(())
    }
}

fn apply_authored_spacing(run: &TextRun, glyphs: &mut [crate::text::ShapedGlyph]) {
    if run.metadata.spacing.letter != 0.0 {
        let spaced_glyphs = glyphs.len().saturating_sub(1);
        for glyph in glyphs.iter_mut().take(spaced_glyphs) {
            glyph.x_advance += run.metadata.spacing.letter;
        }
    }
    if run.metadata.spacing.word != 0.0 {
        for glyph in glyphs {
            if glyph.unicode.as_slice() == [0x0020] {
                glyph.x_advance += run.metadata.spacing.word;
            }
        }
    }
}

pub(super) fn flex_cell_baseline(
    cell: &FlexCell,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> Option<f32> {
    let mut prior = 0.0;
    let last = cell
        .lines
        .iter()
        .filter(|line| line.runs.iter().any(|run| !run.text.is_empty()))
        .inspect(|line| prior += line.height)
        .last();
    let Some(last) = last else {
        return cell
            .nested_elements
            .iter()
            .find_map(|element| element.atomic_inline_baseline())
            .map(|baseline| cell.border.top.width + cell.padding.top + baseline.baseline_offset());
    };
    prior -= last.height;
    Some(cell.border.top.width + cell.padding.top + prior + line_baseline_ascent(last, fonts))
}

pub(super) fn flex_line_max_baseline(
    cells: &[FlexCell],
    alignment: AlignItems,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> Option<f32> {
    cells
        .iter()
        .filter(|cell| cell.effective_cross_alignment(alignment) == AlignItems::Baseline)
        .filter_map(|cell| flex_cell_baseline(cell, fonts))
        .reduce(f32::max)
}

pub(crate) fn table_row_baseline_shifts(
    cells: &[crate::layout::cells::TableCell],
    fonts: &dyn crate::font_registry::FontRegistry,
) -> Vec<f32> {
    let baselines = cells
        .iter()
        .map(|cell| {
            (cell.layout.alignment.block == crate::style::computed::VerticalAlign::Baseline)
                .then(|| {
                    let line = cell
                        .layout
                        .content
                        .lines
                        .iter()
                        .find(|line| line.runs.iter().any(|run| !run.text.is_empty()))?;
                    Some(
                        cell.layout.box_model.content_insets.top
                            + line_baseline_ascent(line, fonts),
                    )
                })
                .flatten()
        })
        .collect::<Vec<_>>();
    let common = baselines
        .iter()
        .filter_map(|baseline| *baseline)
        .reduce(f32::max);
    baselines
        .into_iter()
        .map(|baseline| match (baseline, common) {
            (Some(own), Some(common)) => (common - own).max(0.0),
            _ => 0.0,
        })
        .collect()
}

pub(super) fn line_baseline_ascent(
    line: &TextLine,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    line.baseline_ascent.unwrap_or_else(|| {
        let (ascent, descent) = line
            .runs
            .iter()
            .filter(|run| run.inline_box.is_none())
            .fold((0.0_f32, 0.0_f32), |(ascent, descent), run| {
                let metrics = crate::fonts::font_metrics_ratios(
                    run.css_font_family(),
                    run.bold,
                    run.font_style.is_slanted(),
                    fonts,
                );
                (
                    ascent.max(metrics.0 * run.font_size),
                    descent.max(metrics.1 * run.font_size),
                )
            });
        ascent + ((line.height - ascent - descent) / 2.0).max(0.0)
    })
}

fn run_width(run: &TextRun, fonts: &dyn crate::font_registry::FontRegistry) -> Option<f32> {
    if run.inline_box.is_some() {
        return None;
    }
    let authored_spacing = || run.metadata.spacing.add_internal_advance(0.0, &run.text);
    crate::text::measure_text_width_with_shaping(
        &run.text,
        run.font_size,
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        run.shaping,
        fonts,
    )
    .map(|width| run.inline_advance(width + authored_spacing()))
    .or_else(|| {
        Some(run.inline_advance(
            crate::fonts::str_width(&run.text, run.font_size, &run.font_family, run.bold)
                + authored_spacing(),
        ))
    })
}

fn merged_runs(runs: &[TextRun]) -> Vec<TextRun> {
    let mut merged: Vec<TextRun> = Vec::new();
    for run in runs {
        if run.inline_box.is_some() {
            merged.push(run.clone());
            continue;
        }
        if run.text.is_empty() {
            continue;
        }
        let compatible = merged.last().is_some_and(|previous| {
            previous.inline_box.is_none()
                && previous.font_size == run.font_size
                && previous.bold == run.bold
                && previous.font_style == run.font_style
                && previous.color == run.color
                && previous.font_family == run.font_family
                && previous.font_synthesis == run.font_synthesis
                && previous.vertical_align == run.vertical_align
                && previous.font_variant_position == run.font_variant_position
                && previous.metadata.spacing == run.metadata.spacing
                && previous.background_color == run.background_color
                && previous.text_shadow.is_empty()
                && run.text_shadow.is_empty()
        });
        if compatible {
            if let Some(previous) = merged.last_mut() {
                previous.text.push_str(&run.text);
            }
        } else {
            merged.push(run.clone());
        }
    }
    merged
}
