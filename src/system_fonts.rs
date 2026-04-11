use crate::parser::css::{CssRule, CssValue, parse_inline_style};
use crate::parser::dom::DomNode;
use crate::parser::ttf::{TtfFont, parse_ttf, parse_ttf_with_index};
use crate::style::computed::{FontFamily, FontStack, parse_font_stack};
use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::process::Command;

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
            FontFamily::Helvetica | FontFamily::TimesRoman | FontFamily::Courier => {
                return family.clone();
            }
            FontFamily::Custom(_) => {}
        }
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
pub(crate) fn load_unicode_fallback_font(fonts: &mut HashMap<String, TtfFont>) {
    if fonts.contains_key(UNICODE_FALLBACK_KEY) {
        return;
    }

    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    for family in UNICODE_FALLBACK_FAMILIES {
        let query = SystemFontQuery::new(family, FontVariant::new(false, false));
        if let Some(font) = query_fontdb_font(&db, &query).or_else(|| query_fontconfig_font(&query))
        {
            fonts.insert(UNICODE_FALLBACK_KEY.to_string(), font);
            return;
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

    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    for family in requested {
        load_family_variants(&db, &family, fonts);
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
        query_fontconfig_font(query).or_else(|| query_fontdb_font(db, query))
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

fn query_fontconfig_font(query: &SystemFontQuery<'_>) -> Option<TtfFont> {
    let mut child = Command::new("fc-match")
        .args([
            query.fontconfig_pattern().as_str(),
            "-f",
            "%{family}\n%{file}",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Timeout after 5 seconds to avoid blocking on slow/absent fontconfig.
    let timeout = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
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

    parse_ttf(std::fs::read(path).ok()?).ok()
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
            data: vec![],
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
