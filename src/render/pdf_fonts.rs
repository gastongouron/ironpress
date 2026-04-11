use crate::layout::engine::{LayoutElement, Page, TextLine, TextRun};
use crate::parser::ttf::TtfFont;
use crate::style::computed::FontFamily;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(crate) type PreparedCustomFonts = BTreeMap<String, PreparedCustomFont>;
type ToUnicodeMap = Vec<(u16, Vec<u16>)>;

pub(crate) struct PreparedCustomFont {
    pub(crate) base_font_name: String,
    pub(crate) font_data: Vec<u8>,
    pub(crate) widths: Vec<f32>,
    pub(crate) to_unicode_map: ToUnicodeMap,
    glyph_id_map: HashMap<u16, u16>,
}

impl PreparedCustomFont {
    pub(crate) fn pdf_glyph_id(&self, old_glyph_id: u16) -> u16 {
        self.glyph_id_map
            .get(&old_glyph_id)
            .copied()
            .unwrap_or(old_glyph_id)
    }
}

#[derive(Default)]
struct FontUsage {
    glyphs: BTreeSet<u16>,
    to_unicode_map: BTreeMap<u16, Vec<u16>>,
}

impl FontUsage {
    fn record_glyph(&mut self, glyph_id: u16, unicode: Vec<u16>) {
        self.glyphs.insert(glyph_id);
        if !unicode.is_empty() {
            self.to_unicode_map.entry(glyph_id).or_insert(unicode);
        }
    }
}

pub(crate) fn prepare_custom_fonts(
    pages: &[Page],
    custom_fonts: &HashMap<String, TtfFont>,
) -> PreparedCustomFonts {
    collect_font_usage(pages, custom_fonts)
        .into_iter()
        .filter_map(|(resolved_name, usage)| {
            custom_fonts
                .get(&resolved_name)
                .map(|ttf| (resolved_name, prepare_font(ttf, &usage)))
        })
        .collect()
}

fn collect_font_usage(
    pages: &[Page],
    custom_fonts: &HashMap<String, TtfFont>,
) -> BTreeMap<String, FontUsage> {
    let mut usage = BTreeMap::new();
    for page in pages {
        for (_, element) in &page.elements {
            collect_font_usage_from_element(element, custom_fonts, &mut usage);
        }
    }
    usage
}

fn collect_font_usage_from_element(
    element: &LayoutElement,
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<String, FontUsage>,
) {
    match element {
        LayoutElement::TextBlock { lines, .. } => {
            collect_font_usage_from_lines(lines, custom_fonts, usage)
        }
        LayoutElement::TableRow { cells, .. } | LayoutElement::GridRow { cells, .. } => {
            for cell in cells {
                collect_font_usage_from_lines(&cell.lines, custom_fonts, usage);
                for nested in &cell.nested_rows {
                    collect_font_usage_from_element(nested, custom_fonts, usage);
                }
            }
        }
        LayoutElement::FlexRow { cells, .. } => {
            for cell in cells {
                collect_font_usage_from_lines(&cell.lines, custom_fonts, usage);
            }
        }
        _ => {}
    }
}

fn collect_font_usage_from_lines(
    lines: &[TextLine],
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<String, FontUsage>,
) {
    for line in lines {
        for run in &line.runs {
            collect_font_usage_from_run(run, custom_fonts, usage);
        }
    }
}

fn collect_font_usage_from_run(
    run: &TextRun,
    custom_fonts: &HashMap<String, TtfFont>,
    usage: &mut BTreeMap<String, FontUsage>,
) {
    let FontFamily::Custom(name) = &run.font_family else {
        return;
    };
    let Some((resolved_name, ttf)) =
        crate::system_fonts::find_font(custom_fonts, name, run.bold, run.italic)
    else {
        return;
    };

    let font_usage = usage.entry(resolved_name.to_string()).or_default();
    if let Some(shaped_run) = crate::text::shape_text_run(run, custom_fonts) {
        for glyph in shaped_run.glyphs {
            font_usage.record_glyph(glyph.glyph_id, glyph.unicode);
        }
        return;
    }

    for codepoint in run.text.encode_utf16() {
        if let Some(glyph_id) = ttf.cmap.get(&codepoint).copied() {
            font_usage.record_glyph(glyph_id, vec![codepoint]);
        }
    }
}

fn prepare_font(ttf: &TtfFont, usage: &FontUsage) -> PreparedCustomFont {
    let glyphs: Vec<u16> = usage.glyphs.iter().copied().collect();
    let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs);

    subsetter::subset(&ttf.data, 0, &remapper)
        .ok()
        .map(|font_data| subset_font(ttf, usage, &remapper, font_data))
        .unwrap_or_else(|| fallback_font(ttf))
}

fn subset_font(
    ttf: &TtfFont,
    usage: &FontUsage,
    remapper: &subsetter::GlyphRemapper,
    font_data: Vec<u8>,
) -> PreparedCustomFont {
    let mut glyph_id_map = HashMap::with_capacity(remapper.num_gids() as usize);
    let mut widths = vec![0.0; remapper.num_gids() as usize];

    for old_glyph_id in remapper.remapped_gids() {
        let Some(new_glyph_id) = remapper.get(old_glyph_id) else {
            continue;
        };
        glyph_id_map.insert(old_glyph_id, new_glyph_id);
        if let Some(width) = widths.get_mut(new_glyph_id as usize) {
            *width = ttf.glyph_width_pdf_value(old_glyph_id);
        }
    }

    PreparedCustomFont {
        base_font_name: subset_base_font_name(&ttf.font_name, remapper.num_gids()),
        font_data,
        widths,
        to_unicode_map: to_unicode_map_for_subset(usage, remapper),
        glyph_id_map,
    }
}

fn fallback_font(ttf: &TtfFont) -> PreparedCustomFont {
    PreparedCustomFont {
        base_font_name: sanitize_pdf_font_name(&ttf.font_name),
        font_data: ttf.data.clone(),
        widths: (0..ttf.glyph_widths.len())
            .map(|glyph_id| ttf.glyph_width_pdf_value(glyph_id as u16))
            .collect(),
        to_unicode_map: to_unicode_map_for_full_font(ttf),
        glyph_id_map: HashMap::new(),
    }
}

fn to_unicode_map_for_subset(
    usage: &FontUsage,
    remapper: &subsetter::GlyphRemapper,
) -> ToUnicodeMap {
    let mut mappings = BTreeMap::new();
    for (&old_glyph_id, unicode) in &usage.to_unicode_map {
        if let Some(new_glyph_id) = remapper.get(old_glyph_id) {
            mappings
                .entry(new_glyph_id)
                .or_insert_with(|| unicode.clone());
        }
    }
    mappings.into_iter().collect()
}

fn to_unicode_map_for_full_font(ttf: &TtfFont) -> ToUnicodeMap {
    let mut mappings = BTreeMap::new();
    for (&char_code, &glyph_id) in &ttf.cmap {
        if glyph_id != 0 {
            mappings.entry(glyph_id).or_insert_with(|| vec![char_code]);
        }
    }
    mappings.into_iter().collect()
}

fn subset_base_font_name(font_name: &str, glyph_count: u16) -> String {
    let sanitized_name = sanitize_pdf_font_name(font_name);
    let mut hash = 0xcbf29ce484222325u64;
    for byte in sanitized_name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= u64::from(glyph_count);
    hash = hash.wrapping_mul(0x100000001b3);

    let mut tag = String::with_capacity(6);
    let mut value = hash;
    for _ in 0..6 {
        let letter = b'A' + (value % 26) as u8;
        tag.push(char::from(letter));
        value /= 26;
    }

    format!("{tag}+{sanitized_name}")
}

fn sanitize_pdf_font_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '+'))
        .collect();

    if sanitized.is_empty() {
        "CustomFont".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── sanitize_pdf_font_name ───────────────────────────────────────────────

    #[test]
    fn sanitize_pdf_font_name_normal() {
        assert_eq!(sanitize_pdf_font_name("OpenSans"), "OpenSans");
    }

    #[test]
    fn sanitize_pdf_font_name_with_allowed_special_chars() {
        assert_eq!(
            sanitize_pdf_font_name("Open-Sans_Bold+Italic"),
            "Open-Sans_Bold+Italic"
        );
    }

    #[test]
    fn sanitize_pdf_font_name_strips_spaces_and_punctuation() {
        // spaces, slashes, dots, and other punctuation must be removed
        let result = sanitize_pdf_font_name("Open Sans / Bold.ttf");
        // Only alphanumeric, '-', '_', '+' survive
        assert_eq!(result, "OpenSansBoldttf");
    }

    #[test]
    fn sanitize_pdf_font_name_empty_returns_custom_font() {
        assert_eq!(sanitize_pdf_font_name(""), "CustomFont");
    }

    #[test]
    fn sanitize_pdf_font_name_all_special_chars_returns_custom_font() {
        assert_eq!(sanitize_pdf_font_name("!@#$%^&*()"), "CustomFont");
    }

    #[test]
    fn sanitize_pdf_font_name_unicode_alphanumeric_kept() {
        // Digits and ASCII letters are always kept.
        let result = sanitize_pdf_font_name("Font123");
        assert_eq!(result, "Font123");
    }

    // ── subset_base_font_name ────────────────────────────────────────────────

    #[test]
    fn subset_base_font_name_format() {
        let name = subset_base_font_name("OpenSans", 42);
        // Must be "XXXXXX+<sanitized_name>"
        let parts: Vec<&str> = name.splitn(2, '+').collect();
        assert_eq!(parts.len(), 2, "expected exactly one '+' separator");
        let tag = parts[0];
        let base = parts[1];
        assert_eq!(tag.len(), 6, "tag must be exactly 6 characters");
        assert!(
            tag.chars().all(|c| c.is_ascii_uppercase()),
            "tag must be uppercase ASCII letters"
        );
        assert_eq!(base, "OpenSans");
    }

    #[test]
    fn subset_base_font_name_deterministic() {
        // Same inputs must always produce the same output.
        let a = subset_base_font_name("Roboto", 10);
        let b = subset_base_font_name("Roboto", 10);
        assert_eq!(a, b);
    }

    #[test]
    fn subset_base_font_name_different_glyph_count_differs() {
        let a = subset_base_font_name("Roboto", 10);
        let b = subset_base_font_name("Roboto", 20);
        assert_ne!(a, b, "different glyph counts should produce different tags");
    }

    #[test]
    fn subset_base_font_name_different_name_differs() {
        let a = subset_base_font_name("Roboto", 10);
        let b = subset_base_font_name("OpenSans", 10);
        assert_ne!(a, b, "different font names should produce different tags");
    }

    #[test]
    fn subset_base_font_name_sanitizes_input() {
        // Special characters in the name are stripped before embedding.
        let name = subset_base_font_name("Open Sans", 5);
        assert!(
            name.ends_with("+OpenSans"),
            "sanitized name should appear after '+'"
        );
    }

    // ── FontUsage::record_glyph ──────────────────────────────────────────────

    #[test]
    fn font_usage_record_glyph_stores_glyph_id() {
        let mut usage = FontUsage::default();
        usage.record_glyph(42, vec![0x0041]); // 'A'
        assert!(usage.glyphs.contains(&42));
    }

    #[test]
    fn font_usage_record_glyph_stores_unicode_mapping() {
        let mut usage = FontUsage::default();
        usage.record_glyph(7, vec![0x0048, 0x0069]); // "Hi"
        assert_eq!(
            usage.to_unicode_map.get(&7),
            Some(&vec![0x0048u16, 0x0069u16])
        );
    }

    #[test]
    fn font_usage_record_glyph_empty_unicode_does_not_insert_mapping() {
        let mut usage = FontUsage::default();
        usage.record_glyph(99, vec![]);
        assert!(usage.glyphs.contains(&99));
        assert!(!usage.to_unicode_map.contains_key(&99));
    }

    #[test]
    fn font_usage_record_glyph_first_mapping_wins() {
        let mut usage = FontUsage::default();
        usage.record_glyph(1, vec![0x0041]); // 'A'
        usage.record_glyph(1, vec![0x0042]); // 'B' — second call should not overwrite
        assert_eq!(usage.to_unicode_map.get(&1), Some(&vec![0x0041u16]));
    }

    #[test]
    fn font_usage_record_glyph_multiple_glyphs() {
        let mut usage = FontUsage::default();
        for glyph_id in [1u16, 2, 3, 5, 8] {
            usage.record_glyph(glyph_id, vec![glyph_id]);
        }
        assert_eq!(usage.glyphs.len(), 5);
        // BTreeSet is sorted — collect to verify all are present
        let ids: Vec<u16> = usage.glyphs.iter().copied().collect();
        assert_eq!(ids, vec![1, 2, 3, 5, 8]);
    }
}
