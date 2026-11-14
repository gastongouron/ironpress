//! The marks of pictures: rasterized text, replaced images and generated
//! images.

use super::super::geometry::PdfRect;
use super::recorder::MarkKind;
use crate::layout::elements::{PictureMeaning, SpokenText};
use crate::layout::engine::{InlineImage, TextLine, TextRun};

impl MarkKind {
    /// A run painted as an image (a blurred line): the reader sees its text
    /// instead of the picture (ISO 32000-1 14.9.4).
    pub(crate) fn rasterized_run(run: &TextRun) -> Self {
        Self::Text {
            actual_text: Some(run.metadata.hyphenation.spoken_text(&run.text)),
        }
    }

    /// Lines shown through one mark, as a picture or as a text clip whose
    /// outlines may repeat: the reader sees their text once.
    pub(crate) fn shown_once(lines: &[TextLine]) -> Self {
        Self::spoken(&SpokenText::of_lines(lines))
    }

    /// A picture of boxes: the reader sees their text, or nothing in
    /// particular when they held none.
    fn spoken(text: &SpokenText) -> Self {
        Self::Text {
            actual_text: (!text.is_empty()).then(|| text.as_str().to_string()),
        }
    }

    /// The mark of a generated image (`content: url()`) painted in `bbox`
    /// (default user space): a Figure named by its alternative text,
    /// decoration without one (css-content-3 §1.2).
    pub(crate) fn generated_figure(image: &InlineImage, bbox: PdfRect) -> Self {
        Self::GeneratedFigure {
            bbox,
            alt: image.alt().map(str::to_string),
        }
    }

    /// The mark of a picture painted in `bbox` (default user space): a Figure
    /// for a replaced image, the text of the boxes a filter rasterized.
    pub(crate) fn picture(meaning: &PictureMeaning, bbox: PdfRect) -> Self {
        match meaning {
            PictureMeaning::Figure => Self::Figure { bbox },
            PictureMeaning::RasterizedBoxes(text) => Self::spoken(text),
        }
    }
}
