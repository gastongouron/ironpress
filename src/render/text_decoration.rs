//! Renderer-independent geometry for horizontal CSS text decorations.

use crate::layout::engine::TextRun;

mod ink_skip;

pub(crate) use ink_skip::ink_skip_intervals;

/// An inline-axis interval relative to the shaped run origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct InlineInterval {
    pub start: f32,
    pub end: f32,
}

impl InlineInterval {
    pub(crate) const fn new(start: f32, end: f32) -> Self {
        Self { start, end }
    }

    pub(crate) const fn translated(self, distance: f32) -> Self {
        Self::new(self.start + distance, self.end + distance)
    }
}

/// Portions of `bounds` that remain after ordered exclusion intervals.
pub(crate) fn visible_segments(
    bounds: InlineInterval,
    exclusions: impl IntoIterator<Item = InlineInterval>,
) -> Vec<InlineInterval> {
    let mut cursor = bounds.start;
    let mut visible = Vec::new();
    for exclusion in merge_intervals(exclusions.into_iter().collect()) {
        let start = exclusion.start.clamp(bounds.start, bounds.end);
        let end = exclusion.end.clamp(bounds.start, bounds.end);
        if start > cursor {
            visible.push(InlineInterval::new(cursor, start));
        }
        cursor = cursor.max(end);
    }
    if cursor < bounds.end {
        visible.push(InlineInterval::new(cursor, bounds.end));
    }
    visible
}

/// Which decoration line is being painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecorationLine {
    Underline,
    LineThrough,
    Overline,
}

impl DecorationLine {
    pub(crate) const fn can_skip_ink(self) -> bool {
        matches!(self, Self::Underline | Self::Overline)
    }
}

pub(crate) fn thickness(run: &TextRun, decoration: &crate::style::computed::TextDecoration) -> f32 {
    let thickness = if let Some(thickness) = decoration.thickness {
        thickness
    } else if decoration.style == crate::style::computed::TextDecorationStyle::Wavy {
        run.font_size * 0.075
    } else {
        run.font_size * 0.085
    };
    thickness.max(crate::fonts::PT_PER_CSS_PX)
}

pub(crate) fn underline_distance_from_baseline(
    run: &TextRun,
    decoration: &crate::style::computed::TextDecoration,
) -> f32 {
    let top_offset = decoration
        .underline_offset
        .unwrap_or_else(|| crate::fonts::ceil_to_css_pixel(thickness(run, decoration) / 2.0));
    top_offset + thickness(run, decoration) / 2.0
}

pub(crate) fn overline_lift(run: &TextRun) -> f32 {
    run.font_size * 0.065
}

/// Width of leading and trailing whitespace excluded from a decoration line.
pub(crate) fn whitespace_insets(
    run: &TextRun,
    custom_fonts: &dyn crate::font_registry::FontRegistry,
) -> (f32, f32) {
    if run.inline_box.is_some() {
        return (0.0, 0.0);
    }
    let leading: String = run.text.chars().take_while(|c| c.is_whitespace()).collect();
    let trailing: String = run
        .text
        .chars()
        .rev()
        .take_while(|c| c.is_whitespace())
        .collect();
    let measure = |text: &str| {
        if text.is_empty() {
            0.0
        } else {
            crate::layout::text::estimate_word_width(
                text,
                run.font_size,
                &run.font_family,
                run.bold,
                run.font_style.is_slanted(),
                custom_fonts,
            )
        }
    };
    (measure(&leading), measure(&trailing))
}

pub(super) fn merge_intervals(mut intervals: Vec<InlineInterval>) -> Vec<InlineInterval> {
    intervals.sort_by(|left, right| left.start.total_cmp(&right.start));
    let mut merged: Vec<InlineInterval> = Vec::with_capacity(intervals.len());
    for interval in intervals {
        if let Some(previous) = merged.last_mut()
            && interval.start <= previous.end
        {
            previous.end = previous.end.max(interval.end);
        } else {
            merged.push(interval);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::{DecorationLine, ink_skip_intervals};
    use crate::layout::engine::TextRun;
    use crate::style::computed::FontFamily;
    use std::collections::HashMap;

    fn parity_sans() -> HashMap<String, crate::parser::ttf::TtfFont> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/parity/fonts/ParitySans.ttf");
        let font = crate::parser::ttf::parse_ttf(std::fs::read(path).unwrap()).unwrap();
        HashMap::from([("ParitySans".to_owned(), font)])
    }

    fn underlined_run(font_size: f32, bold: bool) -> TextRun {
        TextRun {
            text: "AgBb".to_owned(),
            font_size,
            bold,
            font_family: FontFamily::Custom("ParitySans".to_owned()),
            decorations: vec![crate::style::computed::TextDecoration {
                lines: crate::style::computed::TextDecorationLines {
                    underline: true,
                    ..Default::default()
                },
                thickness: Some(1.5),
                underline_offset: Some(2.25),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn ink_skip_ignores_a_tangent_descender_but_breaks_a_crossing_descender() {
        let fonts = parity_sans();
        let normal = ink_skip_intervals(
            &underlined_run(12.0, false),
            &underlined_run(12.0, false).decorations[0],
            DecorationLine::Underline,
            -3.0,
            &fonts,
        );
        let bold = ink_skip_intervals(
            &underlined_run(13.5, true),
            &underlined_run(13.5, true).decorations[0],
            DecorationLine::Underline,
            -3.0,
            &fonts,
        );
        assert!(normal.is_empty(), "normal: {normal:?}; bold: {bold:?}");
        assert_eq!(bold.len(), 1, "normal: {normal:?}; bold: {bold:?}");
    }

    #[test]
    fn skip_ink_none_keeps_the_decoration_continuous() {
        let fonts = parity_sans();
        let mut run = underlined_run(13.5, true);
        run.decorations[0].skip_ink = crate::style::computed::TextDecorationSkipInk::None;

        assert!(
            ink_skip_intervals(
                &run,
                &run.decorations[0],
                DecorationLine::Underline,
                -3.0,
                &fonts
            )
            .is_empty()
        );
    }
}
