use super::*;
use resvg::tiny_skia;

impl PdfWriter {
    pub(super) fn add_type3_font(
        &mut self,
        name: &str,
        ttf: &TtfFont,
        prepared_font: &PreparedCustomFont,
    ) -> String {
        let resource_name = sanitize_pdf_name(name);
        let mut char_procs = Vec::with_capacity(prepared_font.type3_glyphs().len());

        for glyph in prepared_font.type3_glyphs() {
            let object_id = self.next_id();
            let content = type3_char_proc(ttf, glyph.glyph_id, prepared_font.type3_glyph_style());
            self.add_type3_char_proc(object_id, &content);
            char_procs.push((glyph.code, object_id));
        }

        let first_char = char_procs.first().map_or(0, |(code, _)| *code);
        let last_char = char_procs.last().map_or(0, |(code, _)| *code);
        let widths = prepared_font
            .widths
            .iter()
            .map(|width| format_pdf_number(*width))
            .collect::<Vec<_>>()
            .join(" ");
        let char_proc_entries = char_procs
            .iter()
            .map(|(code, object_id)| format!("/g{code} {object_id} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        let differences = char_procs
            .iter()
            .map(|(code, _)| format!("/g{code}"))
            .collect::<Vec<_>>()
            .join(" ");
        let to_unicode_id = self.next_id();
        let to_unicode = build_single_byte_tounicode_cmap(&prepared_font.to_unicode_map);
        self.objects.push(format!(
            "{to_unicode_id} 0 obj\n<< /Length {} >>\nstream\n{to_unicode}endstream\nendobj",
            to_unicode.len(),
        ));

        let font_id = self.next_id();
        let font_matrix_scale = 1.0 / ttf.units_per_em.max(1) as f32;
        let [left, bottom, right, top] = type3_font_bbox(ttf, prepared_font.type3_glyph_style());
        self.objects.push(format!(
            "{font_id} 0 obj\n<< /Type /Font /Subtype /Type3 /Name /{base_font_name} /FontBBox [{left} {bottom} {right} {top}] /FontMatrix [{font_matrix_scale} 0 0 -{font_matrix_scale} 0 0] /CharProcs << {char_proc_entries} >> /Encoding << /Type /Encoding /Differences [{first_char} {differences}] >> /FirstChar {first_char} /LastChar {last_char} /Widths [{widths}] /Resources << >> /ToUnicode {to_unicode_id} 0 R >>\nendobj",
            base_font_name = prepared_font.base_font_name,
            font_matrix_scale = format_pdf_number(font_matrix_scale),
        ));

        self.custom_font_entries.push(CustomFontEntry {
            resource_name: resource_name.clone(),
            font_obj_id: font_id,
        });
        resource_name
    }

    fn add_type3_char_proc(&mut self, object_id: usize, content: &str) {
        if let Some(compressed) = flate_compress(content.as_bytes()) {
            self.objects.push(format!(
                "{object_id} 0 obj\n<< /Length {} /Filter /FlateDecode >>\nstream\n",
                compressed.len(),
            ));
            self.binary_objects.insert(object_id, compressed);
        } else {
            self.objects.push(format!(
                "{object_id} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj",
                content.len(),
            ));
        }
    }
}

fn type3_char_proc(ttf: &TtfFont, glyph_id: u16, glyph_style: Type3GlyphStyle) -> String {
    let width = ttf.glyph_width(glyph_id);
    let Some(face) = ttf.program.shaping_face() else {
        let [left, bottom, right, top] = type3_font_bbox(ttf, glyph_style);
        return format!("{width} 0 {left} {bottom} {right} {top} d1\n");
    };

    let glyph = rustybuzz::ttf_parser::GlyphId(glyph_id);
    let Some(source) = glyph_path(face, glyph) else {
        return format!("{width} 0 d0\n");
    };
    let bounds = face.glyph_bounding_box(glyph).map_or(ttf.bbox, |bbox| {
        [bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max]
    });
    let [left, bottom, right, top] = expanded_type3_bbox(bounds, ttf, glyph_style);
    let mut content = format!("{width} 0 {left} {bottom} {right} {top} d1\n");
    if glyph_style == Type3GlyphStyle::SyntheticWeight
        && let Some(expanded) = synthetic_weight_glyph_path(&source, ttf.units_per_em)
    {
        content.push_str(&expanded);
    } else {
        append_tiny_skia_path(&mut content, &source);
    }
    content.push_str("f\n");
    content
}

const SYNTHETIC_WEIGHT_EM_RATIO: f32 = 0.03125;

fn type3_font_bbox(ttf: &TtfFont, glyph_style: Type3GlyphStyle) -> [i16; 4] {
    expanded_type3_bbox(ttf.bbox, ttf, glyph_style)
}

fn expanded_type3_bbox(bounds: [i16; 4], ttf: &TtfFont, glyph_style: Type3GlyphStyle) -> [i16; 4] {
    let padding = if glyph_style == Type3GlyphStyle::SyntheticWeight {
        (f32::from(ttf.units_per_em) * SYNTHETIC_WEIGHT_EM_RATIO / 2.0).ceil() as i16
    } else {
        0
    };
    flip_type3_bbox([
        bounds[0].saturating_sub(padding),
        bounds[1].saturating_sub(padding),
        bounds[2].saturating_add(padding),
        bounds[3].saturating_add(padding),
    ])
}

fn synthetic_weight_glyph_path(source: &tiny_skia::Path, units_per_em: u16) -> Option<String> {
    let stroke = tiny_skia::Stroke {
        width: f32::from(units_per_em) * SYNTHETIC_WEIGHT_EM_RATIO,
        line_cap: tiny_skia::LineCap::Butt,
        line_join: tiny_skia::LineJoin::Miter,
        ..Default::default()
    };
    let expanded = source.stroke(&stroke, 1.0)?;
    let mut content = String::new();
    append_tiny_skia_path(&mut content, &expanded);
    append_tiny_skia_path(&mut content, source);
    Some(content)
}

fn glyph_path(
    face: &rustybuzz::ttf_parser::Face<'_>,
    glyph: rustybuzz::ttf_parser::GlyphId,
) -> Option<tiny_skia::Path> {
    let mut builder = TinySkiaGlyphPath::default();
    face.outline_glyph(glyph, &mut builder)?;
    builder.finish()
}

#[derive(Default)]
struct TinySkiaGlyphPath {
    builder: tiny_skia::PathBuilder,
}

impl TinySkiaGlyphPath {
    fn finish(self) -> Option<tiny_skia::Path> {
        self.builder.finish()
    }
}

impl rustybuzz::ttf_parser::OutlineBuilder for TinySkiaGlyphPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(x, -y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(x, -y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quad_to(x1, -y1, x, -y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_to(x1, -y1, x2, -y2, x, -y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

fn append_tiny_skia_path(content: &mut String, path: &tiny_skia::Path) {
    let mut current = None;
    for segment in path.segments() {
        match segment {
            tiny_skia::PathSegment::MoveTo(point) => {
                content.push_str(&format!(
                    "{} {} m\n",
                    format_pdf_number(point.x),
                    format_pdf_number(point.y)
                ));
                current = Some(point);
            }
            tiny_skia::PathSegment::LineTo(point) => {
                content.push_str(&format!(
                    "{} {} l\n",
                    format_pdf_number(point.x),
                    format_pdf_number(point.y)
                ));
                current = Some(point);
            }
            tiny_skia::PathSegment::QuadTo(control, point) => {
                let Some(start) = current else {
                    content.push_str(&format!(
                        "{} {} m\n",
                        format_pdf_number(point.x),
                        format_pdf_number(point.y)
                    ));
                    current = Some(point);
                    continue;
                };
                let control_1 = tiny_skia::Point::from_xy(
                    start.x + (control.x - start.x) * (2.0 / 3.0),
                    start.y + (control.y - start.y) * (2.0 / 3.0),
                );
                let control_2 = tiny_skia::Point::from_xy(
                    point.x + (control.x - point.x) * (2.0 / 3.0),
                    point.y + (control.y - point.y) * (2.0 / 3.0),
                );
                content.push_str(&format!(
                    "{} {} {} {} {} {} c\n",
                    format_pdf_number(control_1.x),
                    format_pdf_number(control_1.y),
                    format_pdf_number(control_2.x),
                    format_pdf_number(control_2.y),
                    format_pdf_number(point.x),
                    format_pdf_number(point.y)
                ));
                current = Some(point);
            }
            tiny_skia::PathSegment::CubicTo(control_1, control_2, point) => {
                content.push_str(&format!(
                    "{} {} {} {} {} {} c\n",
                    format_pdf_number(control_1.x),
                    format_pdf_number(control_1.y),
                    format_pdf_number(control_2.x),
                    format_pdf_number(control_2.y),
                    format_pdf_number(point.x),
                    format_pdf_number(point.y)
                ));
                current = Some(point);
            }
            tiny_skia::PathSegment::Close => content.push_str("h\n"),
        }
    }
}

/// Chrome's Type 3 CFF fallback uses its usual top-down glyph coordinate
/// system. PDF's text matrix is then responsible for restoring the page's
/// upward Y axis, so every glyph-space Y coordinate is negated consistently.
fn flip_type3_bbox([left, bottom, right, top]: [i16; 4]) -> [i16; 4] {
    [left, top.saturating_neg(), right, bottom.saturating_neg()]
}

fn build_single_byte_tounicode_cmap(mappings: &[(u16, Vec<u16>)]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
12 dict begin\n\
begincmap\n\
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
/CMapName /Adobe-Identity-UCS def\n\
/CMapType 2 def\n\
1 begincodespacerange\n\
<00> <FF>\n\
endcodespacerange\n",
    );

    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (code, unicode) in chunk {
            let unicode_hex = unicode
                .iter()
                .map(|code_unit| format!("{code_unit:04X}"))
                .collect::<String>();
            cmap.push_str(&format!("<{code:02X}> <{unicode_hex}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }

    cmap.push_str(
        "endcmap\n\
CMapName currentdict /CMap defineresource pop\n\
end\n\
end\n",
    );
    cmap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type3_coordinates_use_chromes_negative_y_convention() {
        let mut path = TinySkiaGlyphPath::default();
        rustybuzz::ttf_parser::OutlineBuilder::move_to(&mut path, 2.0, 3.0);
        rustybuzz::ttf_parser::OutlineBuilder::line_to(&mut path, 4.0, -5.0);
        let path = path.finish().expect("two-point path");
        let mut content = String::new();
        append_tiny_skia_path(&mut content, &path);

        assert_eq!(content, "2 -3 m\n4 5 l\n");
        assert_eq!(flip_type3_bbox([-2, i16::MIN, 3, 4]), [-2, -4, 3, i16::MAX]);
    }

    #[test]
    fn single_byte_tounicode_keeps_code_and_utf16_mapping() {
        let cmap =
            build_single_byte_tounicode_cmap(&[(1, vec![0x4e8b]), (255, vec![0xd83d, 0xde00])]);

        assert!(cmap.contains("<01> <4E8B>"));
        assert!(cmap.contains("<FF> <D83DDE00>"));
    }

    #[test]
    fn synthetic_weight_is_a_filled_expanded_type3_outline() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans font");
        let glyph_id = *font.cmap.get(&('B' as u32)).expect("B glyph");

        let plain = type3_char_proc(&font, glyph_id, Type3GlyphStyle::Plain);
        let synthetic = type3_char_proc(&font, glyph_id, Type3GlyphStyle::SyntheticWeight);

        assert!(synthetic.len() > plain.len());
        assert!(synthetic.matches(" m\n").count() > plain.matches(" m\n").count());
        assert!(!synthetic.contains(" w\n"));
        assert!(synthetic.ends_with("f\n"));
    }

    #[test]
    fn outline_less_glyph_uses_width_only_type3_metrics() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans font");
        let glyph_id = *font.cmap.get(&(' ' as u32)).expect("space glyph");

        let char_proc = type3_char_proc(&font, glyph_id, Type3GlyphStyle::SyntheticWeight);

        assert!(char_proc.ends_with("0 d0\n"), "{char_proc}");
        assert!(!char_proc.contains(" d1\n"), "{char_proc}");
    }
}
