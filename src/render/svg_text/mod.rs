pub(crate) mod fonts;

use crate::layout::engine::TextShaping;
use crate::parser::svg::{SvgFontSize, SvgLetterSpacing};

/// Resolved SVG text dimensions shared by font discovery and PDF painting.
///
/// Keeping size-dependent spacing and shaping together prevents the subset
/// collector from preparing a different glyph sequence than the renderer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SvgTextSizing {
    font_size: f32,
    letter_spacing: f32,
}

impl SvgTextSizing {
    pub(crate) const fn initial(font_size: f32) -> Self {
        Self {
            font_size,
            letter_spacing: 0.0,
        }
    }

    /// Advance inherited SVG text sizing to one element's computed values.
    ///
    /// Relative letter spacing is resolved where it is declared. Descendants
    /// therefore inherit an absolute computed length even when they change
    /// their own font size.
    pub(crate) fn cascade(
        self,
        font_size: Option<SvgFontSize>,
        letter_spacing: Option<&SvgLetterSpacing>,
    ) -> Self {
        let font_size = font_size
            .map(|size| size.resolve_user_units(self.font_size))
            .unwrap_or(self.font_size);
        let letter_spacing = letter_spacing
            .map(|spacing| spacing.resolve_user_units(font_size))
            .unwrap_or(self.letter_spacing);
        Self {
            font_size,
            letter_spacing,
        }
    }

    pub(crate) const fn font_size(self) -> f32 {
        self.font_size
    }

    pub(crate) const fn letter_spacing(self) -> f32 {
        self.letter_spacing
    }

    pub(crate) fn shaping(self) -> TextShaping {
        if self.letter_spacing == 0.0 {
            TextShaping::default()
        } else {
            TextShaping::KERNING_ONLY
        }
    }
}
