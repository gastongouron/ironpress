use crate::accessibility::{DocumentLanguage, DocumentMetadata, DocumentTitle};
use crate::render::pdf::PdfWriter;
use crate::render::pdf::writer_output::PagePaintStreams;

fn metadata(title: &str, language: &str) -> DocumentMetadata {
    DocumentMetadata {
        title: title.parse::<DocumentTitle>().unwrap(),
        language: language.parse::<DocumentLanguage>().unwrap(),
    }
}

fn one_page_pdf(tagged: Option<&DocumentMetadata>) -> String {
    let mut writer = PdfWriter::new();
    writer.opts.compress = false;
    writer.add_page(
        100.0,
        100.0,
        PagePaintStreams::document_only("0 0 10 10 re f"),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut pdf = Vec::new();
    writer.finish_to_writer(&mut pdf, &[], tagged).unwrap();
    String::from_utf8(pdf).unwrap()
}

fn object(pdf: &str, id: usize) -> &str {
    let start = pdf.find(&format!("\n{id} 0 obj\n")).unwrap() + 1;
    let end = start + pdf[start..].find("endobj").unwrap();
    &pdf[start..end]
}

/// The object number of the indirect reference that follows `key`.
fn referenced_id(text: &str, key: &str) -> usize {
    let at = text.find(key).unwrap() + key.len();
    text[at..]
        .trim_start()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn tagged_documents_declare_pdf_ua_1_in_the_catalog_and_metadata() {
    // veraPDF ua1 rules: 6.1-1 (a 1.7 header is allowed), 6.2-1 (/MarkInfo
    // /Marked true), 7.1-11 (/StructTreeRoot present), 7.2-2 (catalog /Lang),
    // 7.1-8 (XMP dc:title) and 7.1-10 (/ViewerPreferences /DisplayDocTitle
    // true); ISO 32000-1 14.3.2 (/Metadata stream typed /Metadata /XML) and
    // 14.7.2 (every structure element has /P).
    let pdf = one_page_pdf(Some(&metadata("Quarterly report", "en-GB")));
    assert!(pdf.starts_with("%PDF-1.7\n"), "{pdf}");

    let trailer = &pdf[pdf.find("trailer").unwrap()..];
    let catalog = object(&pdf, referenced_id(trailer, "/Root"));
    assert!(catalog.contains("/Type /Catalog"), "{catalog}");
    assert!(
        catalog.contains("/MarkInfo << /Marked true >>"),
        "{catalog}"
    );
    assert!(catalog.contains("/Lang (en-GB)"), "{catalog}");
    assert!(
        catalog.contains("/ViewerPreferences << /DisplayDocTitle true >>"),
        "{catalog}"
    );

    let root = object(&pdf, referenced_id(catalog, "/StructTreeRoot"));
    assert!(root.contains("/Type /StructTreeRoot /K ["), "{root}");
    let document = object(&pdf, referenced_id(root, "/K ["));
    assert!(
        document.contains("/Type /StructElem /S /Document /P "),
        "{document}"
    );
    assert!(document.contains("/Lang (en-GB)"), "{document}");

    let xmp = object(&pdf, referenced_id(catalog, "/Metadata"));
    assert!(xmp.contains("/Type /Metadata /Subtype /XML"), "{xmp}");
    assert!(xmp.contains("<pdfuaid:part>1</pdfuaid:part>"), "{xmp}");
    assert!(
        xmp.contains("<rdf:li xml:lang=\"x-default\">Quarterly report</rdf:li>"),
        "{xmp}"
    );

    let info = object(&pdf, referenced_id(trailer, "/Info"));
    assert!(info.contains("/Title (Quarterly report)"), "{info}");
}

#[test]
fn tagged_cross_reference_offsets_account_for_the_longer_header() {
    // ISO 32000-1 7.5.4: every in-use xref entry holds the byte offset of its
    // object.
    let pdf = one_page_pdf(Some(&metadata("T", "en")));
    let xref = &pdf[pdf.find("xref\n").unwrap()..];
    let offsets: Vec<usize> = xref
        .lines()
        .skip(3)
        .take_while(|line| line.ends_with(" n "))
        .map(|line| line[..10].parse().unwrap())
        .collect();
    assert!(offsets.len() >= 4, "{xref}");
    for (index, offset) in offsets.iter().enumerate() {
        let expected = format!("{} 0 obj\n", index + 1);
        assert!(
            pdf[*offset..].starts_with(&expected),
            "object {} is not at offset {offset}",
            index + 1
        );
    }
}

#[test]
fn untagged_documents_keep_the_historical_header_and_carry_no_tagging_keys() {
    let pdf = one_page_pdf(None);
    assert!(pdf.starts_with("%PDF-1.4\n"), "{pdf}");
    for key in [
        "/MarkInfo",
        "/StructTreeRoot",
        "/Metadata",
        "/Info",
        "/Lang",
        "/ViewerPreferences",
        "pdfuaid",
    ] {
        assert!(
            !pdf.contains(key),
            "{key} must not appear in untagged output"
        );
    }
}

#[test]
fn non_ascii_titles_are_written_as_utf16_text_strings() {
    // ISO 32000-1 7.9.2.2: text strings outside PDFDocEncoding use UTF-16BE
    // with a byte-order mark.
    let pdf = one_page_pdf(Some(&metadata("Résumé", "fr")));
    assert!(
        pdf.contains("/Title <FEFF005200E900730075006D00E9>"),
        "{pdf}"
    );
}
