//! Tagged PDF structure: the objects that make an output file PDF/UA-1.
//!
//! Everything here is appended to the object list in `finish_to_writer`, after
//! the page tree and before the catalog, the same way the outline is.

mod metadata;

use super::pdf_text::PdfTextString;
use crate::accessibility::DocumentMetadata;
use metadata::XmpPacket;

/// Objects appended for a tagged document, with the catalog and trailer
/// entries that reference them.
pub(super) struct TaggedDocumentObjects {
    catalog_entries: String,
    trailer_entries: String,
}

impl TaggedDocumentObjects {
    /// Push the structure tree root, its `Document` element, the XMP metadata
    /// stream and the `/Info` dictionary.
    pub(super) fn append_to(objects: &mut Vec<String>, metadata: &DocumentMetadata) -> Self {
        let root_id = objects.len() + 1;
        let document_id = root_id + 1;
        let xmp_id = document_id + 1;
        let info_id = xmp_id + 1;
        let language = PdfTextString::from(metadata.language.as_str());
        let title = PdfTextString::from(metadata.title.as_str());

        // ISO 32000-1 14.7.2: the structure hierarchy root. Every element,
        // the single `Document` included, carries `/P` (veraPDF 7.1-11).
        objects.push(format!(
            "{root_id} 0 obj\n<< /Type /StructTreeRoot /K [{document_id} 0 R] >>\nendobj"
        ));
        objects.push(format!(
            "{document_id} 0 obj\n<< /Type /StructElem /S /Document /P {root_id} 0 R /Lang {language} >>\nendobj"
        ));

        // ISO 14289-1 7.1: XMP metadata with pdfuaid:part and dc:title. veraPDF
        // recognizes the stream only with both `/Type /Metadata /Subtype /XML`.
        let packet = XmpPacket::new(metadata, PRODUCER).to_xml();
        objects.push(format!(
            "{xmp_id} 0 obj\n<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{packet}\nendstream\nendobj",
            packet.len()
        ));

        // Viewer courtesy: the document information dictionary mirrors the
        // XMP title so `/DisplayDocTitle` has a value in every reader.
        objects.push(format!(
            "{info_id} 0 obj\n<< /Title {title} /Producer ({PRODUCER}) >>\nendobj"
        ));

        Self {
            catalog_entries: format!(
                " /MarkInfo << /Marked true >> /StructTreeRoot {root_id} 0 R /Lang {language} \
                 /ViewerPreferences << /DisplayDocTitle true >> /Metadata {xmp_id} 0 R"
            ),
            trailer_entries: format!(" /Info {info_id} 0 R"),
        }
    }

    /// Entries for the catalog dictionary, starting with a space.
    pub(super) fn catalog_entries(&self) -> &str {
        &self.catalog_entries
    }

    /// Entries for the trailer dictionary, starting with a space.
    pub(super) fn trailer_entries(&self) -> &str {
        &self.trailer_entries
    }
}

const PRODUCER: &str = concat!("ironpress ", env!("CARGO_PKG_VERSION"));
