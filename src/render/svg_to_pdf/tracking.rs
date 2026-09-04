use crate::text::is_default_ignorable;
use unicode_segmentation::UnicodeSegmentation;

/// CSS Text's typographic-character segmentation for SVG tracking.
pub(super) struct TypographicCharacterUnits<'a> {
    text: &'a str,
    ranges: Vec<std::ops::Range<usize>>,
}

impl<'a> TypographicCharacterUnits<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        let ranges = text
            .grapheme_indices(true)
            .filter_map(|(start, unit)| {
                (!unit.chars().all(is_default_ignorable)).then_some(start..start + unit.len())
            })
            .collect();
        Self { text, ranges }
    }

    pub(super) fn unit_count(&self) -> usize {
        self.ranges.len()
    }

    pub(super) fn segments(&self) -> impl Iterator<Item = &str> {
        self.ranges
            .iter()
            .filter_map(|range| self.text.get(range.clone()))
    }

    pub(super) fn tracking_after_glyphs(&self, glyph_clusters: &[usize]) -> Vec<bool> {
        let mut tracking = vec![false; glyph_clusters.len()];
        let mut current_unit = None;
        let mut previous_glyph = None;
        for (glyph_index, cluster) in glyph_clusters.iter().copied().enumerate() {
            let unit = self
                .ranges
                .partition_point(|range| range.start <= cluster)
                .checked_sub(1);
            if let Some(unit) = unit
                && current_unit.is_some_and(|current| current != unit)
                && let Some(previous) = previous_glyph
                && let Some(slot) = tracking.get_mut(previous)
            {
                *slot = true;
            }
            if unit.is_some() {
                current_unit = unit;
            }
            previous_glyph = Some(glyph_index);
        }
        tracking
    }
}
