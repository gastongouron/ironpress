use crate::layout::engine::TextRun;
use crate::parser::ttf::TtfFont;
use crate::style::computed::FontFamily;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub(crate) struct ShapedGlyph {
    pub glyph_id: u16,
    pub x_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub unicode: Vec<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
}

pub(crate) fn resolve_custom_font<'a>(
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<(&'a str, &'a TtfFont)> {
    let FontFamily::Custom(name) = font_family else {
        return None;
    };

    crate::system_fonts::find_font(fonts, name, bold, italic)
}

pub(crate) fn measure_text_width(
    text: &str,
    font_size: f32,
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    let (_, font) = resolve_custom_font(font_family, bold, italic, fonts)?;
    shape_text_with_font(text, font_size, font).map(|run| run.width)
}

pub(crate) fn custom_font_line_height(
    font_family: &FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    let (_, font) = resolve_custom_font(font_family, bold, italic, fonts)?;
    Some(
        font.layout_vertical_metrics()
            .line_height_ratio(font.units_per_em),
    )
}

pub(crate) fn shape_text_run(run: &TextRun, fonts: &HashMap<String, TtfFont>) -> Option<ShapedRun> {
    let (_, font) = resolve_custom_font(&run.font_family, run.bold, run.italic, fonts)?;
    shape_text_with_font(&run.text, run.font_size, font)
}

/// Try to shape `run` with the Unicode fallback font.
///
/// Returns `Some((shaped_run, font_key))` when the run uses a standard PDF font,
/// contains non-WinAnsi characters, and the fallback font is loaded and can shape
/// the text.  The returned `font_key` is the key into the custom fonts map.
pub(crate) fn shape_with_unicode_fallback<'a>(
    run: &TextRun,
    fonts: &'a HashMap<String, TtfFont>,
) -> Option<(ShapedRun, &'a str, &'a TtfFont)> {
    // For standard PDF fonts, fall back when text has non-WinAnsi characters.
    // For custom fonts (including bundled Liberation), fall back when the
    // primary font cannot shape the text (missing glyphs for CJK, Arabic, etc.).
    if matches!(run.font_family, FontFamily::Custom(_)) {
        // Check if all characters in the run have glyphs in the primary font's
        // cmap table. If any character is missing, fall back to the unicode font.
        let all_covered = if let Some((_, primary_font)) =
            crate::system_fonts::find_font(fonts, run.font_family.name(), run.bold, run.italic)
        {
            run.text.chars().all(|ch| {
                let cp = ch as u32;
                primary_font.cmap.contains_key(&cp)
            })
        } else {
            false
        };
        if all_covered {
            return None;
        }
        // Font doesn't cover all characters — try unicode fallback
    } else if crate::render::pdf::is_winansi_encodable(&run.text) {
        return None;
    }
    // Try fallback fonts in order: Arabic → Multilingual (Noto Sans) →
    // Emoji → System Unicode (CJK).
    let fallback_keys = [
        crate::system_fonts::ARABIC_FALLBACK_KEY,
        crate::system_fonts::MULTILINGUAL_FALLBACK_KEY,
        crate::system_fonts::EMOJI_FALLBACK_KEY,
        crate::system_fonts::UNICODE_FALLBACK_KEY,
    ];
    for fk in fallback_keys {
        if let Some((key, font)) = fonts.get_key_value(fk) {
            if let Some(shaped) = shape_text_with_font(&run.text, run.font_size, font) {
                // Only use this font if ALL glyphs are resolved (no .notdef)
                let all_resolved =
                    !shaped.glyphs.is_empty() && shaped.glyphs.iter().all(|g| g.glyph_id != 0);
                if all_resolved {
                    return Some((shaped, key.as_str(), font));
                }
            }
        }
    }
    None
}

/// Check if a run needs unicode fallback (has characters the primary font can't cover).
pub(crate) fn needs_unicode_fallback(run: &TextRun, fonts: &HashMap<String, TtfFont>) -> bool {
    if let FontFamily::Custom(name) = &run.font_family {
        if let Some((_, font)) = crate::system_fonts::find_font(fonts, name, run.bold, run.italic) {
            return run.text.chars().any(|ch| {
                let cp = ch as u32;
                !font.cmap.contains_key(&cp)
            });
        }
    }
    !crate::render::pdf::is_winansi_encodable(&run.text)
}

/// Split a text run into segments at font-coverage boundaries.
/// Returns `(text, use_fallback)` pairs — `use_fallback=true` means the
/// segment should be rendered with the unicode fallback font.
pub(crate) fn split_run_by_font_coverage(
    run: &TextRun,
    fonts: &HashMap<String, TtfFont>,
) -> Vec<(String, bool)> {
    let primary_font = if let FontFamily::Custom(name) = &run.font_family {
        crate::system_fonts::find_font(fonts, name, run.bold, run.italic).map(|(_, f)| f)
    } else {
        None
    };

    let mut segments: Vec<(String, bool)> = Vec::new();
    let mut current = String::new();
    let mut current_is_fallback = false;

    for ch in run.text.chars() {
        let needs_fallback = if let Some(font) = primary_font {
            let cp = ch as u32;
            !font.cmap.contains_key(&cp)
        } else {
            !crate::render::pdf::is_winansi_char(ch)
        };

        if current.is_empty() {
            current_is_fallback = needs_fallback;
        } else if needs_fallback != current_is_fallback {
            segments.push((std::mem::take(&mut current), current_is_fallback));
            current_is_fallback = needs_fallback;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        segments.push((current, current_is_fallback));
    }
    segments
}

fn shape_text_with_font(text: &str, font_size: f32, font: &TtfFont) -> Option<ShapedRun> {
    if text.is_empty() {
        return Some(ShapedRun {
            glyphs: Vec::new(),
            width: 0.0,
        });
    }

    let face = rustybuzz::Face::from_slice(&font.data, 0)?;
    let units_per_em = (face.units_per_em() as f32).max(1.0);
    let scale = font_size / units_per_em;

    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();

    let shaped = rustybuzz::shape(&face, &[], buffer);
    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();
    if infos.len() != positions.len() {
        return None;
    }
    let clusters = infos
        .iter()
        .map(|info| usize::try_from(info.cluster).ok())
        .collect::<Option<Vec<_>>>()?;
    let cluster_unicode = glyph_cluster_unicode(text, &clusters)?;

    let mut width = 0.0;
    let mut glyphs = Vec::with_capacity(infos.len());
    for ((info, position), unicode) in infos
        .iter()
        .zip(positions.iter())
        .zip(cluster_unicode.into_iter())
    {
        let x_advance = position.x_advance as f32 * scale;
        glyphs.push(ShapedGlyph {
            glyph_id: info.glyph_id as u16,
            x_advance,
            x_offset: position.x_offset as f32 * scale,
            y_offset: position.y_offset as f32 * scale,
            unicode,
        });
        width += x_advance;
    }

    Some(ShapedRun { glyphs, width })
}

fn glyph_cluster_unicode(text: &str, clusters: &[usize]) -> Option<Vec<Vec<u16>>> {
    let mut cluster_starts = clusters.to_vec();
    cluster_starts.push(text.len());
    cluster_starts.sort_unstable();
    cluster_starts.dedup();

    let mut cluster_text = HashMap::with_capacity(cluster_starts.len());
    for window in cluster_starts.windows(2) {
        let start = window[0];
        let end = window[1];
        let slice = text.get(start..end)?;
        cluster_text.insert(start, slice.encode_utf16().collect());
    }

    let mut seen_clusters = HashSet::with_capacity(clusters.len());
    clusters
        .iter()
        .map(|cluster| {
            if seen_clusters.insert(*cluster) {
                cluster_text.get(cluster).cloned()
            } else {
                Some(Vec::new())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ShapedGlyph, ShapedRun, custom_font_line_height, glyph_cluster_unicode, measure_text_width,
        resolve_custom_font, shape_text_run, shape_text_with_font,
    };
    use crate::layout::engine::TextRun;
    use crate::style::computed::FontFamily;
    use std::collections::HashMap;

    #[test]
    fn glyph_cluster_unicode_emits_cluster_text_once_per_cluster() {
        let unicode = glyph_cluster_unicode("fi", &[0, 0]).unwrap();
        assert_eq!(unicode, vec![vec![0x0066, 0x0069], Vec::new()]);
    }

    #[test]
    fn glyph_cluster_unicode_handles_reordered_clusters() {
        let unicode = glyph_cluster_unicode("ab", &[1, 0]).unwrap();
        assert_eq!(unicode, vec![vec![0x0062], vec![0x0061]]);
    }

    // --- shape_text_with_font ---

    // shape_text_with_font is private; we need a real TtfFont to call it with a
    // non-empty string.  For the empty-string branch we can verify the fast path
    // without any font data by constructing a minimal stub.
    fn make_stub_font() -> crate::parser::ttf::TtfFont {
        use crate::parser::ttf::{FontVerticalMetrics, TtfFont};
        TtfFont {
            font_name: "Stub".into(),
            units_per_em: 1000,
            bbox: [0, 0, 0, 0],
            pdf_metrics: FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: FontVerticalMetrics::new(800, -200, 0),
            cmap: HashMap::new(),
            glyph_widths: Vec::new(),
            num_h_metrics: 0,
            flags: 0,
            data: std::sync::Arc::new(Vec::new()),
        }
    }

    #[test]
    fn shape_text_with_font_empty_string_returns_zero_width() {
        let font = make_stub_font();
        let run = shape_text_with_font("", 12.0, &font).unwrap();
        assert_eq!(run.width, 0.0);
        assert!(run.glyphs.is_empty());
    }

    // --- resolve_custom_font ---

    #[test]
    fn resolve_custom_font_returns_none_for_helvetica() {
        let fonts = HashMap::new();
        assert!(resolve_custom_font(&FontFamily::Helvetica, false, false, &fonts).is_none());
    }

    #[test]
    fn resolve_custom_font_returns_none_for_times_roman() {
        let fonts = HashMap::new();
        assert!(resolve_custom_font(&FontFamily::TimesRoman, false, false, &fonts).is_none());
    }

    #[test]
    fn resolve_custom_font_returns_none_for_courier() {
        let fonts = HashMap::new();
        assert!(resolve_custom_font(&FontFamily::Courier, false, false, &fonts).is_none());
    }

    #[test]
    fn resolve_custom_font_returns_none_when_custom_font_not_in_map() {
        let fonts = HashMap::new();
        let family = FontFamily::Custom("MyFont".into());
        assert!(resolve_custom_font(&family, false, false, &fonts).is_none());
    }

    // --- measure_text_width ---

    #[test]
    fn measure_text_width_returns_none_for_standard_font() {
        let fonts = HashMap::new();
        let result =
            measure_text_width("hello", 12.0, &FontFamily::Helvetica, false, false, &fonts);
        assert!(result.is_none());
    }

    #[test]
    fn measure_text_width_returns_none_when_custom_font_not_found() {
        let fonts = HashMap::new();
        let family = FontFamily::Custom("Missing".into());
        let result = measure_text_width("hello", 12.0, &family, false, false, &fonts);
        assert!(result.is_none());
    }

    // --- custom_font_line_height ---

    #[test]
    fn custom_font_line_height_returns_none_for_helvetica() {
        let fonts = HashMap::new();
        assert!(custom_font_line_height(&FontFamily::Helvetica, false, false, &fonts).is_none());
    }

    #[test]
    fn custom_font_line_height_returns_none_for_times_roman() {
        let fonts = HashMap::new();
        assert!(custom_font_line_height(&FontFamily::TimesRoman, false, false, &fonts).is_none());
    }

    #[test]
    fn custom_font_line_height_returns_none_for_courier() {
        let fonts = HashMap::new();
        assert!(custom_font_line_height(&FontFamily::Courier, false, false, &fonts).is_none());
    }

    #[test]
    fn custom_font_line_height_returns_none_when_custom_font_not_found() {
        let fonts = HashMap::new();
        let family = FontFamily::Custom("Ghost".into());
        assert!(custom_font_line_height(&family, false, false, &fonts).is_none());
    }

    // -----------------------------------------------------------------------
    // ShapedGlyph / ShapedRun – struct field access, Clone, Debug
    // -----------------------------------------------------------------------

    #[test]
    fn shaped_glyph_fields_and_clone() {
        let g = ShapedGlyph {
            glyph_id: 42,
            x_advance: 10.5,
            x_offset: 1.0,
            y_offset: -2.0,
            unicode: vec![0x0041],
        };
        let g2 = g.clone();
        assert_eq!(g2.glyph_id, 42);
        assert_eq!(g2.x_advance, 10.5);
        assert_eq!(g2.x_offset, 1.0);
        assert_eq!(g2.y_offset, -2.0);
        assert_eq!(g2.unicode, vec![0x0041u16]);
        // Debug must not panic
        let _ = format!("{:?}", g);
    }

    #[test]
    fn shaped_run_fields_and_clone() {
        let run = ShapedRun {
            glyphs: vec![ShapedGlyph {
                glyph_id: 1,
                x_advance: 5.0,
                x_offset: 0.0,
                y_offset: 0.0,
                unicode: vec![0x0061],
            }],
            width: 5.0,
        };
        let run2 = run.clone();
        assert_eq!(run2.width, 5.0);
        assert_eq!(run2.glyphs.len(), 1);
        assert_eq!(run2.glyphs[0].glyph_id, 1);
        let _ = format!("{:?}", run);
    }

    // -----------------------------------------------------------------------
    // shape_text_run – None when font is missing from map
    // -----------------------------------------------------------------------

    #[test]
    fn shape_text_run_returns_none_when_font_not_found() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "hello".into(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Custom("Missing".into()),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        assert!(shape_text_run(&run, &fonts).is_none());
    }

    #[test]
    fn shape_text_run_returns_none_for_standard_font_family() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "hello".into(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Helvetica,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        assert!(shape_text_run(&run, &fonts).is_none());
    }

    // -----------------------------------------------------------------------
    // shape_text_with_font – returns None when font.data is not a valid face
    // -----------------------------------------------------------------------

    #[test]
    fn shape_text_with_font_returns_none_for_invalid_font_data() {
        let font = make_stub_font(); // data is Vec::new(), rustybuzz can't parse it
        assert!(shape_text_with_font("hello", 12.0, &font).is_none());
    }

    // -----------------------------------------------------------------------
    // Helper to load a real system font so we can test the shaping hot path.
    // The font path is macOS-specific; the tests are gated accordingly.
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    fn load_real_font() -> Option<crate::parser::ttf::TtfFont> {
        let data = std::fs::read("/System/Library/Fonts/Geneva.ttf").ok()?;
        crate::parser::ttf::parse_ttf(data).ok()
    }

    #[cfg(target_os = "macos")]
    fn make_real_font_map() -> HashMap<String, crate::parser::ttf::TtfFont> {
        let font = match load_real_font() {
            Some(f) => f,
            None => return HashMap::new(),
        };
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::font_variant_key("Geneva", false, false),
            font,
        );
        fonts
    }

    // -----------------------------------------------------------------------
    // shape_text_with_font – full shaping path with a real font
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn shape_text_with_font_shapes_ascii_text_with_real_font() {
        let font = match load_real_font() {
            Some(f) => f,
            None => return, // font not available on this machine, skip
        };
        let result = shape_text_with_font("Hi", 12.0, &font);
        let run = result.expect("shaping should succeed with a real font");
        assert_eq!(run.glyphs.len(), 2, "two glyphs for two-character input");
        assert!(run.width > 0.0, "shaped width must be positive");
        // Each glyph should carry the right character
        assert!(!run.glyphs[0].unicode.is_empty());
        assert!(!run.glyphs[1].unicode.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shape_text_with_font_glyph_fields_are_populated() {
        let font = match load_real_font() {
            Some(f) => f,
            None => return,
        };
        let run = shape_text_with_font("A", 10.0, &font).unwrap();
        assert_eq!(run.glyphs.len(), 1);
        let g = &run.glyphs[0];
        // x_advance should be a non-negative scaled value for a normal glyph
        assert!(g.x_advance >= 0.0);
        assert_eq!(run.width, g.x_advance);
    }

    // -----------------------------------------------------------------------
    // shape_text_run – full path with a real font
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn shape_text_run_returns_some_when_font_found() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return; // font not available, skip
        }
        let run = TextRun {
            text: "Hi".into(),
            font_size: 14.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Custom("Geneva".into()),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        let result = shape_text_run(&run, &fonts);
        let shaped = result.expect("shape_text_run must return Some for a found font");
        assert_eq!(shaped.glyphs.len(), 2);
        assert!(shaped.width > 0.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shape_text_run_empty_text_returns_zero_width_run() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return;
        }
        let run = TextRun {
            text: String::new(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: (0.0, 0.0, 0.0),
            link_url: None,
            font_family: FontFamily::Custom("Geneva".into()),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
        };
        let shaped = shape_text_run(&run, &fonts).expect("empty text still returns Some");
        assert_eq!(shaped.width, 0.0);
        assert!(shaped.glyphs.is_empty());
    }

    // -----------------------------------------------------------------------
    // measure_text_width – returns Some when font is present
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn measure_text_width_returns_some_when_font_found() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return;
        }
        let family = FontFamily::Custom("Geneva".into());
        let result = measure_text_width("hello", 12.0, &family, false, false, &fonts);
        let width = result.expect("must return Some for a found custom font");
        assert!(width > 0.0, "width of non-empty text must be positive");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn measure_text_width_empty_string_returns_zero() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return;
        }
        let family = FontFamily::Custom("Geneva".into());
        let result = measure_text_width("", 12.0, &family, false, false, &fonts);
        assert_eq!(result, Some(0.0));
    }

    // -----------------------------------------------------------------------
    // custom_font_line_height – returns Some when font is present
    // -----------------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn custom_font_line_height_returns_some_when_font_found() {
        let fonts = make_real_font_map();
        if fonts.is_empty() {
            return;
        }
        let family = FontFamily::Custom("Geneva".into());
        let result = custom_font_line_height(&family, false, false, &fonts);
        let ratio = result.expect("must return Some for a found custom font");
        // line_height_ratio is clamped to at least 1.0
        assert!(
            ratio >= 1.0,
            "line height ratio must be >= 1.0, got {}",
            ratio
        );
    }
}
