use crate::parser::css::{CssRule, CssValue, parse_inline_style};
use crate::parser::dom::DomNode;
use crate::parser::ttf::{TtfFont, parse_ttf};
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
    let output = Command::new("fc-match")
        .args([
            query.fontconfig_pattern().as_str(),
            "-f",
            "%{family}\n%{file}",
        ])
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

    parse_ttf(std::fs::read(path).ok()?).ok()
}

fn fontconfig_family_matches(query: &SystemFontQuery<'_>, family_output: &str) -> bool {
    let requested = query.normalized_family().trim();
    family_output
        .split(',')
        .map(str::trim)
        .any(|family| family.eq_ignore_ascii_case(requested))
}

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
    db.with_face_data(face_id, |data, _face_index| parse_ttf(data.to_vec()).ok())?
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
