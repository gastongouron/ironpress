use crate::layout::elements::LayoutNode;
use crate::layout::text::resolve_style_font_family;
use crate::style::computed::{ComputedStyle, Display, FontWeight, VerticalAlign};
use crate::types::Size;

use super::svg::resolve_svg_size;

/// How an inline replaced element contributes the font's lower baseline extent.
///
/// Ordinary replaced-element flow preserves the font's fractional metrics.
/// An atomic native inline box instead participates in the CSS inline line box,
/// whose block-end extent is resolved on the CSS-pixel grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InlineBaselineGapRounding {
    #[default]
    Fractional,
    CssPixel,
}

impl InlineBaselineGapRounding {
    fn apply(self, gap: f32) -> f32 {
        match self {
            Self::Fractional => gap,
            Self::CssPixel => crate::fonts::ceil_to_css_pixel(gap),
        }
    }
}

/// Placement of replaced-image content inside its box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImagePlacement {
    pub width: f32,
    pub height: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub clip: bool,
}

/// Compute CSS replaced-content placement from an intrinsic size already in
/// points. Raster and SVG elements share these `object-fit` rules.
pub(crate) fn compute_replaced_content_placement(
    content_box: Size,
    intrinsic_size: Size,
    object_fit: crate::style::computed::ObjectFit,
    object_position: crate::style::computed::ObjectPosition,
) -> ImagePlacement {
    use crate::style::computed::ObjectFit;

    let Size {
        width: box_w,
        height: box_h,
    } = content_box;
    let Size {
        width: intrinsic_w,
        height: intrinsic_h,
    } = intrinsic_size;
    if intrinsic_w <= 0.0 || intrinsic_h <= 0.0 || box_w <= 0.0 || box_h <= 0.0 {
        return ImagePlacement {
            width: box_w,
            height: box_h,
            offset_x: 0.0,
            offset_y: 0.0,
            clip: false,
        };
    }

    let contain_scale = (box_w / intrinsic_w).min(box_h / intrinsic_h);
    let cover_scale = (box_w / intrinsic_w).max(box_h / intrinsic_h);
    let (width, height) = match object_fit {
        ObjectFit::Fill => (box_w, box_h),
        ObjectFit::Contain => (intrinsic_w * contain_scale, intrinsic_h * contain_scale),
        ObjectFit::Cover => (intrinsic_w * cover_scale, intrinsic_h * cover_scale),
        ObjectFit::None => (intrinsic_w, intrinsic_h),
        ObjectFit::ScaleDown => {
            let scale = contain_scale.min(1.0);
            (intrinsic_w * scale, intrinsic_h * scale)
        }
    };
    let offset_x = object_position.x.resolve(box_w - width);
    let offset_y = object_position.y.resolve(box_h - height);
    let clip =
        offset_x < 0.0 || offset_y < 0.0 || offset_x + width > box_w || offset_y + height > box_h;

    ImagePlacement {
        width,
        height,
        offset_x,
        offset_y,
        clip,
    }
}

/// Compute where to draw a replaced image inside its box per CSS `object-fit`
/// and `object-position`. The box is `box_w` x `box_h` points; the image's
/// intrinsic pixel size is converted to points (1px = 0.75pt).
pub(crate) fn compute_image_placement(
    box_w: f32,
    box_h: f32,
    source_width: u32,
    source_height: u32,
    object_fit: crate::style::computed::ObjectFit,
    object_position: crate::style::computed::ObjectPosition,
) -> ImagePlacement {
    compute_replaced_content_placement(
        Size::new(box_w, box_h),
        Size::new(source_width as f32 * 0.75, source_height as f32 * 0.75),
        object_fit,
        object_position,
    )
}

/// The intrinsic viewport size used by CSS replaced-content fitting for SVG.
pub(crate) fn svg_intrinsic_size(tree: &crate::parser::svg::SvgTree) -> Size {
    let (width, height) = resolve_svg_size(tree, 0.0, 0.0, false, false);
    Size::new(width, height)
}

/// The used size of a replaced element together with which dimensions remain
/// automatic. Constraints may adjust an automatic counterpart to preserve the
/// intrinsic ratio, but must not silently rewrite a specified CSS dimension.
#[derive(Clone, Copy, Debug)]
pub(super) struct ReplacedBoxSize {
    width: f32,
    height: f32,
    width_is_auto: bool,
    height_is_auto: bool,
}

impl ReplacedBoxSize {
    pub(super) const fn new(
        width: f32,
        height: f32,
        width_is_auto: bool,
        height_is_auto: bool,
    ) -> Self {
        Self {
            width,
            height,
            width_is_auto,
            height_is_auto,
        }
    }

    pub(super) fn constrain(
        mut self,
        available_width: f32,
        max_width: Option<f32>,
        max_height: Option<f32>,
    ) -> Self {
        if self.width <= 0.0 || self.height <= 0.0 {
            self.width = self.width.max(0.0);
            self.height = self.height.max(0.0);
            return self;
        }

        let width_limit = available_width
            .is_finite()
            .then_some(available_width)
            .filter(|limit| *limit > 0.0)
            .into_iter()
            .chain(max_width.filter(|limit| limit.is_finite() && *limit > 0.0))
            .reduce(f32::min);
        if let Some(limit) = width_limit.filter(|limit| self.width > *limit) {
            let scale = limit / self.width;
            self.width = limit;
            if self.height_is_auto {
                self.height *= scale;
            }
        }

        if let Some(limit) = max_height
            .filter(|limit| limit.is_finite() && *limit > 0.0)
            .filter(|limit| self.height > *limit)
        {
            let scale = limit / self.height;
            self.height = limit;
            if self.width_is_auto {
                self.width *= scale;
            }
        }

        self
    }

    pub(super) const fn dimensions(self) -> (f32, f32) {
        (self.width, self.height)
    }
}

pub(crate) fn add_inline_replaced_baseline_gap(
    mut element: LayoutNode,
    style: &ComputedStyle,
    fonts: &dyn crate::font_registry::FontRegistry,
    rounding: InlineBaselineGapRounding,
) -> LayoutNode {
    if style.display != Display::Inline || style.vertical_align != VerticalAlign::Baseline {
        return element;
    }

    let font_family = resolve_style_font_family(style, fonts);
    let (_, descender_ratio) = crate::fonts::font_metrics_ratios(
        &font_family,
        style.font_weight == FontWeight::Bold,
        style.font_style.is_slanted(),
        fonts,
    );
    let baseline_gap = rounding.apply(descender_ratio * style.font_size);
    if baseline_gap <= 0.0 {
        return element;
    }

    if let Some(replaced) = element.replaced_element_mut() {
        replaced.add_baseline_gap(baseline_gap);
    }
    element
}

pub(crate) fn parse_html_image_dimension(raw: Option<&String>) -> Option<f32> {
    let raw = raw?.trim();
    let raw = raw.strip_suffix("px").unwrap_or(raw);
    raw.parse::<f32>().ok().map(|px| px * 0.75)
}
