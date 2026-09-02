use crate::parser::ttf::TtfFont;
use crate::style::computed::{FontFamily, parse_font_stack};
use crate::style::font_family::{CssFontFamily, CssFontFamilyList};
use crate::text::AuthoredFontFaces;
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

/// One Base-14 face selected from an authored SVG font stack.
#[derive(Clone, PartialEq)]
pub(crate) struct SvgBase14TextFace {
    family: FontFamily,
    bold: bool,
    italic: bool,
}

impl SvgBase14TextFace {
    pub(crate) fn from_pdf_name(name: &str, bold: bool, italic: bool) -> Self {
        let family = if name.starts_with("Times") {
            FontFamily::TimesRoman
        } else if name.starts_with("Courier") {
            FontFamily::Courier
        } else {
            FontFamily::Helvetica
        };
        Self {
            family,
            bold,
            italic,
        }
    }

    fn from_css_family(family: &CssFontFamily, bold: bool, italic: bool) -> Option<Self> {
        let lower = family.name().to_ascii_lowercase();
        if family.is_quoted() && matches!(lower.as_str(), "serif" | "sans-serif" | "monospace") {
            return None;
        }
        match parse_font_stack(family.name()).primary() {
            FontFamily::Custom(_) => None,
            family => Some(Self {
                family,
                bold,
                italic,
            }),
        }
    }

    pub(crate) fn pdf_name(&self) -> &'static str {
        crate::fonts::pdf_font_name(self.family.name(), self.bold, self.italic)
    }

    pub(crate) fn family(&self) -> &FontFamily {
        &self.family
    }

    pub(crate) const fn bold(&self) -> bool {
        self.bold
    }
}

/// One registered font face selected for a contiguous SVG text run.
#[derive(Clone, Copy)]
pub(crate) struct SvgCustomTextFace<'fonts> {
    key: &'fonts str,
    font: &'fonts TtfFont,
}

impl<'fonts> SvgCustomTextFace<'fonts> {
    pub(crate) const fn key(self) -> &'fonts str {
        self.key
    }

    pub(crate) const fn font(self) -> &'fonts TtfFont {
        self.font
    }
}

/// Closed set of font technologies that the SVG PDF renderer can paint.
#[derive(Clone)]
pub(crate) enum SvgTextFace<'fonts> {
    Base14(SvgBase14TextFace),
    Custom(SvgCustomTextFace<'fonts>),
}

impl SvgTextFace<'_> {
    fn same_face(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Base14(left), Self::Base14(right)) => left == right,
            (Self::Custom(left), Self::Custom(right)) => left.key == right.key,
            _ => false,
        }
    }
}

/// Text slice whose complete grapheme clusters resolve to one font face.
pub(crate) struct SvgTextFontRun<'text, 'fonts> {
    pub(crate) text: &'text str,
    pub(crate) face: SvgTextFace<'fonts>,
}

/// Resolve an SVG font stack in authored order at grapheme-cluster boundaries.
///
/// A registered named family shadows Ironpress's Base-14 compatibility aliases.
/// Its composite `@font-face` members are tried before matching continues with
/// the next family. Unquoted generic families resolve to their Base-14 face.
pub(crate) fn resolve_svg_text_font_runs<'text, 'fonts>(
    text: &'text str,
    family_stack: &str,
    fallback_pdf_font: &str,
    bold: bool,
    italic: bool,
    fonts: Option<&'fonts HashMap<String, TtfFont>>,
) -> Vec<SvgTextFontRun<'text, 'fonts>> {
    if text.is_empty() {
        return Vec::new();
    }

    let fallback = SvgTextFace::Base14(SvgBase14TextFace::from_pdf_name(
        fallback_pdf_font,
        bold,
        italic,
    ));
    let candidates = SvgTextFontCandidates::new(family_stack, fonts, bold, italic, fallback);
    let mut runs = Vec::new();
    let mut run_start = 0;
    let mut active_face: Option<SvgTextFace<'fonts>> = None;

    for (offset, cluster) in text.grapheme_indices(true) {
        let face = candidates.resolve_cluster(cluster);
        if active_face
            .as_ref()
            .is_some_and(|active| active.same_face(&face))
        {
            continue;
        }
        if let Some(previous) = active_face.replace(face) {
            runs.push(SvgTextFontRun {
                text: &text[run_start..offset],
                face: previous,
            });
            run_start = offset;
        }
    }

    if let Some(face) = active_face {
        runs.push(SvgTextFontRun {
            text: &text[run_start..],
            face,
        });
    }
    runs
}

/// All registered faces eligible for one authored SVG family.
struct SvgAuthoredFamilyCandidate<'fonts> {
    faces: AuthoredFontFaces<'fonts>,
}

impl<'fonts> SvgAuthoredFamilyCandidate<'fonts> {
    /// Select this family's first face covering the complete grapheme cluster.
    fn resolve_cluster(&self, cluster: &str) -> Option<SvgTextFace<'fonts>> {
        let face = self.faces.covering_face(cluster)?;
        Some(SvgTextFace::Custom(SvgCustomTextFace {
            key: face.key(),
            font: face.font(),
        }))
    }
}

/// One parsed family in an SVG `font-family` value.
enum SvgTextFamilyCandidate<'fonts> {
    Authored(SvgAuthoredFamilyCandidate<'fonts>),
    Base14(SvgBase14TextFace),
}

impl<'fonts> SvgTextFamilyCandidate<'fonts> {
    /// Resolve a CSS family to the font technology that can paint it.
    fn from_css_family(
        family: &CssFontFamily,
        fonts: Option<&'fonts HashMap<String, TtfFont>>,
        bold: bool,
        italic: bool,
    ) -> Option<Self> {
        if let Some(fonts) = fonts {
            let authored = AuthoredFontFaces::resolve(
                &FontFamily::Custom(family.name().to_string()),
                bold,
                italic,
                fonts,
            );
            if authored.has_custom_faces() {
                return Some(Self::Authored(SvgAuthoredFamilyCandidate {
                    faces: authored,
                }));
            }
        }
        SvgBase14TextFace::from_css_family(family, bold, italic).map(Self::Base14)
    }

    /// Select a face when this family covers the complete grapheme cluster.
    fn resolve_cluster(&self, cluster: &str) -> Option<SvgTextFace<'fonts>> {
        match self {
            Self::Authored(candidate) => candidate.resolve_cluster(cluster),
            Self::Base14(face) if crate::render::pdf::is_winansi_encodable(cluster) => {
                Some(SvgTextFace::Base14(face.clone()))
            }
            Self::Base14(_) => None,
        }
    }
}

/// Parsed, ordered font candidates shared by SVG collection and rendering.
struct SvgTextFontCandidates<'fonts> {
    families: Vec<SvgTextFamilyCandidate<'fonts>>,
    fonts: Option<&'fonts HashMap<String, TtfFont>>,
    fallback: SvgTextFace<'fonts>,
}

impl<'fonts> SvgTextFontCandidates<'fonts> {
    /// Parse the authored family list and prepare its reusable candidates.
    fn new(
        family_stack: &str,
        fonts: Option<&'fonts HashMap<String, TtfFont>>,
        bold: bool,
        italic: bool,
        fallback: SvgTextFace<'fonts>,
    ) -> Self {
        let families = CssFontFamilyList::parse(family_stack)
            .map(|stack| {
                stack
                    .families()
                    .iter()
                    .filter_map(|family| {
                        SvgTextFamilyCandidate::from_css_family(family, fonts, bold, italic)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            families,
            fonts,
            fallback,
        }
    }

    /// Select a face by authored family order, then compatibility fallback.
    fn resolve_cluster(&self, cluster: &str) -> SvgTextFace<'fonts> {
        if let Some(face) = self
            .families
            .iter()
            .find_map(|candidate| candidate.resolve_cluster(cluster))
        {
            return face;
        }
        if crate::render::pdf::is_winansi_encodable(cluster) {
            return self.fallback.clone();
        }
        if let Some(fonts) = self.fonts {
            let fallbacks = crate::font_pack::FontFallbacks::new(
                crate::font_pack::FontLocale::Unspecified,
                fonts,
            );
            if let Some(key) = fallbacks.resolve_cluster(cluster)
                && let Some(font) = fonts.get(key)
            {
                return SvgTextFace::Custom(SvgCustomTextFace { key, font });
            }
        }
        self.fallback.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parity_font(path: &str) -> TtfFont {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts")
                .join(path),
        )
        .expect("parity test font");
        crate::parser::ttf::parse_ttf(bytes).expect("valid parity test font")
    }

    fn font_covering(character: char, name: &str) -> TtfFont {
        let mut font = parity_font("ParitySans.ttf");
        let glyph = font
            .cmap
            .get(&u32::from('A'))
            .copied()
            .expect("ParitySans covers A");
        font.font_name = name.to_string();
        font.cmap.clear();
        font.cmap.insert(u32::from(character), glyph);
        font
    }

    #[test]
    fn composite_unicode_range_faces_precede_later_family_and_global_fallback() {
        let family_key = crate::system_fonts::font_variant_key("Composite", false, false);
        let latin_range_key = crate::font_face_range_key(&family_key, 0);
        let arabic_range_key = crate::font_face_range_key(&family_key, 1);
        let latin_source_key = crate::font_face_source_key(&family_key, 0);
        let arabic_source_key = crate::font_face_source_key(&family_key, 1);
        let latin = font_covering('A', "Composite Latin");
        let fonts = HashMap::from([
            (family_key.clone(), latin.clone()),
            (latin_range_key.clone(), latin),
            (
                latin_source_key.clone(),
                font_covering('A', "Composite Latin"),
            ),
            (
                arabic_range_key.clone(),
                font_covering('\u{0627}', "Composite Arabic"),
            ),
            (
                arabic_source_key.clone(),
                font_covering('\u{0627}', "Composite Arabic"),
            ),
            (
                "later".to_string(),
                font_covering('\u{0627}', "Later Authored Fallback"),
            ),
            (
                crate::system_fonts::UNICODE_FALLBACK_KEY.to_string(),
                font_covering('\u{0627}', "Global Fallback"),
            ),
        ]);

        let runs = resolve_svg_text_font_runs(
            "A\u{0627}",
            "Composite, Later",
            "Helvetica",
            false,
            false,
            Some(&fonts),
        );
        let keys = runs
            .iter()
            .map(|run| match run.face {
                SvgTextFace::Custom(face) => face.key(),
                SvgTextFace::Base14(_) => "base14",
            })
            .collect::<Vec<_>>();

        assert_eq!(keys, vec![latin_source_key, arabic_source_key]);
    }
}
