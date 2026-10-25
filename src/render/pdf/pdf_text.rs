/// Convert a UTF-8 string to WinAnsi (Windows-1252) encoded bytes.
///
/// Standard PDF fonts (Helvetica, Times-Roman, Courier) use WinAnsi encoding,
/// not UTF-8. Writing raw UTF-8 bytes causes multi-byte characters like em dash
/// to appear as mojibake. This function maps Unicode code points to their
/// WinAnsi byte equivalents.
pub(crate) fn utf8_to_winansi(text: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        match code {
            // ASCII range maps directly
            0x0000..=0x007F => result.push(code as u8),
            // Non-breaking space
            0x00A0 => result.push(0xA0),
            // Latin-1 supplement U+00A1..U+00FF map directly
            0x00A1..=0x00FF => result.push(code as u8),
            // WinAnsi special mappings from the Windows-1252 range 0x80..0x9F
            0x20AC => result.push(0x80), // Euro sign
            0x201A => result.push(0x82), // Single low-9 quotation mark
            0x0192 => result.push(0x83), // Latin small letter f with hook
            0x201E => result.push(0x84), // Double low-9 quotation mark
            0x2026 => result.push(0x85), // Horizontal ellipsis
            0x2020 => result.push(0x86), // Dagger
            0x2021 => result.push(0x87), // Double dagger
            0x02C6 => result.push(0x88), // Modifier letter circumflex accent
            0x2030 => result.push(0x89), // Per mille sign
            0x0160 => result.push(0x8A), // Latin capital letter S with caron
            0x2039 => result.push(0x8B), // Single left-pointing angle quotation mark
            0x0152 => result.push(0x8C), // Latin capital ligature OE
            0x017D => result.push(0x8E), // Latin capital letter Z with caron
            0x2018 => result.push(0x91), // Left single quotation mark
            0x2019 => result.push(0x92), // Right single quotation mark
            0x201C => result.push(0x93), // Left double quotation mark
            0x201D => result.push(0x94), // Right double quotation mark
            0x2022 => result.push(0x95), // Bullet
            0x2013 => result.push(0x96), // En dash
            0x2014 => result.push(0x97), // Em dash
            0x02DC => result.push(0x98), // Small tilde
            0x2122 => result.push(0x99), // Trade mark sign
            0x0161 => result.push(0x9A), // Latin small letter s with caron
            0x203A => result.push(0x9B), // Single right-pointing angle quotation mark
            0x0153 => result.push(0x9C), // Latin small ligature oe
            0x017E => result.push(0x9E), // Latin small letter z with caron
            0x0178 => result.push(0x9F), // Latin capital letter Y with diaeresis
            // Anything else is not representable in WinAnsi — replace with '?'
            _ => result.push(b'?'),
        }
    }
    result
}

/// Returns `true` if every character in `text` can be encoded in WinAnsiEncoding.
///
/// Characters outside this range (CJK, Arabic, Hebrew, emoji, box-drawing, etc.)
/// cannot be rendered by the standard PDF fonts and require a Unicode-capable
/// embedded font instead.
pub(crate) fn is_winansi_encodable(text: &str) -> bool {
    text.chars().all(is_winansi_char)
}

/// Check whether a single character is representable in WinAnsiEncoding.
pub(crate) fn is_winansi_char(ch: char) -> bool {
    let code = ch as u32;
    matches!(code,
        0x0000..=0x007F |
        0x00A0..=0x00FF |
        0x20AC | 0x201A | 0x0192 | 0x201E | 0x2026 |
        0x2020 | 0x2021 | 0x02C6 | 0x2030 | 0x0160 |
        0x2039 | 0x0152 | 0x017D | 0x2018 | 0x2019 |
        0x201C | 0x201D | 0x2022 | 0x2013 | 0x2014 |
        0x02DC | 0x2122 | 0x0161 | 0x203A | 0x0153 |
        0x017E | 0x0178
    )
}

/// Encode a UTF-8 string for use in a PDF text operator (Tj).
///
/// Converts to WinAnsi encoding, then produces a `String` where:
/// - ASCII printable bytes (0x20..=0x7E), except `\`, `(`, `)`, are kept as-is
/// - `\`, `(`, `)` are escaped as `\\`, `\(`, `\)`
/// - All other bytes (0x00..=0x1F, 0x7F..=0xFF) are written as octal escapes `\NNN`
///
/// The returned string is safe to embed in a PDF content stream as `(encoded) Tj`.
pub(crate) fn encode_pdf_text(text: &str) -> String {
    let winansi = utf8_to_winansi(text);
    let mut result = String::with_capacity(winansi.len() * 2);
    for &b in &winansi {
        match b {
            b'\\' => result.push_str("\\\\"),
            b'(' => result.push_str("\\("),
            b')' => result.push_str("\\)"),
            0x20..=0x7E => result.push(b as char),
            _ => {
                // Octal escape: \NNN (3-digit, zero-padded)
                result.push_str(&format!("\\{b:03o}"));
            }
        }
    }
    result
}

/// A PDF text string, delimiters included: a literal `(...)` when the text is
/// printable ASCII, otherwise UTF-16BE with a byte-order mark as `<FEFF...>`
/// (PDF 32000-1 §7.9.2.2), since raw UTF-8 bytes are read as PDFDocEncoding.
pub(super) fn pdf_text_string(s: &str) -> String {
    if s.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        return format!("({})", escape_pdf_string(s));
    }
    let mut hex = String::from("<FEFF");
    for unit in s.encode_utf16() {
        hex.push_str(&format!("{unit:04X}"));
    }
    hex.push('>');
    hex
}

pub(super) fn escape_pdf_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

pub(super) fn build_tounicode_cmap(mappings: &[(u16, Vec<u16>)]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
12 dict begin\n\
begincmap\n\
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
/CMapName /Adobe-Identity-UCS def\n\
/CMapType 2 def\n\
1 begincodespacerange\n\
<0000> <FFFF>\n\
endcodespacerange\n",
    );

    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (glyph_id, unicode) in chunk {
            let unicode_hex: String = unicode
                .iter()
                .map(|code_unit| format!("{code_unit:04X}"))
                .collect();
            cmap.push_str(&format!("<{glyph_id:04X}> <{unicode_hex}>\n"));
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
