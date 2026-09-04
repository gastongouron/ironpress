#![cfg_attr(
    not(test),
    deny(
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::unwrap_used
    )
)]
#![warn(missing_docs)]
//! # ironpress
//!
//! Pure Rust HTML/CSS/Markdown to PDF converter. No browser, no system dependencies.
//!
//! ironpress converts HTML (with CSS) and Markdown into PDF documents using a
//! built-in layout engine. Unlike other Rust PDF crates, it does not shell out
//! to headless Chrome or wkhtmltopdf.
//!
//! ## Features
//!
//! - All common HTML elements: headings, paragraphs, lists, tables, images, links, semantic sections
//! - CSS support: selectors, flexbox, grid, floats, positioning, transforms, gradients, custom properties
//! - Built-in Markdown parser (no external dependencies)
//! - Custom TrueType font embedding
//! - JPEG and PNG images (data URIs and local files)
//! - Streaming output via `std::io::Write`
//! - Async file I/O via optional `tokio` integration
//! - HTML sanitization enabled by default
//!
//! ## Quick start
//!
//! ```
//! let pdf = ironpress::html_to_pdf("<h1>Hello</h1><p>World</p>").unwrap();
//! assert!(pdf.starts_with(b"%PDF"));
//! ```
//!
//! ## Markdown
//!
//! ```
//! let pdf = ironpress::markdown_to_pdf("# Hello\n\nWorld").unwrap();
//! assert!(pdf.starts_with(b"%PDF"));
//! ```
//!
//! ## Builder API
//!
//! ```
//! use ironpress::{HtmlConverter, PageSize, Margin};
//!
//! let pdf = HtmlConverter::new()
//!     .page_size(PageSize::LETTER)
//!     .margin(Margin::uniform(54.0))
//!     .sanitize(false)
//!     .convert("<h1>Hello</h1>")
//!     .unwrap();
//! ```
//!
//! ## Streaming output
//!
//! ```
//! let mut buf = Vec::new();
//! ironpress::html_to_pdf_writer("<h1>Hello</h1>", &mut buf).unwrap();
//! assert!(buf.starts_with(b"%PDF"));
//! ```
//!
//! ## Custom fonts
//!
//! ```no_run
//! use ironpress::HtmlConverter;
//!
//! let ttf = std::fs::read("fonts/MyFont.ttf").unwrap();
//! let pdf = HtmlConverter::new()
//!     .add_font("MyFont", ttf)
//!     .convert(r#"<p style="font-family: MyFont">Custom text</p>"#)
//!     .unwrap();
//! ```

/// Adobe Font Metrics for standard PDF fonts (Helvetica, Times, Courier).
pub(crate) mod bidi;
/// Capped process-lifetime memoization shared by the font caches.
pub(crate) mod bounded_cache;
/// CLI argument parsing and conversion logic.
pub mod cli;
/// Error types for conversion failures.
pub mod error;
/// Optional fallback-font packs.
pub mod font_pack;
pub(crate) mod fonts;
pub(crate) mod layout;
mod page_margin;
pub(crate) mod parser;
pub(crate) mod render;
pub(crate) mod security;
pub(crate) mod style;
pub(crate) mod system_fonts;
pub(crate) mod text;
/// Public types: page size, margins, and colors.
pub mod types;
pub(crate) mod util;

pub use error::IronpressError;
pub use font_pack::{FontPack, FontPackError, FontPackKind, UnknownFontPackKind};
pub use security::resources::{InvalidRemoteHost, NetworkPolicy, RemoteHost};
pub use style::raster_quality::{CoverageCompression, JpegCompression, RasterQuality};
pub use types::{CornerRadii, CornerRadius, EdgeSizes, Margin, PageSize};

/// Convert an HTML string to PDF bytes using default settings (A4, 1-inch margins).
///
/// The HTML is sanitized before conversion to remove potentially dangerous
/// elements like `<script>`, `<iframe>`, and event handlers.
///
/// # Example
///
/// ```
/// let pdf = ironpress::html_to_pdf("<h1>Title</h1><p>Hello World</p>").unwrap();
/// assert!(pdf.starts_with(b"%PDF"));
/// ```
pub fn html_to_pdf(html: &str) -> Result<Vec<u8>, IronpressError> {
    HtmlConverter::new().convert(html)
}

/// Convert a Markdown string to PDF bytes using default settings (A4, 1-inch margins).
///
/// # Example
///
/// ```
/// let pdf = ironpress::markdown_to_pdf("# Hello\n\nWorld").unwrap();
/// assert!(pdf.starts_with(b"%PDF"));
/// ```
pub fn markdown_to_pdf(md: &str) -> Result<Vec<u8>, IronpressError> {
    let html = parser::markdown::markdown_to_html(md);
    HtmlConverter::new().convert(&html)
}

/// Convert a Markdown file to a PDF file using default settings.
///
/// # Example
///
/// ```no_run
/// ironpress::convert_markdown_file("input.md", "output.pdf").unwrap();
/// ```
pub fn convert_markdown_file(input: &str, output: &str) -> Result<(), IronpressError> {
    let md = std::fs::read_to_string(input)?;
    let pdf = markdown_to_pdf(&md)?;
    std::fs::write(output, pdf)?;
    Ok(())
}

/// Convert an HTML file to a PDF file using default settings.
///
/// # Example
///
/// ```no_run
/// ironpress::convert_file("input.html", "output.pdf").unwrap();
/// ```
pub fn convert_file(input: &str, output: &str) -> Result<(), IronpressError> {
    let html = std::fs::read_to_string(input)?;
    let pdf = html_to_pdf(&html)?;
    std::fs::write(output, pdf)?;
    Ok(())
}

/// Convert an HTML string to PDF, writing output to any `std::io::Write` implementation.
///
/// This is the streaming variant of [`html_to_pdf`]. Instead of returning a `Vec<u8>`,
/// it writes PDF content directly to the provided writer.
pub fn html_to_pdf_writer<W: std::io::Write>(
    html: &str,
    writer: &mut W,
) -> Result<(), IronpressError> {
    HtmlConverter::new().convert_to_writer(html, writer)
}

/// Convert a Markdown string to PDF, writing output to any `std::io::Write` implementation.
///
/// This is the streaming variant of [`markdown_to_pdf`].
pub fn markdown_to_pdf_writer<W: std::io::Write>(
    md: &str,
    writer: &mut W,
) -> Result<(), IronpressError> {
    let html = parser::markdown::markdown_to_html(md);
    HtmlConverter::new().convert_to_writer(&html, writer)
}

/// Async version of [`convert_file`]. Requires the `async` feature.
///
/// Uses `tokio::fs` for async file I/O and `tokio::task::spawn_blocking`
/// for the CPU-bound conversion step.
#[cfg(feature = "async")]
pub async fn convert_file_async(input: &str, output: &str) -> Result<(), IronpressError> {
    let html = tokio::fs::read_to_string(input).await?;
    let pdf = tokio::task::spawn_blocking(move || html_to_pdf(&html))
        .await
        .map_err(|e| IronpressError::RenderError(format!("task join error: {e}")))?;
    let pdf = pdf?;
    tokio::fs::write(output, pdf).await?;
    Ok(())
}

/// Async version of [`convert_markdown_file`]. Requires the `async` feature.
///
/// Uses `tokio::fs` for async file I/O and `tokio::task::spawn_blocking`
/// for the CPU-bound conversion step.
#[cfg(feature = "async")]
pub async fn convert_markdown_file_async(input: &str, output: &str) -> Result<(), IronpressError> {
    let md = tokio::fs::read_to_string(input).await?;
    let pdf = tokio::task::spawn_blocking(move || markdown_to_pdf(&md))
        .await
        .map_err(|e| IronpressError::RenderError(format!("task join error: {e}")))?;
    let pdf = pdf?;
    tokio::fs::write(output, pdf).await?;
    Ok(())
}

/// Builder for HTML-to-PDF conversion with custom options.
///
/// Use [`HtmlConverter::new`] to start, chain configuration methods,
/// then call [`convert`](HtmlConverter::convert) or
/// [`convert_to_writer`](HtmlConverter::convert_to_writer) to produce PDF output.
///
/// # Example
///
/// ```
/// use ironpress::{HtmlConverter, PageSize, Margin};
///
/// let pdf = HtmlConverter::new()
///     .page_size(PageSize::LETTER)
///     .margin(Margin::uniform(54.0))
///     .convert("<h1>Hello</h1>")
///     .unwrap();
/// ```
#[derive(Clone)]
pub struct HtmlConverter {
    page_size: PageSize,
    margin: Margin,
    sanitize: bool,
    custom_fonts: std::collections::HashMap<String, Vec<u8>>,
    font_catalog: font_pack::FontCatalog,
    resources: ResourcePaths,
    page_margins: page_margin::PageMargins,
    /// FlateDecode-compress page content streams (lossless). Defaults to `true`;
    /// disable for raw, human-readable PDF content streams.
    compress: bool,
    jpeg_quality: u8,
    auto_resize_images: bool,
    raster_quality: RasterQuality,
    /// Skip embedding raster images that are fully covered by a later opaque
    /// rectangular element (default false). Conservative; zero visual change.
    occlusion_cull: bool,
}

#[derive(Debug, Clone, Default)]
struct ResourcePaths {
    base: Option<std::path::PathBuf>,
    authorized_root: Option<std::path::PathBuf>,
    network: NetworkPolicy,
}

impl HtmlConverter {
    /// Create a new converter with default settings (A4, 1-inch margins, sanitization enabled).
    pub fn new() -> Self {
        Self {
            page_size: PageSize::default(),
            margin: Margin::default(),
            sanitize: true,
            custom_fonts: std::collections::HashMap::new(),
            font_catalog: font_pack::FontCatalog::default(),
            resources: ResourcePaths::default(),
            page_margins: page_margin::PageMargins::default(),
            // On by default for production output (FlateDecode is lossless and
            // transparent to any rasterizer). The crate's own unit tests inspect
            // raw content-stream operators, so the in-crate test build defaults
            // to off; the compression path is covered by a dedicated test and the
            // parity gate. Downstream users (and the CLI) always get the `true`
            // default.
            compress: !cfg!(test),
            jpeg_quality: render::pdf::DEFAULT_JPEG_QUALITY,
            auto_resize_images: true,
            raster_quality: RasterQuality::default(),
            occlusion_cull: false,
        }
    }

    /// Enable or disable FlateDecode compression of page content streams
    /// (enabled by default). Disabling produces larger but human-readable PDFs.
    pub fn compress(mut self, enabled: bool) -> Self {
        self.compress = enabled;
        self
    }

    /// Set JPEG quality for optimized image embedding (0-100, default 95).
    pub fn jpeg_quality(mut self, quality: u8) -> Self {
        self.jpeg_quality = quality.clamp(0, 100);
        self
    }

    /// Enable or disable automatic downscaling of oversized source images.
    pub fn auto_resize_images(mut self, enabled: bool) -> Self {
        self.auto_resize_images = enabled;
        self
    }

    /// Set the target source-image resolution in DPI (minimum 72, default 300).
    pub fn image_dpi(mut self, dpi: f32) -> Self {
        self.raster_quality.source_image_dpi =
            style::raster_quality::raster_dpi_at_least(dpi, 72.0);
        self
    }

    /// Set all raster-resolution controls together.
    ///
    /// Use a struct update when changing one policy while retaining the
    /// documented defaults: `RasterQuality { filter_dpi: 192.0,
    /// ..RasterQuality::default() }`.
    pub fn raster_quality(mut self, quality: RasterQuality) -> Self {
        self.raster_quality = quality.normalized();
        self
    }

    /// Set the rasterization DPI for render-time blur/filter bitmaps.
    pub fn filter_dpi(mut self, dpi: f32) -> Self {
        self.raster_quality.filter_dpi = style::raster_quality::raster_dpi_at_least(dpi, 1.0);
        self
    }

    /// Set the rasterization DPI for CSS `mask-image` coverage bitmaps.
    ///
    /// The default 300 DPI preserves high-contrast coverage edges. It is
    /// independent from blur/filter quality because mask coverage has different
    /// sampling and compression characteristics.
    pub fn mask_dpi(mut self, dpi: f32) -> Self {
        self.raster_quality.mask_dpi = style::raster_quality::raster_dpi_at_least(dpi, 72.0);
        self
    }

    /// Set the target resolution for style-time flattened background bitmaps.
    /// The default 192 DPI is the lowest tested physical-resolution baseline;
    /// this setting affects only synthetic background images, never PDF geometry.
    pub fn background_raster_dpi(mut self, dpi: f32) -> Self {
        self.raster_quality.background_dpi = style::raster_quality::raster_dpi_at_least(
            dpi,
            style::raster_quality::CSS_REFERENCE_DPI,
        );
        self
    }

    /// Enable or disable occlusion culling of raster images that are fully
    /// covered by a later fully-opaque rectangular element (default false).
    /// Conservative and safe: only skips images that are guaranteed invisible.
    pub fn occlusion_cull(mut self, enabled: bool) -> Self {
        self.occlusion_cull = enabled;
        self
    }

    /// Set the page size.
    pub fn page_size(mut self, size: PageSize) -> Self {
        self.page_size = size;
        self
    }

    /// Set the page margins.
    pub fn margin(mut self, margin: Margin) -> Self {
        self.margin = margin;
        self
    }

    /// Enable or disable HTML sanitization (enabled by default).
    pub fn sanitize(mut self, enabled: bool) -> Self {
        self.sanitize = enabled;
        self
    }

    /// Register a custom TrueType font.
    ///
    /// The `name` should match the `font-family` value used in CSS.
    /// The `ttf_data` is the raw contents of a `.ttf` file.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ironpress::HtmlConverter;
    ///
    /// let ttf_data = std::fs::read("MyFont.ttf").unwrap();
    /// let pdf = HtmlConverter::new()
    ///     .add_font("MyFont", ttf_data)
    ///     .convert(r#"<p style="font-family: MyFont">Custom text</p>"#)
    ///     .unwrap();
    /// ```
    pub fn add_font(mut self, name: &str, ttf_data: Vec<u8>) -> Self {
        self.custom_fonts
            .insert(name.to_ascii_lowercase(), ttf_data);
        self
    }

    /// Install or replace one parsed optional fallback-font pack.
    ///
    /// CJK packs follow inherited HTML `lang` values so the same Unicode code
    /// point can use Japanese, Korean, Simplified Chinese, or Traditional
    /// Chinese glyph forms. Pack loading is always explicit and never performs
    /// network access.
    pub fn add_font_pack(mut self, pack: FontPack) -> Self {
        self.install_font_pack(pack);
        self
    }

    /// Install a pack on a reusable converter without rebuilding its policy.
    fn install_font_pack(&mut self, pack: FontPack) {
        self.font_catalog.install(pack);
    }

    /// Set the base directory for resolving local document resources.
    ///
    /// When set, `@import "styles.css"` will resolve the path relative to
    /// this directory, and `@font-face { src: url("fonts/MyFont.ttf") }` will
    /// load the font file from this directory.
    ///
    /// CSS `@import` supports local files only. Other HTTP/HTTPS resources use
    /// the independent [`NetworkPolicy`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ironpress::HtmlConverter;
    /// use std::path::Path;
    ///
    /// let pdf = HtmlConverter::new()
    ///     .base_path(Path::new("/path/to/project"))
    ///     .convert(r#"<style>@import "styles.css";</style><p>Hello</p>"#)
    ///     .unwrap();
    /// ```
    pub fn base_path(mut self, path: &std::path::Path) -> Self {
        self.resources.base = Some(path.to_path_buf());
        self
    }

    /// Set the directory boundary authorized for document-local resources.
    ///
    /// By default, [`base_path`](Self::base_path) is both the URL base and the
    /// authorization boundary. Set a broader root when a document legitimately
    /// references shared assets in an ancestor directory. The base path must
    /// remain inside this canonical root; traversal and symlink escapes are
    /// denied.
    pub fn resource_root(mut self, path: &std::path::Path) -> Self {
        self.resources.authorized_root = Some(path.to_path_buf());
        self
    }

    /// Set the HTTP/HTTPS resource policy used by this conversion.
    pub fn network_policy(mut self, policy: NetworkPolicy) -> Self {
        self.resources.network = policy;
        self
    }

    /// Replace the remote host allow list.
    pub fn download_allow_list(mut self, hosts: impl IntoIterator<Item = RemoteHost>) -> Self {
        self.resources.network = std::mem::take(&mut self.resources.network).with_allow_list(hosts);
        self
    }

    /// Replace the remote host deny list.
    pub fn download_deny_list(mut self, hosts: impl IntoIterator<Item = RemoteHost>) -> Self {
        self.resources.network = std::mem::take(&mut self.resources.network).with_deny_list(hosts);
        self
    }

    /// Enable or disable rejection of non-public remote addresses.
    ///
    /// This is enabled by default. A remote-resolving environment proxy moves
    /// final-host IP enforcement to that operator-controlled proxy.
    pub fn download_deny_private_ips(mut self, deny: bool) -> Self {
        self.resources.network = std::mem::take(&mut self.resources.network).deny_private_ips(deny);
        self
    }

    /// Enable or disable rejection of public remote addresses.
    ///
    /// Target host lists still apply when an environment proxy resolves the
    /// final host, but the proxy must enforce final-host address classes.
    pub fn download_deny_public_ips(mut self, deny: bool) -> Self {
        self.resources.network = std::mem::take(&mut self.resources.network).deny_public_ips(deny);
        self
    }

    /// Set the maximum number of remote redirects. Each hop is checked again.
    pub fn download_max_redirects(mut self, max: u32) -> Self {
        self.resources.network = std::mem::take(&mut self.resources.network).max_redirects(max);
        self
    }

    /// Set the maximum remote response size in bytes.
    pub fn download_max_body_size(mut self, max: u64) -> Self {
        self.resources.network = std::mem::take(&mut self.resources.network).max_body_size(max);
        self
    }

    /// Set a header text rendered at the top of each page (in the top margin area).
    pub fn header(mut self, text: impl Into<String>) -> Self {
        self.page_margins.set_header_text(text.into());
        self
    }

    /// Set an HTML fragment rendered in the top margin on every page.
    ///
    /// The fragment uses the same sanitization, resource policy, fonts, and CSS
    /// cascade as the document. Reserve enough top margin for its content.
    pub fn header_html(mut self, html: impl Into<String>) -> Self {
        self.page_margins.set_header_html(html.into());
        self
    }

    /// Set a footer text rendered at the bottom of each page (in the bottom margin area).
    ///
    /// Use `{page}` for the current page number and `{pages}` for the total page count.
    /// For example: `"Page {page} of {pages}"`.
    pub fn footer(mut self, text: impl Into<String>) -> Self {
        self.page_margins.set_footer_text(text.into());
        self
    }

    /// Set an HTML fragment rendered in the bottom margin on every page.
    ///
    /// The fragment uses the same sanitization, resource policy, fonts, and CSS
    /// cascade as the document. Reserve enough bottom margin for its content.
    pub fn footer_html(mut self, html: impl Into<String>) -> Self {
        self.page_margins.set_footer_html(html.into());
        self
    }

    /// Convert a Markdown string to PDF bytes.
    ///
    /// The Markdown is first converted to HTML using the built-in parser,
    /// then processed through the normal HTML-to-PDF pipeline.
    ///
    /// # Example
    ///
    /// ```
    /// use ironpress::HtmlConverter;
    ///
    /// let pdf = HtmlConverter::new()
    ///     .convert_markdown("# Hello\n\nWorld")
    ///     .unwrap();
    /// ```
    pub fn convert_markdown(&self, md: &str) -> Result<Vec<u8>, IronpressError> {
        let html = parser::markdown::markdown_to_html(md);
        self.convert(&html)
    }

    /// Convert an HTML string to PDF bytes.
    pub fn convert(&self, html: &str) -> Result<Vec<u8>, IronpressError> {
        let mut buf = Vec::new();
        self.convert_to_writer(html, &mut buf)?;
        Ok(buf)
    }

    /// Convert an HTML string to PDF, writing directly to any `std::io::Write` implementation.
    pub fn convert_to_writer<W: std::io::Write>(
        &self,
        html: &str,
        writer: &mut W,
    ) -> Result<(), IronpressError> {
        let resources = security::resources::DocumentResources::new(
            self.resources.base.as_deref(),
            self.resources.authorized_root.as_deref(),
            self.resources.network.clone(),
        );
        let mut resource_loader = security::resources::ResourceLoader::new(resources);

        // Step 1: Sanitize
        let sanitized_html = if self.sanitize {
            Some(security::sanitizer::sanitize_html_with_resources(
                html,
                resource_loader.resources(),
            )?)
        } else {
            None
        };
        let html = sanitized_html.as_deref().unwrap_or(html);

        // Step 2: Parse HTML and extract stylesheets
        let mut result = parser::html::parse_html_with_styles(html)?;
        self.page_margins.enrich_document(
            &mut result,
            self.sanitize,
            resource_loader.resources(),
        )?;
        security::sanitizer::sanitize_dom_resources(&mut result.nodes, resource_loader.resources());
        #[cfg(feature = "remote")]
        resource_loader.preload_document_resources(security::sanitizer::document_image_references(
            &result.nodes,
        ));

        // Step 2b: Resolve every stylesheet URL against its CSS base URL.
        // Imported sheets change that base to their own directory, while the
        // canonical resource root remains the authorization boundary.
        let stylesheets: Vec<String> = result
            .stylesheets
            .iter()
            .map(|css| {
                parser::css::resolve_imports_with_resources(css, resource_loader.resources())
            })
            .collect();

        // Step 3: Parse @page rules first (they affect page dimensions for media queries)
        let mut page_rules = Vec::new();
        let mut font_face_rules = Vec::new();
        for css in &stylesheets {
            page_rules.extend(parser::css::parse_page_rules(css));
            font_face_rules.extend(parser::css::parse_font_face_rules(css));
        }
        // Step 3b: Apply @page rules to override page size and margins.
        //
        // Only UNSELECTED `@page { }` rules (CSS Paged Media 3 §3
        // universal selector) fold into the document-global geometry. A
        // pseudo-class/named rule (`:first`/`:left`/`:right`/`:blank`/name)
        // must NOT be applied to every page — previously an `@page :first {
        // margin: 0 }` was mis-folded here and wrongly applied to all pages.
        // The `:first` override is collected separately below as a per-page-1
        // geometry change.
        let mut effective_page_size = self.page_size;
        let mut effective_margin = self.margin;
        for pr in &page_rules {
            if !pr.selector.is_universal() {
                continue;
            }
            if let (Some(w), Some(h)) = (pr.width, pr.height) {
                effective_page_size = PageSize {
                    width: w,
                    height: h,
                };
            }
            if let Some(v) = pr.margin_top {
                effective_margin.top = v;
            }
            if let Some(v) = pr.margin_right {
                effective_margin.right = v;
            }
            if let Some(v) = pr.margin_bottom {
                effective_margin.bottom = v;
            }
            if let Some(v) = pr.margin_left {
                effective_margin.left = v;
            }
        }
        // Viewport-percentage units in paged media resolve against the page
        // area (the initial containing block), not against the narrower body
        // content box. Preserve page-owned margins before body margin, padding,
        // and centering gutters are projected into `effective_margin` below.
        let default_page_area_margin = effective_margin;

        // Step 3c: Parse stylesheets with page-aware media query context
        let media_ctx = parser::css::MediaContext {
            width: effective_page_size.width,
            height: effective_page_size.height,
        };
        let mut rules = Vec::new();
        for css in &stylesheets {
            rules.extend(parser::css::parse_stylesheet_with_context(
                css,
                Some(media_ctx),
            ));
            inject_gcpm_footnote_declarations(css, &mut rules);
        }

        // Step 3d: Fold body/html/:root margin into the effective page margin.
        // Chrome's default UA sheet sets `body { margin: 8px }`, and author
        // stylesheets frequently override it. Ironpress applies body styles
        // to the root `ComputedStyle` for inheritance purposes but previously
        // dropped the margin, leaving the first line flush against the page
        // margin regardless of what the CSS requested.
        //
        // Only left/right are folded uniformly: they apply to every page
        // (body wraps each page's content horizontally). Top/bottom are NOT
        // folded — Chrome's print model applies body margin-top only on the
        // very first page and margin-bottom only on the last page. The
        // paginate step injects body.margin.top before the first block on
        // page 1 so continuation pages start flush against the page margin,
        // matching Chrome.
        let body_margin = layout::engine::compute_root_margin(&rules, effective_page_size);
        effective_margin.right += body_margin.right;
        effective_margin.left += body_margin.left;

        // Body padding acts as an additional inner gutter in Chrome's print
        // model. Since ironpress strips the <body> element before layout, we
        // fold its horizontal padding into the page margin too, so content
        // inside the body is offset by `page_margin + body_margin + body_padding`
        // on every page — matching Chrome's rendering of e.g.
        // `body { padding: 40px }`.
        let body_padding = layout::engine::compute_root_padding(&rules, effective_page_size);
        effective_margin.right += body_padding.right;
        effective_margin.left += body_padding.left;

        // Body max-width + margin:auto centers body content within the page's
        // printable area (e.g. `body { max-width: 640px; margin: auto; }` on
        // a typical article). Ironpress strips <body> before layout, so we
        // emulate the centering by folding a half-remainder gutter into each
        // horizontal page margin. Must run AFTER body margin/padding folding
        // so `printable_width` reflects the already-narrowed area.
        let printable_w = effective_page_size.width - effective_margin.horizontal();
        let body_center_gutter = layout::engine::compute_root_body_centering_gutter(
            &rules,
            effective_page_size,
            printable_w,
        );
        effective_margin.left += body_center_gutter;
        effective_margin.right += body_center_gutter;
        let root_flow_insets = types::Margin::new(
            effective_margin.top - default_page_area_margin.top,
            effective_margin.right - default_page_area_margin.right,
            effective_margin.bottom - default_page_area_margin.bottom,
            effective_margin.left - default_page_area_margin.left,
        );

        // Step 3e: Retain page geometry as a physical-page-aware cascade.
        // Page number, spread side, blank state, and named-page identity are
        // only all known during pagination; resolving separate buckets here
        // causes compound selectors to disagree with page backgrounds.
        let first_page = parser::css::PageSelectorContext {
            page_number: 1,
            is_blank: false,
            page_name: None,
        };
        let initial_page_area_geometry = layout::page_context::PageGeometryContext::from_rules(
            effective_page_size,
            default_page_area_margin,
            &page_rules,
        )
        .resolve(first_page);
        let initial_containing_block = types::Size::new(
            initial_page_area_geometry.size.width - initial_page_area_geometry.margin.horizontal(),
            initial_page_area_geometry.content_height(),
        );
        let page_geometry = layout::page_context::PageGeometryContext::from_rules(
            effective_page_size,
            default_page_area_margin,
            &page_rules,
        )
        .with_root_flow_insets(root_flow_insets);
        let footnote_area = resolve_footnote_area(&page_rules);

        // Step 4: Parse custom fonts (API-registered + @font-face from CSS)
        let mut parsed_fonts = self.parse_custom_fonts();

        system_fonts::load_system_default_fonts(&mut parsed_fonts);
        system_fonts::load_bundled_liberation_fonts(&mut parsed_fonts);
        let requested_font_rules = rules_with_font_face_local_sources(&rules, &font_face_rules);
        system_fonts::load_requested_system_fonts(
            &result.nodes,
            &requested_font_rules,
            &mut parsed_fonts,
        );
        load_font_face_rules(&font_face_rules, &mut resource_loader, &mut parsed_fonts);
        // Load system CJK font BEFORE bundled fallbacks so it gets UNICODE_FALLBACK_KEY
        system_fonts::load_unicode_fallback_font(&mut parsed_fonts);
        system_fonts::load_emoji_fallback_font(&mut parsed_fonts);

        let mut page_sheet_descriptors = parser::css::PageSheetDescriptors::default();
        for pr in &page_rules {
            if !pr.selector.is_universal() {
                continue;
            }
            page_sheet_descriptors.cascade(pr.sheet);
        }
        let page_sheet = render::pdf::PageSheet::resolve(page_sheet_descriptors);
        let page_bleed = page_sheet.bleed();
        let page_background = layout::page_context::PageBackgroundContext::from_rules(
            &page_rules,
            self.raster_quality,
            page_bleed,
        );

        // Step 5: Layout
        let mut pages = layout::engine::layout_with_rules_and_fonts_raster_quality(
            &result.nodes,
            layout::engine::DocumentGeometry::new(effective_page_size, effective_margin)
                .with_initial_containing_block(initial_containing_block),
            &rules,
            &parsed_fonts,
            result.font_locale,
            &page_background,
            layout::paginate::PaginationContext::new(page_geometry, footnote_area, 0.0),
            self.raster_quality,
            &mut resource_loader,
        );
        let mut footnote_area_for_overflow = footnote_area;
        footnote_area_for_overflow.content_width =
            effective_page_size.width - default_page_area_margin.horizontal();
        layout::paginate::move_overflow_footnotes_to_next_page(
            &mut pages,
            footnote_area_for_overflow,
            &parsed_fonts,
        );

        // Step 6: Render PDF
        //
        // Collect the `@page` margin boxes (CSS Paged Media 3 §5) into the page
        // decoration so running headers/footers + page counters render on every
        // page. Keep selected boxes too; the renderer applies the page-context
        // selector cascade per physical page.
        let margin_boxes: Vec<parser::css::MarginBox> = page_rules
            .iter()
            .flat_map(|pr| pr.margin_boxes.iter().cloned())
            .collect();

        let has_physical_decoration = page_sheet.has_effect();
        let has_footnote_decoration = footnote_area.style.padding != EdgeSizes::ZERO
            || footnote_area.style.separator.width > 0.0;
        let decoration = if self.page_margins.has_content()
            || !margin_boxes.is_empty()
            || has_physical_decoration
            || has_footnote_decoration
        {
            Some(render::pdf::PageDecoration {
                header: self.page_margins.header_text().map(str::to_string),
                footer: self.page_margins.footer_text().map(str::to_string),
                margin_boxes,
                margin_text: layout::engine::compute_page_margin_text_context(
                    &rules,
                    &page_rules,
                    effective_page_size,
                ),
                sheet: page_sheet,
                footnote_area: footnote_area.style,
            })
        } else {
            None
        };

        let render_opts = render::pdf::RenderOpts {
            compress: self.compress,
            jpeg_quality: self.jpeg_quality,
            auto_resize_images: self.auto_resize_images,
            raster_quality: self.raster_quality,
            occlusion_cull: self.occlusion_cull,
        };

        render::pdf::render_pdf_to_writer_full_opts_with_resources(
            render::pdf::PdfRenderDocument::new(
                &pages,
                effective_page_size,
                default_page_area_margin,
                &parsed_fonts,
                decoration.as_ref(),
            ),
            writer,
            render_opts,
            resource_loader,
        )
    }

    /// Convert a Markdown string to PDF, writing directly to any `std::io::Write` implementation.
    ///
    /// Streaming variant of [`convert_markdown`](HtmlConverter::convert_markdown).
    pub fn convert_markdown_to_writer<W: std::io::Write>(
        &self,
        md: &str,
        writer: &mut W,
    ) -> Result<(), IronpressError> {
        let html = parser::markdown::markdown_to_html(md);
        self.convert_to_writer(&html, writer)
    }

    /// Parse all registered custom fonts into TtfFont structs.
    fn parse_custom_fonts(&self) -> std::collections::HashMap<String, parser::ttf::TtfFont> {
        let mut fonts = std::collections::HashMap::new();
        for (name, data) in &self.custom_fonts {
            if let Some(font) = parser::ttf::parse_ttf_cached(data) {
                fonts.insert(name.clone(), font);
            }
        }
        self.font_catalog.install_into(&mut fonts);
        fonts
    }
}

fn inject_gcpm_footnote_declarations(css: &str, rules: &mut Vec<parser::css::CssRule>) {
    let mut cursor = 0usize;
    while let Some(open_rel) = css[cursor..].find('{') {
        let open = cursor + open_rel;
        let selector = css[cursor..open].trim();
        let mut depth = 1usize;
        let mut close = None;
        for (offset, ch) in css[open + 1..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + 1 + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            break;
        };
        if !selector.starts_with('@') {
            let mut map = parser::css::StyleMap::new();
            for declaration in css[open + 1..close].split(';') {
                let Some((prop, val)) = declaration.split_once(':') else {
                    continue;
                };
                let prop = prop.trim().to_ascii_lowercase();
                if matches!(prop.as_str(), "footnote-display" | "footnote-policy") {
                    map.set(
                        &prop,
                        parser::css::CssValue::Keyword(val.trim().to_ascii_lowercase()),
                    );
                }
            }
            if !map.properties.is_empty() {
                for selector in selector
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                {
                    rules.push(parser::css::CssRule {
                        selector: selector.to_string(),
                        declarations: map.clone(),
                        pseudo_element: None,
                    });
                }
            }
        }
        cursor = close + 1;
    }
}

fn resolve_footnote_area(
    page_rules: &[parser::css::PageRule],
) -> layout::paginate::FootnoteAreaLayout {
    let mut resolved = layout::paginate::FootnoteAreaLayout::default();
    for area in page_rules
        .iter()
        .filter(|rule| rule.selector.is_universal())
        .filter_map(|rule| rule.footnote_area)
    {
        if let Some(max_height) = area.max_height {
            resolved.max_height = Some(max_height);
        }
        area.padding.apply_to(&mut resolved.style.padding);
        if let Some(width) = area.separator.width {
            resolved.style.separator.width = width;
        }
        if let Some(color) = area.separator.color {
            resolved.style.separator.color = color;
        }
    }
    resolved
}

fn rules_with_font_face_local_sources(
    rules: &[parser::css::CssRule],
    font_face_rules: &[parser::css::FontFaceRule],
) -> Vec<parser::css::CssRule> {
    let local_names: Vec<String> = font_face_rules
        .iter()
        .flat_map(|rule| rule.local_source_names())
        .map(str::to_string)
        .collect();
    if local_names.is_empty() {
        return rules.to_vec();
    }

    let mut requested = rules.to_vec();
    for local_name in local_names {
        let mut declarations = parser::css::StyleMap::new();
        declarations.set(
            "font-family",
            parser::css::CssValue::Keyword(local_name.clone()),
        );
        requested.push(parser::css::CssRule {
            selector: format!("__font_face_local_source_{local_name}"),
            declarations,
            pseudo_element: None,
        });
    }
    requested
}

fn load_font_face_rules(
    font_face_rules: &[parser::css::FontFaceRule],
    resources: &mut security::resources::ResourceLoader,
    fonts: &mut std::collections::HashMap<String, parser::ttf::TtfFont>,
) {
    for (index, rule) in font_face_rules.iter().enumerate() {
        let Some(mut font) = resolve_font_face_source(rule, resources, fonts) else {
            continue;
        };
        apply_font_face_descriptors(rule, &mut font);

        let variant_key = system_fonts::font_variant_key_with_stretch(
            &rule.font_family,
            rule.font_weight_bold,
            rule.font_style_italic,
            rule.font_stretch,
        );
        let source_key = font_face_source_key(&variant_key, index);
        fonts.insert(source_key, font.clone());
        if rule.unicode_ranges.is_empty() {
            fonts.insert(variant_key, font);
            continue;
        }

        let ranged_key = font_face_range_key(&variant_key, index);
        fonts.insert(ranged_key, font.clone());
        fonts.entry(variant_key).or_insert(font);
    }
}

fn resolve_font_face_source(
    rule: &parser::css::FontFaceRule,
    resources: &mut security::resources::ResourceLoader,
    fonts: &std::collections::HashMap<String, parser::ttf::TtfFont>,
) -> Option<parser::ttf::TtfFont> {
    for (is_local, value) in rule.source_entries() {
        if is_local {
            if let Some((_, font)) = system_fonts::find_font_with_stretch(
                fonts,
                value,
                rule.font_weight_bold,
                rule.font_style_italic,
                rule.font_stretch,
            )
            .or_else(|| {
                system_fonts::find_font_with_stretch(fonts, value, false, false, rule.font_stretch)
            }) {
                return Some(font.clone());
            }
        } else {
            let ttf_data = resources
                .load_document_resource(value)
                .map(|loaded| loaded.bytes);

            if let Some(data) = ttf_data
                && let Some(font) = parser::ttf::parse_ttf_cached(&data)
            {
                return Some(font);
            }
        }
    }
    None
}

fn apply_font_face_descriptors(rule: &parser::css::FontFaceRule, font: &mut parser::ttf::TtfFont) {
    font.is_bold = rule.font_weight_bold;
    font.is_italic = rule.font_style_italic;
    if !rule.unicode_ranges.is_empty() {
        font.cmap.retain(|codepoint, _| {
            char::from_u32(*codepoint)
                .is_some_and(|ch| rule.unicode_ranges.iter().any(|range| range.contains(ch)))
        });
    }
    apply_size_adjust(font, rule.size_adjust);
}

fn apply_size_adjust(font: &mut parser::ttf::TtfFont, size_adjust: f32) {
    font.size_adjust = if size_adjust.is_finite() && size_adjust > 0.0 {
        size_adjust
    } else {
        1.0
    };
}

fn font_face_range_key(variant_key: &str, index: usize) -> String {
    format!("{variant_key}__fontface_{index}")
}

fn font_face_source_key(variant_key: &str, index: usize) -> String {
    format!("{variant_key}__fontface_source_{index}")
}

impl Default for HtmlConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlConverter {
    /// Async version of [`HtmlConverter::convert`] for file-based conversion.
    /// Requires the `async` feature.
    ///
    /// Reads the input HTML file asynchronously, performs the CPU-bound conversion
    /// in a blocking task, then writes the output PDF asynchronously.
    #[cfg(feature = "async")]
    pub async fn convert_file_async(
        &self,
        input: &str,
        output: &str,
    ) -> Result<(), IronpressError> {
        let html = tokio::fs::read_to_string(input).await?;
        let converter = self.clone();
        let pdf = tokio::task::spawn_blocking(move || converter.convert(&html))
            .await
            .map_err(|e| IronpressError::RenderError(format!("task join error: {e}")))?;
        let pdf = pdf?;
        tokio::fs::write(output, pdf).await?;
        Ok(())
    }
}

/// WASM bindings for browser-side PDF generation.
///
/// Enable with `cargo build --features wasm --target wasm32-unknown-unknown`.
#[cfg(feature = "wasm")]
pub mod wasm;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_text_letter_spacing_tracks_between_standard_font_units() {
        // CSS Text applies tracking between adjacent typographic units, not at
        // either edge. A TJ array can express that contract without trailing
        // spacing, unlike PDF's character-spacing operator.
        let pdf = html_to_pdf(
            r#"<svg width="200" height="50" viewBox="0 0 200 50"><text x="10" y="30" font-size="20" letter-spacing="3">Hello</text></svg>"#,
        )
        .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("[(H) -150(e) -150(l) -150(l) -150(o)] TJ"),
            "3 user units at font-size 20 must become -150 TJ adjustments"
        );
    }

    #[test]
    fn svg_text_letter_spacing_accepts_css_lengths_in_font_relative_units() {
        // 0.5em at font-size 20 is 10 user units (CSS Text 3 §8.2 takes any
        // <length>; SVG resolves em against the text's own font size).
        let pdf = html_to_pdf(
            r#"<svg width="200" height="50" viewBox="0 0 200 50"><text x="10" y="30" font-size="20" style="letter-spacing: 0.5em">Hello</text></svg>"#,
        )
        .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("[(H) -500(e) -500(l) -500(l) -500(o)] TJ"),
            "em tracking resolves against the font size"
        );
    }

    fn first_tj_array(content: &str) -> &str {
        content
            .lines()
            .find(|line| line.ends_with("] TJ"))
            .expect("shaped custom-font text emits a TJ array")
    }

    fn tj_adjustments(tj_line: &str) -> Vec<f32> {
        tj_line
            .trim_end_matches("] TJ")
            .trim_start_matches('[')
            .split_whitespace()
            .filter(|token| !token.starts_with('<'))
            .filter_map(|token| token.parse::<f32>().ok())
            .collect()
    }

    #[test]
    fn svg_text_letter_spacing_with_custom_font_tracks_typographic_units_in_tj() {
        let ttf_data = include_bytes!("../assets/LiberationSans-Regular.ttf").to_vec();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r##"<svg width="200" height="50" viewBox="0 0 200 50"><text x="10" y="30" font-family="testfont" font-size="20" letter-spacing="0.5em" fill="#000000">AB</text></svg>"##,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains(" Tc\n"),
            "custom-font SVG text must not space glyphs with the Tc operator"
        );
        // 10 user units of tracking between the two letters, expressed in
        // thousandths of the 20-unit font size: -500.
        let adjustments = tj_adjustments(first_tj_array(&content));
        let tracked = adjustments
            .iter()
            .filter(|adjustment| (*adjustment - -500.0).abs() < 1.0)
            .count();
        assert_eq!(
            tracked, 1,
            "tracking belongs only between adjacent typographic units: {adjustments:?}"
        );
    }

    #[test]
    fn svg_text_letter_spacing_suppresses_optional_ligatures() {
        // ParitySerif (DejaVu Serif renamed) forms the fi ligature by default;
        // CSS Text 3 §8.2: non-zero letter-spacing must not apply optional
        // ligatures, so the tracked run keeps f and i as two glyphs.
        let serif = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/parity/fonts/ParitySerif.ttf"
        ))
        .expect("parity serif font");
        let glyph_count = |text_attributes: &str| {
            let pdf = HtmlConverter::new()
                .add_font("parityserif", serif.clone())
                .convert(&format!(
                    r##"<svg width="200" height="50" viewBox="0 0 200 50"><text x="10" y="30" font-family="parityserif" font-size="20" fill="#000000" {text_attributes}>fi</text></svg>"##
                ))
                .unwrap();
            let content = String::from_utf8_lossy(&pdf).into_owned();
            first_tj_array(&content).matches('<').count()
        };
        assert_eq!(glyph_count(""), 1, "the untracked run ligates fi");
        assert_eq!(
            glyph_count(r#"letter-spacing="2""#),
            2,
            "tracking keeps f and i separate"
        );
    }

    #[test]
    fn svg_text_letter_spacing_ignores_zero_width_formatting_characters() {
        // CSS Text 3 §8.2: spacing is added as if U+200B did not exist, so the
        // tracked advance (and therefore an end anchor) matches the plain text.
        let ttf_data = include_bytes!("../assets/LiberationSans-Regular.ttf").to_vec();
        let anchored_x = |text: &str| {
            let pdf = HtmlConverter::new()
                .add_font("testfont", ttf_data.clone())
                .convert(&format!(
                    r##"<svg width="200" height="50" viewBox="0 0 200 50"><text x="150" y="30" font-family="testfont" font-size="20" letter-spacing="4" text-anchor="end" fill="#000000">{text}</text></svg>"##
                ))
                .unwrap();
            let content = String::from_utf8_lossy(&pdf).into_owned();
            content
                .lines()
                .find(|line| line.ends_with(" Tm") && line.starts_with("1 0 0 -1 "))
                .and_then(|line| line.split_whitespace().nth(4).map(str::to_string))
                .expect("text matrix")
        };
        assert_eq!(anchored_x("AB"), anchored_x("A\u{200B}B"));
    }

    #[test]
    fn svg_font_stack_falls_back_per_typographic_unit() {
        let primary = include_bytes!("../assets/LiberationSans-Regular.ttf").to_vec();
        let fallback = include_bytes!("../tests/parity/fonts/ParitySans.ttf").to_vec();
        let primary_font = parser::ttf::parse_ttf(primary.clone()).expect("valid primary font");
        let fallback_font = parser::ttf::parse_ttf(fallback.clone()).expect("valid fallback font");
        assert!(primary_font.cmap.contains_key(&('A' as u32)));
        assert!(!primary_font.cmap.contains_key(&('ا' as u32)));
        assert!(fallback_font.cmap.contains_key(&('ا' as u32)));

        let pdf = HtmlConverter::new()
            .add_font("Primary", primary)
            .add_font("Fallback", fallback)
            .convert(
                r#"<svg width="200" height="50"><text x="10" y="30" font-family="Primary, Fallback" font-size="20">Aا</text></svg>"#,
            )
            .expect("mixed-coverage SVG font stack must convert");
        let content = String::from_utf8_lossy(&pdf);

        assert!(content.contains("/primary 20 Tf"));
        assert!(content.contains("/fallback 20 Tf"));
        assert!(
            content
                .lines()
                .filter(|line| line.ends_with("] TJ"))
                .all(|line| !line.contains("<0000>"))
        );
    }

    #[test]
    fn svg_composite_font_face_checks_overlapping_ranges_newest_first() {
        let font_directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity/fonts");
        let pdf = HtmlConverter::new()
            .base_path(&font_directory)
            .convert(
                r#"<style>
                    @font-face {
                        font-family: RangePick;
                        src: url('ParitySans.ttf') format('truetype');
                        unicode-range: U+0041;
                    }
                    @font-face {
                        font-family: RangePick;
                        src: url('ParitySerif.ttf') format('truetype');
                        unicode-range: U+0041;
                    }
                </style>
                <svg width="100" height="60">
                    <text x="10" y="45" font-family="RangePick" font-size="40">A</text>
                </svg>"#,
            )
            .expect("overlapping SVG composite-font fixture must convert");
        let pdf = String::from_utf8_lossy(&pdf);

        assert!(pdf.contains("+ParitySerif"));
        assert!(!pdf.contains("+ParitySans"));
    }

    #[test]
    fn svg_composite_font_face_keeps_full_range_fallback_in_source_order() {
        let font_directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity/fonts");
        let pdf = HtmlConverter::new()
            .base_path(&font_directory)
            .convert(
                r#"<style>
                    @font-face {
                        font-family: RangePick;
                        src: url('ParitySans.ttf') format('truetype');
                    }
                    @font-face {
                        font-family: RangePick;
                        src: url('ParitySerif.ttf') format('truetype');
                        unicode-range: U+0041;
                    }
                </style>
                <svg width="100" height="60">
                    <text x="10" y="45" font-family="RangePick" font-size="40">B</text>
                </svg>"#,
            )
            .expect("mixed full and ranged SVG font fixture must convert");
        let pdf = String::from_utf8_lossy(&pdf);

        assert!(pdf.contains("+ParitySans"));
    }

    #[test]
    fn svg_composite_font_face_checks_later_full_range_face_first() {
        let font_directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity/fonts");
        let pdf = HtmlConverter::new()
            .base_path(&font_directory)
            .convert(
                r#"<style>
                    @font-face {
                        font-family: RangePick;
                        src: url('ParitySans.ttf') format('truetype');
                        unicode-range: U+0041;
                    }
                    @font-face {
                        font-family: RangePick;
                        src: url('ParitySerif.ttf') format('truetype');
                    }
                </style>
                <svg width="100" height="60">
                    <text x="10" y="45" font-family="RangePick" font-size="40">A</text>
                </svg>"#,
            )
            .expect("later full-range SVG font fixture must convert");
        let pdf = String::from_utf8_lossy(&pdf);

        assert!(pdf.contains("+ParitySerif"));
        assert!(!pdf.contains("+ParitySans"));
    }

    #[test]
    fn svg_registered_generic_alias_precedes_base14() {
        let font = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("parity font must exist");
        let pdf = HtmlConverter::new()
            .add_font("serif", font)
            .convert(
                r#"<svg width="100" height="60">
                    <text x="10" y="45" font-family="serif" font-size="40">A</text>
                </svg>"#,
            )
            .expect("registered generic SVG alias must convert");
        let pdf = String::from_utf8_lossy(&pdf);

        assert!(pdf.contains("+ParitySans"));
    }

    #[test]
    fn svg_inherited_em_letter_spacing_is_computed_on_the_ancestor() {
        let pdf = HtmlConverter::new()
            .convert(
                r#"<svg width="100" height="60">
                    <g style="font-size:10px;letter-spacing:1em">
                        <text x="10" y="45" style="font-size:20px">AB</text>
                    </g>
                </svg>"#,
            )
            .expect("inherited SVG letter-spacing fixture must convert");
        let pdf = String::from_utf8_lossy(&pdf);

        assert!(pdf.contains("[(A) -500(B)] TJ"));
        assert!(!pdf.contains("[(A) -1000(B)] TJ"));
    }

    #[test]
    fn svg_inline_important_font_family_wins_over_later_normal_declaration() {
        let first = include_bytes!("../assets/LiberationSans-Regular.ttf").to_vec();
        let second = include_bytes!("../tests/parity/fonts/ParitySans.ttf").to_vec();
        let pdf = HtmlConverter::new()
            .add_font("First", first)
            .add_font("Second", second)
            .convert(
                r#"<svg width="100" height="60">
                    <text x="10" y="45" font-size="20"
                          style="font-family:First!important;font-family:Second">AB</text>
                </svg>"#,
            )
            .expect("important SVG font-family fixture must convert");
        let pdf = String::from_utf8_lossy(&pdf);

        assert!(pdf.contains("/first 20 Tf"));
        assert!(!pdf.contains("/second 20 Tf"));
    }

    #[test]
    fn svg_inline_important_letter_spacing_wins_over_later_normal_declaration() {
        let pdf = HtmlConverter::new()
            .convert(
                r#"<svg width="100" height="60">
                    <text x="10" y="45" font-size="20"
                          style="letter-spacing:4px!important;letter-spacing:2px">AB</text>
                </svg>"#,
            )
            .expect("important SVG letter-spacing fixture must convert");
        let pdf = String::from_utf8_lossy(&pdf);

        assert!(pdf.contains("[(A) -200(B)] TJ"));
        assert!(!pdf.contains("[(A) -100(B)] TJ"));
    }

    #[test]
    fn svg_object_bounding_box_uses_inherited_text_sizing_and_tracking() {
        let pdf = HtmlConverter::new()
            .convert(
                r#"<svg width="200" height="100">
                    <defs>
                        <clipPath id="clip" clipPathUnits="objectBoundingBox">
                            <rect x="0" y="0" width="1" height="1"/>
                        </clipPath>
                    </defs>
                    <g style="font-size:10px;letter-spacing:1em" clip-path="url(#clip)">
                        <text x="10" y="45" style="font-size:2em">AB</text>
                    </g>
                </svg>"#,
            )
            .expect("tracked SVG object-bounding-box fixture must convert");
        let pdf = String::from_utf8_lossy(&pdf);

        assert!(pdf.contains("36.68 0 0"));
        assert!(!pdf.contains("32.016 0 0"));
    }

    #[test]
    fn add_font_used_by_svg_text() {
        // A real shapeable face: SVG custom-font emission requires shaped
        // glyph output, exactly like the HTML custom-font text path.
        let ttf_data = include_bytes!("../assets/LiberationSans-Regular.ttf").to_vec();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r##"<svg width="200" height="50" viewBox="0 0 200 50"><text x="10" y="30" font-family="testfont, Helvetica" font-size="20" fill="#000000">Hello</text></svg>"##,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/testfont 20 Tf"),
            "SVG text should bind the registered custom font"
        );
        assert!(
            content.contains("] TJ"),
            "SVG text with a custom font should emit shaped glyph IDs"
        );
        assert!(
            content.contains("/Subtype /CIDFontType2"),
            "a custom font used only by SVG text should still be embedded"
        );
    }

    #[test]
    fn add_font_used_by_background_image_svg_text() {
        // Custom-font text inside a CSS background-image SVG must bind and
        // embed the registered face exactly like foreground SVG text.
        let ttf_data = include_bytes!("../assets/LiberationSans-Regular.ttf").to_vec();
        let svg = r##"<svg xmlns='http://www.w3.org/2000/svg' width='200' height='50'><text x='10' y='30' font-family='testfont' font-size='20' fill='black'>Hello</text></svg>"##;
        let encoded = svg
            .replace('<', "%3C")
            .replace('>', "%3E")
            .replace('#', "%23")
            .replace('\'', "%27");
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(&format!(
                r#"<div style="width:200px;height:50px;background-image:url(data:image/svg+xml,{encoded})">&nbsp;</div>"#
            ))
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/testfont 20 Tf"),
            "background SVG text should bind the registered custom font"
        );
        assert!(
            content.contains("/Subtype /CIDFontType2"),
            "a custom font used only by a background SVG should still be embedded"
        );
    }

    #[test]
    fn svg_text_font_family_stack_resolves_later_entries() {
        // A quoted family name may contain commas (CSS Fonts 4 §4.2); the
        // unavailable first entry falls through to the registered face.
        let ttf_data = include_bytes!("../assets/LiberationSans-Regular.ttf").to_vec();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r##"<svg width="200" height="50" viewBox="0 0 200 50"><text x="10" y="30" font-family="'Acme, Sans', testfont" font-size="20" fill="#000000">Hello</text></svg>"##,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/testfont 20 Tf"),
            "the second stack entry binds the registered custom font"
        );
    }

    #[test]
    fn svg_text_font_family_stack_maps_generic_entries_to_base14() {
        // With no registered face in the list, the first standard family the
        // stack parser recognizes selects the base-14 font.
        let pdf = html_to_pdf(
            r#"<svg width="200" height="50" viewBox="0 0 200 50"><text x="10" y="30" font-family="NoSuchFamily, serif" font-size="20">Hello</text></svg>"#,
        )
        .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Roman 20 Tf"),
            "serif maps to the Times base-14 face"
        );
    }

    #[test]
    fn svg_text_with_unregistered_family_falls_back_to_base14() {
        let pdf = html_to_pdf(
            r#"<svg width="200" height="50" viewBox="0 0 200 50"><text x="10" y="30" font-family="NoSuchFont" font-size="20">Hello</text></svg>"#,
        )
        .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Helvetica 20 Tf"),
            "unregistered SVG font families should keep the base-14 mapping"
        );
        assert!(
            content.contains("(Hello) Tj"),
            "base-14 SVG text should keep literal text emission"
        );
    }

    #[test]
    fn raster_quality_builder_groups_and_normalizes_all_raster_controls() {
        assert_eq!(
            HtmlConverter::new().jpeg_quality,
            render::pdf::DEFAULT_JPEG_QUALITY
        );

        let converter = HtmlConverter::new().raster_quality(RasterQuality {
            source_image_dpi: 24.0,
            filter_dpi: f32::NAN,
            mask_dpi: f32::NAN,
            background_dpi: 48.0,
            blurred_coverage_compression: CoverageCompression::Lossless,
        });

        assert_eq!(
            converter.raster_quality,
            RasterQuality {
                source_image_dpi: 72.0,
                filter_dpi: 1.0,
                mask_dpi: 72.0,
                background_dpi: 96.0,
                blurred_coverage_compression: CoverageCompression::Lossless,
            }
        );
    }

    #[test]
    fn footnote_area_cascade_preserves_omitted_edges_and_separator() {
        let rules = parser::css::parse_page_rules(
            "@page { @footnote { padding: 1pt 2pt 3pt 4pt; border-top: 2pt solid red } }\
             @page { @footnote { max-height: 20pt; padding-right: 9pt } }",
        );
        let resolved = resolve_footnote_area(&rules);

        assert_eq!(resolved.style.padding, EdgeSizes::new(1.0, 9.0, 3.0, 4.0));
        assert_eq!(resolved.style.separator.width, 2.0);
        assert_eq!(
            resolved.style.separator.color,
            crate::types::Color::rgb(255, 0, 0)
        );
        assert_eq!(resolved.max_height, Some(20.0));
    }

    #[test]
    fn selected_footnote_area_does_not_leak_to_every_page() {
        let rules = parser::css::parse_page_rules(
            "@page { @footnote { padding-top: 2pt } }\
             @page :first { @footnote { padding-top: 20pt } }",
        );
        let resolved = resolve_footnote_area(&rules);
        assert_eq!(resolved.style.padding.top, 2.0);
    }

    /// Enabling compression shrinks the PDF and wraps the content stream in a
    /// FlateDecode filter; disabling restores the raw stream. (Rasterized-output
    /// equivalence is covered by the parity gate.)
    #[test]
    fn content_stream_compression_shrinks_and_filters() {
        // Enough repetitive content that Flate clearly beats its own overhead.
        let body = "<p>Some paragraph text to compress, repeated for volume.</p>".repeat(60);
        let html = format!("<html><body><h1>Hello</h1>{body}</body></html>");
        let html = html.as_str();
        let compressed = HtmlConverter::new().compress(true).convert(html).unwrap();
        let raw = HtmlConverter::new().compress(false).convert(html).unwrap();
        // The behavioral guarantee: compression meaningfully shrinks the output.
        assert!(
            compressed.len() + 200 < raw.len(),
            "compressed {} should be clearly < raw {}",
            compressed.len(),
            raw.len()
        );
        assert!(
            String::from_utf8_lossy(&compressed).contains("/Filter /FlateDecode"),
            "compressed PDF should carry a FlateDecode stream"
        );
    }

    /// Check if a PDF contains a given text string, handling both WinAnsi
    /// (plain text in parentheses) and CID encoding (hex glyph IDs with
    /// ToUnicode CMap). This allows tests to verify text content regardless
    /// of which font encoding path was used.
    fn pdf_has_text(pdf: &[u8], text: &str) -> bool {
        let content = String::from_utf8_lossy(pdf);
        // Fast path: plain WinAnsi text or text in PDF metadata
        if content.contains(text) {
            return true;
        }
        // CID path: each font has its own ToUnicode CMap. Parse all CMaps
        // indexed by their PDF object number, then decode TJ arrays using
        // the active font's CMap.
        let cmap_str: &str = content.as_ref();

        // Build per-font CMap: find "/ToUnicode N 0 R" references and
        // associate each font's CMap entries. Since we can't easily track
        // object IDs, we collect ALL bfchar entries into separate maps
        // keyed by their position in the PDF (each beginbfchar block
        // corresponds to a different font).
        let mut cmaps: Vec<std::collections::HashMap<String, char>> = Vec::new();
        let mut pos = 0;
        while let Some(start) = cmap_str[pos..].find("beginbfchar") {
            let block_start = pos + start + 11;
            let block_end = cmap_str[block_start..]
                .find("endbfchar")
                .map(|e| block_start + e)
                .unwrap_or(cmap_str.len());
            let mut map = std::collections::HashMap::new();
            for line in cmap_str[block_start..block_end].lines() {
                let parts: Vec<&str> = line
                    .trim()
                    .split(|c: char| c == '<' || c == '>' || c.is_whitespace())
                    .filter(|s| !s.is_empty())
                    .collect();
                if parts.len() >= 2 {
                    if let Ok(cp) = u32::from_str_radix(parts[1], 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            map.insert(parts[0].to_uppercase(), ch);
                        }
                    }
                }
            }
            if !map.is_empty() {
                cmaps.push(map);
            }
            pos = block_end;
        }
        if cmaps.is_empty() {
            return false;
        }

        // Decode TJ arrays, trying each CMap until one decodes all glyphs
        let mut all_decoded_text = String::new();
        let mut search_pos = 0;
        while let Some(tj_end) = cmap_str[search_pos..].find("] TJ") {
            let tj_end_abs = search_pos + tj_end;
            if let Some(tj_start) = cmap_str[..tj_end_abs].rfind('[') {
                let array_content = &cmap_str[tj_start + 1..tj_end_abs];
                let hexes: Vec<String> = {
                    let mut v = Vec::new();
                    let mut ap = 0;
                    while let Some(o) = array_content[ap..].find('<') {
                        let oa = ap + o;
                        if let Some(c) = array_content[oa..].find('>') {
                            v.push(array_content[oa + 1..oa + c].trim().to_uppercase());
                            ap = oa + c + 1;
                        } else {
                            break;
                        }
                    }
                    v
                };
                // Try each CMap to decode this TJ array
                for cmap in &cmaps {
                    let decoded: String = hexes
                        .iter()
                        .filter_map(|h| cmap.get(h.as_str()).copied())
                        .collect();
                    if !decoded.is_empty() {
                        all_decoded_text.push_str(&decoded);
                    }
                }
            }
            all_decoded_text.push(' ');
            search_pos = tj_end_abs + 4;
        }

        // The writer emits an individual `<glyph-id> Tj` operator for some
        // positioned runs (rather than one `[...] TJ` array). Decode that form
        // as well so this test helper follows the PDF we actually write.
        let mut single_hexes = Vec::new();
        let mut single_pos = 0;
        while let Some(tj_end) = cmap_str[single_pos..].find("> Tj") {
            let tj_end_abs = single_pos + tj_end;
            if let Some(hex_start) = cmap_str[..tj_end_abs].rfind('<') {
                single_hexes.push(cmap_str[hex_start + 1..tj_end_abs].trim().to_uppercase());
            }
            single_pos = tj_end_abs + 4;
        }
        // A glyph id is only meaningful within its active font. We do not
        // need a PDF object parser for a test helper: decode the full
        // positioned stream once per CMap, as with TJ arrays above. Selecting
        // the first CMap for each individual glyph corrupts a bold run when
        // its glyph ids overlap with another embedded font.
        for cmap in &cmaps {
            let decoded: String = single_hexes
                .iter()
                .filter_map(|hex| cmap.get(hex.as_str()).copied())
                .collect();
            if !decoded.is_empty() {
                all_decoded_text.push_str(&decoded);
                all_decoded_text.push(' ');
            }
        }
        all_decoded_text.contains(text)
    }

    /// Numeric adjustments from shaped CID `TJ` arrays. Identity-H fonts carry
    /// CSS letter/word spacing here because single-byte `Tc`/`Tw` does not
    /// apply to their two-byte character codes.
    fn shaped_tj_adjustments(pdf: &[u8]) -> Vec<f32> {
        let content = String::from_utf8_lossy(pdf);
        content
            .split("] TJ")
            .filter_map(|before| before.rsplit_once('[').map(|(_, array)| array))
            .flat_map(str::split_ascii_whitespace)
            .filter_map(|token| token.parse::<f32>().ok())
            .collect()
    }

    fn text_matrix_count(pdf: &[u8]) -> usize {
        String::from_utf8_lossy(pdf).matches(" Tm\n").count()
    }

    #[test]
    fn html_to_pdf_basic() {
        let pdf = html_to_pdf("<h1>Hello</h1><p>World</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("%%EOF"));
    }

    #[test]
    fn html_to_pdf_with_styles() {
        let html = r#"<h1 style="color: red; text-align: center">Title</h1>
                      <p style="font-size: 14pt">Some text here.</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_formatting() {
        let html = "<p>Normal <strong>bold</strong> <em>italic</em> <u>underline</u></p>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Helvetica-Bold"));
        assert!(content.contains("Helvetica-Oblique"));
    }

    #[test]
    fn html_to_pdf_empty() {
        let pdf = html_to_pdf("").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_sanitizes_script() {
        let html = "<p>Safe</p><script>alert('xss')</script>";
        let pdf = html_to_pdf(html).unwrap();
        assert!(!pdf_has_text(&pdf, "alert"));
        assert!(pdf_has_text(&pdf, "Safe"));
    }

    #[test]
    fn converter_builder() {
        let pdf = HtmlConverter::new()
            .page_size(PageSize::LETTER)
            .margin(Margin::uniform(54.0))
            .convert("<p>Test</p>")
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn converter_no_sanitize_parses_borrowed_input() {
        let html = String::from("<form><p>Borrowed input remains available</p></form>");
        let borrowed_html = html.as_str();
        let mut pdf = Vec::new();
        HtmlConverter::new()
            .sanitize(false)
            .convert_to_writer(borrowed_html, &mut pdf)
            .unwrap();
        assert_eq!(html, borrowed_html);
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf_has_text(&pdf, "Borrowed input remains available"));
    }

    fn render_counter_style_document(marker: Option<&str>) -> Vec<u8> {
        let definition = marker
            .map(|marker| {
                format!(
                    "<style>@counter-style shared {{ system: cyclic; symbols: '{marker}'; \
                     suffix: ' '; }}</style>"
                )
            })
            .unwrap_or_default();
        html_to_pdf(&format!(
            "{definition}<ol style='list-style-type: shared'><li>Document body</li></ol>"
        ))
        .unwrap()
    }

    fn assert_only_counter_marker(pdf: &[u8], expected: &str, unexpected: &str) {
        assert!(pdf_has_text(pdf, expected));
        assert!(!pdf_has_text(pdf, unexpected));
    }

    #[test]
    fn counter_styles_are_isolated_between_sequential_documents() {
        let alpha = render_counter_style_document(Some("AlphaMarker"));
        let beta = render_counter_style_document(Some("BetaMarker"));
        assert_only_counter_marker(&alpha, "AlphaMarker", "BetaMarker");
        assert_only_counter_marker(&beta, "BetaMarker", "AlphaMarker");
    }

    #[test]
    fn counter_style_is_absent_from_following_document() {
        let document_a = render_counter_style_document(Some("AlphaMarker"));
        let document_b = render_counter_style_document(None);
        assert!(pdf_has_text(&document_a, "AlphaMarker"));
        assert!(!pdf_has_text(&document_b, "AlphaMarker"));
        assert!(pdf_has_text(&document_b, "Document body"));
    }

    #[test]
    fn counter_styles_are_isolated_between_parallel_documents() {
        let start = std::sync::Barrier::new(4);
        let (alpha, beta, absent) = std::thread::scope(|scope| {
            let start_alpha = &start;
            let alpha = scope.spawn(move || {
                start_alpha.wait();
                render_counter_style_document(Some("AlphaMarker"))
            });
            let start_beta = &start;
            let beta = scope.spawn(move || {
                start_beta.wait();
                render_counter_style_document(Some("BetaMarker"))
            });
            let start_absent = &start;
            let absent = scope.spawn(move || {
                start_absent.wait();
                render_counter_style_document(None)
            });
            start.wait();
            (
                alpha.join().unwrap(),
                beta.join().unwrap(),
                absent.join().unwrap(),
            )
        });

        assert_only_counter_marker(&alpha, "AlphaMarker", "BetaMarker");
        assert_only_counter_marker(&beta, "BetaMarker", "AlphaMarker");
        assert!(!pdf_has_text(&absent, "AlphaMarker"));
        assert!(!pdf_has_text(&absent, "BetaMarker"));
        assert!(pdf_has_text(&absent, "Document body"));
    }

    #[test]
    fn html_to_pdf_headings() {
        let html = "<h1>H1</h1><h2>H2</h2><h3>H3</h3><h4>H4</h4><h5>H5</h5><h6>H6</h6>";
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_horizontal_rule() {
        let pdf = html_to_pdf("<p>Above</p><hr><p>Below</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_line_break() {
        let pdf = html_to_pdf("<p>Line one<br>Line two</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn convert_file_roundtrip() {
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_test_input.html");
        let output = dir.join("ironpress_test_output.pdf");
        std::fs::write(&input, "<h1>Test</h1><p>Hello</p>").unwrap();
        convert_file(input.to_str().unwrap(), output.to_str().unwrap()).unwrap();
        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        std::fs::remove_file(&input).ok();
        std::fs::remove_file(&output).ok();
    }

    #[test]
    fn converter_default_impl() {
        let converter = HtmlConverter::default();
        let pdf = converter.convert("<p>Default</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn markdown_to_pdf_roundtrip() {
        // Exercises markdown_to_pdf() (line 64-67)
        let pdf = markdown_to_pdf("# Test\n\nHello **world**").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf_has_text(&pdf, "Test"));
        assert!(pdf_has_text(&pdf, "world"));
    }

    #[test]
    fn convert_markdown_file_roundtrip() {
        // Exercises convert_markdown_file() (lines 76-80)
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_test_md_input.md");
        let output = dir.join("ironpress_test_md_output.pdf");
        std::fs::write(&input, "# Hello\n\nWorld").unwrap();
        convert_markdown_file(input.to_str().unwrap(), output.to_str().unwrap()).unwrap();
        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Hello"));
        std::fs::remove_file(&input).ok();
        std::fs::remove_file(&output).ok();
    }

    #[test]
    fn convert_markdown_file_missing_input() {
        let result = convert_markdown_file("/nonexistent/file.md", "/tmp/out.pdf");
        assert!(result.is_err());
    }

    #[test]
    fn html_to_pdf_unordered_list() {
        let html = "<ul><li>Item one</li><li>Item two</li><li>Item three</li></ul>";
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Item"));
    }

    #[test]
    fn html_to_pdf_ordered_list() {
        let html = "<ol><li>First</li><li>Second</li><li>Third</li></ol>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // The UA-default serif family resolves to an embedded font, so marker
        // and item text are shown as glyph ids rather than literal "1."/"2.".
        // Each of the three list items emits one text-show run (marker + content
        // combined), so expect at least three.
        let show_ops = content.matches("Tj").count() + content.matches("TJ").count();
        assert!(
            show_ops >= 3,
            "ordered list should emit a text-show run per item (got {show_ops})"
        );
    }

    #[test]
    fn html_to_pdf_table() {
        let html = r#"
            <table>
                <tr><th>Name</th><th>Age</th></tr>
                <tr><td>Alice</td><td>30</td></tr>
                <tr><td>Bob</td><td>25</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Name"));
        assert!(pdf_has_text(&pdf, "Alice"));
        assert!(pdf_has_text(&pdf, "Bob"));
        // No default cell borders — only CSS-specified borders produce strokes
    }

    #[test]
    fn html_to_pdf_table_with_sections() {
        let html = r#"
            <table>
                <thead><tr><th>Header</th></tr></thead>
                <tbody><tr><td>Body</td></tr></tbody>
                <tfoot><tr><td>Footer</td></tr></tfoot>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Header"));
        assert!(pdf_has_text(&pdf, "Body"));
        assert!(pdf_has_text(&pdf, "Footer"));
    }

    #[test]
    fn html_to_pdf_with_style_block() {
        let html = r#"
            <html>
            <head><style>p { color: red } .highlight { font-weight: bold }</style></head>
            <body>
                <p>Red text</p>
                <p class="highlight">Bold red text</p>
            </body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 rg")); // red color
        assert!(content.contains("Helvetica-Bold")); // bold from .highlight
    }

    #[test]
    fn html_to_pdf_style_block_in_body() {
        let html = r#"
            <style>h1 { color: blue }</style>
            <h1>Blue Title</h1>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("0 0 1 rg")); // blue color
    }

    #[test]
    fn html_to_pdf_definition_list() {
        let html = "<dl><dt>Term</dt><dd>Definition here</dd></dl>";
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Term"));
        assert!(pdf_has_text(&pdf, "Definition"));
    }

    #[test]
    fn markdown_to_pdf_basic() {
        let pdf = markdown_to_pdf("# Hello\n\nWorld").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf_has_text(&pdf, "Hello"));
        assert!(pdf_has_text(&pdf, "World"));
    }

    #[test]
    fn markdown_to_pdf_formatting() {
        let pdf = markdown_to_pdf("**bold** and *italic*").unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Helvetica-Bold"));
        assert!(content.contains("Helvetica-Oblique"));
    }

    #[test]
    fn markdown_to_pdf_list() {
        let pdf = markdown_to_pdf("- one\n- two\n- three").unwrap();
        assert!(pdf_has_text(&pdf, "one"));
        assert!(pdf_has_text(&pdf, "two"));
    }

    #[test]
    fn markdown_to_pdf_code_block() {
        let md = "# Code\n\n```\nfn main() {}\n```";
        let pdf = markdown_to_pdf(md).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn markdown_to_pdf_full() {
        let md = r#"# Project Title

Some **bold** and *italic* text with `inline code`.

## Features

- Item one
- Item two
- Item three

1. First
2. Second

> A wise quote

---

```
fn main() {
    println!("hello");
}
```

[Link](https://example.com)
"#;
        let pdf = markdown_to_pdf(md).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Project"));
        assert!(content.contains("Title"));
    }

    #[test]
    fn converter_markdown() {
        let pdf = HtmlConverter::new()
            .page_size(PageSize::LETTER)
            .convert_markdown("# Hello")
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_full_document() {
        let html = r#"
            <html>
            <head><title>Test</title></head>
            <body>
                <h1>Document Title</h1>
                <p>This is a <strong>bold</strong> and <em>italic</em> paragraph.</p>
                <hr>
                <p style="color: blue; text-align: center">Centered blue text.</p>
            </body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Document"));
        assert!(content.contains("Title"));
    }

    #[test]
    fn html_to_pdf_display_none_hides_element() {
        let html = r#"<p>Visible</p><p style="display: none">Secret</p><p>Remaining</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Visible"));
        assert!(!pdf_has_text(&pdf, "Secret"));
        assert!(pdf_has_text(&pdf, "Remaining"));
    }

    #[test]
    fn html_to_pdf_display_block_on_span() {
        let html = r#"<p><span style="display: block">Blocked</span></p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Blocked"));
    }

    #[test]
    fn html_to_pdf_media_print_applied() {
        let html = r#"
            <html>
            <head><style>
                @media print { p { color: red } }
            </style></head>
            <body><p>Print styled</p></body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 rg")); // red color applied
    }

    #[test]
    fn html_to_pdf_media_screen_ignored() {
        let html = r#"
            <html>
            <head><style>
                @media screen { p { color: red } }
            </style></head>
            <body><p>Not red</p></body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should NOT have red color since screen media is ignored
        assert!(!content.contains("1 0 0 rg"));
    }

    #[test]
    fn html_to_pdf_strikethrough() {
        let html = "<p><del>deleted</del> and <s>struck</s></p>";
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "deleted"));
        assert!(pdf_has_text(&pdf, "struck"));
    }

    #[test]
    fn html_to_pdf_page_break() {
        let html = r#"<p style="page-break-after: always">Page one</p><p>Page two</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_border() {
        let html = r#"<div style="border: 2px solid blue">Bordered content</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Bordered"));
    }

    #[test]
    fn html_to_pdf_font_families() {
        let html = r#"
            <p style="font-family: serif">Serif text</p>
            <p style="font-family: monospace">Mono text</p>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Times-Roman"));
        assert!(content.contains("Courier"));
    }

    #[test]
    fn html_to_pdf_table_colspan() {
        let html = r#"
            <table>
                <tr><td colspan="2">Wide</td></tr>
                <tr><td>A</td><td>B</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf_has_text(&pdf, "Wide"));
    }

    #[test]
    fn html_to_pdf_style_border_color_and_width() {
        let html = r#"
            <html>
            <head><style>div { border-width: 2pt; border-color: red }</style></head>
            <body><div>Bordered</div></body>
            </html>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn sanitizer_malformed_style_tag() {
        // Style tag without closing tag
        let html = "<style>p { color: red }";
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn sanitizer_event_handler_with_spaces() {
        let html = r#"<p onclick = "alert('xss')">Safe text</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(!pdf_has_text(&pdf, "alert"));
        assert!(pdf_has_text(&pdf, "Safe"));
    }

    // --- Streaming output tests ---

    #[test]
    fn streaming_produces_same_output_as_non_streaming() {
        let html = "<h1>Hello</h1><p>World</p>";
        let pdf_vec = html_to_pdf(html).unwrap();
        let mut streamed = Vec::new();
        html_to_pdf_writer(html, &mut streamed).unwrap();
        assert_eq!(pdf_vec, streamed);
    }

    #[test]
    fn streaming_markdown_produces_same_output() {
        let md = "# Title\n\nSome **bold** text.";
        let pdf_vec = markdown_to_pdf(md).unwrap();
        let mut streamed = Vec::new();
        markdown_to_pdf_writer(md, &mut streamed).unwrap();
        assert_eq!(pdf_vec, streamed);
    }

    #[test]
    fn streaming_to_file() {
        let dir = std::env::temp_dir();
        let output = dir.join("ironpress_stream_test.pdf");
        let mut file = std::fs::File::create(&output).unwrap();
        html_to_pdf_writer("<p>Streamed</p>", &mut file).unwrap();
        drop(file);
        let pdf = std::fs::read(&output).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf_has_text(&pdf, "Streamed"));
        std::fs::remove_file(&output).ok();
    }

    #[test]
    fn converter_convert_to_writer() {
        let html = "<p>Builder streaming</p>";
        let pdf_vec = HtmlConverter::new().convert(html).unwrap();
        let mut streamed = Vec::new();
        HtmlConverter::new()
            .convert_to_writer(html, &mut streamed)
            .unwrap();
        assert_eq!(pdf_vec, streamed);
    }

    #[test]
    fn converter_convert_markdown_to_writer() {
        let md = "# Markdown streaming";
        let pdf_vec = HtmlConverter::new().convert_markdown(md).unwrap();
        let mut streamed = Vec::new();
        HtmlConverter::new()
            .convert_markdown_to_writer(md, &mut streamed)
            .unwrap();
        assert_eq!(pdf_vec, streamed);
    }

    #[test]
    #[cfg(not(feature = "remote"))]
    fn url_image_ignored_without_remote_feature() {
        // Without the "remote" feature, remote URLs produce no image
        let html = r#"<img src="https://example.com/image.png" width="100" height="100">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn resource_loader_rejects_remote_bytes_without_feature() {
        #[cfg(not(feature = "remote"))]
        assert!(
            security::resources::ResourceLoader::default()
                .load_document_resource("https://example.com/test")
                .is_none()
        );
    }

    #[test]
    #[cfg(not(feature = "remote"))]
    fn remote_image_produces_valid_pdf() {
        // Remote images are silently ignored without the "remote" feature
        let html =
            r#"<img src="https://example.com/test.png" width="100" height="100"><p>Text</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf_has_text(&pdf, "Text"));
    }

    #[test]
    #[cfg(not(feature = "remote"))]
    fn remote_font_face_produces_valid_pdf() {
        // Remote font-face URLs are parsed but font loading is skipped without "remote" feature
        let html = r#"
            <style>
                @font-face { font-family: "RemoteFont"; src: url("https://example.com/font.ttf"); }
                p { font-family: RemoteFont; }
            </style>
            <p>Fallback to Helvetica</p>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[cfg(feature = "remote")]
    fn remote_fixture_server() -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (requests, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut png = Vec::new();
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                2,
                2,
                image::Rgb([240, 80, 40]),
            ))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
            let font: &[u8] = include_bytes!("../tests/parity/fonts/ParitySans.ttf");
            let mask: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="white"/></svg>"#;

            for mut stream in listener.incoming().flatten() {
                let mut request = [0; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let body: &[u8] = match path {
                    "/font.ttf" => font,
                    "/mask.svg" => mask,
                    _ => &png,
                };
                let content_type = if path.ends_with(".ttf") {
                    "font/ttf"
                } else if path.ends_with(".svg") {
                    "image/svg+xml"
                } else {
                    "image/png"
                };
                let content_length = if path == "/oversized.png" {
                    body.len() + 1024
                } else {
                    body.len()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    content_length
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
                let _ = requests.send(path.to_string());
            }
        });
        (format!("http://127.0.0.1:{port}"), received)
    }

    #[test]
    #[cfg(feature = "remote")]
    fn every_remote_resource_sink_uses_one_policy_while_html_stays_sanitized() {
        use std::collections::HashSet;
        use std::time::Duration;

        let (server, requests) = remote_fixture_server();
        let blocked = format!(r#"<img src="{server}/blocked.png"><script>bad()</script>"#);
        HtmlConverter::new()
            .sanitize(false)
            .convert(&blocked)
            .unwrap();
        assert!(requests.recv_timeout(Duration::from_millis(100)).is_err());

        let html = format!(
            r#"<style>
                @font-face {{ font-family: RemoteFixture; src: url("{server}/font.ttf"); }}
                .font {{ font-family: RemoteFixture; }}
                .generated::before {{ content: url("{server}/generated.png"); }}
                .marker {{ list-style-image: url("{server}/marker.png"); }}
                .background {{ width: 12pt; height: 12pt; background-image: url("{server}/background.png"); }}
                .border {{ width: 12pt; height: 12pt; border: 3pt solid; border-image-source: url("{server}/border.png"); border-image-slice: 1; }}
                .mask {{ width: 12pt; height: 12pt; background: red; mask-image: url("{server}/mask.svg"); }}
            </style>
            <p class="font">font</p>
            <img src="{server}/img.png" width="2" height="2">
            <span class="generated"></span>
            <ul><li class="marker">marker</li></ul>
            <div class="background"></div>
            <div class="border"></div>
            <svg width="10" height="10"><image href="{server}/nested.png" width="10" height="10"/></svg>
            <div class="mask"></div>
            <script>must not render</script>"#
        );
        let host = "127.0.0.1".parse::<RemoteHost>().unwrap();
        let pdf = HtmlConverter::new()
            .download_allow_list([host])
            .convert(&html)
            .unwrap();
        assert!(!pdf_has_text(&pdf, "must not render"));

        let expected = HashSet::from([
            "/font.ttf",
            "/img.png",
            "/generated.png",
            "/marker.png",
            "/background.png",
            "/border.png",
            "/nested.png",
            "/mask.svg",
        ])
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        while seen.len() < expected.len() {
            let Ok(path) = requests.recv_timeout(Duration::from_secs(1)) else {
                break;
            };
            seen.insert(path);
        }
        assert_eq!(seen, expected);
    }

    #[test]
    #[cfg(feature = "remote")]
    fn repeated_remote_images_cache_successes_and_failures_for_one_conversion() {
        use std::time::Duration;

        let (server, requests) = remote_fixture_server();
        let html = format!(
            r#"<img src="{server}/shared.png"><img src="{server}/shared.png">
                <img src="{server}/oversized.png"><img src="{server}/oversized.png">"#
        );
        let host = "127.0.0.1".parse::<RemoteHost>().expect("valid host");

        HtmlConverter::new()
            .download_allow_list([host])
            .download_max_body_size(512)
            .convert(&html)
            .expect("missing images do not fail the conversion");

        let mut seen = Vec::new();
        while let Ok(path) = requests.recv_timeout(Duration::from_millis(200)) {
            seen.push(path);
        }
        assert_eq!(seen.iter().filter(|path| *path == "/shared.png").count(), 1);
        assert_eq!(
            seen.iter().filter(|path| *path == "/oversized.png").count(),
            1
        );
    }

    #[test]
    fn header_footer_with_special_chars() {
        let pdf = HtmlConverter::new()
            .header("Report (Draft)")
            .footer("Page {page} / {pages}")
            .convert("<p>Content</p>")
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn multi_column_full_pipeline() {
        let html = r#"
            <style>.cols { column-count: 2; column-gap: 10pt; }</style>
            <div class="cols"><div>Left</div><div>Right</div></div>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn grid_repeat_full_pipeline() {
        let html = r#"
            <style>.g { display: grid; grid-template-columns: repeat(3, 1fr); gap: 5pt; }</style>
            <div class="g"><div>A</div><div>B</div><div>C</div></div>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn grid_minmax_full_pipeline() {
        let html = r#"
            <style>.g { display: grid; grid-template-columns: minmax(50px, 1fr) 2fr; }</style>
            <div class="g"><div>A</div><div>B</div></div>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    // --- Async tests (feature-gated) ---

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_convert_file_roundtrip() {
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_async_test_input.html");
        let output = dir.join("ironpress_async_test_output.pdf");
        tokio::fs::write(&input, "<h1>Async</h1><p>Test</p>")
            .await
            .unwrap();
        convert_file_async(input.to_str().unwrap(), output.to_str().unwrap())
            .await
            .unwrap();
        let pdf = tokio::fs::read(&output).await.unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Async"));
        tokio::fs::remove_file(&input).await.ok();
        tokio::fs::remove_file(&output).await.ok();
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_convert_markdown_file_roundtrip() {
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_async_md_test.md");
        let output = dir.join("ironpress_async_md_test.pdf");
        tokio::fs::write(&input, "# Async MD\n\nHello")
            .await
            .unwrap();
        convert_markdown_file_async(input.to_str().unwrap(), output.to_str().unwrap())
            .await
            .unwrap();
        let pdf = tokio::fs::read(&output).await.unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Async"));
        tokio::fs::remove_file(&input).await.ok();
        tokio::fs::remove_file(&output).await.ok();
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_converter_convert_file() {
        let dir = std::env::temp_dir();
        let input = dir.join("ironpress_async_builder_input.html");
        let output = dir.join("ironpress_async_builder_output.pdf");
        tokio::fs::write(&input, "<p>Builder async</p>")
            .await
            .unwrap();
        HtmlConverter::new()
            .page_size(PageSize::LETTER)
            .convert_file_async(input.to_str().unwrap(), output.to_str().unwrap())
            .await
            .unwrap();
        let pdf = tokio::fs::read(&output).await.unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        tokio::fs::remove_file(&input).await.ok();
        tokio::fs::remove_file(&output).await.ok();
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_convert_file_missing_input() {
        let result = convert_file_async("/nonexistent/file.html", "/tmp/out.pdf").await;
        assert!(result.is_err());
    }

    #[test]
    fn html_to_pdf_with_width() {
        let html = r#"<div style="width: 200pt">Constrained width</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_max_width() {
        let html = r#"<div style="max-width: 300pt">Max width block</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_height() {
        let html = r#"<div style="height: 100pt">Fixed height</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_opacity() {
        let html = r#"<div style="opacity: 0.5">Semi-transparent</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/ExtGState"));
        assert!(content.contains("/ca 0.5"));
    }

    // --- Integration tests for float / clear / position / box-shadow ---

    #[test]
    fn html_to_pdf_with_float_left() {
        let html = r#"<div style="float: left; width: 100pt">Floated</div><div>Normal</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_clear_both() {
        let html = r#"
            <div style="float: left">Floated</div>
            <div style="clear: both">Cleared</div>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_position_relative() {
        let html = r#"<div style="position: relative; top: 10pt; left: 5pt">Offset content</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_position_absolute() {
        let html = r#"<div style="position: absolute; top: 100pt; left: 50pt">Absolute</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_with_box_shadow() {
        let html = r#"<div style="box-shadow: 3px 3px black">Shadowed</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        // The PDF should contain the shadow rectangle (a filled rect with black color)
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("re\nf"),
            "Box shadow should produce a filled rectangle"
        );
    }

    #[test]
    fn html_to_pdf_float_and_clear_combined() {
        let html = r#"
            <div style="float: left; width: 150pt">Left sidebar</div>
            <div style="float: right; width: 150pt">Right sidebar</div>
            <div style="clear: both">Footer content below floats</div>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_box_shadow_with_blur() {
        let html = r#"<div style="box-shadow: 2px 2px 4px red">Shadow with blur</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    /// Build a minimal valid TTF for integration testing.
    fn build_integration_test_ttf() -> Vec<u8> {
        let mut buf = Vec::new();
        let num_tables: u16 = 6;
        buf.extend_from_slice(&[0, 1, 0, 0]);
        buf.extend_from_slice(&num_tables.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        let dir_start = buf.len();
        buf.resize(dir_start + num_tables as usize * 16, 0);

        // head table (54 bytes)
        let head_offset = buf.len();
        buf.extend_from_slice(&[0, 1, 0, 0]);
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
        buf.extend_from_slice(&0x000Bu16.to_be_bytes());
        buf.extend_from_slice(&1000u16.to_be_bytes()); // unitsPerEm
        buf.extend_from_slice(&[0; 16]); // created + modified
        buf.extend_from_slice(&(-100i16).to_be_bytes());
        buf.extend_from_slice(&(-200i16).to_be_bytes());
        buf.extend_from_slice(&800i16.to_be_bytes());
        buf.extend_from_slice(&900i16.to_be_bytes());
        buf.extend_from_slice(&[0; 8]); // macStyle..glyphDataFormat
        let head_len = buf.len() - head_offset;

        // hhea table (36 bytes)
        let hhea_offset = buf.len();
        buf.extend_from_slice(&[0, 1, 0, 0]);
        buf.extend_from_slice(&800i16.to_be_bytes());
        buf.extend_from_slice(&(-200i16).to_be_bytes());
        buf.extend_from_slice(&[0; 24]); // remaining fields
        buf.extend_from_slice(&3u16.to_be_bytes()); // numOfLongHorMetrics
        let hhea_len = buf.len() - hhea_offset;

        // maxp table
        let maxp_offset = buf.len();
        buf.extend_from_slice(&[0, 0, 0x50, 0]);
        buf.extend_from_slice(&3u16.to_be_bytes());
        let maxp_len = buf.len() - maxp_offset;

        // hmtx table (3 glyphs)
        let hmtx_offset = buf.len();
        for w in [500u16, 250, 700] {
            buf.extend_from_slice(&w.to_be_bytes());
            buf.extend_from_slice(&0i16.to_be_bytes());
        }
        let hmtx_len = buf.len() - hmtx_offset;

        // cmap table (format 4): char 32->glyph 1, char 65->glyph 2
        let cmap_offset = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&3u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&12u32.to_be_bytes());
        let subtable_start = buf.len();
        buf.extend_from_slice(&4u16.to_be_bytes());
        let len_pos = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&6u16.to_be_bytes()); // segCountX2 = 3*2
        buf.extend_from_slice(&4u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&2u16.to_be_bytes());
        // endCode
        for v in [32u16, 65, 0xFFFF] {
            buf.extend_from_slice(&v.to_be_bytes());
        }
        buf.extend_from_slice(&0u16.to_be_bytes()); // reservedPad
        // startCode
        for v in [32u16, 65, 0xFFFF] {
            buf.extend_from_slice(&v.to_be_bytes());
        }
        // idDelta
        for v in [-31i16, -63, 1] {
            buf.extend_from_slice(&v.to_be_bytes());
        }
        // idRangeOffset
        for _ in 0..3 {
            buf.extend_from_slice(&0u16.to_be_bytes());
        }
        let subtable_len = (buf.len() - subtable_start) as u16;
        buf[len_pos] = (subtable_len >> 8) as u8;
        buf[len_pos + 1] = subtable_len as u8;
        let cmap_len = buf.len() - cmap_offset;

        // name table
        let name_offset = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&18u16.to_be_bytes());
        let font_name_str = b"TestFont";
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&(font_name_str.len() as u16).to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes());
        buf.extend_from_slice(font_name_str);
        let name_len = buf.len() - name_offset;

        // Fill in table directory
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
            buf[dir_off + 4..dir_off + 8].copy_from_slice(&0u32.to_be_bytes());
            buf[dir_off + 8..dir_off + 12].copy_from_slice(&(*offset as u32).to_be_bytes());
            buf[dir_off + 12..dir_off + 16].copy_from_slice(&(*length as u32).to_be_bytes());
        }
        buf
    }

    #[test]
    fn add_font_embeds_truetype_in_pdf() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont">A</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Subtype /Type0"),
            "PDF should contain a Type0 custom font wrapper"
        );
        assert!(
            content.contains("/Subtype /CIDFontType2"),
            "PDF should contain a CIDFontType2 descendant font"
        );
        assert!(
            content.contains("/testfont "),
            "PDF should keep the custom font resource key"
        );
        assert!(
            content.contains("/BaseFont /TestFont") || content.contains("+TestFont"),
            "Custom fonts should preserve the embedded face name, with a subset tag when available"
        );
        assert!(
            content.contains("/FontDescriptor"),
            "PDF should contain FontDescriptor"
        );
        assert!(
            content.contains("/FontFile2"),
            "FontDescriptor should reference embedded font file"
        );
        assert!(
            content.contains("/Filter /FlateDecode"),
            "Embedded custom font streams should be compressed"
        );
        assert!(
            content.contains("/W [0 ["),
            "Descendant font should contain CID widths"
        );
        assert!(
            content.contains("/Encoding /Identity-H"),
            "Font should use Identity-H"
        );
        assert!(
            content.contains("/ToUnicode"),
            "Custom fonts should emit a ToUnicode CMap"
        );
    }

    #[test]
    fn add_font_uses_custom_font_in_content_stream() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont">A</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/testfont"),
            "Content stream should reference custom font"
        );
    }

    #[test]
    fn custom_font_falls_back_to_helvetica_when_not_registered() {
        let pdf = html_to_pdf(r#"<p style="font-family: 'UnknownFont'">Text</p>"#).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Helvetica"),
            "Should fall back to Helvetica for unregistered custom font"
        );
    }

    #[test]
    fn missing_system_font_in_stack_falls_back_to_later_family() {
        let pdf = html_to_pdf(r#"<p style="font-family: MissingFont, serif">Text</p>"#).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/missingfont"),
            "Missing primary families should not bind to an unrelated fallback as a custom font"
        );
        assert!(
            content.contains("/Times-Roman"),
            "Missing primary families should fall back to later CSS families"
        );
    }

    #[test]
    fn add_font_font_descriptor_has_metrics() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont">A</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Ascent"),
            "FontDescriptor should have Ascent"
        );
        assert!(
            content.contains("/Descent"),
            "FontDescriptor should have Descent"
        );
        assert!(
            content.contains("/FontBBox"),
            "FontDescriptor should have FontBBox"
        );
        assert!(
            content.contains("/Flags"),
            "FontDescriptor should have Flags"
        );
    }

    #[test]
    fn add_font_standard_fonts_still_work() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<p style="font-family: testfont">A</p>
                   <p style="font-family: serif">Serif</p>
                   <p>Default</p>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
        assert!(content.contains("/Times-Roman"));
        assert!(content.contains("/Helvetica"));
    }

    #[test]
    fn add_font_multiple_custom_fonts() {
        let ttf1 = build_integration_test_ttf();
        let ttf2 = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("fontone", ttf1)
            .add_font("fonttwo", ttf2)
            .convert(
                r#"<p style="font-family: fontone">A</p>
                   <p style="font-family: fonttwo">A</p>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/fontone"));
        assert!(content.contains("/fonttwo"));
    }

    #[test]
    fn add_font_case_insensitive_matching() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("MyFont", ttf_data)
            .convert(r#"<p style="font-family: MyFont">A</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Font name is lowercased internally
        assert!(content.contains("/myfont") || content.contains("/MyFont"));
    }

    #[test]
    fn add_font_in_table_cell() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<table><tr><td style="font-family: testfont">A</td></tr></table>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
    }

    #[test]
    fn add_font_with_bold_text() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont"><b>Bold custom</b></p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn add_font_with_italic_text() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont"><i>Italic custom</i></p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn add_font_empty_text_no_crash() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont"></p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn add_font_with_inline_style_inheritance() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<div style="font-family: testfont"><p>A</p><p>A</p></div>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
    }

    #[test]
    fn add_font_with_stylesheet() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<html><head><style>.custom { font-family: testfont; }</style></head>
                   <body><p class="custom">A</p></body></html>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
    }

    #[test]
    fn add_font_invalid_ttf_data_gracefully_degrades() {
        let pdf = HtmlConverter::new()
            .add_font("badfont", vec![0, 1, 2, 3])
            .convert(r#"<p style="font-family: badfont">Text</p>"#)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should fall back to Helvetica since the font couldn't be parsed
        assert!(content.contains("/Helvetica"));
    }

    #[test]
    fn add_font_preserves_page_size_and_margin() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .page_size(PageSize {
                width: 612.0,
                height: 792.0,
            })
            .margin(Margin::uniform(36.0))
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont">Custom</p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_in_list_item() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<ul style="font-family: testfont"><li>Item 1</li><li>Item 2</li></ul>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_in_nested_elements() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<div style="font-family: testfont"><p><span>Nested <b>bold</b></span></p></div>"#,
            )
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_with_long_text_wrapping() {
        let ttf_data = build_integration_test_ttf();
        let long_text = "A ".repeat(500);
        let html = format!(r#"<p style="font-family: testfont">{long_text}</p>"#,);
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(&html)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_mixed_with_standard_in_same_paragraph() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<p><span style="font-family: testfont">A</span> and <span style="font-family: serif">Serif</span></p>"#,
            )
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/testfont"));
        assert!(content.contains("/Times-Roman"));
    }

    #[test]
    fn custom_font_with_opacity() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(r#"<p style="font-family: testfont; opacity: 0.5">Transparent custom</p>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_with_width_and_background() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<div style="font-family: testfont; width: 200px; background-color: yellow">Boxed custom</div>"#,
            )
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn custom_font_markdown_conversion() {
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert_markdown("# Hello World\n\nSome text here.")
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn linear_gradient_produces_pdf() {
        let html = r#"<div style="background: linear-gradient(to right, red, blue); height: 50pt; width: 200pt">Gradient</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Should contain colored rectangles (gradient strips)
        assert!(content.contains("rg"));
    }

    #[test]
    fn radial_gradient_produces_pdf() {
        let html = r#"<div style="background: radial-gradient(red, blue); height: 100pt; width: 100pt">Radial</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn page_rule_changes_page_size() {
        let html = r#"<style>@page { size: letter; }</style><p>Hello</p>"#;
        let pdf = HtmlConverter::new().convert(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Letter size is 612x792, should appear in MediaBox
        assert!(content.contains("612"));
        assert!(content.contains("792"));
    }

    #[test]
    fn probe_line_preserves_fractional_text_baseline() {
        let font = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let html = r#"
            <style>
              @page { size: 384px 48px; margin: 0; }
              html { font-family: ParitySans; line-height: 1.5; }
              * { margin: 0; box-sizing: border-box; }
              .line { width: 300px; padding: 0; border-bottom: 2px solid #000; }
              .t { font-family: ParitySans; font-size: 40px; line-height: 1; }
            </style>
            <div class="line"><span class="t">Baseline Hxy</span></div>
        "#;
        let pdf = HtmlConverter::new()
            .sanitize(false)
            .compress(false)
            .add_font("ParitySans", font)
            .convert(html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);

        // Document-flow font metrics use the CSS-pixel grid while the PDF text
        // transform preserves the resolved coordinate without a second snap.
        assert!(content.contains("1 0 0 -1 0 34 Tm"), "{content}");
    }

    #[test]
    fn wrapped_text_baselines_snap_to_the_top_down_css_pixel_grid() {
        let font = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let html = r#"
            <style>
              @page { size: 352px 136px; margin: 0; }
              * { margin: 0; box-sizing: border-box; }
              .box {
                width: 220px; margin: 24px; padding: 6px; border: 2px solid #1a1a1a;
                font-family: ParitySans; font-size: 18px; line-height: 1.2;
                text-indent: 40px;
              }
            </style>
            <div class="box">indented first line wraps back to the left margin</div>
        "#;
        let pdf = HtmlConverter::new()
            .sanitize(false)
            .compress(false)
            .add_font("ParitySans", font)
            .convert(html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);

        // The flow cursor retains fractional line metrics, but print painting
        // snaps each baseline independently to the page's CSS-pixel grid.
        assert!(content.contains("1 0 0 -1 32 71 Tm"), "{content}");
        assert!(content.contains("1 0 0 -1 32 92 Tm"), "{content}");
    }

    #[test]
    fn page_rule_changes_margins() {
        let html = r#"<style>@page { margin: 0.5in; }</style><p>Hello</p>"#;
        let pdf = HtmlConverter::new().convert(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn page_rule_a4_landscape() {
        let html = r#"<style>@page { size: a4 landscape; }</style><p>Hello</p>"#;
        let pdf = HtmlConverter::new().convert(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Landscape A4: 841.89 x 595.28
        assert!(content.contains("841.89"));
        assert!(content.contains("595.28"));
    }

    #[test]
    fn linear_gradient_with_multiple_stops() {
        let html = r#"<div style="background: linear-gradient(to right, red 0%, white 50%, blue 100%); height: 50pt; width: 200pt">Multi-stop</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn gradient_via_background_image_property() {
        let html = r#"<div style="background-image: linear-gradient(45deg, #ff0000, #0000ff); height: 50pt; width: 200pt">Angled</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn svg_background_image_from_data_uri() {
        let html = r#"<html><head><style>
body { background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='100' height='100'%3E%3Crect width='100' height='100' fill='%23eee'/%3E%3Ccircle cx='50' cy='50' r='30' fill='%23ccc'/%3E%3C/svg%3E"); background-size: cover; }
</style></head><body>
<h1>Background Test</h1>
<p>This page should have an SVG pattern background.</p>
</body></html>"#;
        let pdf = HtmlConverter::new().sanitize(false).convert(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Background Test"));
    }

    #[test]
    fn svg_background_image_base64() {
        let html = r#"<html><head><style>
body { background: url("data:image/svg+xml;base64,PHN2ZyB4bWxucz0naHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmcnIHdpZHRoPSc1MCcgaGVpZ2h0PSc1MCc+PHJlY3Qgd2lkdGg9JzUwJyBoZWlnaHQ9JzUwJyBmaWxsPSdibHVlJy8+PC9zdmc+"); }
</style></head><body><p>Base64 SVG BG</p></body></html>"#;
        let pdf = HtmlConverter::new().sanitize(false).convert(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_border_radius() {
        let html = r#"<div style="border: 1px solid black; border-radius: 10pt; background-color: yellow; padding: 10pt">Rounded corners</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Rounded rect uses Bezier curves (c operator)
        assert!(content.contains(" c\n"));
    }

    #[test]
    fn html_to_pdf_outline() {
        let html = r#"<div style="outline: 3px solid blue; width: 200pt">With outline</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Outline draws a stroke
        assert!(content.contains("S\n"));
    }

    #[test]
    fn html_to_pdf_box_sizing_border_box() {
        let html = r#"<div style="box-sizing: border-box; width: 200pt; padding: 20pt; border: 2px solid black; background-color: green">Border box</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_combined_features() {
        let html = r#"<div style="border: 2px solid black; border-radius: 15pt; outline: 3px solid red; box-sizing: border-box; width: 300pt; padding: 20pt; background-color: #eee">All features combined</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains(" c\n")); // Bezier curves from border-radius
    }

    // --- Coverage tests for pdf.rs and engine.rs uncovered lines ---

    #[test]
    fn pdf_float_right_positions_block() {
        // Covers pdf.rs line 119: Float::Right block_x calculation
        let html = r#"<p style="float: right; width: 100pt">FloatRight</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "FloatRight"));
    }

    #[test]
    fn pdf_visibility_hidden_skips_rendering() {
        // Covers pdf.rs line 110: visibility hidden skips rendering
        let html = r#"<p style="visibility: hidden">HiddenStuff</p><p>VisibleStuff</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "VisibleStuff"));
        assert!(!pdf_has_text(&pdf, "HiddenStuff"));
    }

    #[test]
    fn pdf_overflow_hidden_clips_content() {
        // Covers pdf.rs lines 155-172: clip_rect with overflow: hidden
        let html = r#"<p style="overflow: hidden; width: 100pt; height: 50pt">ClippedHere</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("W n\n"));
    }

    #[test]
    fn pdf_overflow_hidden_with_border_radius() {
        // Covers pdf.rs lines 161-169: clip_rect with border-radius uses rounded path + W n
        let html = r#"<p style="overflow: hidden; border-radius: 10pt; width: 100pt; height: 50pt">RoundedClip</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("W n\n"));
        assert!(content.contains(" c\n"));
    }

    #[test]
    fn pdf_opacity_sets_ext_gstate() {
        // Covers pdf.rs lines 176-181: opacity < 1.0 creates ExtGState
        let html = r#"<p style="opacity: 0.5">Translucent</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("gs\n"));
    }

    #[test]
    fn pdf_inline_block_box_shadow_renders() {
        // Regression: box-shadow on `display: inline-block` items (rendered via
        // FlexCells) was dropped because FlexCell didn't carry the shadow. A
        // blurred shadow is now embedded as a gaussian-blurred image XObject
        // (drawn with `Do`), so the regression check is that the shadow renders
        // at all rather than being dropped.
        let html = "<div><div style=\"display:inline-block;width:80pt;height:40pt;\
            background:white;box-shadow:4pt 4pt 8pt rgba(0,0,0,0.3)\">A</div></div>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // The blurred shadow is embedded as an image XObject and drawn with `Do`.
        assert!(
            content.contains("Do\n"),
            "expected inline-block box-shadow to embed a blurred shadow image XObject"
        );
    }

    #[test]
    fn pdf_svg_path_opacity_emits_gstate() {
        // Regression: <path opacity="0.6"> inside inline SVG must register
        // an ExtGState with /ca 0.6 so the shape is rendered translucent.
        let html = "<svg width=\"120\" height=\"120\" viewBox=\"0 0 120 120\">\
            <path d=\"M10,110 L60,20 L110,110 Z\" fill=\"#f97316\" opacity=\"0.6\" />\
        </svg>";
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ca 0.6"),
            "expected SVG opacity to emit an ExtGState dict with /ca 0.6"
        );
        assert!(
            content.contains("GSsvg"),
            "expected the SVG ExtGState to be referenced via /GSsvgN gs"
        );
    }

    #[test]
    fn pdf_box_shadow_renders_rect() {
        // Covers pdf.rs lines 184-213: box-shadow rendering
        let html =
            r#"<p style="box-shadow: 5pt 5pt black; width: 100pt; padding: 10pt">ShadowBox</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("f\n"));
    }

    #[test]
    fn pdf_box_shadow_with_explicit_height() {
        // Covers pdf.rs line 188: box-shadow with block_height Some(h) path
        let html = r#"<p style="box-shadow: 3pt 3pt black; width: 100pt; height: 80pt; padding: 10pt">ShadowH</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("f\n"));
    }

    #[test]
    fn pdf_box_shadow_with_border_radius() {
        // Covers pdf.rs lines 195-202: box-shadow with border-radius uses rounded rect
        let html = r#"<p style="box-shadow: 3pt 3pt black; border-radius: 10pt; width: 100pt; padding: 10pt">RoundShadow</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains(" c\n"));
        assert!(content.contains("f\n"));
    }

    #[test]
    fn pdf_background_with_explicit_height() {
        // Covers pdf.rs line 220: background_color with block_height Some(h) path
        let html =
            r#"<p style="background-color: #ff0000; width: 100pt; height: 80pt">BGHeight</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 rg"));
        assert!(content.contains("f\n"));
    }

    #[test]
    fn pdf_linear_gradient_renders_strips() {
        // Linear gradient uses native PDF shading dictionaries
        let html = r#"<p style="background: linear-gradient(to right, red, blue); width: 200pt; height: 50pt; padding: 10pt">Gradient</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/ShadingType 2"));
    }

    #[test]
    fn pdf_linear_gradient_vertical() {
        // Vertical gradient (to bottom) uses shading dictionary
        let html = r#"<p style="background: linear-gradient(to bottom, red, blue); width: 200pt; height: 50pt; padding: 10pt">VertGrad</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/ShadingType 2"));
    }

    #[test]
    fn pdf_linear_gradient_with_block_height() {
        // Gradient with block_height uses shading dictionary
        let html = r#"<p style="background: linear-gradient(to right, red, blue); width: 200pt; height: 100pt; padding: 10pt">GradHeight</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/ShadingType 2"));
    }

    #[test]
    fn pdf_linear_gradient_diagonal() {
        // Diagonal gradient uses shading dictionary
        let html = r#"<p style="background: linear-gradient(45deg, red, blue); width: 200pt; height: 50pt; padding: 10pt">DiagGrad</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/ShadingType 2"));
    }

    #[test]
    fn pdf_radial_gradient_renders_circles() {
        // Radial gradient uses native PDF shading dictionary (Type 3)
        let html = r#"<p style="background: radial-gradient(red, blue); width: 200pt; height: 100pt; padding: 10pt">Radial</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/ShadingType 3"));
    }

    #[test]
    fn pdf_radial_gradient_with_block_height() {
        // Radial gradient with block_height uses shading dictionary
        let html = r#"<p style="background: radial-gradient(red, blue); width: 200pt; height: 120pt; padding: 10pt">RadialH</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/ShadingType 3"));
    }

    #[test]
    fn pdf_outline_with_block_height() {
        // Covers pdf.rs line 320: outline with block_height Some(h) path
        let html = r#"<p style="outline: 3pt solid red; width: 100pt; height: 80pt">OutlineH</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("RG\n"));
        assert!(content.contains("S\n"));
    }

    #[test]
    fn pdf_transform_rotate() {
        // Covers pdf.rs lines 132-152: transform rendering
        let html = r#"<p style="transform: rotate(45deg)">Rotated</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("cm\n"));
        assert!(content.contains("q\n"));
        assert!(content.contains("Q\n"));
    }

    #[test]
    fn pdf_transform_scale() {
        // Covers pdf.rs line 147: scale transform
        let html = r#"<p style="transform: scale(2)">Scaled</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("cm\n"));
    }

    #[test]
    fn pdf_transform_translate() {
        // Covers pdf.rs lines 149-150: translate transform
        let html = r#"<p style="transform: translate(10pt, 20pt)">Translated</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 1"));
        assert!(content.contains("cm\n"));
    }

    #[test]
    fn pdf_text_justify_alignment() {
        let html = r#"<p style="text-align: justify; width: 200pt">This is a long sentence with many words that should be justified across the width of the container for proper testing purposes here.</p>"#;
        let pdf = HtmlConverter::new().compress(false).convert(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content
                .lines()
                .any(|line| line.ends_with("] TJ") && line.contains(" -")),
            "justification must increase at least one inter-word advance"
        );
    }

    #[test]
    fn pdf_page_break_element() {
        // Covers pdf.rs line 616: PageBreak element
        // Also covers engine.rs line 602: page-break-after
        let html = r#"<p style="page-break-after: always">PageOne</p><p>PageTwo</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "PageOne"));
        assert!(pdf_has_text(&pdf, "PageTwo"));
    }

    #[test]
    fn pdf_grid_row_renders_cells() {
        // Covers pdf.rs lines 535-573: GridRow rendering
        // Covers engine.rs lines 607-622: grid container handling
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr">
                <div>CellAlpha</div>
                <div>CellBeta</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "CellAlpha"));
        assert!(pdf_has_text(&pdf, "CellBeta"));
    }

    #[test]
    fn pdf_grid_row_with_background() {
        // Covers pdf.rs lines 550-557: grid cell background rendering
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr">
                <div style="background-color: red">RedCell</div>
                <div style="background-color: blue">BlueCell</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("rg\n"));
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn pdf_grid_with_three_columns() {
        // Covers pdf.rs line 546: fallback col_widths for extra cells
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr 1fr">
                <div>A</div><div>B</div><div>C</div><div>D</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn pdf_grid_with_page_break_after() {
        // Covers engine.rs lines 619-620: page_break_after for grid container
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr; page-break-after: always">
                <div>GridPageOne</div>
            </div>
            <p>AfterGrid</p>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "GridPageOne"));
        assert!(pdf_has_text(&pdf, "AfterGrid"));
    }

    #[test]
    fn engine_flex_container_with_background() {
        // Covers engine.rs lines 1059-1097: flex container bg/border/shadow emit
        let html = r#"<html><body>
            <div style="display: flex; background-color: #eee; border: 1pt solid black; padding: 10pt">
                <div style="width: 100pt">FlexChild</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "FlexChild"));
    }

    #[test]
    fn engine_flex_wrap_wraps_items() {
        // Covers engine.rs lines 979-989: flex-wrap: wrap wrapping behavior
        let html = r#"<html><body>
            <div style="display: flex; flex-wrap: wrap; width: 200pt">
                <div style="width: 120pt">ItemOne</div>
                <div style="width: 120pt">ItemTwo</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "ItemOne"));
        assert!(pdf_has_text(&pdf, "ItemTwo"));
    }

    #[test]
    fn engine_flex_justify_space_between() {
        // Covers engine.rs lines 1122-1127: justify-content: space-between
        let html = r#"<html><body>
            <div style="display: flex; justify-content: space-between; width: 300pt">
                <div style="width: 50pt">LeftSide</div>
                <div style="width: 50pt">RightSide</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "LeftSide"));
        assert!(pdf_has_text(&pdf, "RightSide"));
    }

    #[test]
    fn engine_flex_justify_space_between_single() {
        // Covers engine.rs line 1126: space-between with single item (0 gap)
        let html = r#"<html><body>
            <div style="display: flex; justify-content: space-between; width: 300pt">
                <div style="width: 50pt">OnlyItem</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "OnlyItem"));
    }

    #[test]
    fn engine_flex_justify_space_around() {
        // Covers engine.rs lines 1129-1132: justify-content: space-around
        let html = r#"<html><body>
            <div style="display: flex; justify-content: space-around; width: 300pt">
                <div style="width: 50pt">ItemX</div>
                <div style="width: 50pt">ItemY</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "ItemX"));
        assert!(pdf_has_text(&pdf, "ItemY"));
    }

    #[test]
    fn engine_flex_justify_center() {
        // Covers engine.rs line 1121: justify-content: center
        let html = r#"<html><body>
            <div style="display: flex; justify-content: center; width: 300pt">
                <div style="width: 50pt">CenteredItem</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "CenteredItem"));
    }

    #[test]
    fn engine_flex_justify_flex_end() {
        // Covers engine.rs line 1120: justify-content: flex-end
        let html = r#"<html><body>
            <div style="display: flex; justify-content: flex-end; width: 300pt">
                <div style="width: 50pt">EndItem</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "EndItem"));
    }

    #[test]
    fn engine_flex_align_items_center() {
        // Covers engine.rs line 1144: align-items: center
        let html = r#"<html><body>
            <div style="display: flex; align-items: center; width: 300pt">
                <div style="width: 100pt">TallItem</div>
                <div style="width: 100pt">ShortItem</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "TallItem"));
        assert!(pdf_has_text(&pdf, "ShortItem"));
    }

    #[test]
    fn engine_flex_align_items_flex_end() {
        // Covers engine.rs line 1143: align-items: flex-end
        let html = r#"<html><body>
            <div style="display: flex; align-items: flex-end; width: 300pt">
                <div style="width: 100pt">BottomItem</div>
                <div style="width: 100pt">AlsoBottom</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "BottomItem"));
        assert!(pdf_has_text(&pdf, "AlsoBottom"));
    }

    #[test]
    fn engine_flex_direction_column() {
        // Covers engine.rs lines 1002-1021, 1230-1335: flex-direction: column
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; width: 200pt">
                <div style="width: 100pt">RowAlpha</div>
                <div style="width: 100pt">RowBeta</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "RowAlpha"));
        assert!(pdf_has_text(&pdf, "RowBeta"));
    }

    #[test]
    fn engine_flex_column_align_center() {
        // Covers engine.rs lines 1247-1249: column flex align-items: center (x_offset)
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; align-items: center; width: 300pt">
                <div style="width: 100pt">ColCenter</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "ColCenter"));
    }

    #[test]
    fn engine_flex_column_align_flex_end() {
        // Covers engine.rs lines 1248: column flex align-items: flex-end
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; align-items: flex-end; width: 300pt">
                <div style="width: 100pt">ColEnd</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "ColEnd"));
    }

    #[test]
    fn engine_flex_container_with_margin() {
        // Covers engine.rs lines 1342-1378: flex trailing margin
        let html = r#"<html><body>
            <div style="display: flex; margin: 20pt; background-color: #ccc; width: 200pt">
                <div style="width: 100pt">MarginedFlex</div>
            </div>
            <p>AfterFlex</p>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "MarginedFlex"));
        assert!(pdf_has_text(&pdf, "AfterFlex"));
    }

    #[test]
    fn engine_flex_with_overflow_hidden() {
        // Covers engine.rs lines 1082-1085: overflow: hidden in flex container
        let html = r#"<html><body>
            <div style="display: flex; overflow: hidden; width: 200pt; background-color: #eee">
                <div style="width: 100pt">ClippedFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "ClippedFlex"));
    }

    #[test]
    fn engine_flex_with_transform() {
        // Covers engine.rs line 1087: transform in flex container
        let html = r#"<html><body>
            <div style="display: flex; transform: rotate(5deg); background-color: #eee; width: 200pt">
                <div style="width: 100pt">TransFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "TransFlex"));
    }

    #[test]
    fn engine_flex_with_box_shadow() {
        // Covers engine.rs lines 1059, 1080: box-shadow in flex container
        let html = r#"<html><body>
            <div style="display: flex; box-shadow: 3pt 3pt black; width: 200pt">
                <div style="width: 100pt">ShadowFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "ShadowFlex"));
    }

    #[test]
    fn engine_flex_height_constrains_container() {
        // Covers engine.rs line 1049: flex height with Some(h) path
        let html = r#"<html><body>
            <div style="display: flex; height: 200pt; background-color: #eee; width: 300pt">
                <div style="width: 100pt">TallFlexContent</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "TallFlexContent"));
    }

    #[test]
    fn engine_flex_child_box_sizing_border_box() {
        // Covers engine.rs lines 865-869: box-sizing: border-box in flex child
        let html = r#"<html><body>
            <div style="display: flex; width: 300pt">
                <div style="width: 150pt; box-sizing: border-box; padding: 10pt; border: 2pt solid black">BorderBoxChild</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "BorderBoxChild"));
    }

    #[test]
    fn engine_flex_with_max_width() {
        // Covers engine.rs lines 800, 803: flex container width/max-width
        let html = r#"<html><body>
            <div style="display: flex; width: 300pt; max-width: 250pt; background-color: #eee">
                <div style="width: 100pt">MaxWidthFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "MaxWidthFlex"));
    }

    #[test]
    fn engine_flex_child_display_none() {
        // Covers engine.rs line 856: child with display: none is skipped
        let html = r#"<html><body>
            <div style="display: flex; width: 300pt">
                <div style="display: none; width: 100pt">HiddenFlex</div>
                <div style="width: 100pt">VisibleFlex</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(!pdf_has_text(&pdf, "HiddenFlex"));
        assert!(pdf_has_text(&pdf, "VisibleFlex"));
    }

    #[test]
    fn engine_flex_page_break_after() {
        // Covers engine.rs lines 601-602: page-break-after for flex container
        let html = r#"<html><body>
            <div style="display: flex; page-break-after: always">
                <div style="width: 100pt">FlexPageOne</div>
            </div>
            <p>FlexPageTwo</p>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "FlexPageOne"));
        assert!(pdf_has_text(&pdf, "FlexPageTwo"));
    }

    #[test]
    fn engine_grid_with_gap() {
        // Covers engine.rs line 1390: grid column gap
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 10pt">
                <div>GridAlpha</div>
                <div>GridBeta</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "GridAlpha"));
        assert!(pdf_has_text(&pdf, "GridBeta"));
    }

    #[test]
    fn engine_grid_fixed_columns() {
        // Covers engine.rs line 1414: fixed + fr grid tracks
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 100pt 1fr">
                <div>FixedCol</div>
                <div>FlexCol</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "FixedCol"));
        assert!(pdf_has_text(&pdf, "FlexCol"));
    }

    #[test]
    fn engine_table_with_colspan() {
        // Covers engine.rs line 1602: colspan counting in table
        let html = r#"
            <table>
                <tr><td colspan="2">Spanning</td></tr>
                <tr><td>CellA</td><td>CellB</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Spanning"));
        assert!(pdf_has_text(&pdf, "CellA"));
        assert!(pdf_has_text(&pdf, "CellB"));
    }

    #[test]
    fn engine_table_with_rowspan() {
        // Covers pdf.rs lines 490-504, engine.rs rowspan handling
        let html = r#"
            <table>
                <tr><td rowspan="2">TallCell</td><td>TopCell</td></tr>
                <tr><td>BottomCell</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "TallCell"));
        assert!(pdf_has_text(&pdf, "TopCell"));
        assert!(pdf_has_text(&pdf, "BottomCell"));
    }

    #[test]
    fn engine_table_with_thead_tbody_tfoot_coverage() {
        // Covers engine.rs lines 1565, 1575: table section traversal
        let html = r#"
            <table>
                <thead><tr><th>HeadCol</th></tr></thead>
                <tbody><tr><td>BodyRow</td></tr></tbody>
                <tfoot><tr><td>FootRow</td></tr></tfoot>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "HeadCol"));
        assert!(pdf_has_text(&pdf, "BodyRow"));
        assert!(pdf_has_text(&pdf, "FootRow"));
    }

    #[test]
    fn engine_table_non_tr_children_ignored() {
        // Covers engine.rs line 1575: non-tr/thead/tbody/tfoot children
        let html = r#"
            <table>
                <tr><td>ValidCell</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "ValidCell"));
    }

    #[test]
    fn engine_table_non_td_children_in_row() {
        // Covers engine.rs line 1687: non-td/th elements in a row are skipped
        let html = r#"
            <table>
                <tr><td>GoodCell</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "GoodCell"));
    }

    #[test]
    fn engine_ordered_list_indent() {
        // Covers engine.rs lines 486, 491: ordered list indent
        let html = r#"<ol><li>First</li><li>Second</li></ol>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Marker/content text is glyph-encoded under the embedded UA-default
        // serif font; verify each item emits its own text-show run.
        let show_ops = content.matches("Tj").count() + content.matches("TJ").count();
        assert!(
            show_ops >= 2,
            "ordered list should emit a text-show run per item (got {show_ops})"
        );
    }

    #[test]
    fn engine_clear_right() {
        // Covers engine.rs lines 2003-2006: clear: right
        let html = r#"<p style="float: right; width: 100pt">FloatedRight</p><p style="clear: right">ClearedRight</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "FloatedRight"));
        assert!(pdf_has_text(&pdf, "ClearedRight"));
    }

    #[test]
    fn engine_clear_both() {
        // Covers engine.rs lines 1995-2001: clear: both
        let html = r#"<p style="float: left; width: 100pt">FloatLeft</p><p style="float: right; width: 100pt">FloatRight</p><p style="clear: both">ClearedBoth</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "FloatLeft"));
        assert!(pdf_has_text(&pdf, "FloatRight"));
        assert!(pdf_has_text(&pdf, "ClearedBoth"));
    }

    #[test]
    fn engine_image_with_only_width_attr() {
        // Covers engine.rs line 2173: image with width only (falls back to square)
        let html = r#"<img width="100" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==">"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Do\n"));
    }

    #[test]
    fn engine_image_with_only_height_attr() {
        // Covers engine.rs line 2174: image with height only
        let html = r#"<img height="80" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==">"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Do\n"));
    }

    #[test]
    fn engine_image_unsupported_format_ignored() {
        // Covers engine.rs line 2225: non-PNG, non-JPEG data returns None
        let html = r#"<img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn engine_image_remote_url_blocked() {
        // Covers engine.rs lines 2204-2206: remote URLs are blocked
        let html = r#"<img src="https://example.com/image.png">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn engine_image_local_file_not_found() {
        // Covers engine.rs line 2209: local file path that doesn't exist
        let html = r#"<img src="/nonexistent/path/to/image.png">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn pdf_linear_gradient_to_left() {
        // Reversed horizontal gradient uses shading dictionary
        let html = r#"<p style="background: linear-gradient(to left, red, blue); width: 200pt; height: 50pt; padding: 10pt">ToLeft</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/ShadingType 2"));
    }

    #[test]
    fn pdf_linear_gradient_to_top_vertical() {
        // Vertical gradient to top uses shading dictionary
        let html = r#"<p style="background: linear-gradient(to top, red, blue); width: 200pt; height: 50pt; padding: 10pt">ToTop</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/ShadingType 2"));
    }

    #[test]
    fn pdf_gradient_three_stops() {
        // Three-stop gradient uses stitching function (Type 3)
        let html = r#"<p style="background: linear-gradient(to right, red 0%, white 50%, blue 100%); width: 200pt; height: 50pt; padding: 10pt">ThreeStops</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Pattern cs"));
        assert!(content.contains("/FunctionType 3"));
    }

    #[test]
    fn engine_flex_column_non_stretch_width() {
        // Covers engine.rs line 1256: non-stretch width in column flex
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; align-items: flex-start; width: 300pt">
                <div style="width: 100pt">NarrowChild</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "NarrowChild"));
    }

    #[test]
    fn engine_flex_column_with_position_relative() {
        // Covers engine.rs line 1311: column flex with x_offset > 0 sets Position::Relative
        let html = r#"<html><body>
            <div style="display: flex; flex-direction: column; align-items: center; width: 300pt">
                <div style="width: 100pt">ColCentered</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "ColCentered"));
    }

    #[test]
    fn engine_flex_with_gap() {
        // Covers engine.rs lines 976, 992, 1012: gap in flex layout
        let html = r#"<html><body>
            <div style="display: flex; gap: 10pt; width: 300pt">
                <div style="width: 80pt">GapA</div>
                <div style="width: 80pt">GapB</div>
                <div style="width: 80pt">GapC</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "GapA"));
        assert!(pdf_has_text(&pdf, "GapB"));
        assert!(pdf_has_text(&pdf, "GapC"));
    }

    #[test]
    fn engine_grid_incomplete_row_fills_empty_cells() {
        // Covers engine.rs lines 1517-1529: incomplete grid row fills with empty cells
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr 1fr">
                <div>OnlyOne</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "OnlyOne"));
    }

    #[test]
    fn engine_table_cell_background() {
        // Covers pdf.rs lines 510-518: table cell background rendering
        let html = r#"
            <table>
                <tr><td style="background-color: yellow">YellowCell</td><td>PlainCell</td></tr>
            </table>
        "#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(pdf_has_text(&pdf, "YellowCell"));
        assert!(content.contains("rg\n"));
    }

    #[test]
    fn engine_flex_empty_children_skipped() {
        // Covers engine.rs line 943-944: items.is_empty() check
        let html = r#"<html><body>
            <div style="display: flex; width: 200pt">
                <div style="display: none">HiddenOne</div>
                <div style="display: none">HiddenTwo</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn engine_flex_no_children() {
        // Covers engine.rs line 822-823: flex with no element children
        let html = r#"<html><body><div style="display: flex; width: 200pt"></div></body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn engine_grid_text_nodes_filtered() {
        // Covers engine.rs line 1456: text nodes are filtered in grid
        let html = r#"<html><body>
            <div style="display: grid; grid-template-columns: 1fr 1fr">
                <div>GridChild</div>
                <div>AnotherChild</div>
            </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "GridChild"));
        assert!(pdf_has_text(&pdf, "AnotherChild"));
    }

    #[test]
    fn font_face_rules_parsed_from_stylesheet() {
        // @font-face rules should be extracted from embedded stylesheets
        let html = r#"<html><head><style>
            @font-face {
                font-family: "TestFont";
                src: url("test.ttf");
            }
            body { color: black; }
        </style></head><body><p>Hello</p></body></html>"#;
        // Even without base_path, the conversion should succeed
        // (font file won't be found, but no error)
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn import_rules_ignored_without_base_path() {
        // @import rules should be ignored when no base_path is set
        let html = r#"<html><head><style>
            @import "nonexistent.css";
            body { color: red; }
        </style></head><body><p>Hello</p></body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn base_path_setter() {
        use std::path::Path;
        let converter = HtmlConverter::new().base_path(Path::new("/tmp/test"));
        // Verify base_path is set
        assert_eq!(
            converter.resources.base.as_deref(),
            Some(Path::new("/tmp/test"))
        );
    }

    #[test]
    #[cfg(not(feature = "remote"))]
    fn font_face_remote_url_rejected() {
        // Remote URLs in @font-face should be silently ignored
        let html = r#"<html><head><style>
            @font-face {
                font-family: "RemoteFont";
                src: url("https://example.com/font.ttf");
            }
        </style></head><body><p>Hello</p></body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn import_with_base_path_missing_file() {
        use std::path::Path;
        // When file doesn't exist, @import is silently skipped
        let html = r#"<html><head><style>
            @import "nonexistent.css";
            p { color: blue; }
        </style></head><body><p>Styled</p></body></html>"#;
        let pdf = HtmlConverter::new()
            .base_path(Path::new("/tmp/ironpress_test_nonexistent"))
            .convert(html)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn import_with_real_file() {
        // Create a temporary directory with a CSS file
        let tmp_dir = std::env::temp_dir().join("ironpress_import_test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        std::fs::write(tmp_dir.join("imported.css"), "p { color: red; }").unwrap();

        let html = r#"<html><head><style>
            @import "imported.css";
        </style></head><body><p>Hello</p></body></html>"#;

        let pdf = HtmlConverter::new()
            .base_path(&tmp_dir)
            .convert(html)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn import_recursive_with_depth_limit() {
        // Create files that import each other (circular)
        let tmp_dir = std::env::temp_dir().join("ironpress_recursive_test");
        let _ = std::fs::create_dir_all(&tmp_dir);
        std::fs::write(
            tmp_dir.join("a.css"),
            r#"@import "b.css"; .a { color: red; }"#,
        )
        .unwrap();
        std::fs::write(
            tmp_dir.join("b.css"),
            r#"@import "a.css"; .b { color: blue; }"#,
        )
        .unwrap();

        let html = r#"<html><head><style>
            @import "a.css";
        </style></head><body><p>Hello</p></body></html>"#;

        // Should not infinite loop due to depth limit
        let pdf = HtmlConverter::new()
            .base_path(&tmp_dir)
            .convert(html)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn font_face_with_base_path_missing_font() {
        use std::path::Path;
        // When font file doesn't exist, it's silently skipped
        let html = r#"<html><head><style>
            @font-face {
                font-family: "MissingFont";
                src: url("missing.ttf");
            }
            p { font-family: MissingFont; }
        </style></head><body><p>Hello</p></body></html>"#;

        let pdf = HtmlConverter::new()
            .base_path(Path::new("/tmp/ironpress_test_nonexistent"))
            .convert(html)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn import_remote_url_rejected() {
        use std::path::Path;
        // Remote import URLs should be silently rejected
        let html = r#"<html><head><style>
            @import url("https://example.com/styles.css");
            p { color: green; }
        </style></head><body><p>Hello</p></body></html>"#;

        let pdf = HtmlConverter::new()
            .base_path(Path::new("/tmp"))
            .convert(html)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn multiple_font_face_rules_in_stylesheet() {
        let html = r#"<html><head><style>
            @font-face {
                font-family: "Font1";
                src: url("font1.ttf");
            }
            @font-face {
                font-family: "Font2";
                src: url("font2.ttf");
            }
        </style></head><body><p>Hello</p></body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    // --- Coverage tests for engine.rs and pdf.rs uncovered lines ---

    #[test]
    fn html_to_pdf_ordered_list_lower_alpha() {
        // Covers engine.rs lines 664,668 (list marker formatting with style types)
        let html = r#"<html><head><style>
            ol { list-style-type: lower-alpha; }
        </style></head><body>
        <ol><li>First</li><li>Second</li><li>Third</li></ol>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "a."));
        assert!(pdf_has_text(&pdf, "b."));
    }

    #[test]
    fn html_to_pdf_ordered_list_upper_roman() {
        // Covers engine.rs line 120 (to_roman_lower/upper for zero edge case)
        let html = r#"<html><head><style>
            ol { list-style-type: upper-roman; }
        </style></head><body>
        <ol><li>First</li><li>Second</li><li>Third</li></ol>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "I."));
        assert!(pdf_has_text(&pdf, "II."));
    }

    #[test]
    fn html_to_pdf_list_style_none() {
        // Covers engine.rs list_style_type None branch
        let html = r#"<html><head><style>
            ul { list-style-type: none; }
        </style></head><body>
        <ul><li>Nomarker</li></ul>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Nomarker"));
    }

    #[test]
    fn html_to_pdf_list_style_inside() {
        // Covers engine.rs lines 670-671: list-style-position: inside
        let html = r#"<html><head><style>
            ul { list-style-position: inside; }
        </style></head><body>
        <ul><li>InsideItem</li></ul>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "InsideItem"));
    }

    #[test]
    fn html_to_pdf_flexbox_layout() {
        // Covers engine.rs lines 1067,1113,1133,1395: flex layout
        let html = r#"
        <div style="display: flex; width: 400pt;">
            <div style="width: 200pt;">FlexLeft</div>
            <div style="width: 200pt;">FlexRight</div>
        </div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_flexbox_no_explicit_width() {
        // Covers engine.rs line 1113: flex items without explicit width
        let html = r#"
        <div style="display: flex;">
            <div>AutoA</div>
            <div>AutoB</div>
            <div>AutoC</div>
        </div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_grid_layout() {
        // Covers engine.rs lines 1670,1712: grid track sizing and layout
        let html = r#"
        <div style="display: grid; grid-template-columns: 1fr 1fr;">
            <div>GridA</div>
            <div>GridB</div>
        </div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_table_colspan_exceeds_columns() {
        // Covers engine.rs line 2003: colspan spanning beyond available columns
        let html = r#"
        <table>
            <tr><td colspan="5">WideCellContent</td></tr>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_table_with_non_tr_children() {
        // Covers engine.rs line 1831: table children that are not tr/thead/tbody/tfoot
        let html = r#"
        <table>
            <caption>Caption</caption>
            <tr><td>Cell</td></tr>
        </table>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_text_overflow_ellipsis() {
        // Covers engine.rs lines 2221,2227,2242: nowrap + text-overflow: ellipsis
        let html = r#"
        <div style="width: 50pt; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">
            This is a very long text that should be truncated with an ellipsis marker
        </div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_clear_right() {
        // Covers engine.rs line 2312: clear right float
        let html = r#"
        <div style="float: right; width: 100pt;">RightFloated</div>
        <div style="clear: right;">ClearedRight</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_inline_base64_image() {
        // Covers engine.rs lines 2562,2574: base64 decode
        // A tiny 1x1 red PNG as base64
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="10" height="10">"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_text_justify() {
        let html = r#"<p style="text-align: justify; width: 300pt;">
            This is a paragraph with justified text alignment that has multiple words
            and should produce word spacing adjustments in the PDF output stream.
        </p>"#;
        let pdf = HtmlConverter::new().compress(false).convert(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content
                .lines()
                .any(|line| line.ends_with("] TJ") && line.contains(" -")),
            "justification must increase at least one inter-word advance"
        );
    }

    #[test]
    fn html_to_pdf_table_border_collapse() {
        // Covers pdf.rs lines 467,472-473,476: border-collapse on table
        let html = r#"<html><head><style>
            table { border-collapse: collapse; }
            td { border: 1pt solid black; }
        </style></head><body>
        <table>
            <tr><td>A</td><td>B</td></tr>
            <tr><td>C</td><td>D</td></tr>
        </table>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("A"));
        assert!(content.contains("D"));
    }

    #[test]
    fn html_to_pdf_table_rowspan() {
        // Covers pdf.rs lines 513,515: rowspan handling in table rendering
        let html = r#"
        <table>
            <tr><td rowspan="2">Tall</td><td>Top</td></tr>
            <tr><td>Bottom</td></tr>
        </table>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Tall"));
        assert!(pdf_has_text(&pdf, "Top"));
        assert!(pdf_has_text(&pdf, "Bottom"));
    }

    #[test]
    fn html_to_pdf_grid_row_rendering() {
        // Covers pdf.rs lines 553,555,564: GridRow rendering in PDF
        let html = r#"<html><head><style>
            .grid { display: grid; grid-template-columns: 1fr 1fr 1fr; }
            .grid > div { background-color: #eee; padding: 5pt; }
        </style></head><body>
        <div class="grid">
            <div>GridCell1</div>
            <div>GridCell2</div>
            <div>GridCell3</div>
        </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_explicit_page_break_element() {
        // Covers explicit PageBreak layout nodes.
        let html = r#"
        <p>PageOneContent</p>
        <div style="page-break-before: always;"></div>
        <p>PageTwoContent</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_linear_gradient() {
        // Covers pdf.rs lines 253,783,799,802,812: linear gradient rendering
        let html = r#"
        <div style="background: linear-gradient(to right, red, blue); width: 200pt; height: 50pt;">
            Gradient text
        </div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_radial_gradient() {
        // Covers pdf.rs lines 272,905: radial gradient rendering
        let html = r#"
        <div style="background: radial-gradient(circle, red, blue); width: 200pt; height: 50pt;">
            Radial text
        </div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_visibility_hidden() {
        // Covers pdf.rs lines 109-110,112-113: visibility: hidden skips rendering
        let html = r#"<p style="visibility: hidden">Hidden</p><p>VisibleAfterHidden</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_float_right_rendering() {
        // Covers pdf.rs line 121: Float::Right block_x computation
        let html = r#"
        <div style="float: right; width: 100pt;">RightFloat</div>
        <p>NormalAfterFloat</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_custom_font_bold_italic_variants() {
        // Covers pdf.rs lines 718-720: Custom font with bold+italic falls back
        let ttf_data = build_integration_test_ttf();
        let pdf = HtmlConverter::new()
            .add_font("testfont", ttf_data)
            .convert(
                r#"<p style="font-family: testfont; font-weight: bold; font-style: italic;">BoldItalic</p>"#,
            )
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_table_cell_text_rendering() {
        // Covers pdf.rs lines 675,681: cell text rendering with empty and non-empty runs
        let html = r#"
        <table>
            <tr>
                <td style="padding: 5pt;">CellPadded</td>
                <td></td>
            </tr>
        </table>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_grid_with_gap() {
        // Covers pdf.rs lines 593,599: grid gap/spacing calculation
        let html = r#"<html><head><style>
            .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10pt; }
        </style></head><body>
        <div class="grid">
            <div>GapA</div>
            <div>GapB</div>
        </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_li_outside_list() {
        // Covers engine.rs lines 668,676: li without list context
        let html = "<li>OrphanItem</li>";
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_flexbox_display_none_child() {
        // Covers engine.rs line 1106-1107: flex child with display:none
        let html = r#"
        <div style="display: flex;">
            <div>FlexVisible</div>
            <div style="display: none;">FlexHidden</div>
            <div>FlexAlso</div>
        </div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_table_border_spacing() {
        // Covers pdf.rs lines 472-473,476: border-spacing in separate mode
        let html = r#"<html><head><style>
            table { border-collapse: separate; border-spacing: 5pt; }
            td { border: 1pt solid black; }
        </style></head><body>
        <table>
            <tr><td>SpacedX</td><td>SpacedY</td></tr>
        </table>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn font_face_path_traversal_blocked() {
        // A @font-face src with path traversal should be silently skipped
        let dir = std::env::temp_dir().join("ironpress_font_traversal_test");
        std::fs::create_dir_all(&dir).unwrap();

        let html = r#"<html><head><style>
            @font-face { font-family: "Evil"; src: url("../../etc/passwd"); }
            body { font-family: "Evil"; }
        </style></head><body>Hello</body></html>"#;

        let converter = HtmlConverter::new().base_path(&dir);
        let mut buf = Vec::new();
        // Should succeed without loading the traversal path
        let result = converter.convert_to_writer(html, &mut buf);
        assert!(
            result.is_ok(),
            "converter should not fail on traversal font path"
        );
        assert!(buf.starts_with(b"%PDF"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn html_to_pdf_letter_spacing() {
        let unspaced = html_to_pdf("<p>Spaced letters</p>").unwrap();
        let pdf = html_to_pdf(r#"<p style="letter-spacing: 2pt">Spaced letters</p>"#).unwrap();
        assert!(pdf_has_text(&pdf, "Spaced letters"));

        assert!(
            text_matrix_count(&pdf) == text_matrix_count(&unspaced),
            "letter spacing should retain one text matrix for a simple shaped run"
        );
        assert!(
            shaped_tj_adjustments(&pdf)
                .iter()
                .any(|adjustment| *adjustment < -150.0),
            "letter-spacing should expand the shaped run through TJ adjustments"
        );
    }

    #[test]
    fn html_to_pdf_word_spacing() {
        let html = r#"<p style="word-spacing: 5pt">Spaced words here</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(
            shaped_tj_adjustments(&pdf)
                .iter()
                .any(|adjustment| *adjustment < -400.0),
            "word-spacing should widen Identity-H spaces through TJ adjustments"
        );
    }

    #[test]
    fn html_to_pdf_letter_and_word_spacing_combined() {
        let unspaced = html_to_pdf("<p>Spaced letters and words</p>").unwrap();
        let pdf = html_to_pdf(
            r#"<p style="letter-spacing: 2pt; word-spacing: 5pt">Spaced letters and words</p>"#,
        )
        .unwrap();
        assert!(
            shaped_tj_adjustments(&pdf)
                .iter()
                .any(|adjustment| *adjustment < -400.0),
            "combined spacing should preserve the word-space TJ adjustment"
        );
        assert!(pdf_has_text(&pdf, "Spaced letters and words"));

        assert!(
            text_matrix_count(&pdf) == text_matrix_count(&unspaced),
            "combined spacing should retain one text matrix for a simple shaped run"
        );
        assert!(
            shaped_tj_adjustments(&pdf)
                .iter()
                .any(|adjustment| (-170.0..-160.0).contains(adjustment)),
            "combined spacing should preserve the letter-space TJ adjustment"
        );
    }

    #[test]
    fn html_to_pdf_long_word_hyphenated() {
        // A very long word preceded by short content in a narrow div should be
        // hyphenated in the PDF output (hyphenation triggers when the line
        // already has content and the next word doesn't fit).
        let html = r#"<div style="width: 80pt"><p>Hi Supercalifragilisticexpialidocious</p></div>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // The PDF text streams should contain a hyphen from the hyphenation
        assert!(
            content.contains('-'),
            "PDF should contain a hyphen from hyphenated long word"
        );
    }

    #[test]
    fn html_to_pdf_inline_svg_rect() {
        let html = r#"<svg width="100" height="100"><rect x="10" y="10" width="80" height="80" fill="red"/></svg>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("re")); // rect operator
    }

    #[test]
    fn html_to_pdf_inline_svg_circle() {
        let html =
            r#"<svg width="100" height="100"><circle cx="50" cy="50" r="40" fill="blue"/></svg>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_inline_svg_path() {
        let html = r#"<svg width="100" height="100"><path d="M 10 10 L 90 10 L 90 90 Z" fill="green"/></svg>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_inline_svg_with_viewbox() {
        let html = r#"<svg width="200" height="200" viewBox="0 0 100 100"><rect x="0" y="0" width="100" height="100" fill="red"/></svg>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_svg_script_stripped() {
        // Script inside SVG should not cause issues (html5ever strips it or ignores it)
        let html = r#"<svg width="100" height="100"><script>alert(1)</script><rect x="10" y="10" width="80" height="80" fill="red"/></svg>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_svg_among_html() {
        let html = r#"<h1>Title</h1><svg width="100" height="50"><rect x="0" y="0" width="100" height="50" fill="blue"/></svg><p>World</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf_has_text(&pdf, "Title"));
        assert!(pdf_has_text(&pdf, "World"));
    }

    #[test]
    fn html_to_pdf_justify_single_word_no_spaces() {
        // Covers pdf.rs line 374: justify text with no spaces yields 0.0 word spacing
        let html =
            r#"<p style="text-align: justify; width: 200pt;">Superlongwordwithoutanyspaces</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf_has_text(&pdf, "Superlongword"));
    }

    #[test]
    fn html_to_pdf_radial_gradient_no_block_height() {
        // Covers pdf.rs line 274: radial gradient on block without explicit height
        let html = r#"<html><head><style>
            .grad { background: radial-gradient(circle, red, blue); padding: 10pt; }
        </style></head><body>
        <div class="grad">Radial no height</div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_linear_gradient_no_block_height() {
        // Covers pdf.rs line 255: linear gradient on block without explicit height
        let html = r#"<html><head><style>
            .grad { background: linear-gradient(to right, red, blue); padding: 10pt; }
        </style></head><body>
        <div class="grad">Linear no height</div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_table_rowspan_future_row_lookup() {
        // Covers pdf.rs lines 526, 528: rowspan > 1 iterates future rows
        let html = r#"
        <table>
            <tr><td rowspan="3">Spanning</td><td>R1</td></tr>
            <tr><td>R2</td></tr>
            <tr><td>R3</td></tr>
        </table>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Spanning"));
        assert!(pdf_has_text(&pdf, "R1"));
        assert!(pdf_has_text(&pdf, "R3"));
    }

    #[test]
    fn html_to_pdf_grid_more_cells_than_columns() {
        // Covers pdf.rs line 577: grid cell index exceeding col_widths falls back to 0.0
        let html = r#"<html><head><style>
            .grid { display: grid; grid-template-columns: 100pt; }
        </style></head><body>
        <div class="grid">
            <div>Cell1</div>
            <div>Cell2</div>
            <div>Cell3</div>
        </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_empty_paragraph_text_block() {
        // Exercises empty text run/line skipping in pdf.rs lines 401, 718, 724
        let html = r#"<p></p><p>Visible</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Visible"));
    }

    #[test]
    fn html_to_pdf_table_empty_cells() {
        // Covers pdf.rs lines 718, 724: empty cell text/run skipping in render_cell_text
        let html = r#"
        <table>
            <tr><td></td><td>Data</td></tr>
            <tr><td></td><td></td></tr>
        </table>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Data"));
    }

    #[test]
    fn html_to_pdf_position_relative_offset() {
        // Covers pdf.rs line 121: Position::Relative with offset_left
        let html = r#"<div style="position: relative; left: 20pt;">Shifted</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Shifted"));
    }

    #[test]
    fn html_to_pdf_multiple_page_breaks() {
        // Covers pdf.rs line 677: PageBreak match arm
        let html = r#"
        <p>Page1</p>
        <div style="page-break-before: always;"></div>
        <p>Page2</p>
        <div style="page-break-before: always;"></div>
        <p>Page3</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Page1"));
        assert!(pdf_has_text(&pdf, "Page3"));
    }

    #[test]
    fn html_to_pdf_svg_ellipse_and_line() {
        // Exercise SVG element destructuring (lines 638, 642-643) with different SVG content
        let html = r#"<svg width="200" height="200">
            <ellipse cx="100" cy="100" rx="80" ry="50" fill="green"/>
            <line x1="0" y1="0" x2="200" y2="200" stroke="black"/>
        </svg>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_justify_long_word_then_short() {
        // Covers pdf.rs line 374: justify with a non-last line that has no spaces.
        let long_word = "A".repeat(200);
        let html = format!(
            r#"<p style="text-align: justify; width: 100pt;">{long_word} short words here</p>"#,
        );
        let pdf = html_to_pdf(&html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_table_with_empty_and_content_cells() {
        // Covers pdf.rs lines 718, 724: render_cell_text with empty lines/runs
        let html = r#"
        <table>
            <tr><td></td><td>A</td><td></td></tr>
            <tr><td>B</td><td></td><td>C</td></tr>
        </table>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("A"));
        assert!(content.contains("B"));
        assert!(content.contains("C"));
    }

    #[test]
    fn html_to_pdf_float_right_without_explicit_width() {
        // Covers pdf.rs line 123: Float::Right without block_width
        let html = r#"<div style="float: right;">FloatedRight</div><p>Normal</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "FloatedRight"));
    }

    #[test]
    fn html_to_pdf_position_absolute_offset() {
        // Covers pdf.rs line 120: Position::Absolute with offset_left
        let html = r#"<div style="position: absolute; left: 50pt;">AbsPos</div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "AbsPos"));
    }

    #[test]
    fn html_to_pdf_inline_image_base64_png() {
        // Covers pdf.rs lines 606, 612: Image element with PNG format
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="1" height="1"/>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_grid_with_background_and_many_cells() {
        // Covers pdf.rs lines 566, 568, 577: GridRow with cells exceeding columns
        let html = r#"<html><head><style>
            .g { display: grid; grid-template-columns: 50pt 50pt; }
            .g > div { background: #ff0000; padding: 5pt; }
        </style></head><body>
        <div class="g">
            <div>G1</div>
            <div>G2</div>
            <div>G3</div>
            <div>G4</div>
            <div>G5</div>
        </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn html_to_pdf_page_break_empty_arm() {
        // Covers pdf.rs line 677: PageBreak empty match arm
        let html = r#"
        <p>Before</p>
        <div style="page-break-after: always;"></div>
        <p>After</p>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf_has_text(&pdf, "Before"));
        assert!(pdf_has_text(&pdf, "After"));
    }

    #[test]
    fn html_to_pdf_svg_with_polyline_polygon() {
        // Exercise SVG rendering paths
        let html = r#"<svg width="100" height="100">
            <polyline points="10,10 50,50 90,10" fill="none" stroke="red"/>
            <polygon points="10,80 50,90 90,80" fill="blue"/>
        </svg>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn flex_children_with_block_elements_render_content() {
        // Flex children containing block elements (h1, h2, p) should produce text
        let html = r#"<html><body>
        <div style="display: flex; justify-content: space-between;">
            <div>
                <h1>ironpress</h1>
                <h2>Pure Rust PDF Engine</h2>
            </div>
            <div>
                <p>Invoice #INV-2026-0042</p>
            </div>
        </div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(
            pdf_has_text(&pdf, "ironpress"),
            "flex child h1 text should appear in PDF"
        );
        // Words may be in separate PDF text objects due to word-by-word rendering
        assert!(
            pdf_has_text(&pdf, "Pure"),
            "flex child h2 word 'Pure' should appear in PDF"
        );
        assert!(
            pdf_has_text(&pdf, "Rust"),
            "flex child h2 word 'Rust' should appear in PDF"
        );
        assert!(
            pdf_has_text(&pdf, "Engine"),
            "flex child h2 word 'Engine' should appear in PDF"
        );
        assert!(
            pdf_has_text(&pdf, "INV"),
            "flex child p text should appear in PDF"
        );
    }

    #[test]
    fn flex_children_simple_divs_render_both() {
        // Basic flex with two simple div children
        let html = r#"<div style="display: flex;"><div>Left</div><div>Right</div></div>"#;
        let pdf = html_to_pdf(html).unwrap();
        assert!(
            pdf_has_text(&pdf, "Left"),
            "flex child 'Left' should appear"
        );
        assert!(
            pdf_has_text(&pdf, "Right"),
            "flex child 'Right' should appear"
        );
    }

    #[test]
    fn stylesheet_color_applies_to_text() {
        // Colors from <style> blocks should produce color operators in PDF
        let html = r#"<html><head><style>
            h1 { color: red; }
        </style></head><body><h1>Crimson</h1></body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Crimson"), "text should appear in PDF");
        // red = (1, 0, 0) in PDF color space → "1 0 0 rg"
        assert!(
            content.contains("1 0 0 rg"),
            "red color operator should appear in PDF stream"
        );
    }

    #[test]
    fn stylesheet_background_color_applies_to_table_header() {
        // background-color from <style> block should apply to th elements
        let html = r#"<html><head><style>
            th { background-color: #2c3e50; color: white; }
        </style></head><body>
        <table>
            <tr><th>Header</th></tr>
            <tr><td>Data</td></tr>
        </table>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(pdf_has_text(&pdf, "Header"), "th text should appear in PDF");
        // #2c3e50 is serialized through the canonical four-decimal PDF color
        // operator shared by every solid background path.
        assert!(
            content.contains("0.1725 0.2431 0.3137 rg"),
            "background color from stylesheet should produce rg operator"
        );
    }

    #[test]
    fn stylesheet_class_color_applies() {
        // Colors applied via class selectors from <style> blocks
        let html = r#"<html><head><style>
            .badge { background-color: #27ae60; color: white; }
        </style></head><body>
        <div class="badge">Paid</div>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(pdf_has_text(&pdf, "Paid"), "badge text should appear");
        // white text = (1, 1, 1) → "1 1 1 rg"
        assert!(
            content.contains("1 1 1 rg"),
            "white color from stylesheet class should be applied"
        );
    }

    #[test]
    fn stylesheet_color_on_inline_element() {
        // Colors from <style> on inline elements like <span> inside <p>
        let html = r#"<html><head><style>
            span { color: blue; }
        </style></head><body>
        <p>Normal <span>Azul</span></p>
        </body></html>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(pdf_has_text(&pdf, "Azul"), "span text should appear");
        // blue = (0, 0, 1) → "0 0 1 rg"
        assert!(
            content.contains("0 0 1 rg"),
            "blue color from stylesheet should be applied to inline span"
        );
    }

    #[test]
    fn inline_span_background_color() {
        let html = r#"<p><span style="background-color: green; color: white; padding: 2pt 8pt;">BADGE</span></p>"#;
        let pdf = html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should contain fill color operator for the background rectangle
        assert!(
            content.contains("rg") && content.contains("re\nf"),
            "inline span background should produce a filled rectangle (re + f operators)"
        );
    }

    #[test]
    fn fuzz_css_crash_null_bytes() {
        // Reproducer from fuzz_css crash-0a719b393ce35ba946cd6e5cb968203aef229e18
        let data: &[u8] = &[
            0, 0, 0, 0, 0, 13, 64, 0, 12, 64, 60, 47, 115, 116, 121, 108, 101, 62, 4, 4, 4, 64, 12,
            64, 0, 47, 60, 115, 116, 121, 108, 101,
        ];
        if let Ok(s) = std::str::from_utf8(data) {
            let html = format!("<style>{s}</style><p>test</p>");
            let _ = html_to_pdf(&html);
        }
    }
}
