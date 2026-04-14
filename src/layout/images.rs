use crate::parser::dom::ElementNode;
use crate::parser::png;
use crate::parser::ttf::TtfFont;
use crate::style::computed::{ComputedStyle, Display, FontStyle, FontWeight, VerticalAlign};
use crate::util::decode_base64;
use std::collections::HashMap;

use super::engine::{ImageFormat, LayoutElement, PngMetadata, RasterImageAsset};
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

/// Probe raw bytes for SVG content and parse into an `SvgTree`.
///
/// Uses a heuristic on the first 512 bytes (via `String::from_utf8_lossy` so
/// that non-UTF-8 binary content is safely rejected) and then parses the full
/// content through the HTML parser to extract the `<svg>` element.
pub(crate) fn try_parse_svg_bytes(raw: &[u8]) -> Option<crate::parser::svg::SvgTree> {
    // Heuristic: check if the content looks like SVG (XML with an <svg element).
    let prefix = if raw.len() > 512 { &raw[..512] } else { raw };
    let text = String::from_utf8_lossy(prefix);
    let trimmed = text.trim_start_matches('\u{FEFF}').trim_start();
    let trimmed_lower = trimmed.to_ascii_lowercase();
    if !(trimmed.starts_with("<svg")
        || trimmed.starts_with("<?xml")
        || trimmed.starts_with("<!--")
        || trimmed_lower.starts_with("<!doctype"))
    {
        return None;
    }
    // For the comment case, search the full content (comments may exceed the
    // 512-byte prefix before the <svg> tag appears).
    if trimmed.starts_with("<!--") {
        let full_text = String::from_utf8_lossy(raw);
        if !full_text.contains("<svg") {
            return None;
        }
    }

    // Parse the full SVG content — use lossy conversion so that stray non-UTF-8
    // bytes don't cause the whole parse to fail.
    let svg_str = String::from_utf8_lossy(raw);
    crate::parser::svg::parse_svg_from_string(&svg_str)
}

/// Detect PNG/JPEG format and return a raster asset with source dimensions.
pub(crate) fn load_image_bytes(raw: Vec<u8>) -> Option<RasterImageAsset> {
    if png::is_png(&raw) {
        let png_info = png::parse_png(&raw)?;
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

        sync_svg_tree_to_layout_box(&mut tree, width, height);
        return Some(LayoutElement::Svg {
            tree,
            width,
            height,
            flow_extra_bottom: 0.0,
            margin_top: style.margin.top,
            margin_bottom: style.margin.bottom,
        });
    }

    // Fall back to raster image using the same bytes.
    let image = load_raster_image_bytes(raw, style.blur_radius)?;

    // Determine dimensions from attributes
    let attr_width = parse_html_image_dimension(el.attributes.get("width"));
    let attr_height = parse_html_image_dimension(el.attributes.get("height"));

    let (width, height) = match (attr_width, attr_height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w), // fallback: square
        (None, Some(h)) => (h, h),
        (None, None) => (available_width.min(200.0), 150.0),
    };

    let (width, height) = constrain_replaced_image_size(
        width,
        height,
        available_width,
        style.max_width,
        style.max_height,
    );

    Some(LayoutElement::Image {
        image,
        width,
        height,
        flow_extra_bottom: 0.0,
        margin_top: style.margin.top,
        margin_bottom: style.margin.bottom,
    })
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
        } => LayoutElement::Image {
            image,
            width,
            height,
            flow_extra_bottom: flow_extra_bottom + baseline_gap,
            margin_top,
            margin_bottom,
        },
        LayoutElement::Svg {
            tree,
            width,
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
        } => LayoutElement::Svg {
            tree,
            width,
            height,
            flow_extra_bottom: flow_extra_bottom + baseline_gap,
            margin_top,
            margin_bottom,
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

pub(crate) fn load_raster_image_bytes(raw: Vec<u8>, blur_radius: f32) -> Option<RasterImageAsset> {
    if blur_radius > 0.0 {
        blur_image_bytes(&raw, blur_radius)
    } else {
        load_image_bytes(raw)
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
