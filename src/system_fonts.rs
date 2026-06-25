use crate::parser::css::{CssRule, CssValue, parse_inline_style};
use crate::parser::dom::DomNode;
use crate::parser::ttf::{TtfFont, parse_ttf, parse_ttf_with_index};
use crate::style::computed::{FontFamily, FontStack, parse_font_stack};
use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

const FONT_VARIANTS: &[FontVariant] = &[
    FontVariant::new(false, false),
    FontVariant::new(true, false),
    FontVariant::new(false, true),
    FontVariant::new(true, true),
];

const UI_SANS_FALLBACK_FAMILIES: &[&str] = &[
    "DejaVu Sans",
    "Arial",
    "Noto Sans",
    "Liberation Sans",
    "FreeSans",
];

#[derive(Clone, Copy)]
struct FontVariant {
    bold: bool,
    italic: bool,
}

impl FontVariant {
    const fn new(bold: bool, italic: bool) -> Self {
        Self { bold, italic }
    }

    const fn style(self) -> fontdb::Style {
        if self.italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        }
    }

    const fn weight(self) -> fontdb::Weight {
        if self.bold {
            fontdb::Weight::BOLD
        } else {
            fontdb::Weight::NORMAL
        }
    }

    const fn fontconfig_style(self) -> &'static str {
        match (self.bold, self.italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (false, false) => "Regular",
        }
    }
}

struct SystemFontQuery<'a> {
    family: &'a str,
    variant: FontVariant,
}

impl<'a> SystemFontQuery<'a> {
    fn new(family: &'a str, variant: FontVariant) -> Self {
        Self { family, variant }
    }

    fn variant_key(&self) -> String {
        font_variant_key(self.family, self.variant.bold, self.variant.italic)
    }

    fn normalized_family(&self) -> &str {
        match self.family.trim().to_ascii_lowercase().as_str() {
            "ui-sans-serif" | "system-ui" | "-apple-system" | "blinkmacsystemfont" => "sans-serif",
            "ui-serif" => "serif",
            "ui-monospace" => "monospace",
            _ => self.family.trim(),
        }
    }

    fn prefers_ui_sans_resolution(&self) -> bool {
        matches!(
            self.family.trim().to_ascii_lowercase().as_str(),
            "ui-sans-serif" | "system-ui" | "-apple-system" | "blinkmacsystemfont"
        )
    }

    fn fontconfig_pattern(&self) -> String {
        format!(
            "{}:style={}",
            self.normalized_family(),
            self.variant.fontconfig_style()
        )
    }

    fn fontdb_families(&self) -> Vec<fontdb::Family<'_>> {
        if self.prefers_ui_sans_resolution() {
            return vec![
                fontdb::Family::Name("DejaVu Sans"),
                fontdb::Family::Name("Arial"),
                fontdb::Family::Name("Noto Sans"),
                fontdb::Family::Name("Liberation Sans"),
                fontdb::Family::Name("FreeSans"),
                fontdb::Family::SansSerif,
            ];
        }

        match self.normalized_family() {
            "sans-serif" => vec![fontdb::Family::SansSerif],
            "serif" => vec![fontdb::Family::Serif],
            "monospace" => vec![fontdb::Family::Monospace],
            family => vec![fontdb::Family::Name(family)],
        }
    }
}

pub(crate) fn font_variant_key(family: &str, bold: bool, italic: bool) -> String {
    let base = family.trim().to_ascii_lowercase();
    match (bold, italic) {
        (false, false) => base,
        (true, false) => format!("{base}__bold"),
        (false, true) => format!("{base}__italic"),
        (true, true) => format!("{base}__bold_italic"),
    }
}

fn exact_font_variant_key(family: &str, bold: bool, italic: bool) -> String {
    let base = family.trim();
    match (bold, italic) {
        (false, false) => base.to_string(),
        (true, false) => format!("{base}__bold"),
        (false, true) => format!("{base}__italic"),
        (true, true) => format!("{base}__bold_italic"),
    }
}

/// Returns `true` when the map holds a face for `family` matching the *exact*
/// requested weight/style — without [`find_font`]'s fallback to the regular
/// variant. Used so a generic-name registration (e.g. `add_font("serif", …)`)
/// that supplies only a regular face does not shadow a genuine bold/italic face
/// from the bundled fallback.
fn has_exact_variant(
    fonts: &HashMap<String, TtfFont>,
    family: &str,
    bold: bool,
    italic: bool,
) -> bool {
    fonts.contains_key(&font_variant_key(family, bold, italic))
        || fonts.contains_key(&exact_font_variant_key(family, bold, italic))
}

pub(crate) fn find_font<'a>(
    fonts: &'a HashMap<String, TtfFont>,
    family: &str,
    bold: bool,
    italic: bool,
) -> Option<(&'a str, &'a TtfFont)> {
    let candidates = [
        font_variant_key(family, bold, italic),
        font_variant_key(family, false, false),
        exact_font_variant_key(family, bold, italic),
        exact_font_variant_key(family, false, false),
    ];

    candidates.into_iter().find_map(|key| {
        fonts
            .get_key_value(&key)
            .map(|(name, font)| (name.as_str(), font))
    })
}

/// Returns `true` when a *bold* face is requested for `family` but no genuine
/// bold variant is loaded (so [`find_font`] resolves to the regular weight).
///
/// In that case the renderer synthesises bold by stroking the glyph outlines —
/// matching browsers, which apply algorithmic (faux) bold when a family lacks a
/// real bold face (CSS Fonts 4 §2.3 "synthetic bold"). Returns `false` when the
/// run is not bold, when a real bold face exists, or when no face is found.
pub(crate) fn needs_faux_bold(
    fonts: &HashMap<String, TtfFont>,
    family: &str,
    bold: bool,
    italic: bool,
) -> bool {
    if !bold {
        return false;
    }
    // The face a bold request actually resolves to. A font *query* (e.g. system
    // fontdb) often substitutes the regular weight for a missing bold and even
    // registers it under the `__bold` key, so key presence alone is not enough —
    // check the resolved face's true weight. Synthesise bold only when a face is
    // found AND it is not itself bold.
    match find_font(fonts, family, true, italic) {
        Some((_, font)) => !font.is_bold,
        None => false,
    }
}

/// True when an `italic` request resolves to a face that is NOT genuinely
/// italic — so the renderer must synthesise (faux) italic with an algorithmic
/// shear (CSS Fonts 4 §2.4 `font-synthesis: style`). Mirrors `needs_faux_bold`:
/// a font query often substitutes the upright face for a missing italic, so we
/// check the resolved face's real style, not key presence.
pub(crate) fn needs_faux_italic(
    fonts: &HashMap<String, TtfFont>,
    family: &str,
    bold: bool,
    italic: bool,
) -> bool {
    if !italic {
        return false;
    }
    match find_font(fonts, family, bold, true) {
        Some((_, font)) => !font.is_italic,
        None => false,
    }
}

pub(crate) fn resolve_font_family(
    stack: &FontStack,
    fonts: &HashMap<String, TtfFont>,
    bold: bool,
    italic: bool,
) -> FontFamily {
    for family in stack.families() {
        match family {
            FontFamily::Custom(name) if find_font(fonts, name, bold, italic).is_some() => {
                return FontFamily::Custom(name.clone());
            }
            FontFamily::TimesRoman => {
                // A font explicitly registered under the generic CSS name `serif`
                // (e.g. via `add_font("serif", …)`) takes precedence over the
                // engine's bundled fallback, so the caller's chosen face — and
                // its `line-height: normal` metrics — are honoured. Only honour it
                // when the *requested* weight/style is actually present under that
                // name, so a regular-only generic registration does not shadow a
                // genuine bold/italic face from the bundled fallback.
                if has_exact_variant(fonts, "serif", bold, italic) {
                    return FontFamily::Custom("serif".to_string());
                }
                // Prefer system Times New Roman (exact Chromium match),
                // fall back to bundled Liberation Serif.
                if find_font(fonts, "Times New Roman", bold, italic).is_some() {
                    return FontFamily::Custom("Times New Roman".to_string());
                }
                if find_font(fonts, "Liberation Serif", bold, italic).is_some() {
                    return FontFamily::Custom("Liberation Serif".to_string());
                }
                return FontFamily::TimesRoman;
            }
            FontFamily::Helvetica => {
                if has_exact_variant(fonts, "sans-serif", bold, italic) {
                    return FontFamily::Custom("sans-serif".to_string());
                }
                if find_font(fonts, "Arial", bold, italic).is_some() {
                    return FontFamily::Custom("Arial".to_string());
                }
                if find_font(fonts, "Liberation Sans", bold, italic).is_some() {
                    return FontFamily::Custom("Liberation Sans".to_string());
                }
                return FontFamily::Helvetica;
            }
            FontFamily::Courier => {
                if has_exact_variant(fonts, "monospace", bold, italic) {
                    return FontFamily::Custom("monospace".to_string());
                }
                if find_font(fonts, "Courier New", bold, italic).is_some() {
                    return FontFamily::Custom("Courier New".to_string());
                }
                if find_font(fonts, "Liberation Mono", bold, italic).is_some() {
                    return FontFamily::Custom("Liberation Mono".to_string());
                }
                return FontFamily::Courier;
            }
            FontFamily::Custom(_) => {}
        }
    }

    if find_font(fonts, "Liberation Sans", bold, italic).is_some() {
        return FontFamily::Custom("Liberation Sans".to_string());
    }

    stack
        .families()
        .iter()
        .find(|family| !matches!(family, FontFamily::Custom(_)))
        .cloned()
        .unwrap_or_default()
}

/// Key used to store the Unicode fallback font in the font map.
pub(crate) const UNICODE_FALLBACK_KEY: &str = "__unicode_fallback";
pub(crate) const EMOJI_FALLBACK_KEY: &str = "__emoji_fallback";

/// Candidate font families for emoji rendering.
/// Prefer vector/outline fonts (Noto Emoji, Symbola) over bitmap fonts
/// (Apple Color Emoji, Noto Color Emoji) since our TTF parser doesn't
/// support CBDT/CBLC bitmap tables.
const EMOJI_FALLBACK_FAMILIES: &[&str] = &[
    "Noto Emoji",
    "Symbola",
    "Segoe UI Symbol",
    "Segoe UI Emoji",
    "Noto Color Emoji",
    "Apple Color Emoji",
    "EmojiOne",
    "Twitter Color Emoji",
];

/// Candidate font families to try when looking for a Unicode fallback font.
/// Ordered by breadth of Unicode coverage (CJK, symbols, etc.).
/// Includes macOS-specific CJK fonts (Hiragino, PingFang, STHeiti) alongside
/// cross-platform options (Noto Sans CJK, Arial Unicode MS, DejaVu Sans).
const UNICODE_FALLBACK_FAMILIES: &[&str] = &[
    "Noto Sans CJK SC",
    "Noto Sans CJK",
    "Noto Sans SC",
    "Arial Unicode MS",
    // macOS CJK fonts
    "Hiragino Sans",
    "Hiragino Kaku Gothic Pro",
    "PingFang SC",
    "Heiti SC",
    "STHeiti",
    // Linux / cross-platform
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "FreeSans",
];

/// Load a single Unicode-capable fallback font for characters outside
/// WinAnsiEncoding.  The font is registered under [`UNICODE_FALLBACK_KEY`].
///
/// This enables CJK and other non-Latin characters to render correctly
/// instead of appearing as rectangles/tofu when the document uses a
/// standard PDF font (Helvetica, Times-Roman, Courier).
/// The resolved+parsed system CJK fallback font (large; ~parse cost), cached so
/// it isn't re-parsed on every conversion.
static UNICODE_FALLBACK_CACHE: OnceLock<Option<TtfFont>> = OnceLock::new();

pub(crate) fn load_unicode_fallback_font(fonts: &mut HashMap<String, TtfFont>) {
    if fonts.contains_key(UNICODE_FALLBACK_KEY) {
        return;
    }

    let cached = UNICODE_FALLBACK_CACHE.get_or_init(|| {
        let db = system_fontdb();
        for family in UNICODE_FALLBACK_FAMILIES {
            let query = SystemFontQuery::new(family, FontVariant::new(false, false));
            if let Some(font) =
                query_fontdb_font(db, &query).or_else(|| query_fontconfig_font(&query))
            {
                return Some(font);
            }
        }
        None
    });
    if let Some(font) = cached {
        fonts.insert(UNICODE_FALLBACK_KEY.to_string(), font.clone());
    } else {
        // In WASM builds there are no system fonts available via fontdb —
        // fall back to a bundled Noto Sans CJK subset so Chinese/Japanese
        // kanji + kana at least render. Not bundled in native builds to keep
        // the CLI binary small (users have access to real system CJK fonts).
        #[cfg(feature = "wasm")]
        load_bundled_cjk_font(fonts);
    }
}

/// Bundled Noto Sans CJK subset (~1.4 MB TTF) — covers CJK Unified Ideographs
/// U+4E00-5FFF (most common ~4k Chinese characters / Japanese kanji),
/// Hiragana, Katakana, CJK Symbols/Punctuation, and Halfwidth/Fullwidth
/// Forms. Korean Hangul is **not** included to stay within WASM bundle
/// size budget; users needing Hangul should register a custom font via
/// `HtmlConverter::add_font`.
///
/// Stored as glyf-based TTF (not CFF-based OTF) so ironpress's built-in
/// TTF parser can decode it.
#[cfg(feature = "wasm")]
fn load_bundled_cjk_font(fonts: &mut HashMap<String, TtfFont>) {
    static CJK_SUBSET_DATA: &[u8] = include_bytes!("../assets/NotoSansCJK-Subset.ttf");
    if let Ok(font) = crate::parser::ttf::parse_ttf(CJK_SUBSET_DATA.to_vec()) {
        fonts.insert(UNICODE_FALLBACK_KEY.to_string(), font);
    }
}

/// Load an emoji font for emoji character rendering.
/// Prefers the bundled Noto Emoji (monochrome, vector outlines, 295KB)
/// which works everywhere. Falls back to system fonts only if the
/// bundled font fails to parse.
pub(crate) fn load_emoji_fallback_font(fonts: &mut HashMap<String, TtfFont>) {
    if fonts.contains_key(EMOJI_FALLBACK_KEY) {
        return;
    }

    // Always use the bundled Noto Emoji first — it has vector outlines
    // that our TTF parser can read. System emoji fonts (Apple Color Emoji,
    // Noto Color Emoji) use bitmap tables (CBDT/CBLC) which we can't parse.
    load_bundled_emoji_font(fonts);
    if fonts.contains_key(EMOJI_FALLBACK_KEY) {
        return;
    }

    // Fallback: try system fonts (may be bitmap-only)
    let db = system_fontdb();
    for family in EMOJI_FALLBACK_FAMILIES {
        let query = SystemFontQuery::new(family, FontVariant::new(false, false));
        if let Some(font) = query_fontdb_font(db, &query).or_else(|| query_fontconfig_font(&query))
        {
            fonts.insert(EMOJI_FALLBACK_KEY.to_string(), font);
            return;
        }
    }
}

/// Load the bundled Noto Emoji font (monochrome, vector outlines).
/// This ensures emoji rendering works on all platforms without requiring
/// a system emoji font to be installed.
fn load_bundled_emoji_font(fonts: &mut HashMap<String, TtfFont>) {
    static NOTO_EMOJI_DATA: &[u8] = include_bytes!("../assets/NotoEmoji-Regular.ttf");
    // Parse the 295KB bundled emoji font once, not on every conversion.
    static EMOJI_FONT_CACHE: OnceLock<Option<TtfFont>> = OnceLock::new();

    let cached = EMOJI_FONT_CACHE
        .get_or_init(|| crate::parser::ttf::parse_ttf(NOTO_EMOJI_DATA.to_vec()).ok());
    if let Some(font) = cached {
        fonts.insert(EMOJI_FALLBACK_KEY.to_string(), font.clone());
    }
}

struct BundledFont {
    key: &'static str,
    data: &'static [u8],
}

// Keys must match font_variant_key() format: lowercase + __bold/__italic/__bold_italic
static LIBERATION_FONTS: &[BundledFont] = &[
    BundledFont {
        key: "liberation serif",
        data: include_bytes!("../assets/LiberationSerif-Regular.ttf"),
    },
    BundledFont {
        key: "liberation serif__bold",
        data: include_bytes!("../assets/LiberationSerif-Bold.ttf"),
    },
    BundledFont {
        key: "liberation serif__italic",
        data: include_bytes!("../assets/LiberationSerif-Italic.ttf"),
    },
    BundledFont {
        key: "liberation serif__bold_italic",
        data: include_bytes!("../assets/LiberationSerif-BoldItalic.ttf"),
    },
    BundledFont {
        key: "liberation sans",
        data: include_bytes!("../assets/LiberationSans-Regular.ttf"),
    },
    BundledFont {
        key: "liberation sans__bold",
        data: include_bytes!("../assets/LiberationSans-Bold.ttf"),
    },
    BundledFont {
        key: "liberation sans__italic",
        data: include_bytes!("../assets/LiberationSans-Italic.ttf"),
    },
    BundledFont {
        key: "liberation sans__bold_italic",
        data: include_bytes!("../assets/LiberationSans-BoldItalic.ttf"),
    },
    BundledFont {
        key: "liberation mono",
        data: include_bytes!("../assets/LiberationMono-Regular.ttf"),
    },
    BundledFont {
        key: "liberation mono__bold",
        data: include_bytes!("../assets/LiberationMono-Bold.ttf"),
    },
    BundledFont {
        key: "liberation mono__italic",
        data: include_bytes!("../assets/LiberationMono-Italic.ttf"),
    },
    BundledFont {
        key: "liberation mono__bold_italic",
        data: include_bytes!("../assets/LiberationMono-BoldItalic.ttf"),
    },
];

/// Bundled Noto Sans for multilingual fallback (Hebrew, Greek, Cyrillic, etc.)
static NOTO_SANS_DATA: &[u8] = include_bytes!("../assets/NotoSans-Regular.ttf");

/// Bundled Noto Sans Arabic for Arabic script fallback.
static NOTO_ARABIC_DATA: &[u8] = include_bytes!("../assets/NotoSansArabic-Regular.ttf");

/// Key for the Arabic fallback font.
pub(crate) const ARABIC_FALLBACK_KEY: &str = "__arabic_fallback";

/// Key for the multilingual (Noto Sans) fallback font — covers Hebrew,
/// Greek, Cyrillic, and other non-CJK/non-Arabic scripts.
pub(crate) const MULTILINGUAL_FALLBACK_KEY: &str = "__multilingual_fallback";

/// Load bundled Liberation fonts as the default serif/sans-serif/monospace.
/// Liberation fonts are metrically identical to Times New Roman, Arial, and
/// Courier New, ensuring ironpress output matches Chromium rendering exactly.
/// Also loads Noto Sans as a multilingual fallback for Arabic, Hebrew, etc.
/// Cached parsed bundled fonts — parsed once on first use, then cloned
/// into each conversion's font map. This avoids re-parsing ~5MB of TTF
/// data on every `html_to_pdf()` call.
static BUNDLED_FONTS_CACHE: std::sync::OnceLock<Vec<(String, TtfFont)>> =
    std::sync::OnceLock::new();

/// Shared fontdb database of system fonts. `db.load_system_fonts()` walks the
/// filesystem (/usr/share/fonts, ~/Library/Fonts, etc.) which can take
/// hundreds of ms on Linux CI runners with many font packages installed.
/// Loading once per process — instead of per `html_to_pdf()` call — shaves
/// ~300-400 ms per render on Ubuntu CI.
static SYSTEM_FONTDB_CACHE: std::sync::OnceLock<fontdb::Database> = std::sync::OnceLock::new();

fn system_fontdb() -> &'static fontdb::Database {
    SYSTEM_FONTDB_CACHE.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

fn parse_all_bundled_fonts() -> Vec<(String, TtfFont)> {
    let mut result = Vec::new();
    for bundled in LIBERATION_FONTS {
        if let Ok(font) = crate::parser::ttf::parse_ttf(bundled.data.to_vec()) {
            result.push((bundled.key.to_string(), font));
        }
    }
    if let Ok(font) = crate::parser::ttf::parse_ttf(NOTO_SANS_DATA.to_vec()) {
        result.push((MULTILINGUAL_FALLBACK_KEY.to_string(), font));
    }
    if let Ok(font) = crate::parser::ttf::parse_ttf(NOTO_ARABIC_DATA.to_vec()) {
        result.push((ARABIC_FALLBACK_KEY.to_string(), font));
    }
    result
}

/// Try to load the system's actual serif/sans-serif/monospace fonts
/// (Times New Roman, Arial, Courier New) so the output matches Chromium
/// exactly. Falls through to bundled Liberation fonts when unavailable.
///
/// These literal Windows family names are resolved from the in-process
/// `fontdb` **only** — never via `fc-match`. On a host where they are absent
/// (the common Linux case, where only Liberation*/DejaVu* are installed),
/// `fontdb` returns nothing and the family is left unloaded, so the document
/// falls through to the bundled Liberation faces exactly as before.
///
/// The previous implementation fell through to `fc-match`, which spawned 12
/// subprocesses per *cold* render (one per family × variant). Those spawns
/// also never contributed a font: `fc-match` substitutes e.g. Liberation Serif
/// for "Times New Roman", and the family-match guard in `query_fontconfig_font`
/// rejected that mismatch, so nothing was inserted. Restricting this probe to
/// `fontdb` therefore preserves byte-identical output while eliminating the
/// per-cold-render subprocess spawns. (`fc-match` remains available, cached and
/// at-most-once, for genuinely *requested* unknown families via
/// `load_requested_system_fonts`.)
/// Parsed default-family variants (Times/Arial/Courier x regular/bold/italic/
/// bold-italic), resolved from the system fontdb once. Resolving + parsing these
/// is ~12 TTF parses; caching avoids repeating that on every conversion (it was
/// ~12ms per call), so warm/batch generation stays well under a millisecond.
static DEFAULT_FONTS_CACHE: OnceLock<Vec<(String, TtfFont)>> = OnceLock::new();

pub(crate) fn load_system_default_fonts(fonts: &mut HashMap<String, TtfFont>) {
    let cached = DEFAULT_FONTS_CACHE.get_or_init(|| {
        let families = [
            ("Times New Roman", "serif"),
            ("Arial", "sans-serif"),
            ("Courier New", "monospace"),
        ];
        let db = system_fontdb();
        let mut out = Vec::new();
        for (family, _generic) in &families {
            for variant in FONT_VARIANTS {
                let query = SystemFontQuery::new(family, *variant);
                // fontdb-only: do not fall through to query_fontconfig_font (fc-match).
                if let Some(font) = query_fontdb_font(db, &query) {
                    out.push((query.variant_key(), font));
                }
            }
        }
        out
    });
    for (key, font) in cached {
        // Don't override fonts already provided (e.g. @font-face, add_font).
        fonts.entry(key.clone()).or_insert_with(|| font.clone());
    }
}

pub(crate) fn load_bundled_liberation_fonts(fonts: &mut HashMap<String, TtfFont>) {
    let cached = BUNDLED_FONTS_CACHE.get_or_init(parse_all_bundled_fonts);
    for (key, font) in cached {
        if !fonts.contains_key(key) {
            fonts.insert(key.clone(), font.clone());
        }
    }
}

pub(crate) fn load_requested_system_fonts(
    nodes: &[DomNode],
    rules: &[CssRule],
    fonts: &mut HashMap<String, TtfFont>,
) {
    let requested = requested_families(nodes, rules);
    if requested.is_empty() {
        return;
    }

    let db = system_fontdb();

    for family in requested {
        load_family_variants(db, &family, fonts);
    }
}

fn requested_families(nodes: &[DomNode], rules: &[CssRule]) -> BTreeSet<String> {
    let mut families = BTreeSet::new();

    for rule in rules {
        collect_style_map_family(&rule.declarations, &mut families);
    }

    collect_node_families(nodes, &mut families);
    families
}

fn collect_node_families(nodes: &[DomNode], families: &mut BTreeSet<String>) {
    for node in nodes {
        if let DomNode::Element(element) = node {
            if let Some(style_attr) = element.style_attr() {
                let style_map = parse_inline_style(style_attr);
                collect_style_map_family(&style_map, families);
            }
            collect_node_families(&element.children, families);
        }
    }
}

fn collect_style_map_family(
    style_map: &crate::parser::css::StyleMap,
    families: &mut BTreeSet<String>,
) {
    let Some(CssValue::Keyword(family)) = style_map.get("font-family") else {
        return;
    };

    for entry in parse_font_stack(family).families() {
        let FontFamily::Custom(name) = entry else {
            continue;
        };
        if should_try_system_font(name) {
            families.insert(name.to_ascii_lowercase());
        }
    }
}

fn should_try_system_font(family: &str) -> bool {
    !matches!(
        family.to_ascii_lowercase().as_str(),
        "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy"
    )
}

fn load_family_variants(db: &fontdb::Database, family: &str, fonts: &mut HashMap<String, TtfFont>) {
    for variant in FONT_VARIANTS {
        let query = SystemFontQuery::new(family, *variant);
        match fonts.entry(query.variant_key()) {
            Entry::Occupied(_) => {}
            Entry::Vacant(slot) => {
                let Some(font) = load_system_font(db, &query) else {
                    continue;
                };
                slot.insert(font);
            }
        }
    }
}

fn load_system_font(db: &fontdb::Database, query: &SystemFontQuery<'_>) -> Option<TtfFont> {
    if query.prefers_ui_sans_resolution() {
        load_preferred_family_font(db, query, UI_SANS_FALLBACK_FAMILIES)
            .or_else(|| query_fontdb_font(db, query))
            .or_else(|| query_fontconfig_font(query))
    } else {
        // Prefer fontdb (fast, no subprocess) over fontconfig (fc-match can hang)
        query_fontdb_font(db, query).or_else(|| query_fontconfig_font(query))
    }
}

fn load_preferred_family_font(
    db: &fontdb::Database,
    query: &SystemFontQuery<'_>,
    families: &[&str],
) -> Option<TtfFont> {
    families.iter().find_map(|family| {
        let preferred = SystemFontQuery::new(family, query.variant);
        query_fontdb_font(db, &preferred).or_else(|| query_fontconfig_font(&preferred))
    })
}

/// Whether `fc-match` is available on this system. Probed at most once per
/// process: if the binary is missing (the common case on minimal/CI hosts),
/// every subsequent fontconfig query short-circuits without spawning anything.
fn fontconfig_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("fc-match")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

/// Process-wide cache of resolved fontconfig file paths, keyed by the exact
/// `fc-match` pattern. Ensures `fc-match` is spawned **at most once per
/// distinct pattern for the entire process lifetime** (and never on the hot
/// path once warm), instead of once per font variant × family × render.
///
/// We cache the resolved file path (not the parsed `TtfFont`) so the entry is
/// cheap to clone and shared across threads; the small per-call `parse_ttf` is
/// negligible next to a subprocess spawn + fontconfig re-init.
fn fontconfig_path_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn query_fontconfig_font(query: &SystemFontQuery<'_>) -> Option<TtfFont> {
    let pattern = query.fontconfig_pattern();

    // Fast path: serve from the cache without spawning.
    if let Ok(cache) = fontconfig_path_cache().lock() {
        if let Some(cached) = cache.get(&pattern) {
            return cached
                .as_ref()
                .and_then(|path| parse_ttf(std::fs::read(path).ok()?).ok());
        }
    }

    // Cache miss: resolve via fc-match at most once for this pattern, then store
    // the result (including negative results) so it is never spawned again.
    let resolved = resolve_fontconfig_path(query, &pattern);
    if let Ok(mut cache) = fontconfig_path_cache().lock() {
        cache.insert(pattern, resolved.clone());
    }

    resolved.and_then(|path| parse_ttf(std::fs::read(path).ok()?).ok())
}

/// Spawn `fc-match` once for `pattern` and return the resolved file path if the
/// returned family matches the requested family (same guard as before). No
/// sleep-polling: `output()` blocks on the child directly.
fn resolve_fontconfig_path(query: &SystemFontQuery<'_>, pattern: &str) -> Option<String> {
    if !fontconfig_available() {
        return None;
    }

    let output = Command::new("fc-match")
        .args([pattern, "-f", "%{family}\n%{file}"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let output = String::from_utf8(output.stdout).ok()?;
    let (family, path) = output.split_once('\n')?;
    if !fontconfig_family_matches(query, family) {
        return None;
    }

    let path = path.trim();
    if path.is_empty() {
        return None;
    }

    Some(path.to_string())
}

fn fontconfig_family_matches(query: &SystemFontQuery<'_>, family_output: &str) -> bool {
    let requested = query.normalized_family().trim();
    family_output
        .split(',')
        .map(str::trim)
        .any(|family| family.eq_ignore_ascii_case(requested))
}

#[cfg(test)]
fn build_fontconfig_pattern(query: &SystemFontQuery<'_>) -> String {
    query.fontconfig_pattern()
}

fn query_fontdb_font(db: &fontdb::Database, query: &SystemFontQuery<'_>) -> Option<TtfFont> {
    let families = query.fontdb_families();
    let face_id = db.query(&fontdb::Query {
        families: &families,
        weight: query.variant.weight(),
        stretch: fontdb::Stretch::Normal,
        style: query.variant.style(),
    })?;
    db.with_face_data(face_id, |data, face_index| {
        parse_ttf_with_index(data.to_vec(), face_index as usize).ok()
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ttf::{FontVerticalMetrics, TtfFont};
    use crate::style::computed::{FontFamily, FontStack, parse_font_stack};

    fn stub_font(name: &str) -> TtfFont {
        let metrics = FontVerticalMetrics::new(800, -200, 0);
        TtfFont {
            font_name: name.to_string(),
            units_per_em: 1000,
            bbox: [0, -200, 1000, 800],
            pdf_metrics: metrics,
            layout_metrics: metrics,
            cmap: std::collections::HashMap::new(),
            glyph_widths: vec![500],
            num_h_metrics: 1,
            flags: 0,
            is_bold: false,
            is_italic: false,
            x_height: 0,
            zero_advance: 0,
            data: std::sync::Arc::new(vec![]),
        }
    }

    // ── find_font ────────────────────────────────────────────────────────────

    #[test]
    fn find_font_exact_match_regular() {
        let mut fonts = HashMap::new();
        fonts.insert("arial".to_string(), stub_font("Arial"));
        let (key, _) = find_font(&fonts, "Arial", false, false).unwrap();
        assert_eq!(key, "arial");
    }

    #[test]
    fn find_font_exact_match_bold() {
        let mut fonts = HashMap::new();
        fonts.insert("arial__bold".to_string(), stub_font("Arial Bold"));
        let (key, _) = find_font(&fonts, "Arial", true, false).unwrap();
        assert_eq!(key, "arial__bold");
    }

    #[test]
    fn find_font_exact_match_italic() {
        let mut fonts = HashMap::new();
        fonts.insert("arial__italic".to_string(), stub_font("Arial Italic"));
        let (key, _) = find_font(&fonts, "Arial", false, true).unwrap();
        assert_eq!(key, "arial__italic");
    }

    #[test]
    fn find_font_exact_match_bold_italic() {
        let mut fonts = HashMap::new();
        fonts.insert("arial__bold_italic".to_string(), stub_font("Arial BI"));
        let (key, _) = find_font(&fonts, "Arial", true, true).unwrap();
        assert_eq!(key, "arial__bold_italic");
    }

    #[test]
    fn find_font_case_insensitive_match() {
        let mut fonts = HashMap::new();
        fonts.insert("arial".to_string(), stub_font("Arial"));
        // Stored under lowercase key; look up with mixed case
        assert!(find_font(&fonts, "ARIAL", false, false).is_some());
        assert!(find_font(&fonts, "Arial", false, false).is_some());
    }

    #[test]
    fn find_font_bold_falls_back_to_regular() {
        let mut fonts = HashMap::new();
        // Only the regular variant is stored.
        fonts.insert("arial".to_string(), stub_font("Arial"));
        let (key, _) = find_font(&fonts, "Arial", true, false).unwrap();
        assert_eq!(key, "arial");
    }

    #[test]
    fn find_font_italic_falls_back_to_regular() {
        let mut fonts = HashMap::new();
        fonts.insert("arial".to_string(), stub_font("Arial"));
        let (key, _) = find_font(&fonts, "Arial", false, true).unwrap();
        assert_eq!(key, "arial");
    }

    #[test]
    fn find_font_returns_none_when_family_absent() {
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        assert!(find_font(&fonts, "NonExistent", false, false).is_none());
    }

    // ── resolve_font_family ──────────────────────────────────────────────────

    #[test]
    fn resolve_font_family_returns_custom_when_font_loaded() {
        let mut fonts = HashMap::new();
        fonts.insert("roboto".to_string(), stub_font("Roboto"));
        let stack = parse_font_stack("Roboto, sans-serif");
        let result = resolve_font_family(&stack, &fonts, false, false);
        assert_eq!(result, FontFamily::Custom("roboto".to_string()));
    }

    #[test]
    fn resolve_font_family_returns_builtin_helvetica() {
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let stack = FontStack::from_family(FontFamily::Helvetica);
        let result = resolve_font_family(&stack, &fonts, false, false);
        assert_eq!(result, FontFamily::Helvetica);
    }

    #[test]
    fn resolve_font_family_returns_builtin_times_roman() {
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let stack = FontStack::from_family(FontFamily::TimesRoman);
        let result = resolve_font_family(&stack, &fonts, false, false);
        assert_eq!(result, FontFamily::TimesRoman);
    }

    #[test]
    fn resolve_font_family_returns_builtin_courier() {
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let stack = FontStack::from_family(FontFamily::Courier);
        let result = resolve_font_family(&stack, &fonts, false, false);
        assert_eq!(result, FontFamily::Courier);
    }

    #[test]
    fn resolve_font_family_skips_missing_custom_and_falls_back_to_builtin() {
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        // "Roboto" is parsed as Custom; "serif" maps to TimesRoman.
        let stack = parse_font_stack("Roboto, serif");
        let result = resolve_font_family(&stack, &fonts, false, false);
        assert_eq!(result, FontFamily::TimesRoman);
    }

    #[test]
    fn resolve_generic_prefers_font_registered_under_generic_name() {
        // A face explicitly registered under the generic CSS family name
        // (as `add_font("monospace", …)` does) wins over the bundled fallback,
        // so the caller's chosen outlines and `line-height: normal` metrics win.
        for (css, key) in [
            ("monospace", "monospace"),
            ("sans-serif", "sans-serif"),
            ("serif", "serif"),
        ] {
            let mut fonts = HashMap::new();
            fonts.insert(key.to_string(), stub_font(key));
            let result = resolve_font_family(&parse_font_stack(css), &fonts, false, false);
            assert_eq!(result, FontFamily::Custom(key.to_string()), "css={css}");
        }
    }

    #[test]
    fn resolve_generic_regular_only_does_not_shadow_real_bold() {
        // A regular-only generic registration must NOT shadow a genuine bold
        // face available under another key: the bold request falls through.
        let mut fonts = HashMap::new();
        fonts.insert("sans-serif".to_string(), stub_font("Generic Sans"));
        let mut bold = stub_font("Liberation Sans Bold");
        bold.is_bold = true;
        fonts.insert("liberation sans__bold".to_string(), bold);
        // Regular -> the registered generic face.
        let reg = resolve_font_family(&parse_font_stack("sans-serif"), &fonts, false, false);
        assert_eq!(reg, FontFamily::Custom("sans-serif".to_string()));
        // Bold -> the real bold face, not the regular-only generic.
        let b = resolve_font_family(&parse_font_stack("sans-serif"), &fonts, true, false);
        assert_eq!(b, FontFamily::Custom("Liberation Sans".to_string()));
    }

    #[test]
    fn resolve_font_family_defaults_to_helvetica_when_all_custom_missing() {
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let stack = FontStack::from_family(FontFamily::Custom("Roboto".to_string()));
        let result = resolve_font_family(&stack, &fonts, false, false);
        assert_eq!(result, FontFamily::Helvetica);
    }

    // ── should_try_system_font ───────────────────────────────────────────────

    #[test]
    fn should_try_system_font_rejects_cursive_and_fantasy() {
        assert!(!should_try_system_font("cursive"));
        assert!(!should_try_system_font("fantasy"));
    }

    #[test]
    fn should_try_system_font_is_case_insensitive() {
        assert!(!should_try_system_font("Serif"));
        assert!(!should_try_system_font("MONOSPACE"));
        assert!(should_try_system_font("Roboto"));
    }

    // ── font_variant_key with mixed-case / whitespace ────────────────────────

    #[test]
    fn font_variant_key_lowercases_input() {
        assert_eq!(font_variant_key("Arial", false, false), "arial");
        assert_eq!(font_variant_key("Arial", true, false), "arial__bold");
        assert_eq!(font_variant_key("Arial", false, true), "arial__italic");
        assert_eq!(font_variant_key("Arial", true, true), "arial__bold_italic");
    }

    #[test]
    fn font_variant_key_trims_whitespace() {
        assert_eq!(font_variant_key("  Arial  ", false, false), "arial");
        assert_eq!(
            font_variant_key("  Arial  ", true, true),
            "arial__bold_italic"
        );
    }

    // ── SystemFontQuery::normalized_family ───────────────────────────────────

    #[test]
    fn normalized_family_maps_ui_sans_aliases_to_sans_serif() {
        for alias in &[
            "ui-sans-serif",
            "system-ui",
            "-apple-system",
            "blinkmacsystemfont",
        ] {
            let q = SystemFontQuery::new(alias, FontVariant::new(false, false));
            assert_eq!(q.normalized_family(), "sans-serif", "failed for {alias}");
        }
    }

    #[test]
    fn normalized_family_maps_ui_serif_to_serif() {
        let q = SystemFontQuery::new("ui-serif", FontVariant::new(false, false));
        assert_eq!(q.normalized_family(), "serif");
    }

    #[test]
    fn normalized_family_maps_ui_monospace_to_monospace() {
        let q = SystemFontQuery::new("ui-monospace", FontVariant::new(false, false));
        assert_eq!(q.normalized_family(), "monospace");
    }

    #[test]
    fn normalized_family_passes_through_custom_name() {
        let q = SystemFontQuery::new("Roboto", FontVariant::new(false, false));
        assert_eq!(q.normalized_family(), "Roboto");
    }

    #[test]
    fn normalized_family_trims_surrounding_whitespace() {
        let q = SystemFontQuery::new("  Roboto  ", FontVariant::new(false, false));
        assert_eq!(q.normalized_family(), "Roboto");
    }

    // ── SystemFontQuery::prefers_ui_sans_resolution ──────────────────────────

    #[test]
    fn prefers_ui_sans_resolution_true_for_all_aliases() {
        for alias in &[
            "ui-sans-serif",
            "system-ui",
            "-apple-system",
            "blinkmacsystemfont",
        ] {
            let q = SystemFontQuery::new(alias, FontVariant::new(false, false));
            assert!(q.prefers_ui_sans_resolution(), "expected true for {alias}");
        }
    }

    #[test]
    fn prefers_ui_sans_resolution_false_for_other_families() {
        for family in &["ui-serif", "ui-monospace", "sans-serif", "Roboto"] {
            let q = SystemFontQuery::new(family, FontVariant::new(false, false));
            assert!(
                !q.prefers_ui_sans_resolution(),
                "expected false for {family}"
            );
        }
    }

    // ── SystemFontQuery::fontdb_families ─────────────────────────────────────

    #[test]
    fn fontdb_families_ui_sans_returns_fallback_list_plus_generic() {
        let q = SystemFontQuery::new("ui-sans-serif", FontVariant::new(false, false));
        let families = q.fontdb_families();
        // Should include the named fallbacks and the generic SansSerif sentinel.
        assert!(families.contains(&fontdb::Family::Name("DejaVu Sans")));
        assert!(families.contains(&fontdb::Family::Name("Arial")));
        assert!(families.contains(&fontdb::Family::SansSerif));
    }

    #[test]
    fn fontdb_families_sans_serif_returns_single_generic() {
        let q = SystemFontQuery::new("sans-serif", FontVariant::new(false, false));
        assert_eq!(q.fontdb_families(), vec![fontdb::Family::SansSerif]);
    }

    #[test]
    fn fontdb_families_serif_returns_single_generic() {
        let q = SystemFontQuery::new("serif", FontVariant::new(false, false));
        assert_eq!(q.fontdb_families(), vec![fontdb::Family::Serif]);
    }

    #[test]
    fn fontdb_families_monospace_returns_single_generic() {
        let q = SystemFontQuery::new("monospace", FontVariant::new(false, false));
        assert_eq!(q.fontdb_families(), vec![fontdb::Family::Monospace]);
    }

    #[test]
    fn fontdb_families_named_returns_name_family() {
        let q = SystemFontQuery::new("Roboto", FontVariant::new(false, false));
        assert_eq!(q.fontdb_families(), vec![fontdb::Family::Name("Roboto")]);
    }

    #[test]
    fn font_variant_key_suffixes_are_stable() {
        assert_eq!(
            font_variant_key("ui-sans-serif", false, false),
            "ui-sans-serif"
        );
        assert_eq!(
            font_variant_key("ui-sans-serif", true, false),
            "ui-sans-serif__bold"
        );
        assert_eq!(
            font_variant_key("ui-sans-serif", false, true),
            "ui-sans-serif__italic"
        );
        assert_eq!(
            font_variant_key("ui-sans-serif", true, true),
            "ui-sans-serif__bold_italic"
        );
    }

    #[test]
    fn generic_css_families_do_not_trigger_system_loading() {
        assert!(!should_try_system_font("serif"));
        assert!(!should_try_system_font("sans-serif"));
        assert!(!should_try_system_font("monospace"));
        assert!(should_try_system_font("ui-sans-serif"));
        assert!(should_try_system_font("roboto"));
    }

    #[test]
    fn fontconfig_pattern_maps_ui_generics() {
        let query = SystemFontQuery::new("ui-sans-serif", FontVariant::new(true, false));
        assert_eq!(build_fontconfig_pattern(&query), "sans-serif:style=Bold");
    }

    #[test]
    fn fontconfig_family_match_requires_requested_family() {
        let query = SystemFontQuery::new("MissingFont", FontVariant::new(false, false));
        assert!(!fontconfig_family_matches(&query, "Noto Sans"));
    }

    #[test]
    fn fontconfig_family_match_accepts_matching_alias_list() {
        let query = SystemFontQuery::new("DejaVu Sans", FontVariant::new(false, false));
        assert!(fontconfig_family_matches(
            &query,
            "DejaVu Sans,DejaVu Sans Condensed"
        ));
    }

    // ── load_unicode_fallback_font ──────────────────────────────────────────

    #[test]
    fn unicode_fallback_key_is_dunder_prefixed() {
        assert!(UNICODE_FALLBACK_KEY.starts_with("__"));
    }

    #[test]
    fn load_unicode_fallback_font_does_not_panic() {
        let mut fonts = HashMap::new();
        // Should not panic regardless of which system fonts are installed.
        load_unicode_fallback_font(&mut fonts);
    }

    #[test]
    fn load_unicode_fallback_font_is_idempotent() {
        let mut fonts = HashMap::new();
        load_unicode_fallback_font(&mut fonts);
        let count_after_first = fonts.len();
        load_unicode_fallback_font(&mut fonts);
        assert_eq!(
            fonts.len(),
            count_after_first,
            "calling load_unicode_fallback_font twice should not add a second entry"
        );
    }

    #[test]
    fn load_unicode_fallback_font_skips_when_key_already_present() {
        let mut fonts = HashMap::new();
        let sentinel = stub_font("Sentinel");
        fonts.insert(UNICODE_FALLBACK_KEY.to_string(), sentinel);
        load_unicode_fallback_font(&mut fonts);
        // The sentinel font should remain unchanged.
        assert_eq!(
            fonts.get(UNICODE_FALLBACK_KEY).unwrap().font_name,
            "Sentinel"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn load_unicode_fallback_font_loads_a_font_on_macos() {
        let mut fonts = HashMap::new();
        load_unicode_fallback_font(&mut fonts);
        assert!(
            fonts.contains_key(UNICODE_FALLBACK_KEY),
            "macOS should have at least one of the candidate Unicode fallback fonts"
        );
    }
}
