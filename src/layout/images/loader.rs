use crate::layout::elements::{
    Image, ImagePaint, ImageSampling, IntoLayoutNode, LayoutNode, Positioning, ReplacedGeometry,
    Svg, SvgPaint,
};
use crate::layout::engine::{ImageFormat, LayoutBorder, PngMetadata, RasterImageAsset};
use crate::layout::flow_metrics::BlockMargins;
use crate::parser::dom::ElementNode;
use crate::parser::png;
use crate::style::computed::ComputedStyle;
use crate::types::Size;
use crate::util::decode_base64;

use super::placement::{ReplacedBoxSize, parse_html_image_dimension};
use super::raster::decode_png_to_rgb_asset;
use super::source::{fetch_remote_url, percent_decode};
use super::svg::{resolve_svg_size, sync_svg_tree_to_layout_box};
use crate::security::resources::{DocumentResources, ResourceAccess};

/// Per-conversion capability that authorises a document resource reference
/// against the policy and loads (once, memoised) its bytes. It is the single
/// gate through which document-referenced bytes enter the process: callers hold
/// a `ResourceLoader` instead of calling [`load_src_bytes`] directly, so every
/// load is authorised and cached at one place.
/// Loaded resource bytes plus the optional MIME reported by the source.
pub(crate) type LoadedResource = (std::sync::Arc<Vec<u8>>, Option<String>);

pub(crate) struct ResourceLoader {
    policy: DocumentResources,
    /// Memoised loads keyed by the resolved (authorised) reference. `None`
    /// records a denied or failed reference so it is not retried per repaint.
    cache: std::cell::RefCell<std::collections::HashMap<String, Option<LoadedResource>>>,
}

impl ResourceLoader {
    pub(crate) fn new(policy: DocumentResources) -> Self {
        Self {
            policy,
            cache: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }

    /// A loader that authorises any local or network reference (Trusted, no
    /// root). Test-only: standalone rendering/layout of local-file content that
    /// does not go through a policy-bearing `HtmlConverter` conversion.
    #[cfg(test)]
    pub(crate) fn trusted() -> Self {
        Self::new(DocumentResources::new(ResourceAccess::Trusted, None, None))
    }

    /// Authorise `reference` against the policy (returning `None` when denied),
    /// then load its bytes. File/URL loads are memoised; inline `data:`
    /// references are not (they decode to unique bytes that are rarely repeated,
    /// and caching them would key the whole multi-KB URI for no reuse). `base`
    /// is the directory relative references resolve against.
    pub(crate) fn load(
        &self,
        reference: &str,
        base: Option<&std::path::Path>,
    ) -> Option<LoadedResource> {
        let resolved = self.policy.resolve(reference, base)?;
        if is_inline_reference(&resolved) {
            return load_src_bytes(&resolved).map(|(bytes, mime)| (std::sync::Arc::new(bytes), mime));
        }
        if let Some(hit) = self.cache.borrow().get(&resolved) {
            return hit.clone();
        }
        let loaded = self.load_resolved(&resolved);
        self.cache.borrow_mut().insert(resolved, loaded.clone());
        loaded
    }

    /// Load an already-authorised (file or network) reference. Network fetches
    /// additionally pass the document's [`NetworkPolicy`] so the SSRF controls
    /// (deny/allow lists, IP-class rejection) apply at the point of connection.
    fn load_resolved(&self, resolved: &str) -> Option<LoadedResource> {
        if is_network_reference(resolved) {
            return fetch_remote_url(resolved, self.policy.network())
                .map(|bytes| (std::sync::Arc::new(bytes), None));
        }
        load_src_bytes(resolved).map(|(bytes, mime)| (std::sync::Arc::new(bytes), mime))
    }
}

impl Default for ResourceLoader {
    /// Deny-by-default: only inline `data:`/fragment references resolve; local
    /// files and network are refused. Used where no document policy is supplied.
    fn default() -> Self {
        Self::new(DocumentResources::new(ResourceAccess::Sanitized, None, None))
    }
}

/// A `data:` reference: its bytes are self-contained, so loading is cheap and
/// caching would key the entire URI for no reuse.
fn is_inline_reference(reference: &str) -> bool {
    reference
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"))
}

thread_local! {
    /// The ambient resource loader for the current conversion. A stack so that a
    /// conversion nested on the same thread restores the outer loader on exit.
    static CURRENT_LOADER: std::cell::RefCell<Vec<std::rc::Rc<ResourceLoader>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Installs `loader` as the ambient loader for the current thread until the
/// returned guard is dropped. Every resource sink loads through the ambient
/// loader, so authorization/caching is configured once per conversion rather
/// than threaded through layout and render.
#[must_use = "the ambient loader is only active while the guard is alive"]
pub(crate) fn enter_loader(loader: std::rc::Rc<ResourceLoader>) -> LoaderScope {
    CURRENT_LOADER.with(|stack| stack.borrow_mut().push(loader));
    LoaderScope { _private: () }
}

/// Install a trusted ambient loader for tests that render/lay out content
/// referencing local files directly (outside a policy-bearing conversion).
#[cfg(test)]
pub(crate) fn trusted_scope() -> LoaderScope {
    enter_loader(std::rc::Rc::new(ResourceLoader::trusted()))
}

/// RAII guard that pops the ambient loader it installed.
pub(crate) struct LoaderScope {
    _private: (),
}

impl Drop for LoaderScope {
    fn drop(&mut self) {
        CURRENT_LOADER.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Load a document resource through the ambient loader. When no loader is
/// installed the load is denied by default (only inline `data:` resolves), so a
/// stray load outside a conversion cannot reach the filesystem or network.
pub(crate) fn load_resource(
    reference: &str,
    base: Option<&std::path::Path>,
) -> Option<LoadedResource> {
    let loader = CURRENT_LOADER.with(|stack| stack.borrow().last().cloned());
    match loader {
        Some(loader) => loader.load(reference, base),
        None => ResourceLoader::default().load(reference, base),
    }
}

/// Load raw bytes from a `src` attribute value.
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
    } else if is_network_reference(src) {
        // Network loads are authorised and fetched by ResourceLoader::load_resolved.
        None
    } else {
        Some((std::fs::read(src).ok()?, None))
    }
}

/// An `http`/`https` reference (fetched, subject to the network policy).
fn is_network_reference(reference: &str) -> bool {
    crate::security::resources::is_network_url(reference)
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
        // The final PDF writer extracts raw IDAT for PDF FlateDecode embedding,
        // but the layout asset keeps the complete PNG so later optimization
        // stages can decode and resize it before embedding.
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
            return Some(RasterImageAsset::source(
                raw,
                png_info.width,
                png_info.height,
                ImageFormat::PngAlpha,
                None,
            ));
        }
        let metadata = PngMetadata {
            channels: png_info.channels,
            bit_depth: png_info.bit_depth,
        };
        Some(RasterImageAsset::source(
            raw,
            png_info.width,
            png_info.height,
            ImageFormat::Png,
            Some(metadata),
        ))
    } else if raw.starts_with(&[0xFF, 0xD8]) {
        let (source_width, source_height) = crate::parser::jpeg::parse_jpeg_dimensions(&raw)?;
        Some(RasterImageAsset::source(
            raw,
            source_width,
            source_height,
            ImageFormat::Jpeg,
            None,
        ))
    } else {
        None
    }
}

/// Load image data from an <img> element and return a LayoutElement.
///
/// Bytes are fetched exactly once from the source.  When the content is SVG it
/// is parsed as vector graphics ([`Svg`]); otherwise it falls back to a raster
/// PNG/JPEG [`Image`].
pub(crate) fn load_image_from_element(
    el: &ElementNode,
    available_width: f32,
    available_height: f32,
    style: &ComputedStyle,
    _filter_dpi: f32,
) -> Option<LayoutNode> {
    let src = el.attributes.get("src")?;

    // Load bytes once.
    let (raw, mime) = load_resource(src, None)?;

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

        let (width, height) = ReplacedBoxSize::new(
            width,
            height,
            html_attr_width.is_none(),
            html_attr_height.is_none(),
        )
        .constrain(available_width, style.max_width, style.max_height)
        .dimensions();

        let border = LayoutBorder::from_computed(&style.border, style.color);
        let content_width = (width - border.horizontal_width()).max(0.0);
        let content_height = (height - border.vertical_width()).max(0.0);
        sync_svg_tree_to_layout_box(&mut tree, content_width, content_height);
        return Some(
            Svg {
                tree,
                geometry: ReplacedGeometry::new(
                    Size::new(width, height),
                    BlockMargins::new(style.margin.top, style.margin.bottom),
                    border,
                ),
                positioning: Positioning::from_style(style),
                paint: SvgPaint {
                    background_color: style.background_color,
                    border_image: style.border_image.paint(),
                    border_radii: style.resolve_corner_radii(width, height),
                    group: crate::layout::elements::PaintGroup::from_style(style),
                },
                replaced: crate::layout::engine::ReplacedContent {
                    object_fit: style.object_fit,
                    object_position: style.object_position,
                    ..Default::default()
                },
            }
            .boxed(),
        );
    }

    // The filter property applies to the complete replaced-element
    // SourceGraphic, including its background and border. Keep image loading
    // unfiltered; the shared post-layout filter compositor owns the operation
    // list for every element kind.
    let image = load_image_bytes(raw.to_vec())?;

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

    let (width, height) =
        ReplacedBoxSize::new(width, height, attr_width.is_none(), attr_height.is_none())
            .constrain(available_width, style.max_width, style.max_height)
            .dimensions();

    Some(
        Image {
            source: image,
            geometry: ReplacedGeometry::new(
                Size::new(width, height),
                BlockMargins::new(style.margin.top, style.margin.bottom),
                LayoutBorder::from_computed(&style.border, style.color),
            ),
            positioning: Positioning::from_style(style),
            sampling: ImageSampling {
                replaced: crate::layout::engine::ReplacedContent {
                    object_fit: style.object_fit,
                    object_position: style.object_position,
                    ..Default::default()
                },
                rendering: style.image_rendering,
            },
            paint: ImagePaint {
                background_color: style.background_color,
                border_image: style.border_image.paint(),
                border_radii: style.resolve_corner_radii(width, height),
                filter_effect: None,
                group: crate::layout::elements::PaintGroup::from_style(style),
                ..Default::default()
            },
        }
        .boxed(),
    )
}

#[cfg(test)]
mod loader_tests {
    use super::*;

    #[test]
    fn cache_key_is_the_resolved_path_so_a_relative_ref_never_serves_the_wrong_base() {
        let root = std::env::temp_dir().join(format!("ip-loader-{}", std::process::id()));
        let a = root.join("a");
        let b = root.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("x"), b"AAAA").unwrap();
        std::fs::write(b.join("x"), b"BBBB").unwrap();

        let loader = ResourceLoader::new(DocumentResources::new(
            ResourceAccess::Sanitized,
            None,
            Some(root.as_path()),
        ));

        let from_a = loader.load("x", Some(a.as_path())).expect("a/x authorised").0;
        let from_b = loader.load("x", Some(b.as_path())).expect("b/x authorised").0;
        assert_eq!(&from_a[..], b"AAAA");
        assert_eq!(
            &from_b[..],
            b"BBBB",
            "a different base must resolve to a different file, not a stale cache hit"
        );
        assert_eq!(&loader.load("x", Some(a.as_path())).unwrap().0[..], b"AAAA");

        let _ = std::fs::remove_dir_all(&root);
    }
}
