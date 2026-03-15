/// Errors that can occur during HTML-to-PDF conversion.
#[derive(Debug, thiserror::Error)]
pub enum IronpressError {
    /// HTML could not be parsed (malformed tags, encoding issues).
    #[error("HTML parsing error: {0}")]
    ParseError(String),

    /// CSS could not be parsed (invalid syntax, unsupported features).
    #[error("CSS parsing error: {0}")]
    CssError(String),

    /// Layout engine failed (e.g. content exceeds page constraints).
    #[error("Layout error: {0}")]
    LayoutError(String),

    /// PDF rendering failed (e.g. font embedding, image encoding).
    #[error("PDF rendering error: {0}")]
    RenderError(String),

    /// TrueType font could not be parsed or embedded.
    #[error("Font error: {0}")]
    FontError(String),

    /// File I/O error during read or write operations.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Input was rejected by the HTML sanitizer (size limit, nesting depth, etc.).
    #[error("Security error: input rejected: {0}")]
    SecurityError(String),
}
