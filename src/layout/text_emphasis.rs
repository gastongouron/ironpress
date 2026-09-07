//! Resolved CSS `text-emphasis` state and ruby-annotation geometry.
//!
//! Emphasis geometry follows the computed CSS family even when a missing glyph
//! is painted by a fallback face. Resolving that geometry once on each final
//! text run keeps layout and paint in agreement without changing the inline box.

use crate::{
    layout::engine::TextRun,
    style::computed::{FontFamily, TextEmphasisPosition},
    types::Color,
};

/// All run-local state for CSS `text-emphasis`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TextEmphasis {
    pub(crate) mark: bool,
    /// Resolved `text-emphasis-color`, independent from text decorations.
    pub(crate) color: Color,
    pub(crate) position: TextEmphasisPosition,
    pub(crate) metrics: TextEmphasisMetrics,
}

/// Geometry shared by an emphasized base run and its synthesized mark.
///
/// CSS Text Decoration 4 §3.1 scales a filled emphasis mark to half the text
/// size. Section 3.4 gives it the line-height effect of a ruby annotation;
/// CSS Ruby 1 §§3.1.2 and 3.6 let existing leading absorb part of that
/// annotation before it needs additional line-box space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TextEmphasisMetrics {
    /// PDF-space offset for the base glyph baseline. Negative moves it toward
    /// CSS block-end.
    pub(crate) baseline_shift: f32,
    /// Extra block-end extent needed by the line box.
    pub(crate) line_box_end_extension: f32,
    /// Offset from the line's normal baseline to the mark baseline.
    pub(crate) mark_baseline_offset: f32,
}

impl TextEmphasisMetrics {
    pub(crate) const MARK_FONT_SCALE: f32 = 0.5;

    pub(crate) const fn from_run(run: &TextRun) -> Self {
        run.metadata.emphasis.metrics
    }

    fn resolve(run: &TextRun, fonts: &dyn crate::font_registry::FontRegistry) -> Self {
        if !run.metadata.emphasis.mark {
            return Self::default();
        }

        let face = EmphasisFaceMetrics::resolve(run, fonts);
        let base_line_height = face.normal_line_height(run.line_height_font_size());
        let half_leading = (run_line_height(run) - base_line_height) / 2.0;
        let block_start_leading = crate::fonts::floor_to_css_pixel(half_leading);
        let block_end_leading = crate::fonts::ceil_to_css_pixel(half_leading);
        let mark_line_height = face.normal_line_height(run.font_size * Self::MARK_FONT_SCALE);
        let is_under = run.metadata.emphasis.position.is_under();
        let annotation_overflow = (mark_line_height
            - if is_under {
                block_end_leading
            } else {
                block_start_leading
            })
        .max(0.0);
        let baseline_shift = if is_under { 0.0 } else { -annotation_overflow };
        let mark_center =
            face.mark_center_ratio.unwrap_or(0.37) * run.font_size * Self::MARK_FONT_SCALE;
        let base_glyph_top = face.base_glyph_top_ratio.unwrap_or(face.ascent_ratio) * run.font_size;
        let mark_anchor = base_glyph_top + run.font_size * 0.3 - mark_center;
        let mark_baseline_offset = if is_under {
            // The block-end annotation starts after the base's distributed
            // leading. On an odd CSS-pixel remainder, Ruby puts that extra
            // pixel on the block-end side, so both shares are retained here.
            block_start_leading + block_end_leading - mark_anchor
        } else {
            baseline_shift + mark_anchor
        };

        Self {
            baseline_shift,
            // The reference browser extends the used line on its block-end
            // side for both `over` and `under` annotations. `over` also moves
            // the base into available block-start leading; `under` leaves that
            // base baseline in place and uses the end extension for the mark.
            line_box_end_extension: annotation_overflow,
            mark_baseline_offset,
        }
    }
}

/// Cache the selected face's emphasis geometry after shaping/fallback has
/// settled each final text run. This is intentionally a mutating preparation
/// step, not a paint-time fallback: layout and paint consume the same values.
pub(crate) fn resolve_text_emphasis_metrics(
    runs: &mut [TextRun],
    fonts: &dyn crate::font_registry::FontRegistry,
) {
    for run in runs {
        run.metadata.emphasis.metrics = TextEmphasisMetrics::resolve(run, fonts);
    }
}

#[derive(Clone, Copy)]
struct EmphasisFaceMetrics {
    ascent_ratio: f32,
    descent_ratio: f32,
    line_gap_ratio: f32,
    base_glyph_top_ratio: Option<f32>,
    mark_center_ratio: Option<f32>,
}

impl EmphasisFaceMetrics {
    fn resolve(run: &TextRun, fonts: &dyn crate::font_registry::FontRegistry) -> Self {
        if let FontFamily::Custom(name) = run.css_font_family()
            && let Some((_, font)) =
                crate::system_fonts::find_font(fonts, name, run.bold, run.font_style.is_slanted())
        {
            let metrics = font.layout_vertical_metrics();
            let size_adjust = font.size_adjust_factor();
            return Self {
                ascent_ratio: metrics.ascender_ratio(font.units_per_em) * size_adjust,
                descent_ratio: metrics.descender_ratio(font.units_per_em) * size_adjust,
                line_gap_ratio: metrics.line_gap_ratio(font.units_per_em) * size_adjust,
                base_glyph_top_ratio: run
                    .text
                    .chars()
                    .find(|ch| !ch.is_whitespace())
                    .and_then(|ch| font.glyph_top_ratio(ch))
                    .map(|ratio| ratio * size_adjust),
                mark_center_ratio: font
                    .glyph_center_ratio('•')
                    .map(|ratio| ratio * size_adjust),
            };
        }

        let ascent_ratio = crate::fonts::ascender_ratio(run.css_font_family());
        let descent_ratio = crate::fonts::descender_ratio(run.css_font_family());
        let normal_ratio = crate::fonts::normal_line_height_factor(
            run.css_font_family(),
            run.bold,
            run.font_style.is_slanted(),
            fonts,
        );
        Self {
            ascent_ratio,
            descent_ratio,
            line_gap_ratio: (normal_ratio - ascent_ratio - descent_ratio).max(0.0),
            base_glyph_top_ratio: None,
            mark_center_ratio: None,
        }
    }

    fn normal_line_height(self, font_size: f32) -> f32 {
        crate::fonts::round_to_css_pixel(self.ascent_ratio * font_size)
            + crate::fonts::round_to_css_pixel(self.descent_ratio * font_size)
            + crate::fonts::round_to_css_pixel(self.line_gap_ratio * font_size)
    }
}

fn run_line_height(run: &TextRun) -> f32 {
    let factor = if run.line_height_factor.is_finite() {
        run.line_height_factor.max(0.0)
    } else {
        1.2
    };
    run.line_height_font_size() * factor
}

#[cfg(test)]
mod tests {
    use super::{TextEmphasisMetrics, resolve_text_emphasis_metrics};
    use crate::{
        layout::engine::TextRun,
        parser::ttf::TtfFont,
        style::computed::{FontFamily, TextEmphasisPosition},
    };
    use std::collections::HashMap;

    fn parity_sans_fonts() -> HashMap<String, TtfFont> {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans TTF");
        HashMap::from([("paritysans".to_string(), font)])
    }

    #[test]
    fn emphasis_metrics_use_selected_font_leading_and_annotation_height() {
        let fonts = parity_sans_fonts();
        let mut run = TextRun {
            text: "重要".to_owned(),
            font_family: FontFamily::Custom("ParitySans".to_string()),
            font_size: 21.0,
            line_height_basis: 21.0,
            line_height_factor: 1.5,
            ..Default::default()
        };
        resolve_text_emphasis_metrics(std::slice::from_mut(&mut run), &fonts);
        assert_eq!(
            TextEmphasisMetrics::from_run(&run),
            TextEmphasisMetrics::default()
        );

        run.metadata.emphasis.mark = true;
        resolve_text_emphasis_metrics(std::slice::from_mut(&mut run), &fonts);
        let tight = TextEmphasisMetrics::from_run(&run);
        assert_eq!(tight.baseline_shift, -9.0);
        assert_eq!(tight.line_box_end_extension, 9.0);

        run.line_height_factor = 3.0;
        resolve_text_emphasis_metrics(std::slice::from_mut(&mut run), &fonts);
        let roomy = TextEmphasisMetrics::from_run(&run);
        assert_eq!(roomy.baseline_shift, 0.0);
        assert_eq!(roomy.line_box_end_extension, 0.0);
    }

    #[test]
    fn under_marks_leave_the_base_baseline_but_preserve_whitespace_flow() {
        let fonts = parity_sans_fonts();
        let mut run = TextRun {
            text: "重要".to_owned(),
            font_family: FontFamily::Custom("ParitySans".to_string()),
            font_size: 21.0,
            line_height_basis: 21.0,
            line_height_factor: 1.5,
            ..Default::default()
        };
        run.metadata.emphasis.mark = true;
        run.metadata.emphasis.position = TextEmphasisPosition::UnderRight;
        resolve_text_emphasis_metrics(std::slice::from_mut(&mut run), &fonts);
        assert_eq!(TextEmphasisMetrics::from_run(&run).baseline_shift, 0.0);
        assert_eq!(
            TextEmphasisMetrics::from_run(&run).line_box_end_extension,
            8.25
        );

        run.text = " ".to_owned();
        resolve_text_emphasis_metrics(std::slice::from_mut(&mut run), &fonts);
        assert_eq!(
            TextEmphasisMetrics::from_run(&run).line_box_end_extension,
            8.25
        );
    }

    #[test]
    fn fallback_glyph_face_keeps_the_authored_emphasis_geometry() {
        let mut fonts = parity_sans_fonts();
        let japanese = crate::parser::ttf::parse_ttf(
            include_bytes!("../../tests/fonts/IronpressCjkVertical.ttf").to_vec(),
        )
        .expect("valid Japanese fixture font");
        fonts.insert("japanese".to_string(), japanese);
        let mut authored = TextRun {
            text: "重要".to_owned(),
            font_family: FontFamily::Custom("ParitySans".to_string()),
            font_size: 21.0,
            line_height_basis: 21.0,
            line_height_factor: 1.5,
            ..Default::default()
        };
        authored.metadata.emphasis.mark = true;
        let mut fallback = authored
            .clone()
            .with_glyph_fallback(FontFamily::Custom("japanese".to_string()));

        resolve_text_emphasis_metrics(std::slice::from_mut(&mut authored), &fonts);
        resolve_text_emphasis_metrics(std::slice::from_mut(&mut fallback), &fonts);

        assert_eq!(
            TextEmphasisMetrics::from_run(&fallback),
            TextEmphasisMetrics::from_run(&authored)
        );
    }
}
