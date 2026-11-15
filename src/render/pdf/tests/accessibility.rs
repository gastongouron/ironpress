use crate::accessibility::{
    DocumentLanguage, DocumentMetadata, DocumentStructure, DocumentTitle, MissingAltPolicy,
    StructureRef, StructureSkeleton,
};
use crate::render::pdf::PdfWriter;
use crate::render::pdf::structure::{MarkKind, StructureRecorder};
use crate::render::pdf::writer_output::PagePaintStreams;

fn tagged_writer(title: &str, language: &str) -> PdfWriter {
    let mut writer = PdfWriter::new();
    writer.opts.compress = false;
    writer.structure = StructureRecorder::for_document(&DocumentStructure {
        metadata: DocumentMetadata {
            title: title.parse::<DocumentTitle>().unwrap(),
            language: language.parse::<DocumentLanguage>().unwrap(),
        },
        skeleton: StructureSkeleton::assign(&mut []),
        missing_alt: MissingAltPolicy::Error,
    });
    writer
}

/// One page with a marked text object and an unmarked filled rectangle.
fn one_page_pdf(mut writer: PdfWriter) -> String {
    let mut content = String::new();
    let mark = writer
        .structure
        .begin_content(&mut content, StructureRef::Untagged, MarkKind::TEXT);
    content.push_str("BT /F1 12 Tf (x) Tj ET\n");
    writer.structure.end(&mut content, mark);
    content.push_str("0 0 10 10 re\nf\n");
    writer.structure.normalize_page_stream(&mut content);
    writer.add_page(
        100.0,
        100.0,
        PagePaintStreams::document_only(&content),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let mut pdf = Vec::new();
    writer.finish_to_writer(&mut pdf, &[]).unwrap();
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
    let pdf = one_page_pdf(tagged_writer("Quarterly report", "en-GB"));
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

    let root_id = referenced_id(catalog, "/StructTreeRoot");
    let root = object(&pdf, root_id);
    let document_id = root_id + 1;
    assert!(
        root.contains(&format!("/Type /StructTreeRoot /K [{document_id} 0 R]")),
        "{root}"
    );
    let document = object(&pdf, document_id);
    assert!(
        document.contains(&format!("/Type /StructElem /S /Document /P {root_id} 0 R")),
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
fn marked_content_resolves_through_the_parent_tree() {
    // ISO 32000-1 14.7.4.4: the page's /StructParents key maps each MCID to
    // its structure element; veraPDF 7.1-3 walks that path to decide whether
    // content is tagged. Unmarked paint is wrapped as a Layout artifact.
    let pdf = one_page_pdf(tagged_writer("T", "en"));
    let trailer = &pdf[pdf.find("trailer").unwrap()..];
    let catalog = object(&pdf, referenced_id(trailer, "/Root"));
    let root_id = referenced_id(catalog, "/StructTreeRoot");
    let document_id = root_id + 1;

    let root = object(&pdf, root_id);
    assert!(
        root.contains(&format!("/ParentTree << /Nums [0 [{document_id} 0 R]] >>")),
        "{root}"
    );
    assert!(root.contains("/ParentTreeNextKey 1"), "{root}");

    let document = object(&pdf, document_id);
    assert!(document.contains("/K [0]"), "{document}");
    assert!(document.contains("/Pg "), "{document}");

    let page = object(&pdf, referenced_id(document, "/Pg"));
    assert!(page.contains("/StructParents 0 /Tabs /S"), "{page}");

    assert!(
        pdf.contains("/Document << /MCID 0 >> BDC\nBT /F1 12 Tf (x) Tj ET\nEMC\n"),
        "{pdf}"
    );
    assert!(
        pdf.contains("/Artifact << /Type /Layout >> BDC\n0 0 10 10 re\nf\nEMC\n"),
        "{pdf}"
    );
}

#[test]
fn tagged_cross_reference_offsets_account_for_the_longer_header() {
    // ISO 32000-1 7.5.4: every in-use xref entry holds the byte offset of its
    // object.
    let pdf = one_page_pdf(tagged_writer("T", "en"));
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
    let mut writer = PdfWriter::new();
    writer.opts.compress = false;
    let pdf = one_page_pdf(writer);
    assert!(pdf.starts_with("%PDF-1.4\n"), "{pdf}");
    for key in [
        "/MarkInfo",
        "/StructTreeRoot",
        "/StructParents",
        "/Metadata",
        "/Info",
        "/Lang",
        "/ViewerPreferences",
        "pdfuaid",
        "BDC",
        "EMC",
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
    let pdf = one_page_pdf(tagged_writer("Résumé", "fr"));
    assert!(
        pdf.contains("/Title <FEFF005200E900730075006D00E9>"),
        "{pdf}"
    );
}

/// ISO 14289-1 7.21.4.1 concerns the fonts text reaches, so a tagged
/// document's font resources list only the standard fonts its content
/// selects, in their usual order; an untagged one lists all thirteen as it
/// always has.
#[test]
fn tagged_font_resources_list_only_the_standard_fonts_selected() {
    let content = "BT /Symbol 12 Tf (a) Tj /Helvetica-Bold 12 Tf (b) Tj ET\n";
    let pdf_with = |mut writer: PdfWriter| {
        writer.add_page(
            100.0,
            100.0,
            PagePaintStreams::document_only(content),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let mut pdf = Vec::new();
        writer.finish_to_writer(&mut pdf, &[]).unwrap();
        String::from_utf8(pdf).unwrap()
    };
    let tagged = pdf_with(tagged_writer("Fonts", "en"));
    let listed: Vec<&str> = tagged
        .lines()
        .filter_map(|line| line.strip_prefix("<< /Type /Font /Subtype /Type1 /BaseFont /"))
        .filter_map(|rest| rest.split(' ').next())
        .collect();
    assert_eq!(listed, ["Helvetica-Bold", "Symbol"], "{tagged}");

    let mut plain = PdfWriter::new();
    plain.opts.compress = false;
    assert_eq!(pdf_with(plain).matches("/Subtype /Type1 ").count(), 13);
}
