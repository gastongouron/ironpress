use super::*;

#[test]
fn try_parse_svg_bytes_accepts_utf8_bom_prefix() {
    let raw = b"\xEF\xBB\xBF<svg width=\"20\" height=\"10\"></svg>";
    let tree = try_parse_svg_bytes(raw).expect("expected BOM-prefixed SVG to parse");
    assert_eq!(tree.width, 20.0);
    assert_eq!(tree.height, 10.0);
}

#[test]
fn fetch_remote_url_returns_none_without_feature() {
    // Without the "remote" feature, fetch_remote_url always returns None
    let result = fetch_remote_url(
        "https://example.com/image.png",
        &crate::security::resources::NetworkPolicy::default(),
    );
    #[cfg(not(feature = "remote"))]
    assert!(result.is_none());
    // With the feature enabled, it would attempt a real HTTP request
    // (which may or may not succeed depending on network)
    let _ = result;
}

#[test]
fn load_image_data_http_without_feature() {
    let result = load_image_data("http://example.com/test.jpg");
    #[cfg(not(feature = "remote"))]
    assert!(
        result.is_none(),
        "HTTP images should be None without remote feature"
    );
    let _ = result;
}

#[test]
fn load_image_data_https_without_feature() {
    let result = load_image_data("https://example.com/test.png");
    #[cfg(not(feature = "remote"))]
    assert!(
        result.is_none(),
        "HTTPS images should be None without remote feature"
    );
    let _ = result;
}

/// Build a tiny RGBA PNG (1x1, single transparent pixel) for decode tests.
#[cfg(test)]
fn build_rgba_png() -> Vec<u8> {
    use crate::parser::png::PNG_SIGNATURE;
    use std::io::Write;
    // Filter byte (0) + RGBA pixel, zlib-compressed.
    let raw_scanline = [0u8, 10, 20, 30, 128];
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&raw_scanline).unwrap();
    let idat = encoder.finish().unwrap();

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&1u32.to_be_bytes()); // width
    ihdr.extend_from_slice(&1u32.to_be_bytes()); // height
    ihdr.push(8); // bit depth
    ihdr.push(6); // color type 6 = RGBA
    ihdr.extend_from_slice(&[0, 0, 0]); // compression/filter/interlace

    let mut png = Vec::new();
    png.extend_from_slice(&PNG_SIGNATURE);
    let append = |buf: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]| {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc_input = ty.to_vec();
        crc_input.extend_from_slice(data);
        buf.extend_from_slice(ty);
        buf.extend_from_slice(data);
        // Our parser ignores CRC; write zeros.
        buf.extend_from_slice(&[0, 0, 0, 0]);
    };
    append(&mut png, b"IHDR", &ihdr);
    append(&mut png, b"IDAT", &idat);
    append(&mut png, b"IEND", &[]);
    png
}

#[test]
fn rgba_png_is_loaded_as_alpha_with_raw_bytes_preserved() {
    let png = build_rgba_png();
    let asset = load_image_bytes(png.clone()).expect("RGBA PNG should load");
    // The alpha channel must be preserved by carrying the original PNG bytes
    // (decoded into an SMask at embed time) rather than flattened to RGB.
    assert_eq!(asset.format, ImageFormat::PngAlpha);
    assert!(asset.png_metadata.is_none());
    assert_eq!(asset.data, png, "raw PNG bytes should be carried through");
    assert_eq!(asset.source_width, 1);
    assert_eq!(asset.source_height, 1);
}

#[test]
fn opaque_png_crop_decodes_full_png_asset() {
    let img = image::RgbImage::from_fn(4, 3, |x, y| {
        image::Rgb([(x * 50) as u8, (y * 80) as u8, (x * 20 + y * 30) as u8])
    });
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();

    let asset = load_image_bytes(png.clone()).expect("opaque PNG should load");
    assert_eq!(asset.format, ImageFormat::Png);
    assert_eq!(asset.data, png);

    let crop = RasterCrop::aligned(
        Rect::from_xywh(1.0, 1.0, 2.0, 1.0),
        RasterDimensions {
            width: 4,
            height: 3,
        },
    )
    .expect("whole in-bounds source pixels should form a crop");
    let cropped = crop_raster_asset(&asset, crop).expect("full-PNG asset should crop");
    assert_eq!(cropped.source_width, 2);
    assert_eq!(cropped.source_height, 1);
    assert!(
        png::is_png(&cropped.data),
        "cropped opaque PNG should remain decodable for later resizing"
    );

    let rgba = decode_asset_to_rgba(&cropped).expect("cropped asset should decode");
    assert_eq!(rgba.width(), 2);
    assert_eq!(rgba.height(), 1);
    assert_eq!(rgba.get_pixel(0, 0).0[..3], [50, 80, 50]);
    assert_eq!(rgba.get_pixel(1, 0).0[..3], [100, 80, 70]);
}

#[test]
fn raster_crop_parses_only_aligned_in_bounds_pixels() {
    let source = RasterDimensions {
        width: 8,
        height: 4,
    };

    assert!(RasterCrop::aligned(Rect::from_xywh(2.0, 0.0, 4.0, 4.0), source).is_some());
    assert!(RasterCrop::aligned(Rect::from_xywh(2.25, 0.0, 4.0, 4.0), source).is_none());
    assert!(RasterCrop::aligned(Rect::from_xywh(6.0, 0.0, 4.0, 4.0), source).is_none());
}

#[test]
fn base64_decode_roundtrip() {
    let data = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    let encoded = base64_encode(data);
    let decoded = decode_base64(&encoded).unwrap();
    assert_eq!(decoded, data);
}
#[test]
fn try_parse_svg_bytes_rejects_binary_data() {
    let raw = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
    assert!(
        try_parse_svg_bytes(raw).is_none(),
        "JPEG binary data should not parse as SVG"
    );
}

#[test]
fn try_parse_svg_bytes_accepts_xml_declaration() {
    let raw = b"<?xml version=\"1.0\"?><svg width=\"10\" height=\"10\"></svg>";
    let tree = try_parse_svg_bytes(raw).expect("XML declaration SVG should parse");
    assert_eq!(tree.width, 10.0);
}

#[test]
fn try_parse_svg_bytes_accepts_comment_prefix() {
    let raw = b"<!-- comment --><svg width=\"30\" height=\"15\"></svg>";
    let tree = try_parse_svg_bytes(raw).expect("Comment-prefixed SVG should parse");
    assert_eq!(tree.width, 30.0);
}

#[test]
fn try_parse_svg_bytes_rejects_comment_without_svg() {
    let raw = b"<!-- just a comment, no SVG here -->";
    assert!(
        try_parse_svg_bytes(raw).is_none(),
        "Comment without <svg> should return None"
    );
}
#[test]
fn percent_decode_basic() {
    assert_eq!(percent_decode("%3Csvg%3E"), "<svg>");
    assert_eq!(percent_decode("hello%20world"), "hello world");
    assert_eq!(percent_decode("no%encoding"), "no%encoding");
}
