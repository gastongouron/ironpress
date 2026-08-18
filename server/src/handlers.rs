//! Route handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};

use crate::config::ServerConfig;
use crate::error::AppError;
use crate::{form, header_footer, params};

/// `GET /health` — liveness probe. Mirrors Gotenberg's `{"status":"up"}`.
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "up" }))
}

/// `GET /version` — reports the underlying ironpress and server versions.
pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({
        "ironpress": ironpress::VERSION,
        "server": env!("CARGO_PKG_VERSION"),
    }))
}

/// `POST /convert/html` — convert an uploaded `index.html` (plus optional
/// header/footer/asset files and rendering options) into a PDF.
pub async fn convert_html(
    State(config): State<Arc<ServerConfig>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, AppError> {
    let data = form::parse(multipart).await?;

    let index = data
        .index_html
        .ok_or_else(|| AppError::bad_request("missing required file `index.html`"))?;

    // Stage the document's assets in a private temp directory. ironpress
    // resolves relative URLs and the resource-authorization boundary against
    // this directory, so a request can only reach files it uploaded.
    let dir = tempfile::tempdir()
        .map_err(|e| AppError::internal(format!("failed to create temp directory: {e}")))?;
    for (name, bytes) in &data.assets {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes)
            .map_err(|e| AppError::internal(format!("failed to write asset `{name}`: {e}")))?;
    }

    let header = data
        .header_html
        .as_deref()
        .map(header_footer::to_text)
        .filter(|s| !s.is_empty());
    let footer = data
        .footer_html
        .as_deref()
        .map(header_footer::to_text)
        .filter(|s| !s.is_empty());

    let converter = params::build(
        &data.fields,
        dir.path(),
        config.sanitize,
        &config.network,
        header,
        footer,
    )?;

    // Conversion is CPU-bound and synchronous; keep it off the async runtime.
    let pdf = tokio::task::spawn_blocking(move || converter.convert(&index))
        .await
        .map_err(|e| AppError::internal(format!("conversion task failed: {e}")))?
        .map_err(AppError::from_conversion)?;

    // The temp directory must outlive conversion (assets are read from disk);
    // it is still in scope here, so drop it only after the PDF is produced.
    drop(dir);

    let filename = output_filename(&headers);
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        pdf,
    )
        .into_response())
}

/// Resolve the output filename from the `Gotenberg-Output-Filename` request
/// header, sanitized to a bare `*.pdf` name. Defaults to `output.pdf`.
fn output_filename(headers: &HeaderMap) -> String {
    let raw = headers
        .get("Gotenberg-Output-Filename")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| std::path::Path::new(v).file_name().and_then(|n| n.to_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("output");

    if raw.to_ascii_lowercase().ends_with(".pdf") {
        raw.to_owned()
    } else {
        format!("{raw}.pdf")
    }
}
