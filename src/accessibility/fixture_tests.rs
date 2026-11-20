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

/// ISO 14289-1 7.21.4.1: every font used for rendering is embedded, so no
/// text operator in accessible output selects a Base-14 font (page furniture,
/// formulas and SVG text included).
#[test]
fn report_fixture_selects_no_base14_font() {
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .compress(false)
            .header("Quarterly report")
            .footer("Page {page} of {pages}")
            .convert(REPORT)
            .unwrap(),
    );
    let base14: Vec<&str> = pdf
        .lines()
        .filter(|line| {
            line.ends_with(" Tf")
                && [
                    "/Helvetica",
                    "/Times",
                    "/Courier",
                    "/Symbol",
                    "/ZapfDingbats",
                ]
                .iter()
                .any(|font| line.starts_with(font))
        })
        .collect();
    assert!(base14.is_empty(), "{base14:?}");
}

/// ISO 32000-1 Table 335: a definition list is an `L` whose items are `LI`
/// elements holding one or more `Lbl` (terms) and `LBody` (descriptions);
/// a term after a description starts a new item. PDF/UA-1 7.2 (veraPDF
/// 7.2-40): the list declares its numbering, `None` for terms.
#[test]
fn definition_lists_group_terms_and_descriptions_into_items() {
    let pdf = tagged(
        r#"<html lang="en"><head><title>D</title></head><body>
        <dl><dt>Alpha</dt><dd>First</dd><dt>Beta</dt><dt>Gamma</dt><dd>Second</dd><dd>Third</dd></dl>
        </body></html>"#,
    );
    assert_eq!(pdf.matches("/S /L ").count(), 1, "{pdf}");
    assert!(pdf.contains("/ListNumbering /None"), "{pdf}");
    assert_eq!(pdf.matches("/S /Lbl ").count(), 3, "{pdf}");
    assert_eq!(pdf.matches("/S /LBody ").count(), 3, "{pdf}");
    assert!(!pdf.contains("/S /P "), "{pdf}");
    let item_kids: Vec<usize> = pdf
        .match_indices("/S /LI ")
        .map(|(at, _)| {
            let kids = &pdf[at..];
            let kids = &kids[kids.find("/K [").unwrap() + "/K [".len()..];
            kids[..kids.find(']').unwrap()].matches(" 0 R").count()
        })
        .collect();
    assert_eq!(item_kids, vec![2, 4], "{pdf}");
}

/// ISO 32000-1 12.3.2.3: an `#id` link is a named destination resolved
/// through the catalog's `/Dests` name tree (keys in byte order, 7.9.6) to
/// the anchor's page. A link to no element is dropped: a `Link` whose
/// annotation leads nowhere is dead for assistive technology (ISO 14289-1
/// 7.18.5).
#[test]
fn anchor_links_resolve_to_named_destinations() {
    let pdf = tagged(
        r##"<html lang="en"><head><title>A</title></head><body>
        <h1 id="top">Top</h1><p><a href="#end">Go to end</a></p>
        <p style="height: 800pt">x</p><h2 id="end">End</h2>
        <p><a href="#top">Back</a> <a href="#missing">Nowhere</a></p></body></html>"##,
    );
    assert_eq!(pdf.matches("/Subtype /Link").count(), 2, "{pdf}");
    assert!(
        pdf.contains("/Dest (end) /F 4 /Contents (Go to end)"),
        "{pdf}"
    );
    assert!(pdf.contains("/Dest (top) /F 4 /Contents (Back)"), "{pdf}");
    assert!(!pdf.contains("missing"), "{pdf}");
    assert!(pdf.contains("/Names << /Dests "), "{pdf}");
    let kids = &pdf[pdf.find("/Kids [").unwrap() + "/Kids [".len()..];
    let second_page = kids.split(' ').nth(3).unwrap();
    let tree = &pdf[pdf.find("/Names [(end) [").unwrap()..];
    assert!(
        tree.starts_with(&format!("/Names [(end) [{second_page} 0 R /XYZ null ")),
        "{pdf}"
    );
    assert!(tree.contains(" null] (top) ["), "{pdf}");
}

/// Matterhorn 18-001 / ISO 14289-1 7.1: a `position: fixed` box is content
/// where it is authored and page furniture on every later page, so its text
/// is a content item exactly once and each copy is a Pagination artifact.
#[test]
fn fixed_boxes_are_content_once_and_pagination_artifacts_afterwards() {
    let pdf = tagged(
        r#"<html lang="en"><head><title>F</title></head><body>
        <div style="position: fixed; top: 0">Running note</div>
        <p style="height: 800pt">Body</p></body></html>"#,
    );
    assert_eq!(pdf.matches("/Type /Page /MediaBox").count(), 2, "{pdf}");
    assert_eq!(pdf.matches("/S /Div ").count(), 1, "{pdf}");
    assert_eq!(pdf.matches("/Div << /MCID ").count(), 1, "{pdf}");
    assert_eq!(
        pdf.matches("/Artifact << /Type /Pagination >> BDC").count(),
        1,
        "{pdf}"
    );
}

/// ISO 32000-1 14.9.5: an abbreviation's expansion is the `/E` entry of its
/// structure element; a `<abbr>` without a title adds no element.
#[test]
fn abbreviation_titles_become_expansions() {
    let pdf = tagged(
        r#"<html lang="en"><head><title>E</title></head><body>
        <p>Set in <abbr title="Portable Document Format">PDF</abbr> and <abbr>HTML</abbr>.</p>
        </body></html>"#,
    );
    assert!(
        pdf.contains("/S /Span /P ") && pdf.contains("/E (Portable Document Format)"),
        "{pdf}"
    );
    assert_eq!(pdf.matches("/S /Span ").count(), 1, "{pdf}");
}

fn tagged_body(body: &str) -> String {
    text(
        &HtmlConverter::new()
            .accessible(true)
            .compress(false)
            .convert(&format!(
                r#"<html lang="en"><head><title>B</title></head><body>{body}</body></html>"#
            ))
            .unwrap(),
    )
}

/// The `/ActualText` of every mark whose content is a picture.
fn rasterized_actual_text(pdf: &str) -> Vec<&str> {
    pdf.match_indices("/ActualText ")
        .filter_map(|(at, _)| {
            let mark = &pdf[at + "/ActualText ".len()..];
            let text = &mark[..mark.find(" >> BDC\n")?];
            let body = &mark[..mark.find("EMC")?];
            body.contains(" Do\n").then_some(text)
        })
        .collect()
}

/// ISO 32000-1 14.9.4 and ISO 14289-1 7.1: a filtered block is painted as
/// one picture, so that picture is marked content of the block whose
/// `/ActualText` carries its lines joined by spaces; no text operator and no
/// Figure remain.
#[test]
fn filtered_text_blocks_are_read_through_actual_text() {
    let pdf = tagged_body(r#"<p style="filter: blur(1pt); width: 30pt">Soft text</p>"#);
    assert_eq!(rasterized_actual_text(&pdf), vec!["(Soft text)"], "{pdf}");
    assert_eq!(pdf.matches("/ActualText ").count(), 1, "{pdf}");
    assert!(!pdf.contains("/Figure"), "{pdf}");
    assert!(!pdf.contains(" Tj\n") && !pdf.contains(" TJ\n"), "{pdf}");
}

/// ISO 32000-1 14.9.4: a filtered flex item is one picture too, marked with
/// the item's text.
#[test]
fn filtered_flex_items_are_read_through_actual_text() {
    let pdf = tagged_body(
        r#"<div style="display: flex"><div style="filter: blur(1pt)">Cell <b>text</b></div></div>"#,
    );
    assert_eq!(rasterized_actual_text(&pdf), vec!["(Cell text)"], "{pdf}");
    assert!(!pdf.contains(" Tj\n") && !pdf.contains(" TJ\n"), "{pdf}");
}

/// UAX #9 and ISO 14289-1 7.1: a right-to-left line is painted in visual
/// order, but its marks are kids of the paragraph in logical order; under
/// `unicode-bidi: bidi-override` the reversed glyphs carry the logical text
/// in `/ActualText` (ISO 32000-1 14.9.4).
#[test]
fn right_to_left_lines_read_in_logical_order() {
    let pdf = tagged_body(
        r#"<p dir="rtl">אב <b>גד</b></p>
        <p style="direction: rtl; unicode-bidi: bidi-override">abc</p>"#,
    );
    let first = pdf.find("/S /P ").unwrap();
    let paragraph = &pdf[first..first + pdf[first..].find("endobj").unwrap()];
    assert!(paragraph.contains("/K [1 0]"), "{pdf}");
    assert!(
        pdf.contains("/P << /MCID 2 /ActualText (abc) >> BDC"),
        "{pdf}"
    );
}

/// ISO 14289-1 7.21.4.1 concerns the fonts text reaches, so accessible
/// output lists only the standard fonts its content selects: none for the
/// report, whose text is all embedded; historical output keeps all thirteen.
#[test]
fn accessible_output_lists_only_the_standard_fonts_it_selects() {
    let report = tagged(REPORT);
    assert_eq!(report.matches("/Subtype /Type1 ").count(), 0, "{report}");
    let plain = text(&HtmlConverter::new().convert(REPORT).unwrap());
    assert_eq!(plain.matches("/Subtype /Type1 ").count(), 13, "{plain}");
}

/// ISO 32000-1 Table 112: a Type 3 font (here the synthetic bold of a
/// single-weight face) carries a font descriptor in a Tagged PDF document
/// and none otherwise; Table 122 requires its Flags, ItalicAngle and StemV.
#[test]
fn type3_fonts_carry_a_descriptor_in_tagged_output_only() {
    let font = include_bytes!("../../assets/LiberationSans-Regular.ttf").to_vec();
    let html = r#"<html lang="en"><head><title>T</title></head><body>
        <p style="font-family: single; font-weight: bold">Bold</p></body></html>"#;
    let tagged = text(
        &HtmlConverter::new()
            .accessible(true)
            .add_font("single", font.clone())
            .convert(html)
            .unwrap(),
    );
    let type3 = tagged
        .lines()
        .find(|line| line.contains("/Subtype /Type3 "))
        .unwrap_or_default();
    assert!(type3.contains(" /FontDescriptor "), "{tagged}");
    assert_eq!(
        tagged
            .matches("<< /Type /FontDescriptor /Flags 32 /ItalicAngle 0 /StemV 80 >>")
            .count(),
        1,
        "{tagged}"
    );
    let plain = text(
        &HtmlConverter::new()
            .add_font("single", font)
            .convert(html)
            .unwrap(),
    );
    assert!(plain.contains("/Subtype /Type3 "), "{plain}");
    assert!(!plain.contains("/FontDescriptor"), "{plain}");
}

/// css-content-3 §1.2 and ISO 14289-1 7.3 (veraPDF 7.3-1): a generated
/// image with alternative text (`content: url() / "…"`) is a Figure of its
/// own under the element that generated it, with that text as `/Alt` and its
/// box as the Layout `/BBox`; `attr()` reads the element's attribute; an
/// image without alternative text is decoration, painted as an artifact.
#[test]
fn generated_images_are_figures_when_named_and_artifacts_otherwise() {
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .compress(false)
            .convert(&format!(
                r#"<html lang="en"><head><title>G</title><style>
                .named::before {{ content: url("{PIXEL}") / "Warning"; }}
                .attr::before {{ content: url("{PIXEL}") / attr(data-alt); }}
                .plain::before {{ content: url("{PIXEL}"); }}
                </style></head><body>
                <p class="named">Named</p><p class="attr" data-alt="Note">Attr</p>
                <p class="plain">Plain</p></body></html>"#
            ))
            .unwrap(),
    );
    assert_eq!(pdf.matches("/S /Figure ").count(), 2, "{pdf}");
    assert!(
        pdf.contains("/Alt (Warning) /A << /O /Layout /BBox ["),
        "{pdf}"
    );
    assert!(
        pdf.contains("/Alt (Note) /A << /O /Layout /BBox ["),
        "{pdf}"
    );
    assert_eq!(pdf.matches("/Figure << /MCID ").count(), 2, "{pdf}");
    assert_eq!(pdf.matches("/Artifact BMC\n").count(), 1, "{pdf}");
    assert_eq!(pdf.matches("/S /P ").count(), 3, "{pdf}");
}

/// ISO 32000-1 9.3.6 and 14.9.4: a `background-clip: text` line clips its
/// gradient with one text object, which is the line's content read through
/// `/ActualText` (a synthetic weight repeats the outlines inside it); the
/// gradient fill after it is a Layout artifact and the text is not shown
/// again.
#[test]
fn background_clip_text_lines_are_read_once() {
    let pdf = tagged_body(
        r#"<p style="background: linear-gradient(red, blue); -webkit-background-clip: text; background-clip: text; color: transparent">Clipped text</p>"#,
    );
    let mark = "/P << /MCID 0 /ActualText (Clipped text) >> BDC\nBT\n7 Tr\n";
    assert!(pdf.contains(mark), "{pdf}");
    let after = &pdf[pdf.find(mark).unwrap() + mark.len()..];
    let fill = &after[after.find("EMC\n").unwrap()..];
    let fill = &fill[..fill.find("\nf\n").unwrap()];
    assert!(
        fill.contains("/Artifact << /Type /Layout >> BDC\n"),
        "{pdf}"
    );
    assert_eq!(pdf.matches("/ActualText ").count(), 1, "{pdf}");
    assert_eq!(pdf.matches("BT\n").count(), 1, "{pdf}");
}

/// ISO 32000-1 14.9.4: a blurred line painted run by run marks each picture
/// with its run's text, so the runs read back as the authored text.
#[test]
fn blurred_text_runs_are_read_through_actual_text() {
    let pdf =
        tagged_body(r#"<p style="filter: blur(1pt); border-radius: 2pt">Soft <b>text</b></p>"#);
    let runs = rasterized_actual_text(&pdf);
    assert_eq!(runs.len(), 2, "{pdf}");
    assert_eq!(pdf.matches("/ActualText ").count(), 2, "{pdf}");
    let spoken: String = runs
        .iter()
        .map(|run| run.trim_matches(|c| c == '(' || c == ')'))
        .collect();
    assert_eq!(spoken, "Soft text", "{pdf}");
    assert!(!pdf.contains(" Tj\n") && !pdf.contains(" TJ\n"), "{pdf}");
}

/// ISO 32000-1 14.9.4: a hyphen the line breaker inserts at a soft hyphen
/// is replaced by U+00AD in `/ActualText`, so the word reads unbroken.
#[test]
fn synthesized_line_end_hyphen_carries_actual_text() {
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .compress(false)
            .convert(
                r#"<html lang="en"><head><title>Hy</title></head><body>
                <p style="width: 20pt; hyphens: manual">hy&shy;phenation</p></body></html>"#,
            )
            .unwrap(),
    );
    assert!(
        pdf.contains("/P << /MCID 0 /ActualText <FEFF0068007900AD> >> BDC"),
        "{pdf}"
    );
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
        "/ListNumbering /None",
        "/S /LBody ",
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
        "/S /Reference ",
        "/S /Note ",
        "/S /Formula ",
        "/S /Span /P ",
        "/E (Great British Pounds)",
        "/Dest (detail) /F 4 /Contents (detail table)",
        "/Names << /Dests ",
        "/ID (fn",
        "/IDTree << /Names [(fn",
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

/// Matterhorn 18-001: a table header or footer row that pagination repeats
/// is page furniture. When the table is split as a whole, the `thead` copy at
/// the top of the continuation and the `tfoot` copy at the bottom of the first
/// fragment paint as `Pagination` artifacts, so every cell is marked once and
/// a link in a repeated row gets no second annotation.
#[test]
fn rows_repeated_by_a_table_split_are_artifacts() {
    let rows = 60;
    let body: String = (1..=rows)
        .map(|row| format!("<tr><td>Row {row}</td><td>{row}</td></tr>"))
        .collect();
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .compress(false)
            .convert(&format!(
                r##"<html lang="en"><head><title>T</title></head><body>
                <table>
                <thead><tr><th>Row</th><th><a href="https://example.com/value">Value</a></th></tr></thead>
                <tbody>{body}</tbody>
                <tfoot><tr><td>Total</td><td>sum</td></tr></tfoot>
                </table></body></html>"##
            ))
            .unwrap(),
    );
    assert!(
        pdf.matches("/Type /Page ").count() >= 2,
        "the table must split"
    );
    // One mark per authored cell: "Row", the "Value" link, every body cell
    // and the two footer cells.
    assert_eq!(pdf.matches("/TH << /MCID ").count(), 1, "{pdf}");
    assert_eq!(pdf.matches("/Link << /MCID ").count(), 1, "{pdf}");
    assert_eq!(pdf.matches("/TD << /MCID ").count(), 2 * rows + 2, "{pdf}");
    assert!(pdf.contains("/Artifact << /Type /Pagination >> BDC"));
    assert_eq!(pdf.matches("/Subtype /Link ").count(), 1, "{pdf}");
}

/// ISO 32000-1 14.7.4.4: content in a form XObject with its own marks is
/// reached through `/MCR /Stm` kids. A mark around the form's invocation
/// would claim the same content again, so the opacity group's `Div` and the
/// blended `P` own no page mark; each paragraph is marked once, inside its
/// form, and neither invocation sits in an artifact (ISO 14289-1 7.1).
#[test]
fn content_in_isolated_groups_is_claimed_once() {
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .compress(false)
            .convert(
                r#"<html lang="en"><head><title>G</title></head><body>
                <div style="opacity: 0.6; background: #eef"><p>Faded</p></div>
                <p style="mix-blend-mode: multiply">Blended</p>
                </body></html>"#,
            )
            .unwrap(),
    );
    assert_eq!(pdf.matches("/P << /MCID ").count(), 2, "{pdf}");
    assert!(!pdf.contains("/Div << /MCID "), "{pdf}");
    assert_eq!(
        pdf.matches("/FormType 1 /StructParents ").count(),
        2,
        "{pdf}"
    );
    assert!(!pdf.contains("/Type /Layout >> BDC\n/Fm"), "{pdf}");
}

/// css-content-3 §1.2: a generated image is read where it is generated. A
/// `::before` image is its paragraph's first kid, ahead of the text and of
/// the paragraph's inline elements, although it is painted after them.
#[test]
fn generated_figures_are_read_where_they_are_generated() {
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .compress(false)
            .convert(&format!(
                r#"<html lang="en"><head><title>F</title><style>
                p::before {{ content: url("{PIXEL}") / "Approved"; }}
                </style></head><body>
                <p>See <a href="https://example.com/a">a link</a> and the
                <abbr title="Portable Document Format">PDF</abbr> abbreviation.</p>
                </body></html>"#
            ))
            .unwrap(),
    );
    let lines: Vec<&str> = pdf.lines().collect();
    let figure = lines
        .windows(2)
        .find(|pair| pair[1].contains("/S /Figure "))
        .map(|pair| pair[0].trim_end_matches(" 0 obj"))
        .expect("a Figure element");
    let paragraph = lines
        .iter()
        .find(|line| line.contains("/S /P "))
        .expect("a P element");
    assert!(
        paragraph.contains(&format!("/K [{figure} 0 R ")),
        "{paragraph}"
    );
}

/// ISO 32000-1 14.9.4: `ToUnicode` maps each glyph once, so a base letter
/// that also appears on its own cannot carry a combining mark. The run's
/// `/ActualText` keeps the mark: "x" followed by U+0301.
#[test]
fn combining_marks_are_kept_in_actual_text() {
    let pdf = text(
        &HtmlConverter::new()
            .accessible(true)
            .compress(false)
            .convert(
                r#"<html lang="en"><head><title>C</title></head><body>
                <p>Accent: x&#x301; and a plain x.</p>
                </body></html>"#,
            )
            .unwrap(),
    );
    assert!(pdf.contains("00780301"), "{pdf}");
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

const EFFECTS: &str = include_str!("../../tests/pdfua/effects.html");

#[test]
fn composited_containers_keep_their_inner_structure() {
    // ISO 32000-1 14.7.4.4: content cut into a form XObject is addressed
    // through the form's own /StructParents entry and /MCR /Stm kids, so the
    // heading, link and list inside an opacity container stay navigable.
    let pdf = tagged(EFFECTS);
    assert!(pdf.contains("/FormType 1 /StructParents "), "{pdf}");
    assert!(
        pdf.contains("/Type /MCR /Pg ") && pdf.contains(" /Stm "),
        "{pdf}"
    );
    assert_eq!(pdf.matches("/S /H2 ").count(), 1, "{pdf}");
    assert_eq!(pdf.matches("/S /Link ").count(), 1, "{pdf}");
    assert_eq!(pdf.matches("/S /LI ").count(), 2, "{pdf}");
}

#[test]
fn continuation_glyphs_get_a_unicode_value_in_accessible_output() {
    // ISO 14289-1 7.21.7 (veraPDF 7.21.7-1): every shown glyph maps to
    // Unicode; a combining mark shaped into its base's cluster maps to a
    // zero-width space, and default output is unchanged.
    let tagged_pdf = tagged(EFFECTS);
    assert!(tagged_pdf.contains("<200B>"), "{tagged_pdf}");
    let default_pdf = text(&HtmlConverter::new().convert(EFFECTS).unwrap());
    assert!(!default_pdf.contains("<200B>"), "{default_pdf}");
}
