//! Tagged PDF structure: the objects that make an output file PDF/UA-1.
//!
//! Marks are recorded while pages paint ([`recorder`]), unmarked paint is
//! turned into artifacts ([`artifacts`]), and the hierarchy, parent tree,
//! metadata and document information are appended in `finish_to_writer`,
//! after the page tree and before the catalog, the same way the outline is.

mod artifacts;
mod lists;
mod metadata;
mod rasterized;
mod recorder;
mod tables;
mod tree;

pub(crate) use recorder::{AnnotationRecord, ArtifactKind, MarkKind, OpenMark, StructureRecorder};

use super::pdf_text::PdfTextString;
use crate::error::IronpressError;
use metadata::XmpPacket;
use recorder::Tagging;
use tree::StructureTree;

/// Objects appended for a tagged document, with the catalog and trailer
/// entries that reference them.
pub(super) struct TaggedDocumentObjects {
    catalog_entries: String,
    trailer_entries: String,
}

impl TaggedDocumentObjects {
    /// Push the structure tree, the XMP metadata stream and the `/Info`
    /// dictionary.
    pub(super) fn append_to(
        objects: &mut Vec<String>,
        tagging: Box<Tagging>,
        page_ids: &[usize],
    ) -> Result<Self, IronpressError> {
        let tree = StructureTree::build(&tagging)?;
        let root_id = tree.write(objects, &tagging, page_ids);
        // ISO 32000-1 14.7.4.4: a form XObject holding marked content carries
        // the key of its own parent-tree entry.
        for form in &tagging.forms {
            if let Some(object) = objects.get_mut(form.object_id - 1) {
                *object = object.replacen(
                    "/FormType 1",
                    &format!("/FormType 1 /StructParents {}", form.key),
                    1,
                );
            }
        }

        let xmp_id = objects.len() + 1;
        let info_id = xmp_id + 1;
        let language = PdfTextString::from(tagging.metadata.language.as_str());
        let title = PdfTextString::from(tagging.metadata.title.as_str());

        // ISO 14289-1 7.1: XMP metadata with pdfuaid:part and dc:title. veraPDF
        // recognizes the stream only with both `/Type /Metadata /Subtype /XML`.
        let packet = XmpPacket::new(&tagging.metadata, PRODUCER).to_xml();
        objects.push(format!(
            "{xmp_id} 0 obj\n<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{packet}\nendstream\nendobj",
            packet.len()
        ));

        // Viewer courtesy: the document information dictionary mirrors the
        // XMP title so `/DisplayDocTitle` has a value in every reader.
        objects.push(format!(
            "{info_id} 0 obj\n<< /Title {title} /Producer ({PRODUCER}) >>\nendobj"
        ));

        Ok(Self {
            catalog_entries: format!(
                " /MarkInfo << /Marked true >> /StructTreeRoot {root_id} 0 R /Lang {language} \
                 /ViewerPreferences << /DisplayDocTitle true >> /Metadata {xmp_id} 0 R"
            ),
            trailer_entries: format!(" /Info {info_id} 0 R"),
        })
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
