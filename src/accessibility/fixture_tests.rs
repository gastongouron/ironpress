//! The PDF/UA fixture document through the public converter, checked for
//! the structure veraPDF's `ua1` profile requires.

use crate::{HtmlConverter, IronpressError, MissingAltPolicy, TaggedPdf};

const REPORT: &str = include_str!("../../tests/pdfua/report.html");
const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

fn text(pdf: &[u8]) -> String {
    String::from_utf8_lossy(pdf).into_owned()
}

fn tagged(html: &str) -> String {
    text(&HtmlConverter::new().accessible(true).convert(html).unwrap())
}

#[test]
fn report_fixture_carries_every_required_structure_type() {
    // ISO 32000-1 Tables 333–338 types as PDF/UA-1 7.1–7.9 and 7.18 use them;
    // veraPDF rules 7.2-40 (ListNumbering), 7.5-1 (TH Scope), 7.3-1 (Figure
    // Alt), 7.18.5-1/2 (Link with OBJR and /Contents), Matterhorn 18-001
    // (header and footer as Pagination artifacts).
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .header("Quarterly report")
            .footer("Page {page} of {pages}")
            .convert(REPORT)
            .unwrap(),
    );
    for needle in [
        "/S /H1 ",
        "/S /H2 ",
        "/S /P ",
        "/S /L ",
        "/S /LI ",
        "/S /Lbl ",
        "/S /LBody ",
        "/S /Table ",
        "/S /THead ",
        "/S /TBody ",
        "/S /TR ",
        "/S /TH ",
        "/S /TD ",
        "/S /Caption ",
        "/S /Figure ",
        "/S /Link ",
        "/Type /OBJR",
        "/ListNumbering /Disc",
        "/ListNumbering /Decimal",
        "/Scope /Column",
        "/Scope /Row",
        "/ColSpan 2",
        "/Alt (Revenue by region, North leading)",
        "/Alt (Upward trend)",
        "/O /Layout /BBox [",
        "/StructParent ",
        "/Contents (the third quarter)",
        "/Artifact << /Type /Pagination >> BDC",
        "/Lbl << /MCID ",
        "/ActualText <FEFF2022>",
    ] {
        assert!(pdf.contains(needle), "missing {needle:?}");
    }
    assert!(
        !pdf.contains("/S /H "),
        "the unnumbered H type is never used"
    );
    assert!(
        !pdf.contains("/Helvetica 9 Tf"),
        "header text must use an embedded font"
    );
    // Two pages of the detail table: the repeated header row is furniture,
    // so exactly one THead per table remains.
    assert_eq!(pdf.matches("/S /THead ").count(), 2, "{pdf}");
    assert!(pdf.matches("/S /Document ").count() == 1);
}

#[test]
fn ragged_rows_are_padded_in_the_output() {
    // veraPDF 7.2-41: the second body row has one data cell for two columns.
    let pdf = tagged(
        "<html lang='en'><head><title>T</title></head><body>\
         <table><tr><th>A</th><th>B</th><th>C</th></tr><tr><td>1</td></tr></table></body></html>",
    );
    assert_eq!(pdf.matches("/S /TD ").count(), 3, "{pdf}");
    assert_eq!(pdf.matches("/Scope /Column").count(), 3, "{pdf}");
}

#[test]
fn heading_levels_are_normalized_in_structure_order() {
    // PDF/UA-1 7.4.2 (veraPDF 7.4.4-x): the first heading is H1 and no level
    // is skipped going down; the outline keeps the authored levels.
    let pdf = tagged(
        "<html lang='en'><head><title>T</title></head><body>\
         <h3>First</h3><h1>Then</h1><h4>Deep</h4></body></html>",
    );
    assert_eq!(pdf.matches("/S /H1 ").count(), 2, "{pdf}");
    assert_eq!(pdf.matches("/S /H2 ").count(), 1, "{pdf}");
    assert!(
        !pdf.contains("/S /H3 ") && !pdf.contains("/S /H4 "),
        "{pdf}"
    );
}

#[test]
fn hidden_content_is_an_artifact_and_its_links_are_dropped() {
    // ISO 14289-1 7.1 and 7.18.5: aria-hidden content is an artifact, and an
    // artifact cannot carry a link annotation.
    let pdf = tagged(
        "<html lang='en'><head><title>T</title></head><body>\
         <p>Visible <span aria-hidden=\"true\">secret <a href=\"https://x\">link</a></span></p>\
         </body></html>",
    );
    assert!(pdf.contains("/Artifact BMC"), "{pdf}");
    assert!(!pdf.contains("/S /Link "), "{pdf}");
    assert!(!pdf.contains("/Subtype /Link"), "{pdf}");
}

#[test]
fn images_without_alt_fail_unless_the_policy_hides_them() {
    // veraPDF 7.3-1: a Figure needs /Alt; the default refuses to guess one.
    let html = format!(
        "<html lang='en'><head><title>T</title></head><body>\
         <p>x</p><img src=\"{PIXEL}\" width=\"10\" height=\"10\"></body></html>"
    );
    let error = HtmlConverter::new()
        .accessible(true)
        .convert(&html)
        .unwrap_err();
    assert!(
        matches!(&error, IronpressError::RenderError(message)
            if message.starts_with("PDF/UA: <img src=") && message.contains("alt")),
        "{error}"
    );

    let pdf = text(
        &HtmlConverter::new()
            .tagged_pdf(TaggedPdf {
                missing_alt: MissingAltPolicy::Artifact,
                ..TaggedPdf::default()
            })
            .convert(&html)
            .unwrap(),
    );
    assert!(!pdf.contains("/S /Figure "), "{pdf}");
    assert!(pdf.contains("/Artifact BMC"), "{pdf}");

    let decorative = tagged(&html.replace("width=", "alt=\"\" width="));
    assert!(!decorative.contains("/S /Figure "), "{decorative}");
}

#[test]
fn every_mcid_in_the_fixture_resolves_through_the_parent_tree() {
    // veraPDF 7.1-3 walks each MCID's parent-tree entry to the root; a mark
    // that no element owns would make its content untagged.
    let pdf = tagged(REPORT);
    // Marks on a later page than their element's /Pg are MCR dictionaries,
    // which also spell /MCID but are references, not content.
    let marked = pdf.matches("/MCID ").count() - pdf.matches("/Type /MCR").count();
    let nums_start = pdf.find("/ParentTree << /Nums [").unwrap();
    let nums_end = nums_start + pdf[nums_start..].find("] >>").unwrap();
    let references = pdf[nums_start..nums_end].matches(" 0 R").count();
    assert!(marked > 0);
    assert!(
        references >= marked,
        "{marked} marks but {references} parent-tree references"
    );
}
