//! HTTP error type mapping conversion and request failures onto status codes.

use axum::extract::multipart::MultipartError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ironpress::IronpressError;

/// An error surfaced to the HTTP client as a status code plus a plain-text body.
pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    /// A `400 Bad Request` caused by malformed or invalid client input.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    /// A `500 Internal Server Error` caused by a server-side failure.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    /// Classify an ironpress conversion failure. Errors rooted in the caller's
    /// document (parsing, CSS, layout, fonts, sanitizer rejection) are client
    /// errors; rendering and I/O failures are server errors.
    pub fn from_conversion(err: IronpressError) -> Self {
        match &err {
            IronpressError::ParseError(_)
            | IronpressError::CssError(_)
            | IronpressError::LayoutError(_)
            | IronpressError::FontError(_)
            | IronpressError::SecurityError(_) => Self::bad_request(err.to_string()),
            IronpressError::RenderError(_) | IronpressError::IoError(_) => {
                Self::internal(err.to_string())
            }
        }
    }
}

impl From<MultipartError> for AppError {
    fn from(err: MultipartError) -> Self {
        Self::bad_request(format!("invalid multipart request: {err}"))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}
