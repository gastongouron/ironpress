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
    let face = rustybuzz::Face::from_slice(&font.data, 0)?;
    let units_per_em = (face.units_per_em() as f32).max(1.0);
    let height = f32::from(face.height()).abs() / units_per_em;
    Some(height.max(1.0))
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
    use super::glyph_cluster_unicode;

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
}
