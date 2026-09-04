//! The shaping view a parsed font owns over its own bytes.
//!
//! `rustybuzz::Face` borrows the font buffer it was built from, so keeping a
//! face alive means keeping that buffer alive. Storing the two together in one
//! value is what lets a parsed font hold its face outright, instead of a
//! process-wide table handing out faces over a buffer nothing owns.

use std::fmt;
use std::sync::Arc;

use self_cell::self_cell;

/// The shaping tables rustybuzz derives from a font's bytes.
type Face<'a> = rustybuzz::Face<'a>;

self_cell!(
    /// A `rustybuzz::Face` together with the buffer it reads.
    ///
    /// Building a face parses the font's shaping tables. Layout shapes every
    /// run at least twice — once to measure, again to paint — so deriving the
    /// face per run is the dominant cost of text. Deriving it once with the
    /// font removes that cost without any lifetime extension: the face lives
    /// exactly as long as this value, and this value's owner keeps its bytes.
    pub(crate) struct ShapingFace {
        owner: Arc<Vec<u8>>,

        #[covariant]
        dependent: Face,
    }
);

impl ShapingFace {
    /// Build the shaping face for `face_index` within `data`.
    ///
    /// `None` means rustybuzz rejected the bytes. Shaping then declines the run
    /// rather than substituting another face, which is the same outcome a
    /// failed `Face::from_slice` produced when each run built its own face.
    pub(crate) fn parse(data: Arc<Vec<u8>>, face_index: u32) -> Option<Self> {
        Self::try_new(data, |data| {
            rustybuzz::Face::from_slice(data.as_slice(), face_index).ok_or(())
        })
        .ok()
    }

    /// The shaping face, borrowed for as long as this value lives.
    pub(crate) fn face(&self) -> &rustybuzz::Face<'_> {
        self.borrow_dependent()
    }
}

impl fmt::Debug for ShapingFace {
    /// `rustybuzz::Face` is not `Debug`, and dumping its parsed tables would
    /// bury the surrounding font. The buffer size is the part that identifies
    /// which face a font is carrying.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShapingFace")
            .field("bytes", &self.borrow_owner().len())
            .finish()
    }
}
