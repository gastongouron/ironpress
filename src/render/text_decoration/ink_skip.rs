use crate::layout::engine::TextRun;
use crate::types::Point;

use super::{DecorationLine, InlineInterval, merge_intervals, thickness};

/// Glyph-ink intervals crossed by an underline or overline.
///
/// The returned ranges stay in the run's vector coordinate system. PDF paint
/// splits its decoration path around them; the filter surface uses the same
/// ranges before raster compositing. CSS Text Decoration 4 deliberately leaves
/// the exact interruption contour and side clearance to the UA.
pub(crate) fn ink_skip_intervals(
    run: &TextRun,
    decoration: &crate::style::computed::TextDecoration,
    line: DecorationLine,
    axis_from_baseline: f32,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> Vec<InlineInterval> {
    use crate::style::computed::TextDecorationSkipInk;

    if !line.can_skip_ink() || decoration.skip_ink == TextDecorationSkipInk::None {
        return Vec::new();
    }
    let Some((_, font)) = crate::text::resolve_custom_font(
        &run.font_family,
        run.bold,
        run.font_style.is_slanted(),
        custom_fonts,
    ) else {
        return Vec::new();
    };
    let Some(shaped) = crate::text::shape_text_run(run, custom_fonts) else {
        return Vec::new();
    };
    let Some(face) = font.program.shaping_face() else {
        return Vec::new();
    };
    let units_per_em = (face.units_per_em() as f32).max(1.0);
    let scale = font.adjusted_font_size(run.font_size) / units_per_em;
    let thickness = thickness(run, decoration);
    let synthetic_bold = run
        .synthetic_bold_stroke_width(custom_fonts)
        .unwrap_or_default()
        / 2.0;
    // A grazing contact with only the outer antialias fringe of a thick line
    // should not punch a conspicuous hole in the decoration. Sample the line's
    // central ink band; synthetic weight expands both the glyph and that band.
    let collision_half_band = thickness * 0.1 + synthetic_bold;
    let shear = run.synthetic_italic_shear(custom_fonts).unwrap_or_default();
    let clearance = crate::fonts::PT_PER_CSS_PX / 2.0 + thickness / 2.0;
    let mut cursor = 0.0;
    let mut intervals = Vec::new();

    for glyph in &shaped.glyphs {
        let skip_for_script = decoration.skip_ink == TextDecorationSkipInk::All
            || !glyph_unicode_is_cjk(&glyph.unicode);
        if skip_for_script {
            let mut outline = GlyphOutline::default();
            if face
                .outline_glyph(rustybuzz::ttf_parser::GlyphId(glyph.glyph_id), &mut outline)
                .is_some()
            {
                outline.finish();
                let band = InlineInterval::new(
                    (axis_from_baseline - glyph.y_offset - collision_half_band) / scale,
                    (axis_from_baseline - glyph.y_offset + collision_half_band) / scale,
                );
                let middle_y = (band.start + band.end) / 2.0;
                intervals.extend(
                    outline
                        .horizontal_ink_intervals(band)
                        .into_iter()
                        .map(|ink| {
                            InlineInterval::new(
                                cursor + glyph.x_offset + (ink.start + shear * middle_y) * scale
                                    - synthetic_bold
                                    - clearance,
                                cursor
                                    + glyph.x_offset
                                    + (ink.end + shear * middle_y) * scale
                                    + synthetic_bold
                                    + clearance,
                            )
                        }),
                );
            }
        }
        cursor += glyph.x_advance;
    }

    merge_intervals(intervals)
}

fn glyph_unicode_is_cjk(unicode: &[u16]) -> bool {
    char::decode_utf16(unicode.iter().copied())
        .filter_map(Result::ok)
        .any(|ch| {
            matches!(
                u32::from(ch),
                0x3040..=0x30ff
                    | 0x3100..=0x312f
                    | 0x31a0..=0x31bf
                    | 0x3400..=0x4dbf
                    | 0x4e00..=0x9fff
                    | 0xf900..=0xfaff
                    | 0xac00..=0xd7af
            )
        })
}

/// Flattened vector glyph contours used only for horizontal scan-line
/// intersections. This keeps ink skipping independent of output resolution and
/// avoids turning vector text decorations into raster masks.
#[derive(Default)]
struct GlyphOutline {
    contours: Vec<Vec<Point>>,
    current: Vec<Point>,
}

impl GlyphOutline {
    const CURVE_STEPS: usize = 12;
    const BAND_SAMPLES: usize = 5;

    fn finish(&mut self) {
        self.finish_contour();
    }

    fn finish_contour(&mut self) {
        if self.current.len() < 2 {
            self.current.clear();
            return;
        }
        if self.current.first() != self.current.last()
            && let Some(first) = self.current.first().copied()
        {
            self.current.push(first);
        }
        self.contours.push(std::mem::take(&mut self.current));
    }

    fn push_curve(&mut self, controls: &[Point]) {
        let Some(start) = self.current.last().copied() else {
            return;
        };
        for step in 1..=Self::CURVE_STEPS {
            let t = step as f32 / Self::CURVE_STEPS as f32;
            let point = match controls {
                [control, end] => quadratic_point(start, *control, *end, t),
                [control_1, control_2, end] => cubic_point(start, *control_1, *control_2, *end, t),
                _ => return,
            };
            self.current.push(point);
        }
    }

    fn horizontal_ink_intervals(&self, band: InlineInterval) -> Vec<InlineInterval> {
        if !band.start.is_finite() || !band.end.is_finite() || band.end < band.start {
            return Vec::new();
        }
        let mut intervals = Vec::new();
        for sample in 0..Self::BAND_SAMPLES {
            let ratio = sample as f32 / (Self::BAND_SAMPLES - 1) as f32;
            let y = band.start + (band.end - band.start) * ratio;
            let mut intersections = self
                .contours
                .iter()
                .flat_map(|contour| contour.windows(2))
                .filter_map(|edge| scanline_intersection(edge[0], edge[1], y))
                .collect::<Vec<_>>();
            intersections.sort_by(f32::total_cmp);
            intervals.extend(
                intersections
                    .chunks_exact(2)
                    .map(|pair| InlineInterval::new(pair[0], pair[1])),
            );
        }
        merge_intervals(intervals)
    }
}

impl rustybuzz::ttf_parser::OutlineBuilder for GlyphOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        self.current.push(Point::new(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.current.push(Point::new(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.push_curve(&[Point::new(x1, y1), Point::new(x, y)]);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.push_curve(&[Point::new(x1, y1), Point::new(x2, y2), Point::new(x, y)]);
    }

    fn close(&mut self) {
        self.finish_contour();
    }
}

fn scanline_intersection(start: Point, end: Point, y: f32) -> Option<f32> {
    let crosses = (start.y <= y && end.y > y) || (end.y <= y && start.y > y);
    if !crosses {
        return None;
    }
    let ratio = (y - start.y) / (end.y - start.y);
    Some(start.x + (end.x - start.x) * ratio)
}

fn quadratic_point(start: Point, control: Point, end: Point, t: f32) -> Point {
    let inverse = 1.0 - t;
    Point::new(
        inverse * inverse * start.x + 2.0 * inverse * t * control.x + t * t * end.x,
        inverse * inverse * start.y + 2.0 * inverse * t * control.y + t * t * end.y,
    )
}

fn cubic_point(start: Point, control_1: Point, control_2: Point, end: Point, t: f32) -> Point {
    let inverse = 1.0 - t;
    Point::new(
        inverse.powi(3) * start.x
            + 3.0 * inverse * inverse * t * control_1.x
            + 3.0 * inverse * t * t * control_2.x
            + t.powi(3) * end.x,
        inverse.powi(3) * start.y
            + 3.0 * inverse * inverse * t * control_1.y
            + 3.0 * inverse * t * t * control_2.y
            + t.powi(3) * end.y,
    )
}
