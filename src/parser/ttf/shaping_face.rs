//! The shaping view a parsed font owns over its own bytes.

use std::fmt;
use std::sync::Arc;

/// Text whose guessed segment properties stand for most runs: Latin, and
/// script-neutral characters such as digits and punctuation.
const COMMON_SAMPLES: [&str; 2] = ["a", "0"];

/// A `harfrust::Font` built over a parsed font's bytes.
///
/// Building the font parses its shaping tables and harfrust caches the layout
/// data inside it, so shaping a run only wraps it in a `ShaperFont`. Layout
/// shapes every run at least twice — once to measure, again to paint — which
/// is why the parsed font keeps this value instead of rebuilding it per run.
pub(crate) struct ShapingFace {
    font: harfrust::Font,
    bytes: usize,
    /// Plans for the segment properties nearly every run has: Latin or
    /// script-neutral text (digits, punctuation) with default features.
    /// Compiling a plan collects and sorts the font's lookups, which costs
    /// more than shaping a typical run, so these are compiled with the face
    /// and only other runs compile their own.
    common_plans: Vec<harfrust::ShapePlan>,
}

impl ShapingFace {
    /// Build the shaping font for `face_index` within `data`.
    ///
    /// `None` means harfrust rejected the bytes. Shaping then declines the run
    /// rather than substituting another face.
    pub(crate) fn parse(data: Arc<Vec<u8>>, face_index: u32) -> Option<Self> {
        let bytes = data.len();
        let blob: Arc<dyn AsRef<[u8]> + Send + Sync> = data;
        let font = harfrust::Font::new(blob, face_index)?;
        let common_plans = COMMON_SAMPLES
            .iter()
            .filter_map(|sample| {
                // The plan keys a run will look up are the properties that
                // `guess_segment_properties` gives its buffer, so derive
                // them the same way.
                let mut buffer = harfrust::Buffer::new();
                buffer.push_str(sample);
                buffer.guess_segment_properties();
                (buffer.direction() != harfrust::Direction::Invalid).then(|| {
                    harfrust::ShapePlan::new(
                        &font,
                        buffer.direction(),
                        buffer.script(),
                        buffer.language(),
                        &[],
                    )
                })
            })
            .collect();
        Some(Self {
            font,
            bytes,
            common_plans,
        })
    }

    /// The plan compiled with the face for runs with `buffer`'s segment
    /// properties and `features`, if there is one.
    pub(crate) fn common_plan(
        &self,
        buffer: &harfrust::Buffer,
        features: &[harfrust::Feature],
    ) -> Option<&harfrust::ShapePlan> {
        let key = harfrust::ShapePlanKey::new(&self.font, buffer.script(), buffer.direction())
            .language(buffer.language())
            .features(features);
        self.common_plans.iter().find(|plan| key.matches(plan))
    }

    /// The shaping font.
    pub(crate) fn font(&self) -> &harfrust::Font {
        &self.font
    }
}

impl fmt::Debug for ShapingFace {
    /// Dumping the parsed tables would bury the surrounding font. The buffer
    /// size is the part that identifies which face a font is carrying.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShapingFace")
            .field("bytes", &self.bytes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parity_serif() -> ShapingFace {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySerif.ttf"),
        )
        .expect("ParitySerif test font");
        ShapingFace::parse(Arc::new(bytes), 0).expect("harfrust accepts ParitySerif")
    }

    fn guessed(text: &str) -> harfrust::Buffer {
        let mut buffer = harfrust::Buffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer
    }

    fn shaped(
        face: &ShapingFace,
        text: &str,
        plan: Option<&harfrust::ShapePlan>,
    ) -> Vec<(u32, u32, i32, i32)> {
        let mut buffer = guessed(text);
        let shaper = harfrust::ShaperFont::new(face.font());
        harfrust::shape(
            &shaper,
            &mut buffer,
            harfrust::ShapeOptions::new().plan(plan),
        )
        .expect("shaping succeeds");
        buffer
            .glyph_infos()
            .iter()
            .zip(buffer.glyph_positions())
            .map(|(info, position)| {
                (
                    info.glyph_id,
                    info.cluster,
                    position.x_advance,
                    position.x_offset,
                )
            })
            .collect()
    }

    #[test]
    fn latin_and_neutral_runs_with_default_features_use_a_face_plan() {
        let face = parity_serif();
        assert!(face.common_plan(&guessed("Heading"), &[]).is_some());
        assert!(face.common_plan(&guessed("12.5"), &[]).is_some());
        let no_kerning = [harfrust::Feature::new(harfrust::Tag::new(b"kern"), 0, ..)];
        assert!(face.common_plan(&guessed("Heading"), &no_kerning).is_none());
        assert!(face.common_plan(&guessed("שלום"), &[]).is_none());
    }

    #[test]
    fn shaping_with_a_face_plan_equals_shaping_without_one() {
        let face = parity_serif();
        for text in ["Typography, AVA 1.25", "0123456789 (x) ff fi"] {
            let plan = face.common_plan(&guessed(text), &[]);
            assert!(plan.is_some(), "{text}");
            assert_eq!(
                shaped(&face, text, plan),
                shaped(&face, text, None),
                "{text}"
            );
        }
    }
}
