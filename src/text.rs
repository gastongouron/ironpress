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
        custom_font_line_height, glyph_cluster_unicode, measure_text_width, resolve_custom_font,
        shape_text_with_font,
    };
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
            data: Vec::new(),
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
}
