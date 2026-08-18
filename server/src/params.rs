//! Mapping of per-request form fields onto an [`HtmlConverter`].
//!
//! Only safe rendering knobs are accepted here. Security-relevant settings
//! (sanitization, network policy) come from [`crate::config::ServerConfig`] and
//! are passed in explicitly — never read from the request.
//!
//! Geometry fields use Gotenberg's units (inches) and are converted to points.
//! Browser-only Gotenberg fields (`scale`, `waitDelay`, `emulatedMediaType`,
//! `nativePageRanges`, JavaScript options, ...) have no meaning without a
//! browser and are ignored.

use std::collections::HashMap;
use std::path::Path;

use ironpress::{HtmlConverter, Margin, NetworkPolicy, PageSize};

use crate::error::AppError;

/// Points per inch. Gotenberg expresses paper and margin sizes in inches;
/// ironpress works in points.
const PT_PER_INCH: f32 = 72.0;

/// Build a converter from the request fields, injecting the operator-controlled
/// `sanitize` policy and the per-request asset base directory.
pub fn build(
    fields: &HashMap<String, String>,
    base: &Path,
    sanitize: bool,
    network: &NetworkPolicy,
    header: Option<String>,
    footer: Option<String>,
) -> Result<HtmlConverter, AppError> {
    // Paper size: start from A4 and override either dimension if supplied.
    let mut page = PageSize::A4;
    if let Some(w) = parse_f32(fields, "paperWidth")? {
        page.width = w * PT_PER_INCH;
    }
    if let Some(h) = parse_f32(fields, "paperHeight")? {
        page.height = h * PT_PER_INCH;
    }
    if parse_bool(fields, "landscape")?.unwrap_or(false) {
        page = PageSize::new(page.height, page.width);
    }

    // Margins: start from the ironpress default (1 inch) and override per side.
    let mut margin = Margin::default();
    if let Some(v) = parse_f32(fields, "marginTop")? {
        margin.top = v * PT_PER_INCH;
    }
    if let Some(v) = parse_f32(fields, "marginBottom")? {
        margin.bottom = v * PT_PER_INCH;
    }
    if let Some(v) = parse_f32(fields, "marginLeft")? {
        margin.left = v * PT_PER_INCH;
    }
    if let Some(v) = parse_f32(fields, "marginRight")? {
        margin.right = v * PT_PER_INCH;
    }

    let mut converter = HtmlConverter::new()
        .page_size(page)
        .margin(margin)
        .sanitize(sanitize)
        .base_path(base)
        .resource_root(base)
        // Operator-controlled remote policy (inert unless the `remote` feature
        // is built in). Never sourced from the request.
        .network_policy(network.clone());

    // ironpress-native rendering knobs.
    if let Some(v) = parse_bool(fields, "compress")? {
        converter = converter.compress(v);
    }
    if let Some(v) = parse_bool(fields, "autoResizeImages")? {
        converter = converter.auto_resize_images(v);
    }
    if let Some(v) = parse_u8_clamped(fields, "jpegQuality", 0, 100)? {
        converter = converter.jpeg_quality(v);
    }
    if let Some(v) = parse_f32(fields, "imageDpi")? {
        converter = converter.image_dpi(v);
    }
    if let Some(v) = parse_f32(fields, "filterDpi")? {
        converter = converter.filter_dpi(v);
    }
    if let Some(v) = parse_f32(fields, "maskDpi")? {
        converter = converter.mask_dpi(v);
    }
    if let Some(v) = parse_f32(fields, "backgroundRasterDpi")? {
        converter = converter.background_raster_dpi(v);
    }
    if let Some(v) = parse_bool(fields, "occlusionCull")? {
        converter = converter.occlusion_cull(v);
    }

    if let Some(text) = header {
        if !text.is_empty() {
            converter = converter.header(text);
        }
    }
    if let Some(text) = footer {
        if !text.is_empty() {
            converter = converter.footer(text);
        }
    }

    // Accepted for Gotenberg compatibility but need no explicit action:
    // ironpress already honors `@page` rules (preferCssPageSize) and always
    // paints backgrounds (printBackground). Parse them so an invalid value is
    // still rejected rather than silently misread.
    let _ = parse_bool(fields, "preferCssPageSize")?;
    let _ = parse_bool(fields, "printBackground")?;

    Ok(converter)
}

fn parse_f32(fields: &HashMap<String, String>, key: &str) -> Result<Option<f32>, AppError> {
    match fields.get(key) {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse::<f32>()
            .map(Some)
            .map_err(|_| AppError::bad_request(format!("field `{key}` must be a number, got `{raw}`"))),
    }
}

fn parse_bool(fields: &HashMap<String, String>, key: &str) -> Result<Option<bool>, AppError> {
    match fields.get(key) {
        None => Ok(None),
        Some(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Some(true)),
            "false" | "0" | "no" | "off" => Ok(Some(false)),
            _ => Err(AppError::bad_request(format!(
                "field `{key}` must be a boolean, got `{raw}`"
            ))),
        },
    }
}

fn parse_u8_clamped(
    fields: &HashMap<String, String>,
    key: &str,
    min: u8,
    max: u8,
) -> Result<Option<u8>, AppError> {
    match fields.get(key) {
        None => Ok(None),
        Some(raw) => raw
            .trim()
            .parse::<i64>()
            .map(|n| Some(n.clamp(i64::from(min), i64::from(max)) as u8))
            .map_err(|_| {
                AppError::bad_request(format!("field `{key}` must be an integer, got `{raw}`"))
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn builds_with_defaults() {
        let base = std::path::Path::new("/tmp");
        let net = NetworkPolicy::default();
        assert!(build(&fields(&[]), base, true, &net, None, None).is_ok());
    }

    #[test]
    fn rejects_non_numeric_paper_width() {
        let base = std::path::Path::new("/tmp");
        let net = NetworkPolicy::default();
        let err = build(&fields(&[("paperWidth", "wide")]), base, true, &net, None, None);
        assert!(err.is_err());
    }

    #[test]
    fn rejects_bad_boolean() {
        let base = std::path::Path::new("/tmp");
        let net = NetworkPolicy::default();
        assert!(build(&fields(&[("landscape", "maybe")]), base, true, &net, None, None).is_err());
    }

    #[test]
    fn accepts_full_option_set() {
        let base = std::path::Path::new("/tmp");
        let net = NetworkPolicy::default();
        let f = fields(&[
            ("paperWidth", "8.5"),
            ("paperHeight", "11"),
            ("marginTop", "0.5"),
            ("landscape", "true"),
            ("jpegQuality", "80"),
            ("imageDpi", "150"),
            ("autoResizeImages", "false"),
            ("compress", "true"),
            ("preferCssPageSize", "true"),
            ("printBackground", "true"),
        ]);
        assert!(build(&f, base, true, &net, Some("Title".into()), Some("{page}".into())).is_ok());
    }
}
