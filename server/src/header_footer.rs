//! Best-effort translation of Gotenberg-style header/footer HTML into the plain
//! text ironpress supports.
//!
//! Gotenberg renders `header.html` / `footer.html` as full HTML documents in
//! the page margins, using Chromium magic classes. ironpress only supports a
//! single line of text per running header/footer with `{page}` / `{pages}`
//! placeholders, so this module extracts text and maps the two page-numbering
//! classes:
//!
//! - `<span class="pageNumber"></span>` -> `{page}`
//! - `<span class="totalPages"></span>` -> `{pages}`
//!
//! All other markup (styling, `.date` / `.title` / `.url` helpers) is stripped
//! to its text content. This is a documented, lossy approximation.

use std::sync::LazyLock;

use regex::Regex;

static PAGE_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?is)<span\b[^>]*\bpageNumber\b[^>]*>.*?</span>"));
static TOTAL_PAGES: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?is)<span\b[^>]*\btotalPages\b[^>]*>.*?</span>"));
static BLOCK_ELEMENTS: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?is)<(style|script|head)\b[^>]*>.*?</(style|script|head)>"));
static TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)<[^>]+>"));
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| regex(r"\s+"));

fn regex(pattern: &str) -> Regex {
    #[allow(clippy::expect_used)]
    Regex::new(pattern).expect("static header/footer regex is valid")
}

/// Translate header/footer HTML into ironpress running-header text. Returns an
/// empty string when nothing textual remains.
pub fn to_text(html: &str) -> String {
    let s = PAGE_NUMBER.replace_all(html, "{page}");
    let s = TOTAL_PAGES.replace_all(&s, "{pages}");
    let s = BLOCK_ELEMENTS.replace_all(&s, "");
    let s = TAG.replace_all(&s, "");
    let s = decode_basic_entities(&s);
    WHITESPACE.replace_all(&s, " ").trim().to_owned()
}

fn decode_basic_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        // Ampersand last so earlier entity names are not corrupted.
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::to_text;

    #[test]
    fn maps_page_numbering_classes() {
        let html = r#"<span class="pageNumber"></span> / <span class="totalPages"></span>"#;
        assert_eq!(to_text(html), "{page} / {pages}");
    }

    #[test]
    fn keeps_static_text_and_strips_markup() {
        let html = r#"<div style="font-size:8px"><b>Quarterly&nbsp;Report</b></div>"#;
        assert_eq!(to_text(html), "Quarterly Report");
    }

    #[test]
    fn drops_style_blocks() {
        let html = "<style>.x{color:red}</style><p>Footer &amp; notes</p>";
        assert_eq!(to_text(html), "Footer & notes");
    }

    #[test]
    fn empty_when_no_text() {
        assert_eq!(to_text("<div></div>"), "");
    }
}
