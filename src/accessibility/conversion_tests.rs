//! End-to-end checks of the accessible mode through the public converter.

use crate::{DocumentLanguage, DocumentTitle, HtmlConverter, IronpressError};

const TITLED: &str = "<html lang=\"en\"><head><title>Quarterly report</title></head>\
                      <body><h1>Q3</h1><p>Revenue grew.</p></body></html>";

fn text(pdf: &[u8]) -> String {
    String::from_utf8_lossy(pdf).into_owned()
}

#[test]
fn accessible_output_declares_pdf_ua_1() {
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .convert(TITLED)
            .unwrap(),
    );
    assert!(pdf.starts_with("%PDF-1.7\n"));
    assert!(pdf.contains("/MarkInfo << /Marked true >>"), "{pdf}");
    assert!(pdf.contains("/Lang (en)"), "{pdf}");
    assert!(pdf.contains("<pdfuaid:part>1</pdfuaid:part>"), "{pdf}");
    assert!(pdf.contains("/Title (Quarterly report)"), "{pdf}");
}

#[test]
fn explicit_title_and_language_override_the_html() {
    let pdf = text(
        &HtmlConverter::new()
            .document_title("Override".parse::<DocumentTitle>().unwrap())
            .document_language("fr-CA".parse::<DocumentLanguage>().unwrap())
            .convert(TITLED)
            .unwrap(),
    );
    assert!(pdf.starts_with("%PDF-1.7\n"));
    assert!(pdf.contains("/Lang (fr-CA)"), "{pdf}");
    assert!(pdf.contains("/Title (Override)"), "{pdf}");
    assert!(!pdf.contains("Quarterly report"), "{pdf}");
}

#[test]
fn markdown_takes_its_title_from_the_first_heading() {
    let pdf = text(
        &HtmlConverter::new()
            .document_language("en".parse().unwrap())
            .convert_markdown("## Release notes\n\nText")
            .unwrap(),
    );
    assert!(pdf.contains("/Title (Release notes)"), "{pdf}");
}

#[test]
fn missing_title_or_language_fails_with_a_hint() {
    let converter = HtmlConverter::new().accessible(true);

    let error = converter.convert("<p>No title</p>").unwrap_err();
    assert!(
        matches!(&error, IronpressError::RenderError(message)
            if message.starts_with("PDF/UA: the document has no title") && message.contains("--title")),
        "{error}"
    );

    let error = converter.convert("<h1>Titled</h1>").unwrap_err();
    assert!(
        matches!(&error, IronpressError::RenderError(message)
            if message.starts_with("PDF/UA: the document language is unknown") && message.contains("--lang")),
        "{error}"
    );
}

#[test]
fn default_output_has_no_tagging() {
    let pdf = text(&HtmlConverter::new().convert(TITLED).unwrap());
    assert!(pdf.starts_with("%PDF-1.4\n"));
    for marker in [
        "/MarkInfo",
        "/StructTreeRoot",
        "/Metadata",
        "/Info",
        "pdfuaid",
        "BDC\n",
        "\nEMC",
    ] {
        assert!(
            !pdf.contains(marker),
            "default output must not contain {marker:?}"
        );
    }
}

#[test]
fn disabling_accessibility_restores_byte_identical_default_output() {
    let default = HtmlConverter::new().convert(TITLED).unwrap();
    assert_eq!(
        HtmlConverter::new()
            .accessible(false)
            .convert(TITLED)
            .unwrap(),
        default
    );
    assert_eq!(
        HtmlConverter::new()
            .document_title("T".parse().unwrap())
            .accessible(false)
            .convert(TITLED)
            .unwrap(),
        default
    );
    assert_eq!(
        HtmlConverter::new()
            .accessible(true)
            .accessible(false)
            .convert(TITLED)
            .unwrap(),
        default
    );
}
