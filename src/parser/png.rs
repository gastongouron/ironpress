//! Minimal PNG parser that extracts enough information for PDF embedding.
//!
//! PNG files contain IDAT chunks with zlib-compressed pixel data including
//! PNG row filters. PDF's FlateDecode filter with PNG predictors can handle
//! this natively, so we pass the raw IDAT data through.

/// PNG file signature (8 bytes).
pub const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Parsed information from a PNG file needed for PDF embedding.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    /// Number of color channels (1=Gray, 2=GrayAlpha, 3=RGB, 4=RGBA).
    pub channels: u8,
    /// Concatenated raw IDAT chunk data (zlib-compressed).
    pub idat_data: Vec<u8>,
}

/// Check whether a byte slice starts with the PNG signature.
pub fn is_png(data: &[u8]) -> bool {
    data.len() >= 8 && data[..8] == PNG_SIGNATURE
}

/// Parse a PNG file and extract the information needed for PDF embedding.
///
/// Returns `None` if the data is not a valid PNG or cannot be parsed.
pub fn parse_png(data: &[u8]) -> Option<PngInfo> {
    if !is_png(data) {
        return None;
    }

    let mut pos = 8; // skip signature
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut ihdr_found = false;
    let mut idat_data = Vec::new();

    while pos + 8 <= data.len() {
        // Each chunk: 4 bytes length, 4 bytes type, `length` bytes data, 4 bytes CRC
        let chunk_len = read_u32_be(data, pos) as usize;
        let chunk_type = &data[pos + 4..pos + 8];

        // Ensure we have enough data for the full chunk (length + type + data + CRC)
        if pos + 12 + chunk_len > data.len() {
            break;
        }

        let chunk_data_start = pos + 8;

        match chunk_type {
            b"IHDR" => {
                if chunk_len < 13 {
                    return None;
                }
                width = read_u32_be(data, chunk_data_start);
                height = read_u32_be(data, chunk_data_start + 4);
                bit_depth = data[chunk_data_start + 8];
                color_type = data[chunk_data_start + 9];
                ihdr_found = true;
            }
            b"IDAT" => {
                idat_data.extend_from_slice(&data[chunk_data_start..chunk_data_start + chunk_len]);
            }
            b"IEND" => {
                break;
            }
            _ => {}
        }

        // Advance past: length(4) + type(4) + data(chunk_len) + CRC(4)
        pos += 12 + chunk_len;
    }

    if !ihdr_found || idat_data.is_empty() || width == 0 || height == 0 {
        return None;
    }

    let channels = match color_type {
        0 => 1,           // Grayscale
        2 => 3,           // RGB
        4 => 2,           // Grayscale + Alpha
        6 => 4,           // RGBA
        _ => return None, // Unsupported (e.g., indexed color type 3)
    };

    Some(PngInfo {
        width,
        height,
        bit_depth,
        color_type,
        channels,
        idat_data,
    })
}

fn read_u32_be(data: &[u8], offset: usize) -> u32 {
    ((data[offset] as u32) << 24)
        | ((data[offset + 1] as u32) << 16)
        | ((data[offset + 2] as u32) << 8)
        | (data[offset + 3] as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_signature_detection() {
        assert!(is_png(&PNG_SIGNATURE));
        assert!(is_png(&[137, 80, 78, 71, 13, 10, 26, 10, 0, 0]));
        assert!(!is_png(&[0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!is_png(&[0xFF, 0xD8, 0xFF])); // JPEG header
        assert!(!is_png(&[]));
        assert!(!is_png(&[137, 80])); // too short
    }

    #[test]
    fn parse_invalid_data() {
        assert!(parse_png(&[]).is_none());
        assert!(parse_png(&[0, 1, 2, 3]).is_none());
        // Valid signature but no chunks
        assert!(parse_png(&PNG_SIGNATURE).is_none());
    }

    #[test]
    fn parse_minimal_valid_png() {
        // Build a minimal valid PNG with IHDR and one IDAT chunk
        let png = build_test_png(
            1,
            1,
            8,
            2,
            &[
                0x78, 0x01, 0x62, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
            ],
        );
        let info = parse_png(&png).unwrap();
        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
        assert_eq!(info.bit_depth, 8);
        assert_eq!(info.color_type, 2);
        assert_eq!(info.channels, 3);
        assert!(!info.idat_data.is_empty());
    }

    #[test]
    fn parse_grayscale_png() {
        let png = build_test_png(2, 2, 8, 0, &[0x78, 0x01, 0x01, 0x00, 0x00]);
        let info = parse_png(&png).unwrap();
        assert_eq!(info.color_type, 0);
        assert_eq!(info.channels, 1);
    }

    #[test]
    fn parse_rgba_png() {
        let png = build_test_png(2, 2, 8, 6, &[0x78, 0x01, 0x01, 0x00, 0x00]);
        let info = parse_png(&png).unwrap();
        assert_eq!(info.color_type, 6);
        assert_eq!(info.channels, 4);
    }

    #[test]
    fn parse_gray_alpha_png() {
        let png = build_test_png(2, 2, 8, 4, &[0x78, 0x01, 0x01, 0x00, 0x00]);
        let info = parse_png(&png).unwrap();
        assert_eq!(info.color_type, 4);
        assert_eq!(info.channels, 2);
    }

    #[test]
    fn parse_unsupported_color_type() {
        // Color type 3 (indexed) is not supported
        let png = build_test_png(1, 1, 8, 3, &[0x78, 0x01, 0x01, 0x00, 0x00]);
        assert!(parse_png(&png).is_none());
    }

    #[test]
    fn parse_multiple_idat_chunks() {
        // Build PNG with two IDAT chunks
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);

        // IHDR chunk
        let ihdr_data = build_ihdr(4, 4, 8, 2);
        append_chunk(&mut png, b"IHDR", &ihdr_data);

        // First IDAT
        let idat1 = [0x78, 0x01, 0x62];
        append_chunk(&mut png, b"IDAT", &idat1);

        // Second IDAT
        let idat2 = [0x60, 0x60, 0x00];
        append_chunk(&mut png, b"IDAT", &idat2);

        // IEND
        append_chunk(&mut png, b"IEND", &[]);

        let info = parse_png(&png).unwrap();
        assert_eq!(info.width, 4);
        assert_eq!(info.height, 4);
        // IDAT data should be concatenated
        assert_eq!(info.idat_data, [0x78, 0x01, 0x62, 0x60, 0x60, 0x00]);
    }

    #[test]
    fn parse_ihdr_too_short() {
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);
        // IHDR with only 5 bytes of data (needs 13)
        append_chunk(&mut png, b"IHDR", &[0; 5]);
        append_chunk(&mut png, b"IEND", &[]);
        assert!(parse_png(&png).is_none());
    }

    /// Helper: build a minimal test PNG with given parameters.
    fn build_test_png(
        width: u32,
        height: u32,
        bit_depth: u8,
        color_type: u8,
        idat_data: &[u8],
    ) -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);

        let ihdr_data = build_ihdr(width, height, bit_depth, color_type);
        append_chunk(&mut png, b"IHDR", &ihdr_data);
        append_chunk(&mut png, b"IDAT", idat_data);
        append_chunk(&mut png, b"IEND", &[]);

        png
    }

    fn build_ihdr(width: u32, height: u32, bit_depth: u8, color_type: u8) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&height.to_be_bytes());
        data.push(bit_depth);
        data.push(color_type);
        data.push(0); // compression method
        data.push(0); // filter method
        data.push(0); // interlace method
        data
    }

    fn append_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(chunk_type);
        buf.extend_from_slice(data);
        // CRC (we just write zeros; our parser doesn't verify CRC)
        buf.extend_from_slice(&[0, 0, 0, 0]);
    }
}
