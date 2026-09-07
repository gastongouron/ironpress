use std::collections::HashMap;
use std::str::FromStr;

use crate::parser::ttf::{TtfFont, parse_ttf};

use super::{
    CJK_JAPANESE_FALLBACK_KEY, CJK_KOREAN_FALLBACK_KEY, CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
    CJK_TRADITIONAL_CHINESE_FALLBACK_KEY,
};

/// A separately distributed Ironpress fallback-font package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FontPackKind {
    /// Japanese CJK glyph forms.
    CjkJapanese,
    /// Korean CJK glyph forms and Hangul.
    CjkKorean,
    /// Simplified Chinese CJK glyph forms.
    CjkSimplifiedChinese,
    /// Traditional Chinese CJK glyph forms.
    CjkTraditionalChinese,
    /// Monochrome outline emoji supported by the PDF font pipeline.
    Emoji,
}

impl FontPackKind {
    /// Stable package name used by release artifacts and JavaScript bindings.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CjkJapanese => "cjk-jp",
            Self::CjkKorean => "cjk-kr",
            Self::CjkSimplifiedChinese => "cjk-sc",
            Self::CjkTraditionalChinese => "cjk-tc",
            Self::Emoji => "emoji",
        }
    }

    /// Internal font-map role occupied by this pack.
    const fn fallback_key(self) -> &'static str {
        match self {
            Self::CjkJapanese => CJK_JAPANESE_FALLBACK_KEY,
            Self::CjkKorean => CJK_KOREAN_FALLBACK_KEY,
            Self::CjkSimplifiedChinese => CJK_SIMPLIFIED_CHINESE_FALLBACK_KEY,
            Self::CjkTraditionalChinese => CJK_TRADITIONAL_CHINESE_FALLBACK_KEY,
            Self::Emoji => crate::system_fonts::EMOJI_FALLBACK_KEY,
        }
    }
}

impl std::fmt::Display for FontPackKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for FontPackKind {
    type Err = UnknownFontPackKind;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "cjk-jp" => Ok(Self::CjkJapanese),
            "cjk-kr" => Ok(Self::CjkKorean),
            "cjk-sc" => Ok(Self::CjkSimplifiedChinese),
            "cjk-tc" => Ok(Self::CjkTraditionalChinese),
            "emoji" => Ok(Self::Emoji),
            _ => Err(UnknownFontPackKind(value.to_string())),
        }
    }
}

/// Error returned when a package name does not identify a published pack.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unknown font pack `{0}`; expected cjk-jp, cjk-kr, cjk-sc, cjk-tc, or emoji")]
pub struct UnknownFontPackKind(String);

impl UnknownFontPackKind {
    /// Return the package name that could not be parsed.
    pub fn value(&self) -> &str {
        &self.0
    }
}

/// Error returned when font-pack bytes cannot become a usable font face.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error(
    "font pack `{kind}` must contain a valid TrueType font: {reason}; use the matching Ironpress pack artifact"
)]
pub struct FontPackError {
    kind: FontPackKind,
    reason: String,
}

impl FontPackError {
    /// Return the semantic pack role that rejected the bytes.
    pub const fn kind(&self) -> FontPackKind {
        self.kind
    }

    /// Return the font parser's reason for rejecting the bytes.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// A parsed fallback font ready to be installed in a converter.
#[derive(Debug, Clone)]
pub struct FontPack {
    kind: FontPackKind,
    font: TtfFont,
}

impl FontPack {
    /// Parse raw package bytes once at the API boundary.
    ///
    /// The role selects fallback order; glyph coverage still comes from the
    /// parsed face, so partial or application-specific packs remain valid.
    pub fn parse(kind: FontPackKind, data: Vec<u8>) -> Result<Self, FontPackError> {
        let font = parse_ttf(data).map_err(|reason| FontPackError { kind, reason })?;
        Ok(Self { kind, font })
    }

    /// Return which published fallback role this pack provides.
    pub const fn kind(&self) -> FontPackKind {
        self.kind
    }
}

/// Parsed optional fonts installed on one converter.
#[derive(Debug, Clone, Default)]
pub(crate) struct FontCatalog {
    /// At most one parsed face for each semantic fallback role.
    packs: HashMap<FontPackKind, TtfFont>,
}

impl FontCatalog {
    /// Install or replace one semantic pack role.
    pub(crate) fn install(&mut self, pack: FontPack) {
        self.packs.insert(pack.kind, pack.font);
    }

    #[cfg(test)]
    pub(crate) fn get(&self, name: &str) -> Option<&TtfFont> {
        self.packs
            .iter()
            .find_map(|(kind, font)| (kind.fallback_key() == name).then_some(font))
    }

    pub(crate) fn visit<'a>(&'a self, visitor: &mut dyn FnMut(&'a str, &'a TtfFont)) {
        for (kind, font) in &self.packs {
            visitor(kind.fallback_key(), font);
        }
    }
}
