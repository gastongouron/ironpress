//! Multipart request parsing.
//!
//! The request follows the Gotenberg convention: files are identified by their
//! `filename`, and everything else is a plain form field. `index.html` is the
//! document; `header.html` / `footer.html` are the optional running header and
//! footer; every other uploaded file is treated as a relative asset.

use std::collections::HashMap;
use std::path::Path;

use axum::extract::Multipart;

use crate::error::AppError;

/// The decomposed parts of a conversion request.
#[derive(Default)]
pub struct FormData {
    /// Contents of the required `index.html` document.
    pub index_html: Option<String>,
    /// Raw HTML of an optional running header file.
    pub header_html: Option<String>,
    /// Raw HTML of an optional running footer file.
    pub footer_html: Option<String>,
    /// Additional uploaded files as `(file_name, bytes)`, referenced by
    /// relative URL from the document.
    pub assets: Vec<(String, Vec<u8>)>,
    /// Non-file form fields (rendering options).
    pub fields: HashMap<String, String>,
}

/// Consume a multipart body into its [`FormData`] parts.
pub async fn parse(mut multipart: Multipart) -> Result<FormData, AppError> {
    let mut data = FormData::default();

    while let Some(field) = multipart.next_field().await? {
        let field_name = field.name().map(str::to_owned);
        let file_name = field.file_name().map(str::to_owned);

        match file_name {
            // A file part.
            Some(name) => {
                let bytes = field.bytes().await?;
                match name.as_str() {
                    "index.html" => {
                        data.index_html = Some(String::from_utf8_lossy(&bytes).into_owned());
                    }
                    "header.html" => {
                        data.header_html = Some(String::from_utf8_lossy(&bytes).into_owned());
                    }
                    "footer.html" => {
                        data.footer_html = Some(String::from_utf8_lossy(&bytes).into_owned());
                    }
                    // Any other file is an asset. Keep only the final path
                    // component so a crafted `filename` cannot escape the
                    // per-request temp directory.
                    _ => {
                        if let Some(base) = Path::new(&name).file_name().and_then(|n| n.to_str()) {
                            data.assets.push((base.to_owned(), bytes.to_vec()));
                        }
                    }
                }
            }
            // A plain form field (rendering option).
            None => {
                if let Some(key) = field_name {
                    let value = field.text().await?;
                    data.fields.insert(key, value);
                }
            }
        }
    }

    Ok(data)
}
