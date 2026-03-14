//! Minimal TrueType font parser.
//!
//! Extracts metrics needed for PDF embedding: font name, character widths,
//! units per em, cmap, bounding box, ascent, and descent.

use std::collections::HashMap;

/// Parsed TrueType font data.
#[derive(Debug, Clone)]
pub struct TtfFont {
    /// Font family name from the `name` table.
    pub font_name: String,
    /// Units per em from the `head` table.
    pub units_per_em: u16,
    /// Font bounding box [xMin, yMin, xMax, yMax] from the `head` table.
    pub bbox: [i16; 4],
    /// Ascent from the `OS/2` or `hhea` table.
    pub ascent: i16,
    /// Descent from the `OS/2` or `hhea` table.
    pub descent: i16,
    /// Character code to glyph ID mapping from the `cmap` table.
    pub cmap: HashMap<u16, u16>,
    /// Advance width for each glyph ID from the `hmtx` table.
    pub glyph_widths: Vec<u16>,
    /// Number of horizontal metrics entries (from `hhea`).
    pub num_h_metrics: u16,
    /// Font flags for the PDF FontDescriptor.
    pub flags: u32,
    /// Raw TTF data for embedding.
    pub data: Vec<u8>,
}

impl TtfFont {
    /// Get the advance width for a character code, in font units.
    pub fn char_width(&self, ch: u16) -> u16 {
        let glyph_id = self.cmap.get(&ch).copied().unwrap_or(0);
        if (glyph_id as usize) < self.glyph_widths.len() {
            self.glyph_widths[glyph_id as usize]
        } else if !self.glyph_widths.is_empty() {
            // Use last long metric for glyphs beyond num_h_metrics
            self.glyph_widths[self.glyph_widths.len() - 1]
        } else {
            0
        }
    }

    /// Get the advance width for a character in PDF units (1/1000 of text space).
    pub fn char_width_pdf(&self, ch: u16) -> u16 {
        let w = self.char_width(ch) as u32;
        ((w * 1000) / self.units_per_em as u32) as u16
    }

    /// Get the advance width scaled to a given font size in points.
    pub fn char_width_scaled(&self, ch: u16, font_size: f32) -> f32 {
        let w = self.char_width(ch) as f32;
        w * font_size / self.units_per_em as f32
    }
}

/// Table directory entry.
#[derive(Debug)]
struct TableRecord {
    offset: u32,
    #[allow(dead_code)]
    length: u32,
}

/// Parse a TrueType font from raw TTF data.
pub fn parse_ttf(data: Vec<u8>) -> Result<TtfFont, String> {
    if data.len() < 12 {
        return Err("TTF data too short for offset table".to_string());
    }

    let num_tables = read_u16(&data, 4);
    if data.len() < 12 + num_tables as usize * 16 {
        return Err("TTF data too short for table directory".to_string());
    }

    // Parse table directory
    let mut tables: HashMap<[u8; 4], TableRecord> = HashMap::new();
    for i in 0..num_tables as usize {
        let offset = 12 + i * 16;
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&data[offset..offset + 4]);
        tables.insert(
            tag,
            TableRecord {
                offset: read_u32(&data, offset + 8),
                length: read_u32(&data, offset + 12),
            },
        );
    }

    // Parse head table
    let head = tables.get(b"head").ok_or("Missing head table")?;
    let head_off = head.offset as usize;
    if data.len() < head_off + 54 {
        return Err("head table too short".to_string());
    }
    let units_per_em = read_u16(&data, head_off + 18);
    let x_min = read_i16(&data, head_off + 36);
    let y_min = read_i16(&data, head_off + 38);
    let x_max = read_i16(&data, head_off + 40);
    let y_max = read_i16(&data, head_off + 42);
    let bbox = [x_min, y_min, x_max, y_max];

    // Parse hhea table
    let hhea = tables.get(b"hhea").ok_or("Missing hhea table")?;
    let hhea_off = hhea.offset as usize;
    if data.len() < hhea_off + 36 {
        return Err("hhea table too short".to_string());
    }
    let hhea_ascent = read_i16(&data, hhea_off + 4);
    let hhea_descent = read_i16(&data, hhea_off + 6);
    let num_h_metrics = read_u16(&data, hhea_off + 34);

    // Parse OS/2 table for ascent/descent if available, else use hhea
    let (ascent, descent) = if let Some(os2) = tables.get(b"OS/2") {
        let os2_off = os2.offset as usize;
        if data.len() >= os2_off + 72 {
            let s_typo_ascender = read_i16(&data, os2_off + 68);
            let s_typo_descender = read_i16(&data, os2_off + 70);
            (s_typo_ascender, s_typo_descender)
        } else {
            (hhea_ascent, hhea_descent)
        }
    } else {
        (hhea_ascent, hhea_descent)
    };

    // Parse maxp table for num_glyphs
    let maxp = tables.get(b"maxp").ok_or("Missing maxp table")?;
    let maxp_off = maxp.offset as usize;
    if data.len() < maxp_off + 6 {
        return Err("maxp table too short".to_string());
    }
    let num_glyphs = read_u16(&data, maxp_off + 4);

    // Parse hmtx table
    let hmtx = tables.get(b"hmtx").ok_or("Missing hmtx table")?;
    let hmtx_off = hmtx.offset as usize;
    let mut glyph_widths = Vec::with_capacity(num_glyphs as usize);
    let mut last_width = 0u16;
    for i in 0..num_glyphs as usize {
        if i < num_h_metrics as usize {
            let entry_off = hmtx_off + i * 4;
            if data.len() < entry_off + 2 {
                break;
            }
            last_width = read_u16(&data, entry_off);
            glyph_widths.push(last_width);
        } else {
            // Glyphs beyond num_h_metrics share the last advance width
            glyph_widths.push(last_width);
        }
    }

    // Parse cmap table
    let cmap_table = tables.get(b"cmap").ok_or("Missing cmap table")?;
    let cmap = parse_cmap(&data, cmap_table.offset as usize)?;

    // Parse name table
    let name_table = tables.get(b"name").ok_or("Missing name table")?;
    let font_name = parse_name_table(&data, name_table.offset as usize)?;

    // Compute flags: bit 5 (Nonsymbolic) = 32 for Latin text
    let flags = 32u32;

    Ok(TtfFont {
        font_name,
        units_per_em,
        bbox,
        ascent,
        descent,
        cmap,
        glyph_widths,
        num_h_metrics,
        flags,
        data,
    })
}

/// Parse the cmap table, looking for a format 4 (BMP Unicode) subtable.
fn parse_cmap(data: &[u8], offset: usize) -> Result<HashMap<u16, u16>, String> {
    if data.len() < offset + 4 {
        return Err("cmap table too short".to_string());
    }
    let num_subtables = read_u16(data, offset + 2);

    // Find a platform 3 (Windows) encoding 1 (Unicode BMP) or
    // platform 0 (Unicode) subtable
    let mut subtable_offset = None;
    for i in 0..num_subtables as usize {
        let record_off = offset + 4 + i * 8;
        if data.len() < record_off + 8 {
            break;
        }
        let platform_id = read_u16(data, record_off);
        let encoding_id = read_u16(data, record_off + 2);
        let sub_offset = read_u32(data, record_off + 4) as usize;

        if (platform_id == 3 && encoding_id == 1) || (platform_id == 0) {
            subtable_offset = Some(offset + sub_offset);
            // Prefer Windows platform
            if platform_id == 3 {
                break;
            }
        }
    }

    let sub_off = subtable_offset.ok_or("No suitable cmap subtable found")?;
    if data.len() < sub_off + 2 {
        return Err("cmap subtable too short".to_string());
    }
    let format = read_u16(data, sub_off);

    match format {
        4 => parse_cmap_format4(data, sub_off),
        0 => parse_cmap_format0(data, sub_off),
        _ => {
            // Return an empty map for unsupported formats rather than failing
            Ok(HashMap::new())
        }
    }
}

/// Parse cmap format 0 (byte encoding table).
fn parse_cmap_format0(data: &[u8], offset: usize) -> Result<HashMap<u16, u16>, String> {
    if data.len() < offset + 262 {
        return Err("cmap format 0 table too short".to_string());
    }
    let mut map = HashMap::new();
    for i in 0..256u16 {
        let glyph_id = data[offset + 6 + i as usize] as u16;
        if glyph_id != 0 {
            map.insert(i, glyph_id);
        }
    }
    Ok(map)
}

/// Parse cmap format 4 (segment mapping to delta values).
fn parse_cmap_format4(data: &[u8], offset: usize) -> Result<HashMap<u16, u16>, String> {
    if data.len() < offset + 14 {
        return Err("cmap format 4 header too short".to_string());
    }

    let seg_count_x2 = read_u16(data, offset + 6);
    let seg_count = seg_count_x2 as usize / 2;

    let end_code_off = offset + 14;
    // +2 for reserved padding after endCode array
    let start_code_off = end_code_off + seg_count * 2 + 2;
    let id_delta_off = start_code_off + seg_count * 2;
    let id_range_offset_off = id_delta_off + seg_count * 2;

    let needed = id_range_offset_off + seg_count * 2;
    if data.len() < needed {
        return Err("cmap format 4 data too short".to_string());
    }

    let mut map = HashMap::new();

    for i in 0..seg_count {
        let end_code = read_u16(data, end_code_off + i * 2);
        let start_code = read_u16(data, start_code_off + i * 2);
        let id_delta = read_i16(data, id_delta_off + i * 2) as i32;
        let id_range_offset = read_u16(data, id_range_offset_off + i * 2);

        if start_code == 0xFFFF {
            break;
        }

        for c in start_code..=end_code {
            let glyph_id = if id_range_offset == 0 {
                ((c as i32 + id_delta) & 0xFFFF) as u16
            } else {
                // idRangeOffset is relative to the current position in the
                // idRangeOffset array
                let range_off = id_range_offset_off + i * 2;
                let glyph_off =
                    range_off + id_range_offset as usize + (c as usize - start_code as usize) * 2;
                if glyph_off + 1 < data.len() {
                    let gid = read_u16(data, glyph_off);
                    if gid != 0 {
                        ((gid as i32 + id_delta) & 0xFFFF) as u16
                    } else {
                        0
                    }
                } else {
                    0
                }
            };
            if glyph_id != 0 {
                map.insert(c, glyph_id);
            }
        }
    }

    Ok(map)
}

/// Parse the name table and extract the font family name (nameID 1 or 4).
fn parse_name_table(data: &[u8], offset: usize) -> Result<String, String> {
    if data.len() < offset + 6 {
        return Err("name table too short".to_string());
    }

    let count = read_u16(data, offset + 2);
    let string_offset = read_u16(data, offset + 4) as usize;
    let storage_off = offset + string_offset;

    // Try to find nameID 4 (full name) first, then nameID 1 (family name)
    let mut best_name: Option<String> = None;
    let mut best_priority = 0u8;

    for i in 0..count as usize {
        let rec_off = offset + 6 + i * 12;
        if data.len() < rec_off + 12 {
            break;
        }
        let platform_id = read_u16(data, rec_off);
        let encoding_id = read_u16(data, rec_off + 2);
        let name_id = read_u16(data, rec_off + 6);
        let length = read_u16(data, rec_off + 8) as usize;
        let str_offset = read_u16(data, rec_off + 10) as usize;

        if name_id != 1 && name_id != 4 {
            continue;
        }

        let priority = if name_id == 4 { 2 } else { 1 };
        if priority <= best_priority {
            continue;
        }

        let start = storage_off + str_offset;
        let end = start + length;
        if end > data.len() {
            continue;
        }

        let name_bytes = &data[start..end];
        let name = if platform_id == 3 || (platform_id == 0 && encoding_id > 0) {
            // UTF-16BE
            decode_utf16be(name_bytes)
        } else {
            // Latin-1/ASCII
            String::from_utf8_lossy(name_bytes).to_string()
        };

        if !name.is_empty() {
            best_name = Some(name);
            best_priority = priority;
        }
    }

    best_name.ok_or_else(|| "No font name found in name table".to_string())
}

/// Decode a UTF-16BE byte slice to a String.
fn decode_utf16be(data: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i + 1 < data.len() {
        let code_unit = ((data[i] as u16) << 8) | data[i + 1] as u16;
        if let Some(ch) = char::from_u32(code_unit as u32) {
            result.push(ch);
        }
        i += 2;
    }
    result
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([data[offset], data[offset + 1]])
}

fn read_i16(data: &[u8], offset: usize) -> i16 {
    i16::from_be_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid TTF file for testing.
    /// Contains: head, hhea, maxp, hmtx, cmap (format 4), name tables.
    fn build_test_ttf() -> Vec<u8> {
        let mut buf = Vec::new();

        // We'll build 6 tables: head, hhea, maxp, hmtx, cmap, name
        let num_tables: u16 = 6;

        // Offset table (12 bytes)
        buf.extend_from_slice(&[0, 1, 0, 0]); // sfVersion = 1.0
        buf.extend_from_slice(&num_tables.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes()); // searchRange
        buf.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
        buf.extend_from_slice(&0u16.to_be_bytes()); // rangeShift

        // Table directory: 6 entries * 16 bytes = 96 bytes
        // Directory starts at offset 12
        // Table data starts at 12 + 96 = 108

        // We'll fill in table records after building each table's data
        let dir_start = buf.len();
        // Reserve space for directory
        buf.resize(dir_start + num_tables as usize * 16, 0);

        let _data_start = buf.len(); // = 108

        // ---- head table (54 bytes) ----
        let head_offset = buf.len();
        buf.extend_from_slice(&[0, 1, 0, 0]); // version
        buf.extend_from_slice(&[0, 0, 0, 0]); // fontRevision
        buf.extend_from_slice(&[0, 0, 0, 0]); // checksumAdjustment
        buf.extend_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]); // magicNumber
        buf.extend_from_slice(&0x000Bu16.to_be_bytes()); // flags
        buf.extend_from_slice(&1000u16.to_be_bytes()); // unitsPerEm = 1000
        buf.extend_from_slice(&[0; 8]); // created
        buf.extend_from_slice(&[0; 8]); // modified
        buf.extend_from_slice(&(-100i16).to_be_bytes()); // xMin
        buf.extend_from_slice(&(-200i16).to_be_bytes()); // yMin
        buf.extend_from_slice(&800i16.to_be_bytes()); // xMax
        buf.extend_from_slice(&900i16.to_be_bytes()); // yMax
        buf.extend_from_slice(&0u16.to_be_bytes()); // macStyle
        buf.extend_from_slice(&8u16.to_be_bytes()); // lowestRecPPEM
        buf.extend_from_slice(&2i16.to_be_bytes()); // fontDirectionHint
        buf.extend_from_slice(&1i16.to_be_bytes()); // indexToLocFormat
        buf.extend_from_slice(&0i16.to_be_bytes()); // glyphDataFormat
        let head_len = buf.len() - head_offset;

        // ---- hhea table (36 bytes) ----
        let hhea_offset = buf.len();
        buf.extend_from_slice(&[0, 1, 0, 0]); // version
        buf.extend_from_slice(&800i16.to_be_bytes()); // ascent
        buf.extend_from_slice(&(-200i16).to_be_bytes()); // descent
        buf.extend_from_slice(&0i16.to_be_bytes()); // lineGap
        buf.extend_from_slice(&700u16.to_be_bytes()); // advanceWidthMax
        buf.extend_from_slice(&0i16.to_be_bytes()); // minLeftSideBearing
        buf.extend_from_slice(&0i16.to_be_bytes()); // minRightSideBearing
        buf.extend_from_slice(&700i16.to_be_bytes()); // xMaxExtent
        buf.extend_from_slice(&1i16.to_be_bytes()); // caretSlopeRise
        buf.extend_from_slice(&0i16.to_be_bytes()); // caretSlopeRun
        buf.extend_from_slice(&0i16.to_be_bytes()); // caretOffset
        buf.extend_from_slice(&[0; 8]); // reserved
        buf.extend_from_slice(&0i16.to_be_bytes()); // metricDataFormat
        buf.extend_from_slice(&3u16.to_be_bytes()); // numOfLongHorMetrics = 3
        let hhea_len = buf.len() - hhea_offset;

        // ---- maxp table (6 bytes for version 0.5, or 32 for 1.0) ----
        let maxp_offset = buf.len();
        buf.extend_from_slice(&[0, 0, 0x50, 0]); // version 0.5 (0x00005000)
        buf.extend_from_slice(&3u16.to_be_bytes()); // numGlyphs = 3
        let maxp_len = buf.len() - maxp_offset;

        // ---- hmtx table ----
        // 3 glyphs: glyph 0 (notdef), glyph 1 (space, char 32), glyph 2 (A, char 65)
        let hmtx_offset = buf.len();
        // Glyph 0: width=500, lsb=0
        buf.extend_from_slice(&500u16.to_be_bytes());
        buf.extend_from_slice(&0i16.to_be_bytes());
        // Glyph 1: width=250, lsb=0 (space)
        buf.extend_from_slice(&250u16.to_be_bytes());
        buf.extend_from_slice(&0i16.to_be_bytes());
        // Glyph 2: width=700, lsb=0 (A)
        buf.extend_from_slice(&700u16.to_be_bytes());
        buf.extend_from_slice(&0i16.to_be_bytes());
        let hmtx_len = buf.len() - hmtx_offset;

        // ---- cmap table (format 4) ----
        let cmap_offset = buf.len();
        // cmap header
        buf.extend_from_slice(&0u16.to_be_bytes()); // version
        buf.extend_from_slice(&1u16.to_be_bytes()); // numSubtables = 1
        // Encoding record: platform 3 (Windows), encoding 1 (Unicode BMP)
        buf.extend_from_slice(&3u16.to_be_bytes()); // platformID
        buf.extend_from_slice(&1u16.to_be_bytes()); // encodingID
        buf.extend_from_slice(&12u32.to_be_bytes()); // offset to subtable (from start of cmap)

        // Format 4 subtable
        // We'll map: char 32 -> glyph 1, char 65 -> glyph 2
        // Two segments: [32..32], [65..65], plus sentinel [0xFFFF..0xFFFF]
        let seg_count = 3u16; // 2 real + 1 sentinel
        let seg_count_x2 = seg_count * 2;

        buf.extend_from_slice(&4u16.to_be_bytes()); // format = 4
        let subtable_len_pos = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes()); // length (placeholder)
        buf.extend_from_slice(&0u16.to_be_bytes()); // language
        buf.extend_from_slice(&seg_count_x2.to_be_bytes()); // segCountX2
        buf.extend_from_slice(&4u16.to_be_bytes()); // searchRange
        buf.extend_from_slice(&1u16.to_be_bytes()); // entrySelector
        buf.extend_from_slice(&2u16.to_be_bytes()); // rangeShift

        // endCode: 32, 65, 0xFFFF
        buf.extend_from_slice(&32u16.to_be_bytes());
        buf.extend_from_slice(&65u16.to_be_bytes());
        buf.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // reservedPad
        buf.extend_from_slice(&0u16.to_be_bytes());
        // startCode: 32, 65, 0xFFFF
        buf.extend_from_slice(&32u16.to_be_bytes());
        buf.extend_from_slice(&65u16.to_be_bytes());
        buf.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // idDelta: for char 32 -> glyph 1: delta = 1 - 32 = -31
        //          for char 65 -> glyph 2: delta = 2 - 65 = -63
        //          sentinel: 1
        buf.extend_from_slice(&(-31i16).to_be_bytes());
        buf.extend_from_slice(&(-63i16).to_be_bytes());
        buf.extend_from_slice(&1i16.to_be_bytes());
        // idRangeOffset: 0, 0, 0
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());

        // Fill in subtable length
        let subtable_start = cmap_offset + 12; // after cmap header + encoding record
        let subtable_len = (buf.len() - subtable_start) as u16;
        buf[subtable_len_pos] = (subtable_len >> 8) as u8;
        buf[subtable_len_pos + 1] = subtable_len as u8;

        let cmap_len = buf.len() - cmap_offset;

        // ---- name table ----
        let name_offset = buf.len();
        // name table header
        buf.extend_from_slice(&0u16.to_be_bytes()); // format
        buf.extend_from_slice(&1u16.to_be_bytes()); // count = 1

        // String storage starts after header + 1 name record = 6 + 12 = 18 bytes from name_offset
        let string_storage_offset = 18u16;
        buf.extend_from_slice(&string_storage_offset.to_be_bytes());

        // Name record: platform 1 (Mac), encoding 0, language 0, nameID 1
        let font_name_str = b"TestFont";
        buf.extend_from_slice(&1u16.to_be_bytes()); // platformID
        buf.extend_from_slice(&0u16.to_be_bytes()); // encodingID
        buf.extend_from_slice(&0u16.to_be_bytes()); // languageID
        buf.extend_from_slice(&1u16.to_be_bytes()); // nameID = 1 (family)
        buf.extend_from_slice(&(font_name_str.len() as u16).to_be_bytes()); // length
        buf.extend_from_slice(&0u16.to_be_bytes()); // offset into string storage

        // String storage
        buf.extend_from_slice(font_name_str);

        let name_len = buf.len() - name_offset;

        // Now fill in the table directory
        let tables_info: [(&[u8; 4], usize, usize); 6] = [
            (b"head", head_offset, head_len),
            (b"hhea", hhea_offset, hhea_len),
            (b"maxp", maxp_offset, maxp_len),
            (b"hmtx", hmtx_offset, hmtx_len),
            (b"cmap", cmap_offset, cmap_len),
            (b"name", name_offset, name_len),
        ];

        for (i, (tag, offset, length)) in tables_info.iter().enumerate() {
            let dir_off = dir_start + i * 16;
            buf[dir_off..dir_off + 4].copy_from_slice(*tag);
            buf[dir_off + 4..dir_off + 8].copy_from_slice(&0u32.to_be_bytes()); // checksum
            buf[dir_off + 8..dir_off + 12].copy_from_slice(&(*offset as u32).to_be_bytes());
            buf[dir_off + 12..dir_off + 16].copy_from_slice(&(*length as u32).to_be_bytes());
        }

        buf
    }

    #[test]
    fn parse_ttf_offset_table() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.units_per_em, 1000);
    }

    #[test]
    fn parse_ttf_head_bbox() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.bbox, [-100, -200, 800, 900]);
    }

    #[test]
    fn parse_ttf_hhea_ascent_descent() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // No OS/2 table, should fall back to hhea values
        assert_eq!(font.ascent, 800);
        assert_eq!(font.descent, -200);
    }

    #[test]
    fn parse_ttf_cmap_format4() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // char 32 (space) -> glyph 1
        assert_eq!(font.cmap.get(&32), Some(&1));
        // char 65 (A) -> glyph 2
        assert_eq!(font.cmap.get(&65), Some(&2));
        // unmapped char should not exist
        assert_eq!(font.cmap.get(&90), None);
    }

    #[test]
    fn parse_ttf_char_widths() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.glyph_widths.len(), 3);
        assert_eq!(font.glyph_widths[0], 500); // notdef
        assert_eq!(font.glyph_widths[1], 250); // space
        assert_eq!(font.glyph_widths[2], 700); // A
    }

    #[test]
    fn parse_ttf_char_width_lookup() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // Space (char 32 -> glyph 1 -> width 250)
        assert_eq!(font.char_width(32), 250);
        // A (char 65 -> glyph 2 -> width 700)
        assert_eq!(font.char_width(65), 700);
        // Unknown char -> glyph 0 -> width 500
        assert_eq!(font.char_width(90), 500);
    }

    #[test]
    fn parse_ttf_font_name() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.font_name, "TestFont");
    }

    #[test]
    fn parse_ttf_char_width_scaled() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // A = 700 units at 1000 upm, font_size = 12
        // width = 700 * 12 / 1000 = 8.4
        let w = font.char_width_scaled(65, 12.0);
        assert!((w - 8.4).abs() < 0.01);
    }

    #[test]
    fn parse_ttf_char_width_pdf() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        // A = 700 units at 1000 upm -> 700 * 1000 / 1000 = 700
        assert_eq!(font.char_width_pdf(65), 700);
        // Space = 250 -> 250
        assert_eq!(font.char_width_pdf(32), 250);
    }

    #[test]
    fn parse_ttf_too_short() {
        let data = vec![0; 4];
        assert!(parse_ttf(data).is_err());
    }

    #[test]
    fn parse_ttf_num_h_metrics() {
        let data = build_test_ttf();
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.num_h_metrics, 3);
    }

    // --- char_width fallback paths (lines 39, 41, 43) ---

    #[test]
    fn char_width_glyph_beyond_widths_falls_back_to_last() {
        // Line 39/41: glyph_id >= glyph_widths.len() -> use last entry
        let font = TtfFont {
            font_name: String::new(),
            units_per_em: 1000,
            bbox: [0; 4],
            ascent: 0,
            descent: 0,
            cmap: {
                let mut m = HashMap::new();
                m.insert(65, 999); // glyph 999 is beyond widths vec
                m
            },
            glyph_widths: vec![500, 700],
            num_h_metrics: 2,
            flags: 32,
            data: vec![],
        };
        assert_eq!(font.char_width(65), 700); // last width
    }

    #[test]
    fn char_width_empty_glyph_widths_returns_zero() {
        // Line 43: empty glyph_widths -> 0
        let font = TtfFont {
            font_name: String::new(),
            units_per_em: 1000,
            bbox: [0; 4],
            ascent: 0,
            descent: 0,
            cmap: HashMap::new(),
            glyph_widths: vec![],
            num_h_metrics: 0,
            flags: 32,
            data: vec![],
        };
        assert_eq!(font.char_width(65), 0);
    }

    // --- parse_ttf error paths ---

    #[test]
    fn parse_ttf_table_directory_too_short() {
        // Line 76: data too short for table directory
        // 12 bytes offset table with num_tables=1, but no room for 16-byte table record
        let mut data = vec![0u8; 12];
        data[4] = 0;
        data[5] = 1; // num_tables = 1
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("too short for table directory"));
    }

    /// Helper: build a TTF with a given set of tables at specified sizes.
    /// Each table entry is (tag, data_bytes). Constructs offset table + directory + data.
    fn build_ttf_with_tables(tables: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let num_tables = tables.len() as u16;
        let mut buf = Vec::new();

        // Offset table (12 bytes)
        buf.extend_from_slice(&[0, 1, 0, 0]);
        buf.extend_from_slice(&num_tables.to_be_bytes());
        buf.extend_from_slice(&[0; 6]); // searchRange, entrySelector, rangeShift

        let dir_start = buf.len();
        buf.resize(dir_start + tables.len() * 16, 0);

        for (i, (tag, table_data)) in tables.iter().enumerate() {
            let offset = buf.len();
            buf.extend_from_slice(table_data);
            let dir_off = dir_start + i * 16;
            buf[dir_off..dir_off + 4].copy_from_slice(*tag);
            buf[dir_off + 8..dir_off + 12].copy_from_slice(&(offset as u32).to_be_bytes());
            buf[dir_off + 12..dir_off + 16]
                .copy_from_slice(&(table_data.len() as u32).to_be_bytes());
        }

        buf
    }

    fn make_head_table(units_per_em: u16) -> Vec<u8> {
        let mut t = vec![0u8; 54];
        t[18] = (units_per_em >> 8) as u8;
        t[19] = units_per_em as u8;
        // bbox: zeros is fine
        t
    }

    fn make_hhea_table(ascent: i16, descent: i16, num_h_metrics: u16) -> Vec<u8> {
        let mut t = vec![0u8; 36];
        t[4..6].copy_from_slice(&ascent.to_be_bytes());
        t[6..8].copy_from_slice(&descent.to_be_bytes());
        t[34..36].copy_from_slice(&num_h_metrics.to_be_bytes());
        t
    }

    fn make_maxp_table(num_glyphs: u16) -> Vec<u8> {
        let mut t = vec![0u8; 6];
        t[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
        t
    }

    fn make_hmtx_table(widths: &[u16]) -> Vec<u8> {
        let mut t = Vec::new();
        for &w in widths {
            t.extend_from_slice(&w.to_be_bytes());
            t.extend_from_slice(&0i16.to_be_bytes()); // lsb
        }
        t
    }

    /// Build a simple cmap format 4 with one segment mapping start..=end with given delta.
    fn make_cmap_format4(start: u16, end: u16, delta: i16) -> Vec<u8> {
        let mut t = Vec::new();
        // cmap header
        t.extend_from_slice(&0u16.to_be_bytes()); // version
        t.extend_from_slice(&1u16.to_be_bytes()); // numSubtables
        // platform 3, encoding 1
        t.extend_from_slice(&3u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&12u32.to_be_bytes()); // offset to subtable

        // Format 4 subtable
        let seg_count: u16 = 2; // 1 real + sentinel
        t.extend_from_slice(&4u16.to_be_bytes()); // format
        t.extend_from_slice(&0u16.to_be_bytes()); // length (unused by parser)
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&(seg_count * 2).to_be_bytes());
        t.extend_from_slice(&2u16.to_be_bytes()); // searchRange
        t.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
        t.extend_from_slice(&0u16.to_be_bytes()); // rangeShift

        // endCode
        t.extend_from_slice(&end.to_be_bytes());
        t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // reservedPad
        t.extend_from_slice(&0u16.to_be_bytes());
        // startCode
        t.extend_from_slice(&start.to_be_bytes());
        t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // idDelta
        t.extend_from_slice(&delta.to_be_bytes());
        t.extend_from_slice(&1i16.to_be_bytes());
        // idRangeOffset
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());

        t
    }

    /// Build a cmap with format 4 that uses idRangeOffset (non-zero).
    fn make_cmap_format4_with_range_offset(start: u16, end: u16, glyph_ids: &[u16]) -> Vec<u8> {
        let mut t = Vec::new();
        // cmap header
        t.extend_from_slice(&0u16.to_be_bytes()); // version
        t.extend_from_slice(&1u16.to_be_bytes()); // numSubtables
        t.extend_from_slice(&3u16.to_be_bytes()); // platform 3
        t.extend_from_slice(&1u16.to_be_bytes()); // encoding 1
        t.extend_from_slice(&12u32.to_be_bytes()); // offset to subtable

        // Format 4 subtable
        let seg_count: u16 = 2; // 1 real + sentinel
        t.extend_from_slice(&4u16.to_be_bytes()); // format
        t.extend_from_slice(&0u16.to_be_bytes()); // length
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&(seg_count * 2).to_be_bytes());
        t.extend_from_slice(&2u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());

        // endCode
        t.extend_from_slice(&end.to_be_bytes());
        t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // reservedPad
        t.extend_from_slice(&0u16.to_be_bytes());
        // startCode
        t.extend_from_slice(&start.to_be_bytes());
        t.extend_from_slice(&0xFFFFu16.to_be_bytes());
        // idDelta: 0 for both segments
        t.extend_from_slice(&0i16.to_be_bytes());
        t.extend_from_slice(&1i16.to_be_bytes());
        // idRangeOffset: for segment 0, points to glyphIdArray right after this array
        // The offset is relative to the current position in idRangeOffset array.
        // idRangeOffset[0] is at position P. glyphIdArray starts at P + 4 (2 entries * 2 bytes).
        // So idRangeOffset = 4.
        let range_offset = seg_count * 2; // = 4
        t.extend_from_slice(&range_offset.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes()); // sentinel

        // glyphIdArray
        for &gid in glyph_ids {
            t.extend_from_slice(&gid.to_be_bytes());
        }

        t
    }

    fn make_name_table_ascii(name_id: u16, name: &[u8]) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // format
        t.extend_from_slice(&1u16.to_be_bytes()); // count
        let storage_offset: u16 = 6 + 12; // header + 1 record
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // record: platform 1 (Mac), encoding 0, language 0
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&name_id.to_be_bytes());
        t.extend_from_slice(&(name.len() as u16).to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes()); // offset into storage
        t.extend_from_slice(name);
        t
    }

    fn make_name_table_utf16be(
        name_id: u16,
        platform_id: u16,
        encoding_id: u16,
        name: &str,
    ) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // format
        t.extend_from_slice(&1u16.to_be_bytes()); // count
        let storage_offset: u16 = 6 + 12;
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // record
        t.extend_from_slice(&platform_id.to_be_bytes());
        t.extend_from_slice(&encoding_id.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&name_id.to_be_bytes());
        let name_bytes: Vec<u8> = name.encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
        t.extend_from_slice(&(name_bytes.len() as u16).to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes()); // offset
        t.extend_from_slice(&name_bytes);
        t
    }

    /// Build a complete valid TTF from individual table data vectors.
    fn build_full_ttf(
        head: &[u8],
        hhea: &[u8],
        maxp: &[u8],
        hmtx: &[u8],
        cmap: &[u8],
        name: &[u8],
        os2: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut table_list: Vec<(&[u8; 4], &[u8])> = vec![
            (b"head", head),
            (b"hhea", hhea),
            (b"maxp", maxp),
            (b"hmtx", hmtx),
            (b"cmap", cmap),
            (b"name", name),
        ];
        if let Some(os2_data) = os2 {
            table_list.push((b"OS/2", os2_data));
        }
        build_ttf_with_tables(&table_list)
    }

    #[test]
    fn parse_ttf_head_too_short() {
        // Line 98
        let data = build_ttf_with_tables(&[(b"head", &[0u8; 10])]);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("head table too short") || err.contains("Missing"));
    }

    #[test]
    fn parse_ttf_missing_hhea() {
        let head = make_head_table(1000);
        let data = build_ttf_with_tables(&[(b"head", &head)]);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("Missing hhea"));
    }

    #[test]
    fn parse_ttf_hhea_too_short() {
        // Line 111
        let head = make_head_table(1000);
        let data = build_ttf_with_tables(&[(b"head", &head), (b"hhea", &[0u8; 10])]);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("hhea table too short"));
    }

    #[test]
    fn parse_ttf_os2_table_used_for_ascent_descent() {
        // Lines 119-123: OS/2 table present and large enough
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 1);
        let maxp = make_maxp_table(1);
        let hmtx = make_hmtx_table(&[500]);
        let cmap = make_cmap_format4(65, 65, -64); // char 65 -> glyph 1
        let name = make_name_table_ascii(1, b"Test");

        let mut os2 = vec![0u8; 72];
        // sTypoAscender at offset 68
        os2[68..70].copy_from_slice(&900i16.to_be_bytes());
        // sTypoDescender at offset 70
        os2[70..72].copy_from_slice(&(-300i16).to_be_bytes());

        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, Some(&os2));
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.ascent, 900);
        assert_eq!(font.descent, -300);
    }

    #[test]
    fn parse_ttf_os2_table_too_short_falls_back_to_hhea() {
        // Line 125: OS/2 present but too short
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 1);
        let maxp = make_maxp_table(1);
        let hmtx = make_hmtx_table(&[500]);
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");

        let os2 = vec![0u8; 10]; // too short for ascent/descent fields
        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, Some(&os2));
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.ascent, 800);
        assert_eq!(font.descent, -200);
    }

    #[test]
    fn parse_ttf_maxp_too_short() {
        // Line 135
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 1);
        let data =
            build_ttf_with_tables(&[(b"head", &head), (b"hhea", &hhea), (b"maxp", &[0u8; 2])]);
        let err = parse_ttf(data).unwrap_err();
        assert!(err.contains("maxp table too short"));
    }

    #[test]
    fn parse_ttf_hmtx_break_on_short_data() {
        // Line 148: hmtx data cut short mid-entry.
        // We need hmtx to be the LAST table in the buffer, and truncate
        // the buffer so that reading the 2nd entry fails.
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 3); // claims 3 h_metrics
        let maxp = make_maxp_table(3); // claims 3 glyphs
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");
        // Only 1 hmtx entry (4 bytes): width=500, lsb=0
        let hmtx = make_hmtx_table(&[500]);

        // Build with hmtx LAST so we can truncate.
        // We build manually to ensure hmtx is at the end.
        let tables: Vec<(&[u8; 4], &[u8])> = vec![
            (b"head", &head),
            (b"hhea", &hhea),
            (b"maxp", &maxp),
            (b"cmap", &cmap),
            (b"name", &name),
            (b"hmtx", &hmtx),
        ];
        let mut data = build_ttf_with_tables(&tables);
        // hmtx is 4 bytes at the end. The parser tries entry_off = hmtx_off + 1*4
        // for i=1, which is hmtx_off + 4 = end of buffer. data.len() < entry_off + 2
        // is data.len() < data.len() + 2 => true => break.
        let font = parse_ttf(data.clone()).unwrap();
        assert_eq!(font.glyph_widths.len(), 1);
        assert_eq!(font.glyph_widths[0], 500);

        // Also test with no hmtx data at all by truncating further
        // to exercise the break at i=0.
        let hmtx_off = data.len() - 4;
        data.truncate(hmtx_off); // remove all hmtx data
        // But we need to keep hmtx table record pointing to the (now past-end) offset.
        // Actually hmtx_off now equals data.len(), so entry_off for i=0 = hmtx_off + 0 = data.len()
        // and data.len() < data.len() + 2 is true => break immediately => 0 widths.
        let font2 = parse_ttf(data).unwrap();
        assert!(font2.glyph_widths.is_empty());
    }

    #[test]
    fn parse_cmap_table_too_short() {
        // Line 186
        let result = parse_cmap(&[0u8; 2], 0);
        assert!(result.unwrap_err().contains("cmap table too short"));
    }

    #[test]
    fn parse_cmap_subtable_record_break() {
        // Line 196: subtable record truncated
        // cmap header says 2 subtables but data only has room for partial second
        let mut data = vec![0u8; 100];
        // offset 0: version=0, numSubtables=2
        data[2] = 0;
        data[3] = 2;
        // First record at offset 4: platform 1 (not matching), 8 bytes
        // We need the data to be short enough that the second record is truncated
        let _result = parse_cmap(&data[..11], 0);
        // Should find no suitable subtable (first one at platform 0 would match but
        // second record breaks)
        // Actually platform_id=0 matches, so let's set first to non-matching
        let mut data2 = vec![0u8; 20];
        data2[3] = 2; // 2 subtables
        // First record: platform 5 (no match)
        data2[4] = 0;
        data2[5] = 5;
        // Second record would be at offset 12, but we cut data short
        let result2 = parse_cmap(&data2[..15], 0);
        assert!(result2.unwrap_err().contains("No suitable cmap subtable"));
    }

    #[test]
    fn parse_cmap_subtable_too_short() {
        // Line 213: subtable offset valid but data too short to read format
        let mut data = vec![0u8; 20];
        data[3] = 1; // 1 subtable
        // platform 3, encoding 1
        data[4] = 0;
        data[5] = 3;
        data[6] = 0;
        data[7] = 1;
        // subtable offset pointing near end of data
        let sub_off = 18u32; // points to offset 0 + 18 = 18
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        // data.len() = 20, sub_off=18, need sub_off+2=20 which equals len -> not <, so it's ok
        // Make it 19 so sub_off + 2 > len
        let result = parse_cmap(&data[..19], 0);
        assert!(result.unwrap_err().contains("cmap subtable too short"));
    }

    #[test]
    fn parse_cmap_unsupported_format() {
        // Line 222: unsupported format returns empty map
        let mut data = vec![0u8; 30];
        data[3] = 1; // 1 subtable
        data[5] = 3; // platform 3
        data[7] = 1; // encoding 1
        let sub_off = 12u32;
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        // At offset 12: format = 6 (unsupported)
        data[12] = 0;
        data[13] = 6;
        let result = parse_cmap(&data, 0).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn parse_cmap_format0() {
        // Lines 228-239: format 0 parsing
        let mut data = vec![0u8; 300];
        data[3] = 1; // 1 subtable
        data[5] = 3; // platform 3
        data[7] = 1; // encoding 1
        let sub_off = 12u32;
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        // At offset 12: format = 0
        data[12] = 0;
        data[13] = 0;
        // format 0: 6 bytes header + 256 bytes glyph array = 262 total
        // glyph array starts at offset 12 + 6 = 18
        data[18 + 65] = 5; // char 65 ('A') -> glyph 5
        data[18 + 66] = 6; // char 66 ('B') -> glyph 6
        // char 0 -> glyph 0 (should not be inserted since glyph_id=0)
        let result = parse_cmap(&data, 0).unwrap();
        assert_eq!(result.get(&65), Some(&5));
        assert_eq!(result.get(&66), Some(&6));
        assert_eq!(result.get(&0), None); // glyph 0 skipped
    }

    #[test]
    fn parse_cmap_format0_too_short() {
        // Line 229-230
        let mut data = vec![0u8; 100];
        data[3] = 1;
        data[5] = 3;
        data[7] = 1;
        let sub_off = 12u32;
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        data[12] = 0;
        data[13] = 0; // format 0
        // data is only 100 bytes, need offset+262 = 274
        let result = parse_cmap(&data, 0);
        assert!(
            result
                .unwrap_err()
                .contains("cmap format 0 table too short")
        );
    }

    #[test]
    fn parse_cmap_format4_header_too_short() {
        // Line 245
        let result = parse_cmap_format4(&[0u8; 10], 0);
        assert!(
            result
                .unwrap_err()
                .contains("cmap format 4 header too short")
        );
    }

    #[test]
    fn parse_cmap_format4_data_too_short() {
        // Line 259: header ok but segment data truncated
        let mut data = vec![0u8; 20];
        // format 4, segCountX2 = 4 (2 segments)
        data[0] = 0;
        data[1] = 4;
        data[6] = 0;
        data[7] = 4; // segCountX2 = 4
        // Need: offset+14 + seg_count*2*4 + seg_count*2 + 2 = lots, but we only have 20
        let result = parse_cmap_format4(&data, 0);
        assert!(result.unwrap_err().contains("cmap format 4 data too short"));
    }

    #[test]
    fn parse_cmap_format4_with_id_range_offset() {
        // Lines 280-288: idRangeOffset != 0 path
        let cmap_data = make_cmap_format4_with_range_offset(65, 67, &[10, 20, 0]);
        // Parse just the cmap
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 21);
        let maxp = make_maxp_table(21);
        let hmtx = make_hmtx_table(&[
            500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 500,
            500, 500, 500, 500,
        ]);
        let name = make_name_table_ascii(1, b"Test");
        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap_data, &name, None);
        let font = parse_ttf(data).unwrap();
        // char 65 -> glyph 10, char 66 -> glyph 20, char 67 -> glyph 0 (not inserted)
        assert_eq!(font.cmap.get(&65), Some(&10));
        assert_eq!(font.cmap.get(&66), Some(&20));
        assert_eq!(font.cmap.get(&67), None); // glyph_id 0 not inserted
    }

    #[test]
    fn parse_cmap_format4_id_range_offset_out_of_bounds() {
        // Line 291: glyph_off + 1 >= data.len() -> returns 0
        // Build a cmap format 4 with idRangeOffset pointing beyond data
        let mut data = vec![0u8; 50];
        // format 4
        data[0] = 0;
        data[1] = 4;
        // segCountX2 = 4 (2 segments: 1 real + sentinel)
        data[6] = 0;
        data[7] = 4;
        let seg_count = 2usize;
        // endCode at offset 14
        let end_code_off = 14;
        data[end_code_off] = 0;
        data[end_code_off + 1] = 65; // endCode[0] = 65
        data[end_code_off + 2] = 0xFF;
        data[end_code_off + 3] = 0xFF; // sentinel
        // reserved pad at end_code_off + 4
        let start_code_off = end_code_off + seg_count * 2 + 2;
        data[start_code_off] = 0;
        data[start_code_off + 1] = 65; // startCode[0] = 65
        data[start_code_off + 2] = 0xFF;
        data[start_code_off + 3] = 0xFF;
        let id_delta_off = start_code_off + seg_count * 2;
        // idDelta = 0
        let id_range_off = id_delta_off + seg_count * 2;
        // idRangeOffset = huge value so glyph_off goes out of bounds
        data[id_range_off] = 0xFF;
        data[id_range_off + 1] = 0xFE;
        // sentinel idRangeOffset = 0
        let result = parse_cmap_format4(&data, 0).unwrap();
        // char 65 should not be in map (glyph_id = 0 from out of bounds)
        assert_eq!(result.get(&65), None);
    }

    #[test]
    fn parse_name_table_too_short() {
        // Line 306
        let result = parse_name_table(&[0u8; 4], 0);
        assert!(result.unwrap_err().contains("name table too short"));
    }

    #[test]
    fn parse_name_table_record_break() {
        // Line 320: record data truncated
        let mut data = vec![0u8; 12];
        data[3] = 2; // count = 2
        data[5] = 100; // string storage offset (large)
        // Only room for partial first record, second record will be cut
        let result = parse_name_table(&data, 0);
        assert!(result.unwrap_err().contains("No font name found"));
    }

    #[test]
    fn parse_name_table_skips_non_name_ids() {
        // Line 329: name_id != 1 && name_id != 4 -> continue
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // format
        t.extend_from_slice(&2u16.to_be_bytes()); // count = 2
        let storage_offset: u16 = 6 + 24; // header + 2 records
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // Record 0: nameID=2 (should skip)
        t.extend_from_slice(&1u16.to_be_bytes()); // platform
        t.extend_from_slice(&0u16.to_be_bytes()); // encoding
        t.extend_from_slice(&0u16.to_be_bytes()); // language
        t.extend_from_slice(&2u16.to_be_bytes()); // nameID=2
        t.extend_from_slice(&4u16.to_be_bytes()); // length
        t.extend_from_slice(&0u16.to_be_bytes()); // offset
        // Record 1: nameID=1 (should use)
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes()); // nameID=1
        t.extend_from_slice(&4u16.to_be_bytes());
        t.extend_from_slice(&4u16.to_be_bytes()); // offset=4 (after "Skip")
        // Storage
        t.extend_from_slice(b"SkipGood");
        let result = parse_name_table(&t, 0).unwrap();
        assert_eq!(result, "Good");
    }

    #[test]
    fn parse_name_table_priority_name_id_4_over_1() {
        // Line 334: priority <= best_priority -> continue (nameID 4 > nameID 1)
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes()); // format
        t.extend_from_slice(&2u16.to_be_bytes()); // count
        let storage_offset: u16 = 6 + 24;
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // Record 0: nameID=4 (full name, priority 2)
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&4u16.to_be_bytes()); // nameID=4
        t.extend_from_slice(&8u16.to_be_bytes()); // length
        t.extend_from_slice(&0u16.to_be_bytes());
        // Record 1: nameID=1 (family, priority 1 - should be skipped)
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes()); // nameID=1
        t.extend_from_slice(&6u16.to_be_bytes());
        t.extend_from_slice(&8u16.to_be_bytes());
        // Storage
        t.extend_from_slice(b"FullNameFamily");
        let result = parse_name_table(&t, 0).unwrap();
        assert_eq!(result, "FullName");
    }

    #[test]
    fn parse_name_table_string_beyond_data() {
        // Line 340: end > data.len() -> continue
        let mut t = Vec::new();
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes()); // count=1
        let storage_offset: u16 = 6 + 12;
        t.extend_from_slice(&storage_offset.to_be_bytes());
        // Record: nameID=1, but string extends beyond data
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&100u16.to_be_bytes()); // length=100 (way beyond)
        t.extend_from_slice(&0u16.to_be_bytes());
        // Only 2 bytes of storage
        t.extend_from_slice(b"AB");
        let result = parse_name_table(&t, 0);
        assert!(result.unwrap_err().contains("No font name found"));
    }

    #[test]
    fn parse_name_table_utf16be_windows_platform() {
        // Lines 346, 362-372: platform 3 triggers UTF-16BE decoding
        let name = make_name_table_utf16be(1, 3, 1, "Hello");
        let result = parse_name_table(&name, 0).unwrap();
        assert_eq!(result, "Hello");
    }

    #[test]
    fn parse_name_table_utf16be_unicode_platform_encoding_gt_0() {
        // Line 346: platform 0, encoding > 0 also triggers UTF-16BE
        let name = make_name_table_utf16be(1, 0, 1, "World");
        let result = parse_name_table(&name, 0).unwrap();
        assert_eq!(result, "World");
    }

    #[test]
    fn decode_utf16be_basic() {
        // Lines 362-372
        let data = [0x00, 0x48, 0x00, 0x69]; // "Hi"
        let result = decode_utf16be(&data);
        assert_eq!(result, "Hi");
    }

    #[test]
    fn decode_utf16be_empty() {
        let result = decode_utf16be(&[]);
        assert_eq!(result, "");
    }

    #[test]
    fn decode_utf16be_odd_byte_ignored() {
        // Line 365: while i + 1 < data.len() - odd trailing byte ignored
        let data = [0x00, 0x41, 0xFF]; // 'A' + trailing byte
        let result = decode_utf16be(&data);
        assert_eq!(result, "A");
    }

    #[test]
    fn parse_cmap_unicode_platform_fallback() {
        // Line 196 + platform 0 selection: platform 0 (Unicode) is accepted
        let mut data = vec![0u8; 300];
        data[3] = 1; // 1 subtable
        // platform 0, encoding 0
        data[4] = 0;
        data[5] = 0;
        data[6] = 0;
        data[7] = 0;
        let sub_off = 12u32;
        data[8..12].copy_from_slice(&sub_off.to_be_bytes());
        // format 0 subtable at offset 12
        data[12] = 0;
        data[13] = 0;
        data[18 + 65] = 3; // char 65 -> glyph 3
        let result = parse_cmap(&data, 0).unwrap();
        assert_eq!(result.get(&65), Some(&3));
    }

    #[test]
    fn parse_ttf_glyphs_beyond_num_h_metrics_share_last_width() {
        // Line 153-155: glyphs beyond num_h_metrics share last advance width
        let head = make_head_table(1000);
        let hhea = make_hhea_table(800, -200, 2); // only 2 h metrics
        let maxp = make_maxp_table(4); // but 4 glyphs
        // hmtx: only 2 long entries
        let hmtx = make_hmtx_table(&[500, 700]);
        let cmap = make_cmap_format4(65, 65, -64);
        let name = make_name_table_ascii(1, b"Test");
        let data = build_full_ttf(&head, &hhea, &maxp, &hmtx, &cmap, &name, None);
        let font = parse_ttf(data).unwrap();
        assert_eq!(font.glyph_widths.len(), 4);
        assert_eq!(font.glyph_widths[0], 500);
        assert_eq!(font.glyph_widths[1], 700);
        assert_eq!(font.glyph_widths[2], 700); // shares last width
        assert_eq!(font.glyph_widths[3], 700); // shares last width
    }
}
