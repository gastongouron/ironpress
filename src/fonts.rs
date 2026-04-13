//! Adobe Font Metrics (AFM) for standard PDF fonts.
//!
//! Character widths are sourced from the Adobe AFM files for the base-14 PDF
//! fonts. Each width is given in units of 1/1000 em. To obtain the width in
//! points, multiply: `afm_width / 1000.0 * font_size`.

use crate::style::computed::FontFamily;
use crate::{parser::ttf::TtfFont, system_fonts, text};
use std::collections::HashMap;

/// Helvetica character widths (AFM units, 1000 per em) for ASCII 32–126.
/// Index 0 corresponds to codepoint 32 (space).
static HELVETICA_WIDTHS: [u16; 95] = [
    278,  // 32 space
    278,  // 33 !
    355,  // 34 "
    556,  // 35 #
    556,  // 36 $
    889,  // 37 %
    667,  // 38 &
    191,  // 39 '
    333,  // 40 (
    333,  // 41 )
    389,  // 42 *
    584,  // 43 +
    278,  // 44 ,
    333,  // 45 -
    278,  // 46 .
    278,  // 47 /
    556,  // 48 0
    556,  // 49 1
    556,  // 50 2
    556,  // 51 3
    556,  // 52 4
    556,  // 53 5
    556,  // 54 6
    556,  // 55 7
    556,  // 56 8
    556,  // 57 9
    278,  // 58 :
    278,  // 59 ;
    584,  // 60 <
    584,  // 61 =
    584,  // 62 >
    556,  // 63 ?
    1015, // 64 @
    667,  // 65 A
    667,  // 66 B
    722,  // 67 C
    722,  // 68 D
    667,  // 69 E
    611,  // 70 F
    778,  // 71 G
    722,  // 72 H
    278,  // 73 I
    500,  // 74 J
    667,  // 75 K
    556,  // 76 L
    833,  // 77 M
    722,  // 78 N
    778,  // 79 O
    667,  // 80 P
    778,  // 81 Q
    722,  // 82 R
    667,  // 83 S
    611,  // 84 T
    722,  // 85 U
    667,  // 86 V
    944,  // 87 W
    667,  // 88 X
    667,  // 89 Y
    611,  // 90 Z
    278,  // 91 [
    278,  // 92 \
    278,  // 93 ]
    469,  // 94 ^
    556,  // 95 _
    333,  // 96 `
    556,  // 97 a
    556,  // 98 b
    500,  // 99 c
    556,  // 100 d
    556,  // 101 e
    278,  // 102 f
    556,  // 103 g
    556,  // 104 h
    222,  // 105 i
    222,  // 106 j
    500,  // 107 k
    222,  // 108 l
    833,  // 109 m
    556,  // 110 n
    556,  // 111 o
    556,  // 112 p
    556,  // 113 q
    333,  // 114 r
    500,  // 115 s
    278,  // 116 t
    556,  // 117 u
    500,  // 118 v
    722,  // 119 w
    500,  // 120 x
    500,  // 121 y
    500,  // 122 z
    334,  // 123 {
    260,  // 124 |
    334,  // 125 }
    584,  // 126 ~
];

/// Helvetica-Bold character widths (AFM units, 1000 per em) for ASCII 32–126.
/// Index 0 corresponds to codepoint 32 (space).
static HELVETICA_BOLD_WIDTHS: [u16; 95] = [
    278, // 32 space
    333, // 33 !
    474, // 34 "
    556, // 35 #
    556, // 36 $
    889, // 37 %
    722, // 38 &
    238, // 39 '
    333, // 40 (
    333, // 41 )
    389, // 42 *
    584, // 43 +
    278, // 44 ,
    333, // 45 -
    278, // 46 .
    278, // 47 /
    556, // 48 0
    556, // 49 1
    556, // 50 2
    556, // 51 3
    556, // 52 4
    556, // 53 5
    556, // 54 6
    556, // 55 7
    556, // 56 8
    556, // 57 9
    333, // 58 :
    333, // 59 ;
    584, // 60 <
    584, // 61 =
    584, // 62 >
    611, // 63 ?
    975, // 64 @
    722, // 65 A
    722, // 66 B
    722, // 67 C
    722, // 68 D
    667, // 69 E
    611, // 70 F
    778, // 71 G
    722, // 72 H
    278, // 73 I
    556, // 74 J
    722, // 75 K
    611, // 76 L
    833, // 77 M
    722, // 78 N
    778, // 79 O
    667, // 80 P
    778, // 81 Q
    722, // 82 R
    667, // 83 S
    611, // 84 T
    722, // 85 U
    667, // 86 V
    944, // 87 W
    667, // 88 X
    667, // 89 Y
    611, // 90 Z
    333, // 91 [
    278, // 92 \
    333, // 93 ]
    584, // 94 ^
    556, // 95 _
    333, // 96 `
    556, // 97 a
    611, // 98 b
    556, // 99 c
    611, // 100 d
    556, // 101 e
    333, // 102 f
    611, // 103 g
    611, // 104 h
    278, // 105 i
    278, // 106 j
    556, // 107 k
    278, // 108 l
    889, // 109 m
    611, // 110 n
    611, // 111 o
    611, // 112 p
    611, // 113 q
    389, // 114 r
    556, // 115 s
    333, // 116 t
    611, // 117 u
    556, // 118 v
    778, // 119 w
    556, // 120 x
    556, // 121 y
    500, // 122 z
    389, // 123 {
    280, // 124 |
    389, // 125 }
    584, // 126 ~
];

/// Default width for characters outside ASCII 32–126 (AFM units).
const DEFAULT_WIDTH: u16 = 556;

/// Ascender height as a fraction of 1 em, from Adobe AFM data.
/// Helvetica: Ascender 718, Times-Roman: 683, Courier: 629.
pub(crate) fn ascender_ratio(font_family: &FontFamily) -> f32 {
    match font_family {
        FontFamily::Helvetica | FontFamily::Custom(_) => 0.718,
        FontFamily::TimesRoman => 0.683,
        FontFamily::Courier => 0.629,
    }
}

/// Descender depth as a fraction of 1 em (positive value), from Adobe AFM data.
/// Helvetica: Descender -207, Times-Roman: -217, Courier: -157.
pub(crate) fn descender_ratio(font_family: &FontFamily) -> f32 {
    match font_family {
        FontFamily::Helvetica | FontFamily::Custom(_) => 0.207,
        FontFamily::TimesRoman => 0.217,
        FontFamily::Courier => 0.157,
    }
}

pub(crate) fn font_metrics_ratios(
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    custom_fonts: &HashMap<String, TtfFont>,
) -> (f32, f32) {
    if let FontFamily::Custom(name) = font_family {
        if let Some((_, ttf)) = system_fonts::find_font(custom_fonts, name, bold, italic) {
            let metrics = ttf.pdf_vertical_metrics();
            return (
                metrics.ascender_ratio(ttf.units_per_em),
                metrics.descender_ratio(ttf.units_per_em),
            );
        }
    }

    (ascender_ratio(font_family), descender_ratio(font_family))
}

pub(crate) fn normal_line_height_factor(
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    custom_fonts: &HashMap<String, TtfFont>,
) -> f32 {
    if matches!(font_family, FontFamily::Custom(_)) {
        if let Some(height) = text::custom_font_line_height(font_family, bold, italic, custom_fonts)
        {
            return height;
        }

        let (ascender, descender) = font_metrics_ratios(font_family, bold, italic, custom_fonts);
        return (ascender + descender).max(1.0);
    }

    1.2
}

/// Courier character width (all glyphs are the same in a monospace font).
const COURIER_WIDTH: u16 = 600;

/// Return the AFM character width for a single character in the given font,
/// scaled to points: `afm_width / 1000.0 * font_size`.
///
/// For `FontFamily::Custom` this falls back to Helvetica metrics (callers
/// should prefer TTF metrics when the custom font data is available).
pub(crate) fn char_width(ch: char, font_size: f32, font_family: &FontFamily, bold: bool) -> f32 {
    let afm = match font_family {
        FontFamily::Courier => COURIER_WIDTH,
        FontFamily::Helvetica | FontFamily::Custom(_) => helvetica_char_afm(ch, bold),
        FontFamily::TimesRoman => {
            // Times-Roman has similar proportions to Helvetica; reuse
            // Helvetica metrics as a reasonable approximation.
            helvetica_char_afm(ch, bold)
        }
    };
    afm as f32 / 1000.0 * font_size
}

/// Return the total width (in points) of a string using AFM metrics.
pub(crate) fn str_width(s: &str, font_size: f32, font_family: &FontFamily, bold: bool) -> f32 {
    s.chars()
        .map(|c| char_width(c, font_size, font_family, bold))
        .sum()
}

/// Return the standard PDF font name for a given base family, weight, and style.
///
/// This is the single source of truth for mapping CSS font properties to PDF
/// base-14 font names. Used by the PDF renderer and the SVG text pipeline.
pub(crate) fn pdf_font_name(base_family: &str, bold: bool, italic: bool) -> &'static str {
    let normalized = base_family.trim();
    let is_times = normalized.eq_ignore_ascii_case("Times-Roman")
        || normalized.eq_ignore_ascii_case("Times")
        || normalized.eq_ignore_ascii_case("Times-Bold")
        || normalized.eq_ignore_ascii_case("Times-Italic")
        || normalized.eq_ignore_ascii_case("Times-BoldItalic")
        || normalized.eq_ignore_ascii_case("Times New Roman");
    let is_courier = normalized.eq_ignore_ascii_case("Courier")
        || normalized.eq_ignore_ascii_case("Courier-Bold")
        || normalized.eq_ignore_ascii_case("Courier-Oblique")
        || normalized.eq_ignore_ascii_case("Courier-BoldOblique")
        || normalized.eq_ignore_ascii_case("Courier New");

    if is_times {
        match (bold, italic) {
            (true, true) => "Times-BoldItalic",
            (true, false) => "Times-Bold",
            (false, true) => "Times-Italic",
            (false, false) => "Times-Roman",
        }
    } else if is_courier {
        match (bold, italic) {
            (true, true) => "Courier-BoldOblique",
            (true, false) => "Courier-Bold",
            (false, true) => "Courier-Oblique",
            (false, false) => "Courier",
        }
    } else {
        match (bold, italic) {
            (true, true) => "Helvetica-BoldOblique",
            (true, false) => "Helvetica-Bold",
            (false, true) => "Helvetica-Oblique",
            (false, false) => "Helvetica",
        }
    }
}

#[cfg(test)]
fn pdf_font_name_for_family(font_family: &FontFamily, bold: bool, italic: bool) -> &'static str {
    let base_family = match font_family {
        FontFamily::Helvetica | FontFamily::Custom(_) => "Helvetica",
        FontFamily::TimesRoman => "Times-Roman",
        FontFamily::Courier => "Courier",
    };
    pdf_font_name(base_family, bold, italic)
}

/// Look up the Helvetica (or Helvetica-Bold) AFM width for a character.
fn helvetica_char_afm(ch: char, bold: bool) -> u16 {
    let code = ch as u32;
    if (32..=126).contains(&code) {
        let idx = (code - 32) as usize;
        if bold {
            HELVETICA_BOLD_WIDTHS[idx]
        } else {
            HELVETICA_WIDTHS[idx]
        }
    } else if is_cjk_char(code) || is_fullwidth_char(code) {
        // CJK ideographs and fullwidth characters are approximately 1em wide
        1000
    } else if is_emoji_char(code) {
        // Emoji are typically rendered at full-width
        1000
    } else if (0x0590..=0x08FF).contains(&code) {
        // Hebrew, Arabic, and related scripts — proportional widths
        if bold { 600 } else { 556 }
    } else if (0x0370..=0x03FF).contains(&code) {
        // Greek characters — similar to Latin proportions
        if bold { 600 } else { 556 }
    } else if (0x2000..=0x206F).contains(&code) {
        // General punctuation (em dash, en dash, etc.)
        match code {
            0x2013 => 500,          // en-dash
            0x2014 => 1000,         // em-dash
            0x2018 | 0x2019 => 222, // single quotes
            0x201C | 0x201D => 333, // double quotes
            0x2026 => 1000,         // ellipsis
            _ => DEFAULT_WIDTH,
        }
    } else if (0x2500..=0x257F).contains(&code) {
        // Box drawing characters — monospaced
        600
    } else {
        DEFAULT_WIDTH
    }
}

/// Returns true for CJK Unified Ideographs and common CJK ranges.
fn is_cjk_char(code: u32) -> bool {
    matches!(code,
        0x4E00..=0x9FFF       // CJK Unified Ideographs
        | 0x3400..=0x4DBF     // CJK Unified Ideographs Extension A
        | 0x3000..=0x303F     // CJK Symbols and Punctuation
        | 0x3040..=0x309F     // Hiragana
        | 0x30A0..=0x30FF     // Katakana
        | 0x31F0..=0x31FF     // Katakana Phonetic Extensions
        | 0xAC00..=0xD7AF     // Hangul Syllables
        | 0xF900..=0xFAFF     // CJK Compatibility Ideographs
        | 0x20000..=0x2A6DF   // CJK Extension B
    )
}

/// Returns true for fullwidth forms.
fn is_fullwidth_char(code: u32) -> bool {
    (0xFF01..=0xFF60).contains(&code) || (0xFFE0..=0xFFE6).contains(&code)
}

/// Returns true for emoji codepoints.
pub(crate) fn is_emoji_char(code: u32) -> bool {
    matches!(code,
        0x1F600..=0x1F64F   // Emoticons
        | 0x1F300..=0x1F5FF // Misc Symbols and Pictographs
        | 0x1F680..=0x1F6FF // Transport and Map
        | 0x1F1E0..=0x1F1FF // Flags (regional indicators)
        | 0x2600..=0x26FF   // Misc symbols
        | 0x2700..=0x27BF   // Dingbats
        | 0x1F900..=0x1F9FF // Supplemental Symbols
        | 0x1FA00..=0x1FA6F // Chess Symbols
        | 0x1FA70..=0x1FAFF // Symbols and Pictographs Extended-A
        | 0xFE00..=0xFE0F   // Variation Selectors
        | 0x200D            // ZWJ
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helvetica_space_width() {
        // Space in Helvetica is 278/1000 em.  At 10pt that's 2.78pt.
        let w = char_width(' ', 10.0, &FontFamily::Helvetica, false);
        assert!((w - 2.78).abs() < 0.01);
    }

    #[test]
    fn helvetica_bold_a_wider_than_regular() {
        let regular = char_width('A', 12.0, &FontFamily::Helvetica, false);
        let bold = char_width('A', 12.0, &FontFamily::Helvetica, true);
        assert!(bold > regular);
    }

    #[test]
    fn courier_fixed_width() {
        let w1 = char_width('i', 10.0, &FontFamily::Courier, false);
        let w2 = char_width('W', 10.0, &FontFamily::Courier, false);
        assert!((w1 - w2).abs() < f32::EPSILON);
    }

    #[test]
    fn str_width_hello() {
        // "Hello" in Helvetica 10pt:
        // H=722, e=556, l=222, l=222, o=556  => total 2278 => 2278/1000*10 = 22.78
        let w = str_width("Hello", 10.0, &FontFamily::Helvetica, false);
        assert!((w - 22.78).abs() < 0.01);
    }

    #[test]
    fn non_ascii_uses_default() {
        // Any character >126 should use 556 default
        let w = char_width('\u{00E9}', 10.0, &FontFamily::Helvetica, false);
        assert!((w - 5.56).abs() < 0.01);
    }

    #[test]
    fn helvetica_uppercase_wider() {
        // 'W' (944) should be wider than 'i' (222) in Helvetica
        let w_upper = char_width('W', 12.0, &FontFamily::Helvetica, false);
        let w_lower = char_width('i', 12.0, &FontFamily::Helvetica, false);
        assert!(
            w_upper > w_lower,
            "W ({w_upper}) should be wider than i ({w_lower})"
        );
    }

    #[test]
    fn ascender_ratio_helvetica() {
        let r = ascender_ratio(&FontFamily::Helvetica);
        assert!((r - 0.718).abs() < f32::EPSILON);
    }

    #[test]
    fn descender_ratio_helvetica() {
        let r = descender_ratio(&FontFamily::Helvetica);
        assert!((r - 0.207).abs() < f32::EPSILON);
    }

    #[test]
    fn ascender_plus_descender_less_than_one() {
        for family in &[
            FontFamily::Helvetica,
            FontFamily::TimesRoman,
            FontFamily::Courier,
        ] {
            let sum = ascender_ratio(family) + descender_ratio(family);
            assert!(
                sum < 1.0,
                "ascender + descender should be < 1.0 em for {family:?}"
            );
        }
    }

    #[test]
    fn bold_wider_than_regular() {
        // Bold 'a' (556) vs regular 'a' (556) — in Helvetica-Bold 'a' is 556 same,
        // but 'b' is 611 bold vs 556 regular
        let regular = char_width('b', 12.0, &FontFamily::Helvetica, false);
        let bold = char_width('b', 12.0, &FontFamily::Helvetica, true);
        assert!(
            bold > regular,
            "Bold 'b' ({bold}) should be wider than regular 'b' ({regular})"
        );
    }

    #[test]
    fn pdf_font_name_helvetica_variants() {
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::Helvetica, false, false),
            "Helvetica"
        );
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::Helvetica, true, false),
            "Helvetica-Bold"
        );
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::Helvetica, false, true),
            "Helvetica-Oblique"
        );
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::Helvetica, true, true),
            "Helvetica-BoldOblique"
        );
    }

    #[test]
    fn pdf_font_name_times_variants() {
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::TimesRoman, false, false),
            "Times-Roman"
        );
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::TimesRoman, true, false),
            "Times-Bold"
        );
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::TimesRoman, false, true),
            "Times-Italic"
        );
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::TimesRoman, true, true),
            "Times-BoldItalic"
        );
    }

    #[test]
    fn pdf_font_name_courier_variants() {
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::Courier, false, false),
            "Courier"
        );
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::Courier, true, true),
            "Courier-BoldOblique"
        );
    }

    #[test]
    fn pdf_font_name_custom_falls_back_to_helvetica() {
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::Custom("MyFont".into()), false, false),
            "Helvetica"
        );
        assert_eq!(
            pdf_font_name_for_family(&FontFamily::Custom("MyFont".into()), true, true),
            "Helvetica-BoldOblique"
        );
    }

    /// BUG P2-3: the default `line-height: normal` factor must be 1.2 for
    /// Helvetica/Arial, matching Chrome's measured value.  A smaller constant
    /// (e.g. 1.0) would produce tighter text than browsers render.
    #[test]
    fn normal_line_height_factor_helvetica_is_1_2() {
        let custom_fonts = HashMap::new();
        let factor = normal_line_height_factor(&FontFamily::Helvetica, false, false, &custom_fonts);
        assert!(
            (factor - 1.2).abs() < 0.001,
            "Helvetica normal line-height should be 1.2 (Chrome parity), got {factor}"
        );
    }

    #[test]
    fn normal_line_height_factor_helvetica_bold_is_1_2() {
        let custom_fonts = HashMap::new();
        let factor = normal_line_height_factor(&FontFamily::Helvetica, true, false, &custom_fonts);
        assert!(
            (factor - 1.2).abs() < 0.001,
            "Helvetica-Bold normal line-height should be 1.2, got {factor}"
        );
    }

    #[test]
    fn normal_line_height_factor_standard_fonts_all_1_2() {
        let custom_fonts = HashMap::new();
        for family in &[
            FontFamily::Helvetica,
            FontFamily::TimesRoman,
            FontFamily::Courier,
        ] {
            for bold in [false, true] {
                let factor = normal_line_height_factor(family, bold, false, &custom_fonts);
                assert!(
                    (factor - 1.2).abs() < 0.001,
                    "{family:?} bold={bold} normal line-height should be 1.2, got {factor}"
                );
            }
        }
    }
}
