use image::ImageDecoder;

pub(crate) struct DecodedJpegImage {
    pub width: u32,
    pub height: u32,
    pub rgb_data: Vec<u8>,
    pub icc_profile: Option<Vec<u8>>,
}

pub(crate) fn parse_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 4 || data.first().copied() != Some(0xFF) || data.get(1).copied() != Some(0xD8) {
        return None;
    }

    let mut pos = 2usize;
    while pos + 3 < data.len() {
        while pos < data.len() && data[pos] == 0xFF {
            pos += 1;
        }
        let marker = *data.get(pos)?;
        pos += 1;
        if marker == 0xD9 || marker == 0xDA {
            break;
        }

        let length = u16::from_be_bytes([*data.get(pos)?, *data.get(pos + 1)?]) as usize;
        if length < 2 || pos + length > data.len() {
            return None;
        }

        if matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        ) {
            if length < 7 {
                return None;
            }
            let height = u16::from_be_bytes([*data.get(pos + 3)?, *data.get(pos + 4)?]) as u32;
            let width = u16::from_be_bytes([*data.get(pos + 5)?, *data.get(pos + 6)?]) as u32;
            if width == 0 || height == 0 {
                return None;
            }
            return Some((width, height));
        }

        pos += length;
    }

    None
}

pub(crate) fn decode_jpeg_for_pdf(data: &[u8]) -> Option<DecodedJpegImage> {
    let cursor = std::io::Cursor::new(data);
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(cursor).ok()?;
    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();
    let icc_profile = decoder.icc_profile().ok().flatten();
    let total_bytes = usize::try_from(decoder.total_bytes()).ok()?;
    let mut pixels = vec![0; total_bytes];
    decoder.read_image(&mut pixels).ok()?;

    let rgb_data = match color_type {
        image::ColorType::Rgb8 => pixels,
        image::ColorType::L8 => pixels
            .into_iter()
            .flat_map(|value| [value, value, value])
            .collect(),
        _ => image::load_from_memory_with_format(data, image::ImageFormat::Jpeg)
            .ok()?
            .to_rgb8()
            .into_raw(),
    };

    Some(DecodedJpegImage {
        width,
        height,
        rgb_data,
        icc_profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    #[test]
    fn decode_jpeg_for_pdf_preserves_icc_profile() {
        let pixels = [255u8, 128, 0];
        let mut encoded = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new(&mut encoded);
        let icc_profile = vec![1, 2, 3, 4];
        encoder
            .set_icc_profile(icc_profile.clone())
            .expect("jpeg encoder should accept ICC profile");
        encoder
            .write_image(&pixels, 1, 1, image::ExtendedColorType::Rgb8)
            .expect("jpeg encoding should succeed");

        let decoded = decode_jpeg_for_pdf(&encoded).expect("jpeg should decode");
        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.height, 1);
        assert_eq!(decoded.icc_profile.as_deref(), Some(icc_profile.as_slice()));
        assert_eq!(decoded.rgb_data.len(), 3);
    }
}
