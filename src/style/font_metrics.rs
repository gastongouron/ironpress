//! Font metrics available while resolving font-relative CSS lengths.
//!
//! The layout traversal already owns the loaded-font map.  Keeping this small
//! borrowed context explicit makes `ex` and `ch` resolution safe, local, and
//! independent of thread-local state.

use crate::parser::ttf::TtfFont;
use crate::style::computed::{ComputedStyle, FontFamily, FontWeight};
#[cfg(test)]
use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
pub(crate) struct FontMetrics<'a> {
    fonts: Option<&'a dyn crate::font_registry::FontRegistry>,
}

impl<'a> FontMetrics<'a> {
    pub(crate) const fn new(fonts: &'a dyn crate::font_registry::FontRegistry) -> Self {
        Self { fonts: Some(fonts) }
    }

    /// Used x-height for the font a style selects.
    ///
    /// Outline-derived browser metrics are grid-fitted by the font's hinting
    /// instructions. Keeping that used resolution here makes direct `ex`
    /// lengths, math expressions, custom-property substitution, and
    /// `font-size` share one metric; explicit OpenType x-height metrics remain
    /// continuously scalable.
    pub(crate) fn style_x_height(self, style: &ComputedStyle) -> Option<f32> {
        self.resolve(style, |font| font.used_x_height(style.font_size))
    }

    /// `ch` advance ratio for the font a style selects.
    pub(crate) fn style_ch_ratio(self, style: &ComputedStyle) -> Option<f32> {
        self.resolve(style, TtfFont::ch_ratio)
    }

    fn resolve(self, style: &ComputedStyle, extract: impl Fn(&TtfFont) -> f32) -> Option<f32> {
        let fonts = self.fonts?;
        let resolved = crate::system_fonts::resolve_font_family(
            &style.font_stack,
            fonts,
            style.font_weight == FontWeight::Bold,
            style.font_style.is_slanted(),
            style.font_stretch,
        );
        let FontFamily::Custom(name) = resolved else {
            return None;
        };
        crate::system_fonts::find_font(
            fonts,
            &name,
            style.font_weight == FontWeight::Bold,
            style.font_style.is_slanted(),
        )
        .map(|(_, font)| extract(font))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::PT_PER_CSS_PX;
    use crate::style::computed::FontStack;

    #[test]
    fn used_x_height_matches_chromium_css_pixel_quantization() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySerif.ttf"),
        )
        .expect("ParitySerif test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySerif TTF");
        let fonts = HashMap::from([("parityserif".to_string(), font)]);
        let metrics = FontMetrics::new(&fonts);
        let mut style = ComputedStyle {
            font_stack: FontStack::from_family(FontFamily::Custom("ParitySerif".to_string())),
            ..Default::default()
        };
        let cases = [
            (8.0, 4.0),
            (9.0, 5.0),
            (10.0, 5.0),
            (11.0, 6.0),
            (12.0, 7.0),
            (13.0, 7.0),
            (14.0, 8.0),
            (15.0, 8.0),
            (16.0, 9.0),
            (17.0, 9.0),
            (18.0, 10.0),
            (20.0, 11.0),
            (24.0, 13.0),
            (30.0, 16.0),
        ];

        let actual = cases.map(|(font_size_px, _)| {
            style.font_size = font_size_px * PT_PER_CSS_PX;
            metrics.style_x_height(&style).unwrap_or_default() / PT_PER_CSS_PX
        });
        assert_eq!(actual, cases.map(|(_, x_height_px)| x_height_px));
    }
}
