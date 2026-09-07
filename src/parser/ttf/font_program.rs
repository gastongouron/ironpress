//! One OpenType program and the shaping face derived from it.

use std::fmt;

use self_cell::self_cell;

use super::FontFaceIndex;

struct FontProgramSource {
    bytes: Vec<u8>,
    face_index: FontFaceIndex,
}

type ShapingFace<'a> = Option<rustybuzz::Face<'a>>;

self_cell!(
    struct FontProgramCell {
        owner: FontProgramSource,

        #[covariant]
        dependent: ShapingFace,
    }
);

/// Owned OpenType bytes and the shaping view derived from the same face.
///
/// Keeping the selected face and its borrowed shaping tables in one value
/// makes it impossible for embedding and shaping to observe different bytes.
pub(crate) struct FontProgram {
    cell: FontProgramCell,
}

impl FontProgram {
    /// Own a font accepted by the surrounding TTF parser and derive its face.
    pub(super) fn from_parsed_font(bytes: Vec<u8>, face_index: FontFaceIndex) -> Self {
        let source = FontProgramSource { bytes, face_index };
        let cell = FontProgramCell::new(source, |source| {
            rustybuzz::Face::from_slice(&source.bytes, source.face_index.get())
        });
        Self { cell }
    }

    /// Raw OpenType bytes used for embedding and outline inspection.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.cell.borrow_owner().bytes
    }

    /// Selected face within a TTC or OTC program.
    pub(crate) fn face_index(&self) -> FontFaceIndex {
        self.cell.borrow_owner().face_index
    }

    /// Shaping tables derived from [`Self::bytes`] and [`Self::face_index`].
    pub(crate) fn shaping_face(&self) -> Option<&rustybuzz::Face<'_>> {
        self.cell.borrow_dependent().as_ref()
    }

    #[cfg(test)]
    pub(crate) fn unshapeable_for_tests(bytes: Vec<u8>) -> Self {
        Self::from_parsed_font(bytes, FontFaceIndex::DEFAULT)
    }
}

impl Clone for FontProgram {
    fn clone(&self) -> Self {
        Self::from_parsed_font(self.bytes().to_vec(), self.face_index())
    }
}

impl fmt::Debug for FontProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontProgram")
            .field("bytes", &self.bytes().len())
            .field("face_index", &self.face_index())
            .field("shapeable", &self.shaping_face().is_some())
            .finish()
    }
}
