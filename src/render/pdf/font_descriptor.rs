//! Font descriptors (ISO 32000-1 9.8).

use crate::parser::ttf::TtfFont;

/// The name, metrics and embedded program of a font (ISO 32000-1 Table 122),
/// in thousandths of the em.
struct FontProgram<'a> {
    font_name: &'a str,
    bbox: [i32; 4],
    ascent: i32,
    descent: i32,
    cap_height: i32,
    font_file_key: &'static str,
    stream_id: usize,
}

/// A font descriptor dictionary. An embedded program states its name and
/// metrics; a Type 3 font, whose glyphs are content streams, states only its
/// classification, the entries Table 122 requires of it.
pub(super) struct FontDescriptor<'a> {
    flags: u32,
    program: Option<FontProgram<'a>>,
}

impl<'a> FontDescriptor<'a> {
    /// The descriptor of an embedded font program, `stream_id` being its
    /// `FontFile2` or `FontFile3` stream.
    pub(super) fn embedded(
        ttf: &TtfFont,
        font_name: &'a str,
        font_file_key: &'static str,
        stream_id: usize,
    ) -> Self {
        let metrics = ttf.pdf_vertical_metrics();
        let thousandths = |units: i16| (units as i32 * 1000) / ttf.units_per_em as i32;
        let ascent = thousandths(metrics.ascent);
        Self {
            flags: ttf.flags,
            program: Some(FontProgram {
                font_name,
                bbox: ttf.bbox.map(thousandths),
                ascent,
                descent: thousandths(metrics.descent),
                // The cap height follows the ascent, as it always has.
                cap_height: ascent,
                font_file_key,
                stream_id,
            }),
        }
    }

    /// The descriptor of a Type 3 font, required in Tagged PDF and absent
    /// otherwise (ISO 32000-1 Table 112).
    pub(super) fn type3(ttf: &TtfFont) -> Self {
        Self {
            flags: ttf.flags,
            program: None,
        }
    }

    /// The indirect object numbered `id`.
    pub(super) fn object(&self, id: usize) -> String {
        let mut entries = String::new();
        if let Some(program) = &self.program {
            entries.push_str(&format!(" /FontName /{}", program.font_name));
        }
        entries.push_str(&format!(" /Flags {}", self.flags));
        if let Some(program) = &self.program {
            entries.push_str(&format!(
                " /FontBBox [{} {} {} {}] /Ascent {} /Descent {}",
                program.bbox[0],
                program.bbox[1],
                program.bbox[2],
                program.bbox[3],
                program.ascent,
                program.descent
            ));
        }
        entries.push_str(" /ItalicAngle 0");
        if let Some(program) = &self.program {
            entries.push_str(&format!(" /CapHeight {}", program.cap_height));
        }
        entries.push_str(" /StemV 80");
        if let Some(program) = &self.program {
            entries.push_str(&format!(
                " /{} {} 0 R",
                program.font_file_key, program.stream_id
            ));
        }
        format!("{id} 0 obj\n<< /Type /FontDescriptor{entries} >>\nendobj")
    }
}
