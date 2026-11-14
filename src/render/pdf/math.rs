use super::*;
use crate::layout::elements::MathBlock;
use crate::layout::math::MathGlyph;
use crate::style::computed::FontStyle;

/// Delimiters taller than this many font sizes are drawn as paths.
const SCALED_DELIMITER_FACTOR: f32 = 1.3;

/// A formula glyph painted as an embedded text run, positioned relative to
/// the formula's layout origin.
pub(in crate::render) struct EmbeddedMathRun {
    pub(in crate::render) run: TextRun,
    pub(in crate::render) dx: f32,
    pub(in crate::render) dy: f32,
}

impl MathBlock {
    /// The formula glyphs an available embedded face covers, as text runs.
    pub(in crate::render) fn embedded_runs(
        &self,
        custom_fonts: &HashMap<String, TtfFont>,
    ) -> Vec<EmbeddedMathRun> {
        self.layout
            .glyphs
            .iter()
            .filter_map(|glyph| glyph.embedded_run(custom_fonts))
            .collect()
    }
}

impl MathGlyph {
    /// This glyph as an embedded text run, when the sans face the formula is
    /// set in or the fallback chain covers it. Rules, radicals and scaled
    /// delimiters are vector paths and have no run.
    pub(in crate::render) fn embedded_run(
        &self,
        custom_fonts: &HashMap<String, TtfFont>,
    ) -> Option<EmbeddedMathRun> {
        let (text, dx, dy, font_size, italic) = match self {
            Self::Char {
                ch,
                x,
                y,
                font_size,
                italic,
            } => (ch.to_string(), *x, *y, *font_size, *italic),
            Self::Text {
                text,
                x,
                y,
                font_size,
            } => (text.clone(), *x, *y, *font_size, false),
            Self::Delimiter {
                ch,
                x,
                y,
                height,
                font_size,
            } if *height <= font_size * SCALED_DELIMITER_FACTOR => {
                (ch.to_string(), *x, *y, *font_size, false)
            }
            Self::Rule { .. } | Self::Radical { .. } | Self::Delimiter { .. } => return None,
        };
        let run = TextRun {
            text,
            font_size,
            font_style: if italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            font_family: crate::system_fonts::resolve_font_family(
                &crate::style::computed::FontStack::from_family(FontFamily::Helvetica),
                custom_fonts,
                false,
                italic,
                crate::parser::css::FontStretch::default(),
            ),
            line_height_factor: 1.0,
            ..Default::default()
        };
        let covered = !crate::text::needs_unicode_fallback(&run, custom_fonts)
            || crate::text::shape_with_unicode_fallback(&run, custom_fonts).is_some();
        covered.then_some(EmbeddedMathRun { run, dx, dy })
    }
}

/// Map a Unicode character to the Adobe Symbol font encoding byte.
pub(super) fn unicode_to_symbol(ch: char) -> Option<u8> {
    match ch {
        // Greek lowercase
        '\u{03B1}' => Some(0x61), // α → a
        '\u{03B2}' => Some(0x62), // β → b
        '\u{03B3}' => Some(0x67), // γ → g
        '\u{03B4}' => Some(0x64), // δ → d
        '\u{03B5}' => Some(0x65), // ε → e
        '\u{03B6}' => Some(0x7A), // ζ → z
        '\u{03B7}' => Some(0x68), // η → h
        '\u{03B8}' => Some(0x71), // θ → q
        '\u{03B9}' => Some(0x69), // ι → i
        '\u{03BA}' => Some(0x6B), // κ → k
        '\u{03BB}' => Some(0x6C), // λ → l
        '\u{03BC}' => Some(0x6D), // μ → m
        '\u{03BD}' => Some(0x6E), // ν → n
        '\u{03BE}' => Some(0x78), // ξ → x
        '\u{03C0}' => Some(0x70), // π → p
        '\u{03C1}' => Some(0x72), // ρ → r
        '\u{03C3}' => Some(0x73), // σ → s
        '\u{03C4}' => Some(0x74), // τ → t
        '\u{03C5}' => Some(0x75), // υ → u
        '\u{03C6}' => Some(0x66), // φ → f
        '\u{03C7}' => Some(0x63), // χ → c
        '\u{03C8}' => Some(0x79), // ψ → y
        '\u{03C9}' => Some(0x77), // ω → w
        // Greek uppercase
        '\u{0393}' => Some(0x47), // Γ → G
        '\u{0394}' => Some(0x44), // Δ → D
        '\u{0398}' => Some(0x51), // Θ → Q
        '\u{039B}' => Some(0x4C), // Λ → L
        '\u{039E}' => Some(0x58), // Ξ → X
        '\u{03A0}' => Some(0x50), // Π → P
        '\u{03A3}' => Some(0x53), // Σ → S
        '\u{03A5}' => Some(0xA1), // Υ
        '\u{03A6}' => Some(0x46), // Φ → F
        '\u{03A8}' => Some(0x59), // Ψ → Y
        '\u{03A9}' => Some(0x57), // Ω → W
        // Large operators
        '\u{2211}' => Some(0xE5), // ∑
        '\u{220F}' => Some(0xD5), // ∏
        '\u{2210}' => Some(0xD5), // ∐ (fallback to ∏)
        '\u{222B}' => Some(0xF2), // ∫
        '\u{222C}' => Some(0xF2), // ∬ (fallback to ∫)
        '\u{222D}' => Some(0xF2), // ∭ (fallback to ∫)
        '\u{222E}' => Some(0xF2), // ∮ (fallback to ∫)
        '\u{22C3}' => Some(0xC8), // ⋃
        '\u{22C2}' => Some(0xC7), // ⋂
        // Relations
        '\u{2264}' => Some(0xA3), // ≤
        '\u{2265}' => Some(0xB3), // ≥
        '\u{2260}' => Some(0xB9), // ≠
        '\u{2248}' => Some(0xBB), // ≈
        '\u{2261}' => Some(0xBA), // ≡
        '\u{221D}' => Some(0xB5), // ∝
        '\u{2282}' => Some(0xCC), // ⊂
        '\u{2283}' => Some(0xC9), // ⊃
        '\u{2286}' => Some(0xCD), // ⊆
        '\u{2287}' => Some(0xCA), // ⊇
        '\u{2208}' => Some(0xCE), // ∈
        '\u{2209}' => Some(0xCF), // ∉
        '\u{22A2}' => Some(0x5E), // ⊢ (fallback)
        '\u{22A8}' => Some(0xF0), // ⊨
        // Arrows
        '\u{2192}' => Some(0xAE), // →
        '\u{2190}' => Some(0xAC), // ←
        '\u{2194}' => Some(0xAB), // ↔
        '\u{21D2}' => Some(0xDE), // ⇒
        '\u{21D0}' => Some(0xDC), // ⇐
        '\u{21D4}' => Some(0xDB), // ⇔
        '\u{21A6}' => Some(0xAE), // ↦ (fallback to →)
        // Binary operators
        '\u{00D7}' => Some(0xB4), // ×
        '\u{00F7}' => Some(0xB8), // ÷
        '\u{22C5}' => Some(0xD7), // ⋅
        '\u{00B1}' => Some(0xB1), // ±
        '\u{2213}' => Some(0xB1), // ∓ (fallback to ±)
        '\u{2218}' => Some(0xB0), // ∘
        '\u{2295}' => Some(0xC5), // ⊕
        '\u{2297}' => Some(0xC4), // ⊗
        '\u{222A}' => Some(0xC8), // ∪
        '\u{2229}' => Some(0xC7), // ∩
        '\u{2227}' => Some(0xD9), // ∧
        '\u{2228}' => Some(0xDA), // ∨
        // Misc math symbols
        '\u{221E}' => Some(0xA5), // ∞
        '\u{2202}' => Some(0xB6), // ∂
        '\u{2207}' => Some(0xD1), // ∇
        '\u{2200}' => Some(0x22), // ∀
        '\u{2203}' => Some(0x24), // ∃
        '\u{00AC}' => Some(0xD8), // ¬
        '\u{2205}' => Some(0xC6), // ∅
        '\u{2135}' => Some(0xC0), // ℵ
        '\u{221A}' => Some(0xD6), // √
        '\u{2032}' => Some(0xA2), // ′
        '\u{2026}' => Some(0xBC), // …
        '\u{22EF}' => Some(0xBC), // ⋯
        '\u{2016}' => Some(0xBD), // ‖
        // Delimiters
        '\u{27E8}' => Some(0xE1), // ⟨
        '\u{27E9}' => Some(0xF1), // ⟩
        '\u{230A}' => Some(0xEB), // ⌊
        '\u{230B}' => Some(0xFB), // ⌋
        '\u{2308}' => Some(0xE9), // ⌈
        '\u{2309}' => Some(0xF9), // ⌉
        _ => None,
    }
}

/// Render math glyphs to PDF content stream operators.
pub(super) fn render_math_glyphs(
    glyphs: &[crate::layout::math::MathGlyph],
    origin_x: f32,
    origin_y: f32,
    content: &mut String,
) {
    use crate::layout::math::MathGlyph;

    for glyph in glyphs {
        match glyph {
            MathGlyph::Char {
                ch,
                x,
                y,
                font_size,
                italic,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;

                // Check if character needs Symbol font
                if let Some(sym_byte) = unicode_to_symbol(*ch) {
                    let encoded = format!("\\{sym_byte:03o}");
                    content.push_str("BT\n");
                    content.push_str(&format!("/Symbol {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                } else {
                    let font_name = if *italic {
                        "Helvetica-Oblique"
                    } else {
                        "Helvetica"
                    };
                    let encoded = encode_pdf_text(&ch.to_string());
                    content.push_str("BT\n");
                    content.push_str(&format!("/{font_name} {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                }
            }
            MathGlyph::Text {
                text,
                x,
                y,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                let encoded = encode_pdf_text(text);
                content.push_str("BT\n");
                content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                content.push_str(&format!("{px} {py} Td\n"));
                content.push_str(&format!("({encoded}) Tj\n"));
                content.push_str("ET\n");
            }
            MathGlyph::Rule {
                x,
                y,
                width,
                thickness,
            } => {
                let px = origin_x + x;
                let py = origin_y + y - thickness / 2.0;
                content.push_str("0 0 0 rg\n");
                content.push_str(&format!("{px} {py} {width} {thickness} re\nf\n"));
            }
            MathGlyph::Radical {
                x,
                y,
                width,
                height,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                let line_w = font_size * 0.04;
                content.push_str(&format!("{line_w} w\n0 0 0 RG\n"));
                // Draw radical sign: short tick down, long line up-right, horizontal overline
                let tick_x = px + width * 0.15;
                let tick_bottom = py - height * 0.3;
                let bottom_x = px + width * 0.35;
                let bottom_y = py - height;
                let top_x = px + width;
                let top_y = py;
                content.push_str(&format!(
                    "{tick_x} {tick_bottom} m\n{bottom_x} {bottom_y} l\n{top_x} {top_y} l\nS\n"
                ));
            }
            MathGlyph::Delimiter {
                ch,
                x,
                y,
                height,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                // For small delimiters, use text; for large, draw paths
                if *height <= font_size * SCALED_DELIMITER_FACTOR {
                    let encoded = encode_pdf_text(&ch.to_string());
                    content.push_str("BT\n");
                    content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                } else {
                    // Draw scaled delimiter using PDF path ops
                    let line_w = font_size * 0.04;
                    content.push_str(&format!("{line_w} w\n0 0 0 RG\n"));
                    let half_h = height / 2.0;
                    match ch {
                        '(' => {
                            // Left parenthesis as cubic bezier
                            let cx = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            let ctrl_offset = height * 0.55;
                            content.push_str(&format!(
                                "{cx} {top_y} m\n{px} {c1y} {px} {c2y} {cx} {bot_y} c\nS\n",
                                c1y = py + ctrl_offset * 0.3,
                                c2y = py - ctrl_offset * 0.3,
                            ));
                        }
                        ')' => {
                            let cx = px;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            let ctrl_offset = height * 0.55;
                            content.push_str(&format!(
                                "{cx} {top_y} m\n{right} {c1y} {right} {c2y} {cx} {bot_y} c\nS\n",
                                c1y = py + ctrl_offset * 0.3,
                                c2y = py - ctrl_offset * 0.3,
                            ));
                        }
                        '[' => {
                            let right = px + font_size * 0.2;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{right} {top_y} m {px} {top_y} l {px} {bot_y} l {right} {bot_y} l S\n"
                            ));
                        }
                        ']' => {
                            let left = px;
                            let right = px + font_size * 0.2;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{left} {top_y} m {right} {top_y} l {right} {bot_y} l {left} {bot_y} l S\n"
                            ));
                        }
                        '{' => {
                            let mid = px + font_size * 0.15;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{right} {top_y} m {mid} {top_y} l {mid} {py} l {px} {py} l S\n\
                                 {px} {py} m {mid} {py} l {mid} {bot_y} l {right} {bot_y} l S\n"
                            ));
                        }
                        '}' => {
                            let mid = px + font_size * 0.1;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{px} {top_y} m {mid} {top_y} l {mid} {py} l {right} {py} l S\n\
                                 {right} {py} m {mid} {py} l {mid} {bot_y} l {px} {bot_y} l S\n"
                            ));
                        }
                        '|' => {
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!("{px} {top_y} m {px} {bot_y} l S\n"));
                        }
                        _ => {
                            // Fallback: render as text character
                            let encoded = encode_pdf_text(&ch.to_string());
                            content.push_str("BT\n");
                            content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                            content.push_str(&format!("{px} {py} Td\n"));
                            content.push_str(&format!("({encoded}) Tj\n"));
                            content.push_str("ET\n");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HtmlConverter;

    const SUM: &str = r#"<html lang="en"><head><title>Math</title></head><body>
        <div class="math-display" data-math="\sum_{k=1}^{n} k = \frac{n(n+1)}{2}"></div>
        </body></html>"#;

    fn pdf(html: &str, accessible: bool) -> String {
        let bytes = HtmlConverter::new()
            .accessible(accessible)
            .compress(false)
            .convert(html)
            .unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Text operators selecting one of the unembedded base-14 fonts.
    fn base14_text_operators(pdf: &str) -> usize {
        pdf.lines()
            .filter(|line| {
                line.ends_with(" Tf")
                    && (line.starts_with("/Symbol ") || line.starts_with("/Helvetica"))
            })
            .count()
    }

    /// ISO 14289-1 7.21.4.1: fonts used for rendering content are embedded;
    /// 7.1: the formula is the content of its `Formula` element.
    #[test]
    fn accessible_display_math_paints_embedded_glyphs_inside_a_formula_mark() {
        let tagged = pdf(SUM, true);
        assert!(tagged.contains("/Formula << /MCID "), "{tagged}");
        assert_eq!(base14_text_operators(&tagged), 0, "{tagged}");
        assert!(
            tagged.contains("/Alt (\\\\sum_{k=1}^{n} k = \\\\frac{n\\(n+1\\)}{2})"),
            "{tagged}"
        );
        assert!(pdf(SUM, false).contains("/Symbol "));
    }

    /// Embedding follows font coverage: the bundled Liberation Sans carries
    /// Greek, no bundled face carries U+2200 FOR ALL, and rules are paths.
    #[test]
    fn embedded_run_follows_bundled_font_coverage() {
        let mut fonts = HashMap::new();
        crate::system_fonts::load_bundled_liberation_fonts(&mut fonts);
        let glyph = |ch| MathGlyph::Char {
            ch,
            x: 1.0,
            y: 2.0,
            font_size: 12.0,
            italic: true,
        };
        let alpha = glyph('α')
            .embedded_run(&fonts)
            .expect("Liberation Sans covers Greek");
        assert_eq!(alpha.run.text, "α");
        assert_eq!((alpha.dx, alpha.dy, alpha.run.font_size), (1.0, 2.0, 12.0));
        assert!(matches!(alpha.run.font_style, FontStyle::Italic));
        assert!(glyph('\u{2200}').embedded_run(&fonts).is_none());
        let rule = MathGlyph::Rule {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            thickness: 0.5,
        };
        assert!(rule.embedded_run(&fonts).is_none());
    }
}
