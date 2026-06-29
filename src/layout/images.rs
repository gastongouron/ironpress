use crate::parser::dom::ElementNode;
use crate::parser::png;
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    ColorFilterOp, ComputedStyle, Display, FontStyle, FontWeight, VerticalAlign,
};
use crate::util::decode_base64;
use std::collections::HashMap;

use super::engine::{ImageFormat, LayoutBorder, LayoutElement, PngMetadata, RasterImageAsset};
use super::text::resolve_style_font_family;

/// Load raw bytes from a `src` attribute value.
///
/// Supports `data:` URIs (base64 and percent-encoded), local file paths, and
/// HTTP/HTTPS URLs (gated behind the `remote` feature).
///
/// For data URIs the MIME header is returned so callers can use it to skip
/// unnecessary probing (e.g. skip SVG probe when the MIME is `image/jpeg`).
pub(crate) fn load_src_bytes(src: &str) -> Option<(Vec<u8>, Option<String>)> {
    if let Some(rest) = src.strip_prefix("data:") {
        let (header, encoded) = rest.split_once(',')?;
        let header_lower = header.to_ascii_lowercase();
        let bytes = if header_lower.contains("base64") {
            decode_base64(encoded)?
        } else {
            // Plain-text or percent-encoded data URI — decode %XX sequences.
            percent_decode(encoded).into_bytes()
        };
        let mime = if header_lower.is_empty() {
            None
        } else {
            Some(header_lower)
        };
        Some((bytes, mime))
    } else if src.starts_with("http://") || src.starts_with("https://") {
        Some((fetch_remote_url(src)?, None))
    } else {
        Some((std::fs::read(src).ok()?, None))
    }
}

/// Heuristic SVG sniff over raw bytes (first 512 bytes, UTF-8-lossy so binary
/// content is safely rejected): true when the content looks like an XML/SVG
/// document. Used to gate both the internal SVG parser and the mask rasteriser.
pub(crate) fn looks_like_svg(raw: &[u8]) -> bool {
    let prefix = if raw.len() > 512 { &raw[..512] } else { raw };
    let text = String::from_utf8_lossy(prefix);
    let trimmed = text.trim_start_matches('\u{FEFF}').trim_start();
    let trimmed_lower = trimmed.to_ascii_lowercase();
    if !(trimmed.starts_with("<svg")
        || trimmed.starts_with("<?xml")
        || trimmed.starts_with("<!--")
        || trimmed_lower.starts_with("<!doctype"))
    {
        return false;
    }
    // For the comment case, search the full content (comments may exceed the
    // 512-byte prefix before the <svg> tag appears).
    if trimmed.starts_with("<!--") {
        return String::from_utf8_lossy(raw).contains("<svg");
    }
    true
}

/// Probe raw bytes for SVG content and parse into an `SvgTree`.
///
/// Uses a heuristic on the first 512 bytes (via `String::from_utf8_lossy` so
/// that non-UTF-8 binary content is safely rejected) and then parses the full
/// content through the HTML parser to extract the `<svg>` element.
pub(crate) fn try_parse_svg_bytes(raw: &[u8]) -> Option<crate::parser::svg::SvgTree> {
    // Heuristic: check if the content looks like SVG (XML with an <svg element).
    if !looks_like_svg(raw) {
        return None;
    }

    // Parse the full SVG content — use lossy conversion so that stray non-UTF-8
    // bytes don't cause the whole parse to fail.
    let svg_str = String::from_utf8_lossy(raw);
    crate::parser::svg::parse_svg_from_string(&svg_str)
}

/// Detect PNG/JPEG format and return a raster asset with source dimensions.
pub(crate) fn load_image_bytes(raw: Vec<u8>) -> Option<RasterImageAsset> {
    if png::is_png(&raw) {
        // The lightweight parser passes raw IDAT through to PDF FlateDecode and
        // only supports color types whose samples map directly to a PDF color
        // space (grayscale/RGB +/- alpha). Indexed (palette) PNGs and other
        // exotic encodings are decoded and normalized to 8-bit RGB instead.
        let Some(png_info) = png::parse_png(&raw) else {
            return decode_png_to_rgb_asset(&raw);
        };
        // The raw-IDAT passthrough writes the sample stream straight into a PDF
        // DeviceRGB/DeviceGray image, which take 3/1 colour components. An alpha
        // colour type (RGBA=4, GrayscaleAlpha=2) cannot be passed through that
        // way (the viewer would read the extra channel as misaligned colour
        // samples). Carry the complete original PNG so the renderer can decode it
        // into a colour stream plus a soft-mask (`/SMask`), preserving the alpha
        // channel rather than dropping it (which rendered transparent regions as
        // opaque black).
        if png_info.channels == 2 || png_info.channels == 4 {
            return Some(RasterImageAsset {
                source_width: png_info.width,
                source_height: png_info.height,
                data: raw,
                format: ImageFormat::PngAlpha,
                png_metadata: None,
            });
        }
        let metadata = PngMetadata {
            channels: png_info.channels,
            bit_depth: png_info.bit_depth,
        };
        Some(RasterImageAsset {
            data: png_info.idat_data,
            source_width: png_info.width,
            source_height: png_info.height,
            format: ImageFormat::Png,
            png_metadata: Some(metadata),
        })
    } else if raw.starts_with(&[0xFF, 0xD8]) {
        let (source_width, source_height) = crate::parser::jpeg::parse_jpeg_dimensions(&raw)?;
        Some(RasterImageAsset {
            data: raw,
            source_width,
            source_height,
            format: ImageFormat::Jpeg,
            png_metadata: None,
        })
    } else {
        None
    }
}

/// Load image data from an <img> element and return a LayoutElement.
///
/// Bytes are fetched exactly once from the source.  When the content is SVG it
/// is parsed as vector graphics (`LayoutElement::Svg`); otherwise it falls back
/// to raster PNG/JPEG (`LayoutElement::Image`).
pub(crate) fn load_image_from_element(
    el: &ElementNode,
    available_width: f32,
    available_height: f32,
    style: &ComputedStyle,
) -> Option<LayoutElement> {
    let src = el.attributes.get("src")?;

    // Load bytes once.
    let (raw, mime) = load_src_bytes(src)?;

    // For data URIs with a non-SVG MIME type, skip the SVG probe entirely.
    let skip_svg = mime
        .as_deref()
        .is_some_and(|m| !m.is_empty() && !m.contains("svg") && !m.contains("xml"));

    // Try SVG path first — render as vector graphics instead of raster.
    if !skip_svg && let Some(mut tree) = try_parse_svg_bytes(&raw) {
        let intrinsic = resolve_svg_size(&tree, available_width, available_height, false, false);
        let html_attr_width = style
            .width
            .or_else(|| parse_html_image_dimension(el.attributes.get("width")));
        let html_attr_height = style
            .height
            .or_else(|| parse_html_image_dimension(el.attributes.get("height")));

        let (width, height) = match (html_attr_width, html_attr_height) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) if intrinsic.0 > 0.0 => (w, intrinsic.1 * (w / intrinsic.0)),
            (Some(w), None) => (w, intrinsic.1),
            (None, Some(h)) if intrinsic.1 > 0.0 => (intrinsic.0 * (h / intrinsic.1), h),
            (None, Some(h)) => (intrinsic.0, h),
            (None, None) => intrinsic,
        };

        let (width, height) = constrain_replaced_image_size(
            width,
            height,
            available_width,
            style.max_width,
            style.max_height,
        );

        let border = LayoutBorder::from_computed(&style.border);
        let content_width = (width - border.horizontal_width()).max(0.0);
        let content_height = (height - border.vertical_width()).max(0.0);
        sync_svg_tree_to_layout_box(&mut tree, content_width, content_height);
        return Some(LayoutElement::Svg {
            tree,
            width,
            height,
            flow_extra_bottom: 0.0,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
            background_color: style.background_color.map(|c| c.to_f32_rgba()),
            mix_blend_mode: style.mix_blend_mode,
            border,
        });
    }

    // Fall back to raster image using the same bytes. `filter: blur()` /
    // `drop-shadow()` need the decoded pixels (with the correct device sigma and
    // transparent feather padding), so those are deferred to the filter raster
    // path below; only the non-blur color filters are baked in here.
    let wants_filter_raster = style.blur_radius > 0.0 || style.drop_shadow.is_some();
    let raw_for_filter = wants_filter_raster.then(|| raw.clone());
    let image = load_raster_image_bytes(raw, 0.0, &style.color_filters)?;

    // Determine dimensions: CSS width/height take precedence over the HTML
    // width/height attributes (matching the SVG path and the CSS cascade).
    let attr_width = style
        .width
        .or_else(|| parse_html_image_dimension(el.attributes.get("width")));
    let attr_height = style
        .height
        .or_else(|| parse_html_image_dimension(el.attributes.get("height")));

    // Raster images carry concrete natural dimensions (the source pixel size,
    // taken as CSS px at 1x → pt). The CSS default sizing algorithm
    // (css-images-3 §5.4) uses them to derive any missing dimension and, when
    // neither is given, to size the box directly.
    let src_w = image.source_width as f32;
    let src_h = image.source_height as f32;
    let natural_w = src_w * 0.75;
    let natural_h = src_h * 0.75;
    let (width, height) = match (attr_width, attr_height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) if src_w > 0.0 => (w, w * (src_h / src_w)),
        (Some(w), None) => (w, w), // fallback: square (intrinsic size unknown)
        (None, Some(h)) if src_h > 0.0 => (h * (src_w / src_h), h),
        (None, Some(h)) => (h, h), // fallback: square (intrinsic size unknown)
        // No width/height specified: use the image's natural dimensions
        // (default sizing algorithm, no-dimensions branch). Fall back to the
        // CSS default object size only when natural dimensions are unusable.
        (None, None) if natural_w > 0.0 && natural_h > 0.0 => (natural_w, natural_h),
        (None, None) => (available_width.min(200.0), 150.0),
    };

    let (width, height) = constrain_replaced_image_size(
        width,
        height,
        available_width,
        style.max_width,
        style.max_height,
    );

    // CSS `filter: blur()` / `drop-shadow()`: rasterize the (color-filtered)
    // pixels into a padded, blurred bitmap so the effect feathers outside the
    // content box. The displayed content box is `width`/`height` (a replaced
    // box has no border by default in these cases); the bitmap carries the
    // extra `blur_overflow` on every side.
    let content_w =
        (width - LayoutBorder::from_computed(&style.border).horizontal_width()).max(0.0);
    let content_h = (height - LayoutBorder::from_computed(&style.border).vertical_width()).max(0.0);
    let (image, blur_overflow) = match raw_for_filter {
        Some(bytes) => build_filter_raster(
            &bytes,
            content_w,
            content_h,
            &style.color_filters,
            style.blur_radius,
            style.drop_shadow,
        )
        .unwrap_or((image, 0.0)),
        None => (image, 0.0),
    };

    Some(LayoutElement::Image {
        image,
        width,
        height,
        flow_extra_bottom: 0.0,
        margin_top: style.margin.top,
        margin_bottom: style.margin.bottom,
        object_fit: style.object_fit,
        object_position: style.object_position,
        background_color: style.background_color.map(|c| c.to_f32_rgba()),
        border: LayoutBorder::from_computed(&style.border),
        blur_overflow,
        src_crop: None,
    })
}

/// Decode `raw`, apply the non-blur color filters, then produce the blurred /
/// drop-shadow raster for `filter: blur()` / `drop-shadow()`. Returns the
/// embeddable asset plus the overflow (points per side) it adds beyond the
/// content box, or `None` if decoding fails (caller keeps the sharp image).
fn build_filter_raster(
    raw: &[u8],
    content_w_pt: f32,
    content_h_pt: f32,
    color_filters: &[ColorFilterOp],
    blur_radius_pt: f32,
    drop_shadow: Option<crate::style::computed::DropShadow>,
) -> Option<(RasterImageAsset, f32)> {
    let rgba = decode_image_for_blur(raw)?.to_rgba8();
    if let Some(ds) = drop_shadow {
        // drop-shadow operates on the (color-filtered) source.
        let (src, _) =
            apply_filter_ops_rgba(&rgba, color_filters, content_w_pt, content_h_pt, 0.0)?;
        return crate::render::blur::drop_shadow_image(
            &src,
            content_w_pt,
            content_h_pt,
            ds.dx,
            ds.dy,
            ds.blur,
            ds.color,
        )
        .map(|b| (b.asset, b.overflow_pt));
    }
    if blur_radius_pt > 0.0 {
        let (buf, overflow) = apply_filter_ops_rgba(
            &rgba,
            color_filters,
            content_w_pt,
            content_h_pt,
            blur_radius_pt,
        )?;
        return crate::render::blur::raster_from_buffer(buf, overflow)
            .map(|b| (b.asset, b.overflow_pt));
    }
    None
}

fn apply_filter_ops_rgba(
    img: &image::RgbaImage,
    ops: &[ColorFilterOp],
    content_w_pt: f32,
    content_h_pt: f32,
    fallback_blur_pt: f32,
) -> Option<(image::RgbaImage, f32)> {
    let mut current = img.clone();
    let mut overflow = 0.0;
    let mut saw_blur = false;
    for op in ops {
        match *op {
            ColorFilterOp::Blur(radius) if radius > 0.0 => {
                let display_w = content_w_pt + 2.0 * overflow;
                let display_h = content_h_pt + 2.0 * overflow;
                let (buf, ov) = crate::render::blur::blur_image_buffer(
                    &current, display_w, display_h, radius, 300.0,
                )?;
                current = buf;
                overflow += ov;
                saw_blur = true;
            }
            ColorFilterOp::Blur(_) => {}
            _ => apply_color_filters_rgba(&mut current, std::slice::from_ref(op)),
        }
    }
    if !saw_blur && fallback_blur_pt > 0.0 {
        let (buf, ov) = crate::render::blur::blur_image_buffer(
            &current,
            content_w_pt,
            content_h_pt,
            fallback_blur_pt,
            300.0,
        )?;
        current = buf;
        overflow += ov;
    }
    Some((current, overflow))
}

/// Apply CSS `filter` color functions to an RGBA image in place, preserving the
/// alpha channel (the RGB-only variant lives alongside the legacy in-place blur
/// path). Mirrors `apply_color_filters` for the RGBA filter raster path.
fn apply_color_filters_rgba(img: &mut image::RgbaImage, ops: &[ColorFilterOp]) {
    let mut rgb = image::RgbImage::new(img.width(), img.height());
    for (dst, src) in rgb.pixels_mut().zip(img.pixels()) {
        *dst = image::Rgb([src[0], src[1], src[2]]);
    }
    apply_color_filters(&mut rgb, ops);
    for (src, dst) in rgb.pixels().zip(img.pixels_mut()) {
        dst[0] = src[0];
        dst[1] = src[1];
        dst[2] = src[2];
    }
}

/// Decode a PNG the lightweight parser cannot pass through (e.g. indexed/palette
/// color) and re-encode it as a clean 8-bit RGB PNG so it flows through the
/// normal FlateDecode embedding path.
fn decode_png_to_rgb_asset(raw: &[u8]) -> Option<RasterImageAsset> {
    let rgb = image::load_from_memory(raw).ok()?.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let mut encoded = Vec::new();
    image::DynamicImage::ImageRgb8(rgb)
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .ok()?;
    let png_info = png::parse_png(&encoded)?;
    Some(RasterImageAsset {
        data: png_info.idat_data,
        source_width: width,
        source_height: height,
        format: ImageFormat::Png,
        png_metadata: Some(PngMetadata {
            channels: png_info.channels,
            bit_depth: png_info.bit_depth,
        }),
    })
}

/// Crop a raster image asset to the source-pixel sub-rectangle `[x, y, w, h]`
/// (rounded to whole pixels and clamped to the source bounds) and return a fresh,
/// self-contained asset holding ONLY the cropped pixels.
///
/// Pagination uses this to SLICE a too-tall raster image across page boundaries:
/// each page embeds just its own portion of the source raster instead of a full
/// copy hidden behind a clip rectangle. Returns `None` if the source cannot be
/// decoded or the crop is empty.
pub(crate) fn crop_raster_asset(
    asset: &RasterImageAsset,
    crop: [f32; 4],
) -> Option<RasterImageAsset> {
    let rgba = decode_asset_to_rgba(asset)?;
    let (sw, sh) = (rgba.width(), rgba.height());
    let x = (crop[0].round().max(0.0) as u32).min(sw);
    let y = (crop[1].round().max(0.0) as u32).min(sh);
    let w = (crop[2].round().max(0.0) as u32).min(sw.saturating_sub(x));
    let h = (crop[3].round().max(0.0) as u32).min(sh.saturating_sub(y));
    if w == 0 || h == 0 {
        return None;
    }
    let sub = image::imageops::crop_imm(&rgba, x, y, w, h).to_image();
    encode_rgba_subimage_as_asset(sub)
}

/// Decode a stored [`RasterImageAsset`] back to RGBA pixels regardless of its
/// on-disk storage format: a JPEG/alpha-PNG asset carries the complete file, an
/// opaque PNG asset carries only the raw IDAT (zlib) stream so a minimal PNG
/// container is rebuilt around it before decoding.
fn decode_asset_to_rgba(asset: &RasterImageAsset) -> Option<image::RgbaImage> {
    match asset.format {
        ImageFormat::Jpeg => Some(image::load_from_memory(&asset.data).ok()?.to_rgba8()),
        ImageFormat::PngAlpha => Some(decode_image_for_blur(&asset.data)?.to_rgba8()),
        ImageFormat::Png => {
            let meta = asset.png_metadata.as_ref()?;
            // PNG color-type from channel count (opaque PNGs are gray=1 or rgb=3,
            // but handle the alpha variants too for robustness).
            let color_type = match meta.channels {
                1 => 0,
                2 => 4,
                3 => 2,
                4 => 6,
                _ => return None,
            };
            let png = reconstruct_png(
                asset.source_width,
                asset.source_height,
                meta.bit_depth,
                color_type,
                &asset.data,
            );
            Some(decode_image_for_blur(&png)?.to_rgba8())
        }
    }
}

/// Re-encode a cropped RGBA buffer into an embeddable asset: a lossless RGB PNG
/// (raw-IDAT passthrough, the common opaque path) when every pixel is opaque, or
/// a full RGBA PNG (the alpha-preserving `/SMask` embedding path) otherwise.
fn encode_rgba_subimage_as_asset(sub: image::RgbaImage) -> Option<RasterImageAsset> {
    let (w, h) = (sub.width(), sub.height());
    let opaque = sub.pixels().all(|p| p[3] == 255);
    if opaque {
        let mut rgb = image::RgbImage::new(w, h);
        for (dst, src) in rgb.pixels_mut().zip(sub.pixels()) {
            *dst = image::Rgb([src[0], src[1], src[2]]);
        }
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgb8(rgb)
            .write_to(
                &mut std::io::Cursor::new(&mut encoded),
                image::ImageFormat::Png,
            )
            .ok()?;
        let info = png::parse_png(&encoded)?;
        Some(RasterImageAsset {
            data: info.idat_data,
            source_width: w,
            source_height: h,
            format: ImageFormat::Png,
            png_metadata: Some(PngMetadata {
                channels: info.channels,
                bit_depth: info.bit_depth,
            }),
        })
    } else {
        let mut encoded = Vec::new();
        image::DynamicImage::ImageRgba8(sub)
            .write_to(
                &mut std::io::Cursor::new(&mut encoded),
                image::ImageFormat::Png,
            )
            .ok()?;
        Some(RasterImageAsset {
            data: encoded,
            source_width: w,
            source_height: h,
            format: ImageFormat::PngAlpha,
            png_metadata: None,
        })
    }
}

/// Wrap a raw IDAT (zlib) stream back into a minimal, valid PNG file (signature +
/// IHDR + IDAT + IEND, each with a correct CRC-32) so the standard image decoder
/// can read pixels from an opaque-PNG asset that only stored its IDAT.
fn reconstruct_png(width: u32, height: u32, bit_depth: u8, color_type: u8, idat: &[u8]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let crc_start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = crc32(&out[crc_start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let mut out = Vec::with_capacity(8 + 25 + idat.len() + 12);
    out.extend_from_slice(&png::PNG_SIGNATURE);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(bit_depth);
    ihdr.push(color_type);
    ihdr.push(0); // compression method
    ihdr.push(0); // filter method
    ihdr.push(0); // interlace method
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Placement of replaced-image content inside its box, in points relative to the
/// box top-left corner. `width`/`height` are the drawn size; `offset_x`/`offset_y`
/// shift the drawn content within the box. `clip` is true when the content can
/// overflow the box and must be clipped (cover, or none/contain larger than box).
pub(crate) struct ImagePlacement {
    pub width: f32,
    pub height: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub clip: bool,
}

/// Compute where to draw a replaced image inside its box per CSS `object-fit`
/// and `object-position`. The box is `box_w` x `box_h` points; the image's
/// intrinsic pixel size is converted to points (1px = 0.75pt).
pub(crate) fn compute_image_placement(
    box_w: f32,
    box_h: f32,
    source_width: u32,
    source_height: u32,
    object_fit: crate::style::computed::ObjectFit,
    object_position: crate::style::computed::ObjectPosition,
) -> ImagePlacement {
    use crate::style::computed::ObjectFit;

    // Intrinsic size in points (CSS px -> pt).
    let intrinsic_w = source_width as f32 * 0.75;
    let intrinsic_h = source_height as f32 * 0.75;

    // Fall back to filling the box when intrinsic dimensions are unusable.
    if intrinsic_w <= 0.0 || intrinsic_h <= 0.0 || box_w <= 0.0 || box_h <= 0.0 {
        return ImagePlacement {
            width: box_w,
            height: box_h,
            offset_x: 0.0,
            offset_y: 0.0,
            clip: false,
        };
    }

    let contain_scale = (box_w / intrinsic_w).min(box_h / intrinsic_h);
    let cover_scale = (box_w / intrinsic_w).max(box_h / intrinsic_h);

    let (draw_w, draw_h) = match object_fit {
        ObjectFit::Fill => (box_w, box_h),
        ObjectFit::Contain => (intrinsic_w * contain_scale, intrinsic_h * contain_scale),
        ObjectFit::Cover => (intrinsic_w * cover_scale, intrinsic_h * cover_scale),
        ObjectFit::None => (intrinsic_w, intrinsic_h),
        ObjectFit::ScaleDown => {
            // The smaller of `none` and `contain`.
            let scale = contain_scale.min(1.0);
            (intrinsic_w * scale, intrinsic_h * scale)
        }
    };

    // object-position aligns the drawn content within the free space (which can
    // be negative when the content is larger than the box, i.e. cropping). A
    // length component is an absolute start-edge offset; a fraction/percentage
    // component scales the free space.
    let offset_x = object_position.x.resolve(box_w - draw_w);
    let offset_y = object_position.y.resolve(box_h - draw_h);

    // Replaced content is always clipped to the content box (css-images-3 §5.5).
    // Clip whenever any edge of the drawn content falls outside the box — either
    // because the content is larger than the box, or because object-position
    // (e.g. a length offset) pushes it past an edge.
    const EPS: f32 = 0.01;
    let clip = offset_x < -EPS
        || offset_y < -EPS
        || offset_x + draw_w > box_w + EPS
        || offset_y + draw_h > box_h + EPS;

    ImagePlacement {
        width: draw_w,
        height: draw_h,
        offset_x,
        offset_y,
        clip,
    }
}

pub(crate) fn constrain_replaced_image_size(
    width: f32,
    height: f32,
    available_width: f32,
    max_width: Option<f32>,
    max_height: Option<f32>,
) -> (f32, f32) {
    if width <= 0.0 || height <= 0.0 {
        return (width.max(0.0), height.max(0.0));
    }

    let mut scale: f32 = 1.0;

    if available_width.is_finite() && available_width > 0.0 {
        scale = scale.min(available_width / width);
    }

    if let Some(limit) = max_width.filter(|limit| limit.is_finite() && *limit > 0.0) {
        scale = scale.min(limit / width);
    }

    if let Some(limit) = max_height.filter(|limit| limit.is_finite() && *limit > 0.0) {
        scale = scale.min(limit / height);
    }

    if scale < 1.0 {
        (width * scale, height * scale)
    } else {
        (width, height)
    }
}

pub(crate) fn add_inline_replaced_baseline_gap(
    element: LayoutElement,
    style: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) -> LayoutElement {
    if style.display != Display::Inline || style.vertical_align != VerticalAlign::Baseline {
        return element;
    }

    let font_family = resolve_style_font_family(style, fonts);
    let (_, descender_ratio) = crate::fonts::font_metrics_ratios(
        &font_family,
        style.font_weight == FontWeight::Bold,
        style.font_style == FontStyle::Italic,
        fonts,
    );
    let baseline_gap = descender_ratio * style.font_size;
    if baseline_gap <= 0.0 {
        return element;
    }

    match element {
        LayoutElement::Image {
            image,
            width,
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
            object_fit,
            object_position,
            background_color,
            border,
            blur_overflow,
            src_crop,
        } => LayoutElement::Image {
            image,
            width,
            height,
            flow_extra_bottom: flow_extra_bottom + baseline_gap,
            margin_top,
            margin_bottom,
            object_fit,
            object_position,
            background_color,
            border,
            blur_overflow,
            src_crop,
        },
        LayoutElement::Svg {
            tree,
            width,
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
            background_color,
            mix_blend_mode,
            border,
        } => LayoutElement::Svg {
            tree,
            width,
            height,
            flow_extra_bottom: flow_extra_bottom + baseline_gap,
            margin_top,
            margin_bottom,
            background_color,
            mix_blend_mode,
            border,
        },
        other => other,
    }
}

pub(crate) fn parse_html_image_dimension(raw: Option<&String>) -> Option<f32> {
    let raw = raw?.trim();
    let raw = raw.strip_suffix("px").unwrap_or(raw);
    raw.parse::<f32>().ok().map(|px| px * 0.75)
}

struct SvgSizeSource<'a> {
    width_raw: Option<&'a str>,
    height_raw: Option<&'a str>,
    natural_width: Option<f32>,
    natural_height: Option<f32>,
    natural_ratio: Option<f32>,
}

impl<'a> SvgSizeSource<'a> {
    fn from_tree(tree: &'a crate::parser::svg::SvgTree) -> Self {
        let explicit_width = tree
            .width_attr
            .as_deref()
            .and_then(crate::parser::svg::parse_absolute_length)
            .filter(|width| *width > 0.0);
        let explicit_height = tree
            .height_attr
            .as_deref()
            .and_then(crate::parser::svg::parse_absolute_length)
            .filter(|height| *height > 0.0);
        let natural_width = explicit_width
            .or_else(|| (tree.view_box.is_none() && tree.width > 0.0).then_some(tree.width));
        let natural_height = explicit_height
            .or_else(|| (tree.view_box.is_none() && tree.height > 0.0).then_some(tree.height));
        Self {
            width_raw: tree.width_attr.as_deref(),
            height_raw: tree.height_attr.as_deref(),
            natural_ratio: svg_natural_ratio(
                explicit_width,
                explicit_height,
                natural_width,
                natural_height,
                tree.view_box,
            ),
            natural_width,
            natural_height,
        }
    }

    fn from_element(el: &'a ElementNode) -> Self {
        let width_raw = el.attributes.get("width").map(String::as_str);
        let height_raw = el.attributes.get("height").map(String::as_str);
        let view_box = el
            .attributes
            .get("viewBox")
            .and_then(|value| crate::parser::svg::parse_viewbox(value));
        let natural_width = width_raw
            .and_then(crate::parser::svg::parse_absolute_length)
            .filter(|width| *width > 0.0);
        let natural_height = height_raw
            .and_then(crate::parser::svg::parse_absolute_length)
            .filter(|height| *height > 0.0);

        Self {
            width_raw,
            height_raw,
            natural_width,
            natural_height,
            natural_ratio: svg_natural_ratio(
                natural_width,
                natural_height,
                natural_width,
                natural_height,
                view_box,
            ),
        }
    }

    fn resolve(
        self,
        available_width: f32,
        available_height: f32,
        allow_percent_width: bool,
        allow_percent_height: bool,
    ) -> (f32, f32) {
        const DEFAULT_OBJECT_WIDTH: f32 = 300.0;
        const DEFAULT_OBJECT_HEIGHT: f32 = 150.0;
        let width = resolve_svg_dimension(self.width_raw, available_width, allow_percent_width);
        let height = resolve_svg_dimension(self.height_raw, available_height, allow_percent_height);

        match (width, height) {
            (Some(width), Some(height)) => (width, height),
            (Some(width), None) => {
                if let Some(ratio) = self.natural_ratio {
                    (width, width * ratio)
                } else {
                    (width, self.natural_height.unwrap_or(DEFAULT_OBJECT_HEIGHT))
                }
            }
            (None, Some(height)) => {
                if let Some(ratio) = self.natural_ratio {
                    (height / ratio.max(f32::EPSILON), height)
                } else {
                    (self.natural_width.unwrap_or(DEFAULT_OBJECT_WIDTH), height)
                }
            }
            (None, None) => {
                if let Some(width) = self.natural_width {
                    if let Some(height) = self.natural_height {
                        (width, height)
                    } else if let Some(ratio) = self.natural_ratio {
                        (width, width * ratio)
                    } else {
                        (width, DEFAULT_OBJECT_HEIGHT)
                    }
                } else if let Some(height) = self.natural_height {
                    if let Some(ratio) = self.natural_ratio {
                        (height / ratio.max(f32::EPSILON), height)
                    } else {
                        (DEFAULT_OBJECT_WIDTH, height)
                    }
                } else if let Some(ratio) = self.natural_ratio {
                    contain_default_object_size(ratio)
                } else {
                    (DEFAULT_OBJECT_WIDTH, DEFAULT_OBJECT_HEIGHT)
                }
            }
        }
    }
}

pub(crate) fn svg_natural_ratio(
    explicit_width: Option<f32>,
    explicit_height: Option<f32>,
    natural_width: Option<f32>,
    natural_height: Option<f32>,
    view_box: Option<crate::parser::svg::ViewBox>,
) -> Option<f32> {
    match (explicit_width, explicit_height) {
        (Some(width), Some(height)) => Some(height / width.max(f32::EPSILON)),
        _ => view_box
            .and_then(|view_box| {
                (view_box.width > 0.0 && view_box.height > 0.0)
                    .then_some(view_box.height / view_box.width)
            })
            .or_else(|| match (natural_width, natural_height) {
                (Some(width), Some(height)) => Some(height / width.max(f32::EPSILON)),
                _ => None,
            }),
    }
}

pub(crate) fn contain_default_object_size(ratio: f32) -> (f32, f32) {
    const DEFAULT_OBJECT_WIDTH: f32 = 300.0;
    const DEFAULT_OBJECT_HEIGHT: f32 = 150.0;

    let default_ratio = DEFAULT_OBJECT_HEIGHT / DEFAULT_OBJECT_WIDTH;
    if ratio > default_ratio {
        (DEFAULT_OBJECT_HEIGHT / ratio, DEFAULT_OBJECT_HEIGHT)
    } else {
        (DEFAULT_OBJECT_WIDTH, DEFAULT_OBJECT_WIDTH * ratio)
    }
}

/// Resolve the rendered size of an SVG from its intrinsic dimensions and raw
/// `width`/`height` attributes.
pub(crate) fn resolve_svg_size(
    tree: &crate::parser::svg::SvgTree,
    available_width: f32,
    available_height: f32,
    allow_percent_width: bool,
    allow_percent_height: bool,
) -> (f32, f32) {
    SvgSizeSource::from_tree(tree).resolve(
        available_width,
        available_height,
        allow_percent_width,
        allow_percent_height,
    )
}

pub(crate) fn resolve_svg_element_size(
    el: &ElementNode,
    available_width: f32,
    available_height: f32,
    allow_percent_width: bool,
    allow_percent_height: bool,
) -> (f32, f32) {
    SvgSizeSource::from_element(el).resolve(
        available_width,
        available_height,
        allow_percent_width,
        allow_percent_height,
    )
}

pub(crate) fn resolve_svg_dimension(
    raw: Option<&str>,
    available_space: f32,
    allow_percent: bool,
) -> Option<f32> {
    let raw = raw?;
    let raw = raw.trim();
    if let Some(pct) = raw.strip_suffix('%') {
        if allow_percent {
            if let Ok(value) = pct.trim().parse::<f32>() {
                if value >= 0.0 {
                    return Some(available_space * (value / 100.0));
                }
            }
        }
        return None;
    }

    // SVG width/height attributes are in CSS px by default.
    // Values with explicit "pt" suffix stay as-is; otherwise convert px→pt.
    if raw.ends_with("pt") {
        let value = crate::parser::svg::parse_length(raw)?;
        return if value >= 0.0 { Some(value) } else { None };
    }
    let value = crate::parser::svg::parse_length(raw)?;
    if value >= 0.0 {
        // Convert px to pt (1px = 0.75pt)
        Some(value * 0.75)
    } else {
        None
    }
}

pub(crate) fn sync_svg_tree_to_layout_box(
    tree: &mut crate::parser::svg::SvgTree,
    width: f32,
    height: f32,
) {
    if tree.view_box.is_none() {
        tree.width = width;
        tree.height = height;
    }
}

pub(crate) fn inject_inherited_svg_color(
    tree: &mut crate::parser::svg::SvgTree,
    inherited_color: (f32, f32, f32),
) {
    let inherit_color = |style: &mut crate::parser::svg::SvgStyle| {
        style.color.get_or_insert(inherited_color);
    };

    match tree.children.as_mut_slice() {
        [crate::parser::svg::SvgNode::Group { style, .. }] => inherit_color(style),
        _ => {
            tree.children = vec![crate::parser::svg::SvgNode::Group {
                transform: None,
                children: std::mem::take(&mut tree.children),
                style: crate::parser::svg::SvgStyle {
                    color: Some(inherited_color),
                    ..crate::parser::svg::SvgStyle::default()
                },
            }];
        }
    }
}

/// Maximum size for remote resources (10 MB).
#[cfg(feature = "remote")]
const MAX_REMOTE_SIZE: usize = 10 * 1024 * 1024;

/// Fetch bytes from an HTTP/HTTPS URL (requires the `remote` feature).
/// Returns `None` if the feature is disabled, the request fails, or the response exceeds 10 MB.
pub(crate) fn fetch_remote_url(url: &str) -> Option<Vec<u8>> {
    #[cfg(feature = "remote")]
    {
        let resp = ureq::get(url).call().ok()?;
        let len = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        if len > MAX_REMOTE_SIZE {
            return None;
        }
        let buf = resp
            .into_body()
            .with_config()
            .limit(MAX_REMOTE_SIZE as u64)
            .read_to_vec()
            .ok()?;
        Some(buf)
    }
    #[cfg(not(feature = "remote"))]
    {
        let _ = url;
        None
    }
}

/// Load image data from a src attribute (supports data: URIs, local files, and remote URLs).
///
/// This is a convenience wrapper around `load_src_bytes` + `load_image_bytes`.
#[cfg(test)]
pub(crate) fn load_image_data(src: &str) -> Option<RasterImageAsset> {
    let (raw, _mime) = load_src_bytes(src)?;
    load_image_bytes(raw)
}

pub(crate) fn build_raster_background_tree(src: &str) -> Option<crate::parser::svg::SvgTree> {
    let image_src = crate::parser::css::extract_url_path(src).unwrap_or_else(|| src.to_string());
    let (raw, _mime) = load_src_bytes(&image_src)?;
    let (width, height) = raster_image_dimensions(&raw)?;

    Some(crate::parser::svg::SvgTree {
        width: width as f32,
        height: height as f32,
        width_attr: None,
        height_attr: None,
        preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
        view_box: None,
        defs: crate::parser::svg::SvgDefs::default(),
        children: vec![crate::parser::svg::SvgNode::Image {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
            href: image_src,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::None,
            style: crate::parser::svg::SvgStyle::default(),
        }],
        text_ctx: crate::parser::svg::SvgTextContext::default(),
        source_markup: None,
    })
}

pub(crate) fn raster_image_dimensions(raw: &[u8]) -> Option<(u32, u32)> {
    if png::is_png(raw) {
        let png_info = png::parse_png(raw)?;
        Some((png_info.width, png_info.height))
    } else {
        let image = image::load_from_memory(raw).ok()?;
        Some((image.width(), image.height()))
    }
}

pub(crate) fn load_raster_image_bytes(
    raw: Vec<u8>,
    blur_radius: f32,
    color_filters: &[ColorFilterOp],
) -> Option<RasterImageAsset> {
    if !color_filters.is_empty() {
        apply_image_filters(&raw, blur_radius, color_filters)
    } else if blur_radius > 0.0 {
        blur_image_bytes(&raw, blur_radius)
    } else {
        load_image_bytes(raw)
    }
}

/// Decode an image, apply blur then the CSS color filters in order, and
/// re-encode losslessly (RGB PNG) so flat-color filtered output stays crisp.
fn apply_image_filters(
    raw: &[u8],
    blur_radius: f32,
    color_filters: &[ColorFilterOp],
) -> Option<RasterImageAsset> {
    let mut decoded = decode_image_for_blur(raw)?;
    if blur_radius > 0.0 {
        decoded = image::DynamicImage::ImageRgba8(image::imageops::blur(&decoded, blur_radius));
    }
    let mut rgb = decoded.to_rgb8();
    apply_color_filters(&mut rgb, color_filters);
    let (width, height) = (rgb.width(), rgb.height());
    let mut encoded = Vec::new();
    image::DynamicImage::ImageRgb8(rgb)
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .ok()?;
    let png_info = png::parse_png(&encoded)?;
    Some(RasterImageAsset {
        data: png_info.idat_data,
        source_width: width,
        source_height: height,
        format: ImageFormat::Png,
        png_metadata: Some(PngMetadata {
            channels: png_info.channels,
            bit_depth: png_info.bit_depth,
        }),
    })
}

/// Apply CSS `filter` color functions to an RGB image, in order. Matrices follow
/// the CSS Filter Effects / SVG feColorMatrix definitions.
fn apply_color_filters(img: &mut image::RgbImage, ops: &[ColorFilterOp]) {
    for pixel in img.pixels_mut() {
        let (mut r, mut g, mut b) = (pixel[0] as f32, pixel[1] as f32, pixel[2] as f32);
        for op in ops {
            let (nr, ng, nb) = apply_one_filter(op, r, g, b);
            r = nr.clamp(0.0, 255.0);
            g = ng.clamp(0.0, 255.0);
            b = nb.clamp(0.0, 255.0);
        }
        pixel[0] = r.round() as u8;
        pixel[1] = g.round() as u8;
        pixel[2] = b.round() as u8;
    }
}

fn apply_one_filter(op: &ColorFilterOp, r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    match *op {
        ColorFilterOp::Brightness(k) => (r * k, g * k, b * k),
        ColorFilterOp::Contrast(c) => (
            (r - 127.5) * c + 127.5,
            (g - 127.5) * c + 127.5,
            (b - 127.5) * c + 127.5,
        ),
        ColorFilterOp::Invert(a) => (
            r * (1.0 - a) + (255.0 - r) * a,
            g * (1.0 - a) + (255.0 - g) * a,
            b * (1.0 - a) + (255.0 - b) * a,
        ),
        ColorFilterOp::Grayscale(amount) => {
            let v = 1.0 - amount;
            (
                (0.2126 + 0.7874 * v) * r + (0.7152 - 0.7152 * v) * g + (0.0722 - 0.0722 * v) * b,
                (0.2126 - 0.2126 * v) * r + (0.7152 + 0.2848 * v) * g + (0.0722 - 0.0722 * v) * b,
                (0.2126 - 0.2126 * v) * r + (0.7152 - 0.7152 * v) * g + (0.0722 + 0.9278 * v) * b,
            )
        }
        ColorFilterOp::Sepia(amount) => {
            let v = 1.0 - amount;
            (
                (0.393 + 0.607 * v) * r + (0.769 - 0.769 * v) * g + (0.189 - 0.189 * v) * b,
                (0.349 - 0.349 * v) * r + (0.686 + 0.314 * v) * g + (0.168 - 0.168 * v) * b,
                (0.272 - 0.272 * v) * r + (0.534 - 0.534 * v) * g + (0.131 + 0.869 * v) * b,
            )
        }
        ColorFilterOp::Saturate(s) => (
            (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b,
            (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b,
            (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b,
        ),
        ColorFilterOp::HueRotate(deg) => {
            let rad = deg.to_radians();
            let (c, s) = (rad.cos(), rad.sin());
            (
                (0.213 + c * 0.787 - s * 0.213) * r
                    + (0.715 - c * 0.715 - s * 0.715) * g
                    + (0.072 - c * 0.072 + s * 0.928) * b,
                (0.213 - c * 0.213 + s * 0.143) * r
                    + (0.715 + c * 0.285 + s * 0.140) * g
                    + (0.072 - c * 0.072 - s * 0.283) * b,
                (0.213 - c * 0.213 - s * 0.787) * r
                    + (0.715 - c * 0.715 + s * 0.715) * g
                    + (0.072 + c * 0.928 + s * 0.072) * b,
            )
        }
        ColorFilterOp::Matrix(m) => (
            m[0] * r + m[1] * g + m[2] * b + m[4] * 255.0,
            m[5] * r + m[6] * g + m[7] * b + m[9] * 255.0,
            m[10] * r + m[11] * g + m[12] * b + m[14] * 255.0,
        ),
        ColorFilterOp::Flood { .. } => (r, g, b),
        ColorFilterOp::Blur(_)
        | ColorFilterOp::Offset { .. }
        | ColorFilterOp::DropShadow(_)
        | ColorFilterOp::MorphologyDilate(_) => (r, g, b),
    }
}

/// sRGB transfer function (IEC 61966-2-1): encoded 0..1 -> linear-light 0..1.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse sRGB transfer function: linear-light 0..1 -> encoded 0..1.
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Apply an ordered list of CSS/SVG color-filter ops to a single solid color,
/// reusing the same per-pixel math (`apply_one_filter`) the image path uses.
/// `color` is straight-alpha RGBA in 0..1 (as produced by `Color::to_f32_rgba`);
/// the alpha channel is preserved unchanged (feColorMatrix saturate/grayscale/
/// hue-rotate touch RGB only). When `linear_rgb` is true the math runs in
/// linear-light (SVG `color-interpolation-filters: linearRGB`, the default for
/// `<filter>` referenced by `filter: url(#id)`); otherwise it runs in sRGB
/// (CSS `filter` *functions*). This lets `filter: url(#id)` recolor a solid
/// box's background/border the same way it recolors an image's pixels.
pub(crate) fn apply_color_filters_to_color(
    color: (f32, f32, f32, f32),
    ops: &[ColorFilterOp],
    linear_rgb: bool,
) -> (f32, f32, f32, f32) {
    let (mut cr, mut cg, mut cb, mut a) = color;
    if linear_rgb {
        cr = srgb_to_linear(cr);
        cg = srgb_to_linear(cg);
        cb = srgb_to_linear(cb);
    }
    let (mut r, mut g, mut b) = (cr * 255.0, cg * 255.0, cb * 255.0);
    for op in ops {
        if let ColorFilterOp::Matrix(m) = op {
            let alpha = a * 255.0;
            let nr = m[0] * r + m[1] * g + m[2] * b + m[3] * alpha + m[4] * 255.0;
            let ng = m[5] * r + m[6] * g + m[7] * b + m[8] * alpha + m[9] * 255.0;
            let nb = m[10] * r + m[11] * g + m[12] * b + m[13] * alpha + m[14] * 255.0;
            let na = m[15] * r + m[16] * g + m[17] * b + m[18] * alpha + m[19] * 255.0;
            r = nr.clamp(0.0, 255.0);
            g = ng.clamp(0.0, 255.0);
            b = nb.clamp(0.0, 255.0);
            a = (na / 255.0).clamp(0.0, 1.0);
        } else {
            let (nr, ng, nb) = apply_one_filter(op, r, g, b);
            r = nr.clamp(0.0, 255.0);
            g = ng.clamp(0.0, 255.0);
            b = nb.clamp(0.0, 255.0);
        }
    }
    let (mut or, mut og, mut ob) = (r / 255.0, g / 255.0, b / 255.0);
    if linear_rgb {
        or = linear_to_srgb(or);
        og = linear_to_srgb(og);
        ob = linear_to_srgb(ob);
    }
    (or, og, ob, a)
}

/// Recolor an element's self-painted surfaces (background-color and each border
/// side color) in place through its resolved `color_filters`. Used for solid
/// boxes carrying a CSS `filter` (color function or resolved `url(#id)`): a
/// `<filter>`'s color-matrix recolors the box the same way it recolors an
/// image's pixels (css-filter-effects-1 §2; SVG filter-effects feColorMatrix).
/// Replaced-image pixels are filtered separately on the image path, so this only
/// affects the box's own paint and never double-applies.
pub(crate) fn apply_color_filters_to_box(style: &mut ComputedStyle, linear_rgb: bool) {
    let ops = style.color_filters.clone();
    let recolor = |c: crate::types::Color| -> crate::types::Color {
        let (r, g, b, a) = apply_color_filters_to_color(c.to_f32_rgba(), &ops, linear_rgb);
        crate::types::Color {
            r: (r * 255.0).round().clamp(0.0, 255.0) as u8,
            g: (g * 255.0).round().clamp(0.0, 255.0) as u8,
            b: (b * 255.0).round().clamp(0.0, 255.0) as u8,
            a: (a * 255.0).round().clamp(0.0, 255.0) as u8,
        }
    };
    if let Some(bg) = style.background_color {
        style.background_color = Some(recolor(bg));
    }
    for side in [
        &mut style.border.top,
        &mut style.border.right,
        &mut style.border.bottom,
        &mut style.border.left,
    ] {
        if let Some(c) = side.color {
            side.color = Some(recolor(c));
        }
    }
    for shadow in &mut style.box_shadow {
        shadow.color = recolor(shadow.color);
    }
    for op in &ops {
        match *op {
            ColorFilterOp::Blur(radius) => {
                style.blur_radius = style.blur_radius.max(radius);
            }
            ColorFilterOp::Offset {
                dx,
                dy,
                keep_source,
                ..
            } => {
                if let Some(bg) = style.background_color {
                    style.box_shadow.push(crate::style::computed::BoxShadow {
                        offset_x: dx,
                        offset_y: dy,
                        blur: 0.0,
                        spread: 0.0,
                        color: bg,
                        inset: false,
                    });
                    if !keep_source {
                        style.background_color = None;
                    }
                }
            }
            ColorFilterOp::DropShadow(shadow) => {
                style.box_shadow.push(crate::style::computed::BoxShadow {
                    offset_x: shadow.dx,
                    offset_y: shadow.dy,
                    blur: shadow.blur,
                    spread: 0.0,
                    color: color_from_filter_tuple(shadow.color),
                    inset: false,
                });
            }
            ColorFilterOp::MorphologyDilate(radius) => {
                if let Some(bg) = style.background_color {
                    style.box_shadow.push(crate::style::computed::BoxShadow {
                        offset_x: 0.0,
                        offset_y: 0.0,
                        blur: 0.0,
                        spread: radius,
                        color: bg,
                        inset: false,
                    });
                }
            }
            _ => {}
        }
    }
}

fn color_from_filter_tuple(color: (f32, f32, f32, f32)) -> crate::types::Color {
    crate::types::Color {
        r: (color.0 * 255.0).round().clamp(0.0, 255.0) as u8,
        g: (color.1 * 255.0).round().clamp(0.0, 255.0) as u8,
        b: (color.2 * 255.0).round().clamp(0.0, 255.0) as u8,
        a: (color.3 * 255.0).round().clamp(0.0, 255.0) as u8,
    }
}

pub(crate) fn blur_image_bytes(raw: &[u8], blur_radius: f32) -> Option<RasterImageAsset> {
    let decoded = decode_image_for_blur(raw)?;
    let blurred = image::imageops::blur(&decoded, blur_radius);
    let mut encoded = Vec::new();
    image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(blurred).to_rgb8())
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Jpeg,
        )
        .ok()?;
    Some(RasterImageAsset {
        data: encoded,
        source_width: decoded.width(),
        source_height: decoded.height(),
        format: ImageFormat::Jpeg,
        png_metadata: None,
    })
}

fn decode_image_for_blur(raw: &[u8]) -> Option<image::DynamicImage> {
    if png::is_png(raw) {
        decode_png_for_blur(raw)
    } else {
        image::load_from_memory(raw).ok()
    }
}

fn decode_png_for_blur(data: &[u8]) -> Option<image::DynamicImage> {
    use image::{DynamicImage, ImageBuffer};

    let mut decoder = png_decoder::Decoder::new(std::io::Cursor::new(data));
    decoder.ignore_checksums(true);
    let mut reader = decoder.read_info().ok()?;
    let output_size = reader.output_buffer_size()?;
    let mut buf = vec![0; output_size];
    let info = reader.next_frame(&mut buf).ok()?;
    let width = info.width;
    let height = info.height;
    let used = info.buffer_size();
    let buf = buf.get(..used)?.to_vec();

    match info.color_type {
        png_decoder::ColorType::Rgba => {
            let image = ImageBuffer::from_raw(width, height, buf)?;
            Some(DynamicImage::ImageRgba8(image))
        }
        png_decoder::ColorType::Rgb => {
            let image = ImageBuffer::from_raw(width, height, buf)?;
            Some(DynamicImage::ImageRgb8(image))
        }
        png_decoder::ColorType::Grayscale => {
            let image = ImageBuffer::from_raw(width, height, buf)?;
            Some(DynamicImage::ImageLuma8(image))
        }
        png_decoder::ColorType::GrayscaleAlpha => {
            let image = ImageBuffer::from_raw(width, height, buf)?;
            Some(DynamicImage::ImageLumaA8(image))
        }
        _ => image::load_from_memory(data).ok(),
    }
}

#[cfg(test)]
pub(crate) fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(*chunk.first().unwrap_or(&0));
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;

        append_base64_char(&mut result, CHARS, ((triple >> 18) & 0x3F) as usize);
        append_base64_char(&mut result, CHARS, ((triple >> 12) & 0x3F) as usize);

        if chunk.len() > 1 {
            append_base64_char(&mut result, CHARS, ((triple >> 6) & 0x3F) as usize);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            append_base64_char(&mut result, CHARS, (triple & 0x3F) as usize);
        } else {
            result.push('=');
        }
    }

    result
}

#[cfg(test)]
fn append_base64_char(out: &mut String, table: &[u8], index: usize) {
    if let Some(&byte) = table.get(index) {
        out.push(char::from(byte));
    }
}

/// Decode percent-encoded strings (e.g. `%3C` → `<`).  Used for plain-text SVG
/// data URIs like `data:image/svg+xml,%3Csvg ...%3E`.
pub(crate) fn percent_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::svg::{SvgTree, ViewBox};
    use crate::util::decode_base64;

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
        let result = fetch_remote_url("https://example.com/image.png");
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
    fn base64_decode_roundtrip() {
        let data = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let encoded = base64_encode(data);
        let decoded = decode_base64(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn svg_size_percent_attrs_do_not_override_intrinsic_image_size() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: Some("100%".to_string()),
            height_attr: Some("50%".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (300.0, 150.0)
        );
    }

    #[test]
    fn svg_size_absolute_width_only_preserves_aspect_ratio() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: Some("120".to_string()),
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 20.0,
                height: 10.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (90.0, 45.0)
        );
    }

    #[test]
    fn svg_size_absolute_height_only_preserves_aspect_ratio() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: None,
            height_attr: Some("60".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 20.0,
                height: 10.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (90.0, 45.0)
        );
    }

    #[test]
    fn svg_size_absolute_width_ignores_disallowed_percent_height() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: Some("120".to_string()),
            height_attr: Some("50%".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 20.0,
                height: 10.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (90.0, 45.0)
        );
    }

    #[test]
    fn svg_size_absolute_height_ignores_disallowed_percent_width() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: Some("50%".to_string()),
            height_attr: Some("60".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 20.0,
                height: 10.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, false, false),
            (90.0, 45.0)
        );
    }

    #[test]
    fn svg_size_intrinsic_is_not_clamped_to_available_width() {
        let tree = SvgTree {
            width: 300.0,
            height: 150.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 200.0, 400.0, false, false),
            (300.0, 150.0)
        );
    }

    #[test]
    fn svg_size_negative_percent_falls_back_to_intrinsic_size() {
        let tree = SvgTree {
            width: 120.0,
            height: 60.0,
            width_attr: Some("-10%".to_string()),
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };

        assert_eq!(
            resolve_svg_size(&tree, 400.0, 400.0, true, false),
            (120.0, 60.0) // falls back to intrinsic size (already in pt)
        );
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
    fn constrain_replaced_image_size_within_available_width() {
        // Image 200x100 in 150 available width => scale down to 150x75
        let (w, h) = constrain_replaced_image_size(200.0, 100.0, 150.0, None, None);
        assert!((w - 150.0).abs() < 0.01);
        assert!((h - 75.0).abs() < 0.01);
    }

    #[test]
    fn constrain_replaced_image_size_with_max_width() {
        // Image 200x100, available 300, max_width 100 => scale to 100x50
        let (w, h) = constrain_replaced_image_size(200.0, 100.0, 300.0, Some(100.0), None);
        assert!((w - 100.0).abs() < 0.01);
        assert!((h - 50.0).abs() < 0.01);
    }

    #[test]
    fn constrain_replaced_image_size_with_max_height() {
        // Image 200x100, max_height 40 => scale to 80x40
        let (w, h) = constrain_replaced_image_size(200.0, 100.0, 500.0, None, Some(40.0));
        assert!((w - 80.0).abs() < 0.01);
        assert!((h - 40.0).abs() < 0.01);
    }

    #[test]
    fn constrain_replaced_image_size_zero_dimensions() {
        // Zero width/height should return (0, 0)
        let (w, h) = constrain_replaced_image_size(0.0, 100.0, 500.0, None, None);
        assert_eq!(w, 0.0);
        assert_eq!(h, 100.0);
    }

    #[test]
    fn constrain_replaced_image_size_no_scaling_needed() {
        // Image fits within available width, no max constraints
        let (w, h) = constrain_replaced_image_size(100.0, 50.0, 500.0, None, None);
        assert_eq!(w, 100.0);
        assert_eq!(h, 50.0);
    }

    #[test]
    fn percent_decode_basic() {
        assert_eq!(percent_decode("%3Csvg%3E"), "<svg>");
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("no%encoding"), "no%encoding");
    }

    #[test]
    fn parse_html_image_dimension_with_px_suffix() {
        assert_eq!(
            parse_html_image_dimension(Some(&"200px".to_string())),
            Some(150.0) // 200 * 0.75
        );
    }

    #[test]
    fn parse_html_image_dimension_without_suffix() {
        assert_eq!(
            parse_html_image_dimension(Some(&"100".to_string())),
            Some(75.0) // 100 * 0.75
        );
    }

    #[test]
    fn parse_html_image_dimension_none_input() {
        assert_eq!(parse_html_image_dimension(None), None);
    }

    #[test]
    fn parse_html_image_dimension_invalid() {
        assert_eq!(parse_html_image_dimension(Some(&"abc".to_string())), None);
    }

    #[test]
    fn svg_natural_ratio_from_viewbox() {
        let vb = crate::parser::svg::ViewBox {
            min_x: 0.0,
            min_y: 0.0,
            width: 200.0,
            height: 100.0,
        };
        let ratio = svg_natural_ratio(None, None, None, None, Some(vb));
        assert!((ratio.unwrap() - 0.5).abs() < 0.001);
    }

    #[test]
    fn svg_natural_ratio_from_explicit_dimensions() {
        let ratio = svg_natural_ratio(Some(100.0), Some(50.0), None, None, None);
        assert!((ratio.unwrap() - 0.5).abs() < 0.001);
    }

    #[test]
    fn contain_default_object_size_tall_ratio() {
        // ratio > default_ratio (0.5): height-constrained
        let (w, h) = contain_default_object_size(2.0);
        assert!((h - 150.0).abs() < 0.01);
        assert!((w - 75.0).abs() < 0.01);
    }

    #[test]
    fn contain_default_object_size_wide_ratio() {
        // ratio < default_ratio (0.5): width-constrained
        let (w, h) = contain_default_object_size(0.25);
        assert!((w - 300.0).abs() < 0.01);
        assert!((h - 75.0).abs() < 0.01);
    }
}
