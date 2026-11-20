use crate::style::computed::{BorderImagePaint, VerticalAlign};
use crate::types::{Color, CornerRadii, EdgeSizes};

use super::engine::{LayoutBorder, LayoutBorderSide, RasterImageAsset, TextLine};

/// A solid vector stroke centered on a box contour.
///
/// Unlike a CSS border, half of this stroke paints outside the nominal box.
/// UA-supplied geometric markers use that contour without changing layout.
#[derive(Debug, Clone, Copy)]
pub struct CenteredStroke {
    side: LayoutBorderSide,
}

impl CenteredStroke {
    pub const fn solid(width: f32, color: Color) -> Self {
        Self {
            side: LayoutBorderSide::solid(width, color),
        }
    }

    pub(crate) const fn side(self) -> LayoutBorderSide {
        self.side
    }

    pub(crate) fn transform_color(&mut self, transform: impl FnOnce(Color) -> Color) {
        self.side.color = transform(self.side.color);
    }
}

/// Paint owned by one atomic inline box.
///
/// Grouping these values keeps decoration decisions independent from inline
/// sizing, baseline participation, and replaced content.
#[derive(Debug, Clone, Default)]
pub struct InlineBoxPaint {
    pub background_color: Option<Color>,
    pub border: LayoutBorder,
    pub border_image: Option<BorderImagePaint>,
    pub border_radii: CornerRadii,
    pub centered_stroke: Option<CenteredStroke>,
}

/// Replaced content of a generated box: a `content: url()` image or a
/// `list-style-image` marker.
#[derive(Debug, Clone)]
pub struct InlineImage {
    pub asset: RasterImageAsset,
    /// The alternative text for speech (css-content-3 §1.2); `None` when none
    /// was written or it is blank, which makes the image decoration.
    alt: Option<String>,
}

impl InlineImage {
    pub(crate) fn new(asset: RasterImageAsset, alt: Option<String>) -> Self {
        Self {
            asset,
            alt: alt
                .map(|alt| alt.trim().to_string())
                .filter(|alt| !alt.is_empty()),
        }
    }

    pub(crate) fn alt(&self) -> Option<&str> {
        self.alt.as_deref()
    }
}

/// An atomic inline-level box laid out inside a line of text.
#[derive(Debug, Clone, Default)]
pub struct InlineBox {
    /// Border-box width (the painted box width).
    pub width: f32,
    /// Border-box height (used to grow the line box and for vertical-align).
    pub height: f32,
    /// Horizontal margins add inline advance but are not painted.
    pub margin_left: f32,
    pub margin_right: f32,
    pub paint: InlineBoxPaint,
    pub padding: EdgeSizes,
    /// CSS `vertical-align` relative to the line baseline.
    pub vertical_align: VerticalAlign,
    /// Distance from the top border edge to this box's baseline.
    pub baseline_ascent: Option<f32>,
    /// Pre-wrapped inner text lines (empty for content-less boxes).
    pub lines: Vec<TextLine>,
    /// Replaced content painted into the content box.
    pub image: Option<InlineImage>,
    /// CSS relative-position paint offset (right, down) in points.
    pub rel_offset_x: f32,
    pub rel_offset_y: f32,
    /// The element this box belongs to; a list bullet carries its item's
    /// label role so the renderer can mark it as the `Lbl`.
    pub structure: crate::accessibility::StructureRef,
    /// Where the box sits in the reading order of its line, copied from its
    /// run once lines are numbered, so a generated figure reads in place.
    pub(crate) reading: crate::layout::engine::ReadingPosition,
}

impl InlineBox {
    /// A non-painting inline advance.
    pub fn advance_only(advance: f32) -> Self {
        Self {
            margin_right: advance,
            ..Self::default()
        }
    }

    /// Total inline advance including horizontal margins.
    pub fn outer_width(&self) -> f32 {
        self.width + self.margin_left + self.margin_right
    }
}
