//! JavaScript-facing WebAssembly API.

use wasm_bindgen::prelude::*;

use crate::{FontPack, FontPackKind, HtmlConverter, Margin, PageSize};

/// Reusable WebAssembly converter for browser and Node.js hosts.
#[wasm_bindgen(js_name = HtmlConverter)]
pub struct WasmHtmlConverter {
    converter: HtmlConverter,
}

impl WasmHtmlConverter {
    fn update(&mut self, configure: impl FnOnce(HtmlConverter) -> HtmlConverter) {
        self.converter = configure(std::mem::take(&mut self.converter));
    }

    fn pdf_bytes(
        result: Result<Vec<u8>, crate::IronpressError>,
    ) -> Result<js_sys::Uint8Array, JsError> {
        let bytes = result.map_err(|error| JsError::new(&error.to_string()))?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }
}

#[wasm_bindgen(js_class = "HtmlConverter")]
impl WasmHtmlConverter {
    /// Create a converter with the safe defaults from the Rust API.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            converter: HtmlConverter::new(),
        }
    }

    /// Set a named page size: A4, Letter, or Legal.
    #[wasm_bindgen(js_name = pageSize)]
    pub fn page_size(&mut self, name: &str) -> Result<(), JsError> {
        let size = match name.to_ascii_lowercase().as_str() {
            "a4" => PageSize::A4,
            "letter" => PageSize::LETTER,
            "legal" => PageSize::LEGAL,
            _ => return Err(JsError::new(&format!("unknown page size `{name}`"))),
        };
        self.update(|converter| converter.page_size(size));
        Ok(())
    }

    /// Set a custom page size in points.
    #[wasm_bindgen(js_name = pageSizeCustom)]
    pub fn page_size_custom(&mut self, width: f32, height: f32) {
        self.update(|converter| converter.page_size(PageSize::new(width, height)));
    }

    /// Set one margin value for every side, in points.
    pub fn margin(&mut self, points: f32) {
        self.update(|converter| converter.margin(Margin::uniform(points)));
    }

    /// Set top, right, bottom, and left margins in points.
    #[wasm_bindgen(js_name = marginSides)]
    pub fn margin_sides(&mut self, top: f32, right: f32, bottom: f32, left: f32) {
        self.update(|converter| converter.margin(Margin::new(top, right, bottom, left)));
    }

    /// Enable or disable PDF content-stream compression.
    pub fn compress(&mut self, enabled: bool) {
        self.update(|converter| converter.compress(enabled));
    }

    /// Set JPEG quality from 0 to 100.
    #[wasm_bindgen(js_name = jpegQuality)]
    pub fn jpeg_quality(&mut self, quality: u8) {
        self.update(|converter| converter.jpeg_quality(quality));
    }

    /// Enable or disable source-image downscaling.
    #[wasm_bindgen(js_name = autoResizeImages)]
    pub fn auto_resize_images(&mut self, enabled: bool) {
        self.update(|converter| converter.auto_resize_images(enabled));
    }

    /// Set the source-image target DPI.
    #[wasm_bindgen(js_name = imageDpi)]
    pub fn image_dpi(&mut self, dpi: f32) {
        self.update(|converter| converter.image_dpi(dpi));
    }

    /// Set the CSS filter rasterization DPI.
    #[wasm_bindgen(js_name = filterDpi)]
    pub fn filter_dpi(&mut self, dpi: f32) {
        self.update(|converter| converter.filter_dpi(dpi));
    }

    /// Set the CSS mask rasterization DPI.
    #[wasm_bindgen(js_name = maskDpi)]
    pub fn mask_dpi(&mut self, dpi: f32) {
        self.update(|converter| converter.mask_dpi(dpi));
    }

    /// Set the flattened-background rasterization DPI.
    #[wasm_bindgen(js_name = backgroundRasterDpi)]
    pub fn background_raster_dpi(&mut self, dpi: f32) {
        self.update(|converter| converter.background_raster_dpi(dpi));
    }

    /// Enable or disable conservative image occlusion culling.
    #[wasm_bindgen(js_name = occlusionCull)]
    pub fn occlusion_cull(&mut self, enabled: bool) {
        self.update(|converter| converter.occlusion_cull(enabled));
    }

    /// Enable or disable HTML sanitization.
    pub fn sanitize(&mut self, enabled: bool) {
        self.update(|converter| converter.sanitize(enabled));
    }

    /// Enable or disable accessible output: a Tagged PDF conforming to
    /// PDF/UA-1. The document needs a title and a language, from the HTML or
    /// from `documentTitle` and `documentLanguage`.
    pub fn accessible(&mut self, enabled: bool) {
        self.update(|converter| converter.accessible(enabled));
    }

    /// Set the document title of accessible output; implies accessibility.
    #[wasm_bindgen(js_name = documentTitle)]
    pub fn document_title(&mut self, title: &str) -> Result<(), JsError> {
        let title = title
            .parse::<crate::accessibility::DocumentTitle>()
            .map_err(|error| JsError::new(&error.to_string()))?;
        self.update(|converter| converter.document_title(title));
        Ok(())
    }

    /// Set the document language of accessible output as a BCP 47 tag;
    /// implies accessibility.
    #[wasm_bindgen(js_name = documentLanguage)]
    pub fn document_language(&mut self, tag: &str) -> Result<(), JsError> {
        let language = tag
            .parse::<crate::accessibility::DocumentLanguage>()
            .map_err(|error| JsError::new(&error.to_string()))?;
        self.update(|converter| converter.document_language(language));
        Ok(())
    }

    /// Register one custom TrueType font.
    #[wasm_bindgen(js_name = addFont)]
    pub fn add_font(&mut self, name: &str, bytes: &[u8]) {
        self.update(|converter| converter.add_font(name, bytes.to_vec()));
    }

    /// Parse and install one downloaded fallback-font pack.
    #[wasm_bindgen(js_name = addFontPack)]
    pub fn add_font_pack(&mut self, kind: &str, bytes: &[u8]) -> Result<(), JsError> {
        let kind = kind
            .parse::<FontPackKind>()
            .map_err(|error| JsError::new(&error.to_string()))?;
        let pack = FontPack::parse(kind, bytes.to_vec())
            .map_err(|error| JsError::new(&error.to_string()))?;
        self.update(|converter| converter.add_font_pack(pack));
        Ok(())
    }

    /// Set the text rendered in the top page margin.
    pub fn header(&mut self, text: &str) {
        self.update(|converter| converter.header(text));
    }

    /// Set an HTML fragment rendered in the top margin of every page.
    #[wasm_bindgen(js_name = headerHtml)]
    pub fn header_html(&mut self, html: &str) {
        self.update(|converter| converter.header_html(html));
    }

    /// Set footer text, with optional `{page}` and `{pages}` placeholders.
    pub fn footer(&mut self, text: &str) {
        self.update(|converter| converter.footer(text));
    }

    /// Set an HTML fragment rendered in the bottom margin of every page.
    #[wasm_bindgen(js_name = footerHtml)]
    pub fn footer_html(&mut self, html: &str) {
        self.update(|converter| converter.footer_html(html));
    }

    /// Convert HTML with this converter's settings.
    #[wasm_bindgen(js_name = htmlToPdf)]
    pub fn html_to_pdf(&self, html: &str) -> Result<js_sys::Uint8Array, JsError> {
        Self::pdf_bytes(self.converter.convert(html))
    }

    /// Convert Markdown with this converter's settings.
    #[wasm_bindgen(js_name = markdownToPdf)]
    pub fn markdown_to_pdf(&self, markdown: &str) -> Result<js_sys::Uint8Array, JsError> {
        Self::pdf_bytes(self.converter.convert_markdown(markdown))
    }
}

impl Default for WasmHtmlConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert HTML to PDF bytes with default settings.
#[wasm_bindgen(js_name = htmlToPdf)]
pub fn html_to_pdf(html: &str) -> Result<js_sys::Uint8Array, JsError> {
    WasmHtmlConverter::pdf_bytes(crate::html_to_pdf(html))
}

/// Convert Markdown to PDF bytes with default settings.
#[wasm_bindgen(js_name = markdownToPdf)]
pub fn markdown_to_pdf(markdown: &str) -> Result<js_sys::Uint8Array, JsError> {
    WasmHtmlConverter::pdf_bytes(crate::markdown_to_pdf(markdown))
}

/// Convert HTML with custom page dimensions and margins, in points.
#[wasm_bindgen(js_name = htmlToPdfCustom)]
pub fn html_to_pdf_custom(
    html: &str,
    page_width: f32,
    page_height: f32,
    margin_top: f32,
    margin_right: f32,
    margin_bottom: f32,
    margin_left: f32,
) -> Result<js_sys::Uint8Array, JsError> {
    let converter = HtmlConverter::new()
        .page_size(PageSize::new(page_width, page_height))
        .margin(Margin::new(
            margin_top,
            margin_right,
            margin_bottom,
            margin_left,
        ));
    WasmHtmlConverter::pdf_bytes(converter.convert(html))
}
