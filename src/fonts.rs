//! Adobe Font Metrics (AFM) for standard PDF fonts.
//!
//! Character widths are sourced from the Adobe AFM files for the base-14 PDF
//! fonts. Each width is given in units of 1/1000 em. To obtain the width in
//! points, multiply: `afm_width / 1000.0 * font_size`.

use crate::style::computed::FontFamily;

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
    } else {
        DEFAULT_WIDTH
    }
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
}
