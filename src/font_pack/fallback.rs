use crate::parser::ttf::TtfFont;
#[cfg(test)]
use std::collections::HashMap;

use super::FontLocale;

pub(crate) const CJK_JAPANESE_FALLBACK_KEY: &str = "__cjk_japanese_fallback";
pub(crate) const CJK_KOREAN_FALLBACK_KEY: &str = "__cjk_korean_fallback";
pub(crate) const CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY: &str = "__cjk_simplified_chinese_fallback";
pub(crate) const CJK_TRADITIONAL_CHINESE_FALLBACK_KEY: &str = "__cjk_traditional_chinese_fallback";

/// Ordered fallback roles for one inherited language context.
pub(crate) fn fallback_keys(locale: FontLocale) -> [&'static str; 8] {
    use crate::system_fonts::{
        ARABIC_FALLBACK_KEY, EMOJI_FALLBACK_KEY, MULTILINGUAL_FALLBACK_KEY, UNICODE_FALLBACK_KEY,
    };

    match locale {
        FontLocale::Japanese => [
            ARABIC_FALLBACK_KEY,
            MULTILINGUAL_FALLBACK_KEY,
            EMOJI_FALLBACK_KEY,
            CJK_JAPANESE_FALLBACK_KEY,
            UNICODE_FALLBACK_KEY,
            CJK_KOREAN_FALLBACK_KEY,
            CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
            CJK_TRADITIONAL_CHINESE_FALLBACK_KEY,
        ],
        FontLocale::Korean => [
            ARABIC_FALLBACK_KEY,
            MULTILINGUAL_FALLBACK_KEY,
            EMOJI_FALLBACK_KEY,
            CJK_KOREAN_FALLBACK_KEY,
            UNICODE_FALLBACK_KEY,
            CJK_JAPANESE_FALLBACK_KEY,
            CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
            CJK_TRADITIONAL_CHINESE_FALLBACK_KEY,
        ],
        FontLocale::SimplifiedChinese => [
            ARABIC_FALLBACK_KEY,
            MULTILINGUAL_FALLBACK_KEY,
            EMOJI_FALLBACK_KEY,
            CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
            UNICODE_FALLBACK_KEY,
            CJK_JAPANESE_FALLBACK_KEY,
            CJK_KOREAN_FALLBACK_KEY,
            CJK_TRADITIONAL_CHINESE_FALLBACK_KEY,
        ],
        FontLocale::TraditionalChinese => [
            ARABIC_FALLBACK_KEY,
            MULTILINGUAL_FALLBACK_KEY,
            EMOJI_FALLBACK_KEY,
            CJK_TRADITIONAL_CHINESE_FALLBACK_KEY,
            UNICODE_FALLBACK_KEY,
            CJK_JAPANESE_FALLBACK_KEY,
            CJK_KOREAN_FALLBACK_KEY,
            CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
        ],
        FontLocale::Unspecified => [
            ARABIC_FALLBACK_KEY,
            MULTILINGUAL_FALLBACK_KEY,
            EMOJI_FALLBACK_KEY,
            UNICODE_FALLBACK_KEY,
            CJK_JAPANESE_FALLBACK_KEY,
            CJK_KOREAN_FALLBACK_KEY,
            CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
            CJK_TRADITIONAL_CHINESE_FALLBACK_KEY,
        ],
    }
}

/// Registered fallback faces resolved for one inherited document language.
pub(crate) struct FontFallbacks<'a> {
    locale: FontLocale,
    fonts: &'a dyn crate::font_registry::FontRegistry,
}

impl<'a> FontFallbacks<'a> {
    /// Bind the fallback policy to the fonts available for one conversion.
    pub(crate) const fn new(
        locale: FontLocale,
        fonts: &'a dyn crate::font_registry::FontRegistry,
    ) -> Self {
        Self { locale, fonts }
    }

    /// Return whether this conversion has at least one fallback face.
    pub(crate) fn is_empty(&self) -> bool {
        !fallback_keys(self.locale)
            .iter()
            .any(|key| self.fonts.contains_key(key))
    }

    /// Resolve the first face that covers one Unicode grapheme cluster.
    pub(crate) fn resolve_cluster(&self, cluster: &str) -> Option<&'a str> {
        if crate::fonts::EmojiPresentation::appears_in(cluster)
            && let Some(key) = self.covering_key(crate::system_fonts::EMOJI_FALLBACK_KEY, cluster)
        {
            return Some(key);
        }

        fallback_keys(self.locale)
            .into_iter()
            .find_map(|key| self.covering_key(key, cluster))
    }

    /// Return an available fallback only when every scalar has a real glyph.
    fn covering_key(&self, key: &str, cluster: &str) -> Option<&'a str> {
        self.fonts
            .get_key_value(key)
            .and_then(|(stored_key, font)| font_covers_cluster(font, cluster).then_some(stored_key))
    }
}

/// Font fallback must preserve a grapheme cluster on one face.
fn font_covers_cluster(font: &TtfFont, cluster: &str) -> bool {
    cluster.chars().all(|character| {
        font.cmap
            .get(&(character as u32))
            .is_some_and(|glyph| *glyph != 0)
    })
}

#[cfg(test)]
mod tests {
    use crate::parser::ttf::parse_ttf;

    use super::*;

    #[test]
    fn regional_pack_precedes_platform_fallback_for_tagged_text() {
        for (locale, regional_key) in [
            (FontLocale::Japanese, CJK_JAPANESE_FALLBACK_KEY),
            (FontLocale::Korean, CJK_KOREAN_FALLBACK_KEY),
            (
                FontLocale::SimplifiedChinese,
                CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
            ),
            (
                FontLocale::TraditionalChinese,
                CJK_TRADITIONAL_CHINESE_FALLBACK_KEY,
            ),
        ] {
            let keys = fallback_keys(locale);
            let regional_position = keys
                .iter()
                .position(|key| *key == regional_key)
                .expect("regional pack in fallback chain");
            let platform_position = keys
                .iter()
                .position(|key| *key == crate::system_fonts::UNICODE_FALLBACK_KEY)
                .expect("platform fallback in fallback chain");

            assert!(regional_position < platform_position);
        }
    }

    #[test]
    fn emoji_presentation_selector_stays_with_its_grapheme_cluster() {
        let emoji =
            parse_ttf(include_bytes!("../../tests/fonts/NotoEmoji-TestSubset.ttf").to_vec())
                .expect("valid emoji fixture font");
        let fonts = HashMap::from([(crate::system_fonts::EMOJI_FALLBACK_KEY.to_string(), emoji)]);
        let fallbacks = FontFallbacks::new(FontLocale::Unspecified, &fonts);

        assert_eq!(
            fallbacks.resolve_cluster("\u{2764}\u{fe0f}"),
            Some(crate::system_fonts::EMOJI_FALLBACK_KEY)
        );
    }
}
