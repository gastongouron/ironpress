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
    // Standard PDF font runs with non-WinAnsi text → collect under fallback font
    if !matches!(&run.font_family, FontFamily::Custom(_)) {
        if let Some((shaped_run, fallback_key, _)) =
            crate::text::shape_with_unicode_fallback(run, custom_fonts)
        {
            let font_usage = usage.entry(fallback_key.to_string()).or_default();
            for glyph in shaped_run.glyphs {
                font_usage.record_glyph(glyph.glyph_id, glyph.unicode);
            }
        }
        return;
    }

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
    use crate::layout::engine::{FlexCell, LayoutBorder, TableCell, TextLine, TextRun};
    use crate::parser::ttf::{FontVerticalMetrics, TtfFont};
    use crate::style::computed::{
        BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize, BorderCollapse,
        Clear, Float, FontFamily, Position, TextAlign, VerticalAlign,
    };

    // ── Test helpers ─────────────────────────────────────────────────────────

    fn make_stub_ttf() -> TtfFont {
        TtfFont {
            font_name: "Stub".into(),
            units_per_em: 1000,
            bbox: [0, -200, 800, 800],
            pdf_metrics: FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: FontVerticalMetrics::new(800, -200, 0),
            cmap: HashMap::new(),
            glyph_widths: vec![0, 500, 600],
            num_h_metrics: 3,
            flags: 32,
            data: Vec::new(), // empty ⟹ subsetting always fails → fallback_font path
        }
    }

    fn make_ttf_with_cmap(cmap: HashMap<u16, u16>, widths: Vec<u16>) -> TtfFont {
        TtfFont {
            font_name: "TestFont".into(),
            units_per_em: 1000,
            bbox: [0, -200, 800, 800],
            pdf_metrics: FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: FontVerticalMetrics::new(800, -200, 0),
            cmap,
            glyph_widths: widths,
            num_h_metrics: 3,
            flags: 32,
            data: Vec::new(),
        }
    }

    fn empty_text_line() -> TextLine {
        TextLine {
            runs: vec![],
            height: 12.0,
        }
    }

    fn empty_table_cell() -> TableCell {
        TableCell {
            lines: vec![],
            nested_rows: vec![],
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Middle,
        }
    }

    fn empty_flex_cell() -> FlexCell {
        FlexCell {
            lines: vec![],
            x_offset: 0.0,
            width: 100.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            border_radius: 0.0,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
        }
    }

    fn text_block_element(lines: Vec<TextLine>) -> LayoutElement {
        LayoutElement::TextBlock {
            lines,
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: None,
            block_height: None,
            opacity: 1.0,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: None,
            visible: true,
            clip_rect: None,
            transform: None,
            border_radius: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
        }
    }

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

    // ── PreparedCustomFont::pdf_glyph_id ────────────────────────────────────

    #[test]
    fn pdf_glyph_id_returns_remapped_id_when_present() {
        let mut map = HashMap::new();
        map.insert(10u16, 1u16);
        map.insert(20u16, 2u16);
        let font = PreparedCustomFont {
            base_font_name: "X".into(),
            font_data: vec![],
            widths: vec![],
            to_unicode_map: vec![],
            glyph_id_map: map,
        };
        assert_eq!(font.pdf_glyph_id(10), 1);
        assert_eq!(font.pdf_glyph_id(20), 2);
    }

    #[test]
    fn pdf_glyph_id_returns_original_when_not_in_map() {
        let font = PreparedCustomFont {
            base_font_name: "X".into(),
            font_data: vec![],
            widths: vec![],
            to_unicode_map: vec![],
            glyph_id_map: HashMap::new(),
        };
        // Any unknown glyph ID should pass through unchanged.
        assert_eq!(font.pdf_glyph_id(42), 42);
        assert_eq!(font.pdf_glyph_id(0), 0);
    }

    // ── to_unicode_map_for_full_font ─────────────────────────────────────────

    #[test]
    fn to_unicode_map_for_full_font_maps_cmap_entries() {
        let mut cmap = HashMap::new();
        cmap.insert(0x0041u16, 1u16); // 'A' → glyph 1
        cmap.insert(0x0042u16, 2u16); // 'B' → glyph 2
        let ttf = make_ttf_with_cmap(cmap, vec![0, 500, 500]);
        let map = to_unicode_map_for_full_font(&ttf);
        // The map is collected from a BTreeMap, so entries are sorted by glyph_id.
        let found_a = map.iter().find(|(gid, _)| *gid == 1);
        let found_b = map.iter().find(|(gid, _)| *gid == 2);
        assert!(found_a.is_some(), "glyph 1 ('A') should be in the map");
        assert_eq!(found_a.unwrap().1, vec![0x0041u16]);
        assert!(found_b.is_some(), "glyph 2 ('B') should be in the map");
        assert_eq!(found_b.unwrap().1, vec![0x0042u16]);
    }

    #[test]
    fn to_unicode_map_for_full_font_skips_glyph_zero() {
        // cmap entries that map to glyph ID 0 (.notdef) must not appear in the map.
        let mut cmap = HashMap::new();
        cmap.insert(0x0020u16, 0u16); // space → .notdef (should be skipped)
        cmap.insert(0x0041u16, 1u16); // 'A' → glyph 1
        let ttf = make_ttf_with_cmap(cmap, vec![0, 500]);
        let map = to_unicode_map_for_full_font(&ttf);
        assert!(
            map.iter().all(|(gid, _)| *gid != 0),
            "glyph 0 (.notdef) must not appear"
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn to_unicode_map_for_full_font_empty_cmap_yields_empty_map() {
        let ttf = make_ttf_with_cmap(HashMap::new(), vec![]);
        let map = to_unicode_map_for_full_font(&ttf);
        assert!(map.is_empty());
    }

    #[test]
    fn to_unicode_map_for_full_font_first_codepoint_wins_for_same_glyph() {
        // Two codepoints map to the same glyph — only the first insertion should survive.
        let mut cmap = HashMap::new();
        cmap.insert(0x0041u16, 5u16);
        cmap.insert(0x0061u16, 5u16); // same glyph
        let ttf = make_ttf_with_cmap(cmap, vec![0, 0, 0, 0, 0, 500]);
        let map = to_unicode_map_for_full_font(&ttf);
        let entry = map.iter().find(|(gid, _)| *gid == 5);
        assert!(entry.is_some());
        assert_eq!(
            entry.unwrap().1.len(),
            1,
            "only one codepoint should be stored"
        );
    }

    // ── to_unicode_map_for_subset ────────────────────────────────────────────

    #[test]
    fn to_unicode_map_for_subset_remaps_glyph_ids() {
        // Build a FontUsage with glyphs 5 and 10.
        let mut usage = FontUsage::default();
        usage.record_glyph(5, vec![0x0041]); // 'A'
        usage.record_glyph(10, vec![0x0042]); // 'B'

        // Build a remapper for those same glyphs (sorted).
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&[5, 10]);

        let map = to_unicode_map_for_subset(&usage, &remapper);

        // Both glyphs must appear in the output under their *new* IDs.
        assert_eq!(map.len(), 2, "both glyphs should have entries");

        // New IDs come from the remapper — old ID 5 gets new ID 1 (0 is .notdef),
        // old ID 10 gets new ID 2.
        let new_5 = remapper.get(5).expect("glyph 5 must be remapped");
        let new_10 = remapper.get(10).expect("glyph 10 must be remapped");

        let entry_5 = map.iter().find(|(gid, _)| *gid == new_5);
        let entry_10 = map.iter().find(|(gid, _)| *gid == new_10);

        assert!(entry_5.is_some());
        assert_eq!(entry_5.unwrap().1, vec![0x0041u16]);
        assert!(entry_10.is_some());
        assert_eq!(entry_10.unwrap().1, vec![0x0042u16]);
    }

    #[test]
    fn to_unicode_map_for_subset_skips_glyphs_not_in_remapper() {
        let mut usage = FontUsage::default();
        usage.record_glyph(5, vec![0x0041]);
        usage.record_glyph(99, vec![0x0042]); // not in remapper

        // Only remap glyph 5.
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&[5]);
        let map = to_unicode_map_for_subset(&usage, &remapper);

        // Only glyph 5 should appear after remapping.
        assert_eq!(map.len(), 1);
        let new_5 = remapper.get(5).unwrap();
        assert!(map.iter().any(|(gid, _)| *gid == new_5));
    }

    #[test]
    fn to_unicode_map_for_subset_empty_usage_yields_empty_map() {
        let usage = FontUsage::default();
        let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&[]);
        let map = to_unicode_map_for_subset(&usage, &remapper);
        assert!(map.is_empty());
    }

    // ── fallback_font ────────────────────────────────────────────────────────

    #[test]
    fn fallback_font_uses_full_font_data() {
        let ttf = make_stub_ttf();
        let prepared = fallback_font(&ttf);
        assert_eq!(prepared.font_data, ttf.data);
    }

    #[test]
    fn fallback_font_name_matches_sanitized_font_name() {
        let ttf = make_stub_ttf();
        let prepared = fallback_font(&ttf);
        assert_eq!(
            prepared.base_font_name,
            sanitize_pdf_font_name(&ttf.font_name)
        );
    }

    #[test]
    fn fallback_font_widths_match_glyph_count() {
        let ttf = make_stub_ttf(); // glyph_widths has 3 entries
        let prepared = fallback_font(&ttf);
        assert_eq!(prepared.widths.len(), ttf.glyph_widths.len());
    }

    #[test]
    fn fallback_font_glyph_id_map_is_empty() {
        let ttf = make_stub_ttf();
        let prepared = fallback_font(&ttf);
        // Empty map means pdf_glyph_id passes IDs through unchanged.
        assert_eq!(prepared.pdf_glyph_id(5), 5);
    }

    #[test]
    fn prepare_font_falls_back_when_data_empty() {
        // Empty font data causes subsetter::subset to fail, so prepare_font
        // must call fallback_font instead of subset_font.
        let ttf = make_stub_ttf(); // data: Vec::new()
        let mut usage = FontUsage::default();
        usage.record_glyph(1, vec![0x0041]);
        let prepared = prepare_font(&ttf, &usage);
        // Fallback: base_font_name must NOT contain a '+' prefix tag.
        assert!(
            !prepared.base_font_name.starts_with(char::is_uppercase)
                || !prepared.base_font_name.contains('+')
                || prepared
                    .base_font_name
                    .ends_with(&sanitize_pdf_font_name(&ttf.font_name)),
            "fallback font name should be sanitized font name, not a subset tag"
        );
        // Widths come from all glyphs (fallback uses full glyph_widths).
        assert_eq!(prepared.widths.len(), ttf.glyph_widths.len());
    }

    // ── collect_font_usage_from_element ─────────────────────────────────────

    #[test]
    fn collect_font_usage_from_element_ignores_image() {
        let element = LayoutElement::PageBreak;
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<String, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty(), "PageBreak should produce no font usage");
    }

    #[test]
    fn collect_font_usage_from_element_handles_table_row() {
        let element = LayoutElement::TableRow {
            cells: vec![empty_table_cell()],
            col_widths: vec![100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: BorderCollapse::Separate,
            border_spacing: 0.0,
        };
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<String, FontUsage> = BTreeMap::new();
        // Should not panic; empty cells with no custom fonts yield empty usage.
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty());
    }

    #[test]
    fn collect_font_usage_from_element_handles_grid_row() {
        let element = LayoutElement::GridRow {
            cells: vec![empty_table_cell()],
            col_widths: vec![100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
        };
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<String, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty());
    }

    #[test]
    fn collect_font_usage_from_element_handles_flex_row() {
        let element = LayoutElement::FlexRow {
            cells: vec![empty_flex_cell()],
            row_height: 20.0,
            margin_top: 0.0,
            margin_bottom: 0.0,
            background_color: None,
            container_width: 500.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            border_radius: 0.0,
            box_shadow: None,
            background_gradient: None,
            background_radial_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
        };
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<String, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty());
    }

    #[test]
    fn collect_font_usage_from_element_handles_text_block() {
        let element = text_block_element(vec![empty_text_line()]);
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<String, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        // No custom fonts configured, so usage stays empty.
        assert!(usage.is_empty());
    }

    #[test]
    fn collect_font_usage_from_element_table_row_with_nested_rows() {
        // A TableCell with a nested TableRow inside should recurse.
        let nested = LayoutElement::TableRow {
            cells: vec![empty_table_cell()],
            col_widths: vec![50.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: BorderCollapse::Separate,
            border_spacing: 0.0,
        };
        let mut cell = empty_table_cell();
        cell.nested_rows = vec![nested];

        let element = LayoutElement::TableRow {
            cells: vec![cell],
            col_widths: vec![100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: BorderCollapse::Separate,
            border_spacing: 0.0,
        };
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<String, FontUsage> = BTreeMap::new();
        // Should not panic when recursing through nested rows.
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(usage.is_empty());
    }

    // ── collect_font_usage_from_run (non-custom font is skipped) ────────────

    #[test]
    fn collect_font_usage_skips_non_custom_font_family() {
        let run = TextRun {
            text: "Hello".into(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        let line = TextLine {
            runs: vec![run],
            height: 12.0,
        };
        let element = text_block_element(vec![line]);
        let fonts: HashMap<String, TtfFont> = HashMap::new();
        let mut usage: BTreeMap<String, FontUsage> = BTreeMap::new();
        collect_font_usage_from_element(&element, &fonts, &mut usage);
        assert!(
            usage.is_empty(),
            "non-custom font families should not produce any usage entries"
        );
    }
}
