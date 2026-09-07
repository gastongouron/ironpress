//! Generic recursive dispatch for filter source painting.

use crate::layout::elements::{
    BoxModel, BoxPaint, Container, FlexRow, LayoutElement, Positioning, TextBlock,
};
use crate::render::borders::CssRoundedRect;
use crate::style::computed::Position;
use crate::types::{Point, Size};

use super::canvas::{PaintBounds, RasterCanvas, SurfaceRect};
use super::geometry::{BlockChildSpace, block_child_frames};
use super::gradient::FilterBackground;

/// Common box state used by the source painter without flattening concrete
/// layout elements into another tagged representation.
pub(super) trait FilterBox {
    fn box_model(&self) -> &BoxModel;
    fn paint(&self) -> &BoxPaint;
    fn positioning(&self) -> &Positioning;
}

impl FilterBox for TextBlock {
    fn box_model(&self) -> &BoxModel {
        &self.box_model
    }

    fn paint(&self) -> &BoxPaint {
        &self.paint
    }

    fn positioning(&self) -> &Positioning {
        &self.positioning
    }
}

impl FilterBox for Container {
    fn box_model(&self) -> &BoxModel {
        &self.box_model
    }

    fn paint(&self) -> &BoxPaint {
        &self.paint
    }

    fn positioning(&self) -> &Positioning {
        &self.positioning
    }
}

impl FilterBox for FlexRow {
    fn box_model(&self) -> &BoxModel {
        &self.box_model
    }

    fn paint(&self) -> &BoxPaint {
        &self.paint
    }

    fn positioning(&self) -> &Positioning {
        &self.positioning
    }
}

/// Coordinates and inherited positioning state for one semantic source box.
/// Keeping this together prevents recursive paint paths from silently
/// re-anchoring absolute descendants to an intervening static box.
#[derive(Clone, Copy)]
pub(super) struct ElementPaintSpace {
    pub(super) border_box: SurfaceRect,
    pub(super) css_pixel_grid_origin: Point,
    inherited_containing_block: Option<SurfaceRect>,
    pub(super) establishes_containing_block: bool,
    pub(super) root_effects: RootEffectHandling,
}

impl ElementPaintSpace {
    pub(super) const fn root(border_box: SurfaceRect, root_effects: RootEffectHandling) -> Self {
        Self {
            border_box,
            css_pixel_grid_origin: border_box.origin,
            inherited_containing_block: None,
            establishes_containing_block: true,
            root_effects,
        }
    }

    const fn child(
        self,
        border_box: SurfaceRect,
        inherited_containing_block: Option<SurfaceRect>,
        root_effects: RootEffectHandling,
    ) -> Self {
        Self {
            border_box,
            inherited_containing_block,
            establishes_containing_block: false,
            root_effects,
            ..self
        }
    }

    pub(super) const fn for_descendant_box(
        self,
        border_box: SurfaceRect,
        root_effects: RootEffectHandling,
    ) -> Self {
        self.child(border_box, self.inherited_containing_block, root_effects)
    }

    pub(super) const fn with_root_effects(self, root_effects: RootEffectHandling) -> Self {
        Self {
            root_effects,
            ..self
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RootEffectHandling {
    Paint,
    DeferToOwner,
}

/// The two CSS box edges needed while painting descendants. Normal-flow
/// children start in the content box; absolutely positioned descendants use
/// the containing padding box.
#[derive(Clone, Copy)]
pub(super) struct DescendantPaintArea {
    pub(super) padding_box: SurfaceRect,
    pub(super) content_box: SurfaceRect,
    pub(super) absolute_containing_block: Option<SurfaceRect>,
    pub(super) direct_child_effects: RootEffectHandling,
}

impl DescendantPaintArea {
    pub(super) fn after_normal_flow(self, consumed: f32) -> Self {
        Self {
            content_box: SurfaceRect::new(
                Point::new(
                    self.content_box.origin.x,
                    self.content_box.origin.y + consumed,
                ),
                Size::new(
                    self.content_box.size.width,
                    (self.content_box.size.height - consumed).max(0.0),
                ),
            ),
            ..self
        }
    }
}

pub(super) struct SourcePainter<'a> {
    pub(super) canvas: RasterCanvas<'a>,
    pub(super) space: ElementPaintSpace,
    pub(super) fonts: &'a dyn crate::font_registry::FontRegistry,
    pub(super) filter_dpi: f32,
    pub(super) result: Option<()>,
}

impl<'a> SourcePainter<'a> {
    pub(super) const fn new(
        canvas: RasterCanvas<'a>,
        space: ElementPaintSpace,
        fonts: &'a dyn crate::font_registry::FontRegistry,
        filter_dpi: f32,
    ) -> Self {
        Self {
            canvas,
            space,
            fonts,
            filter_dpi,
            result: None,
        }
    }

    pub(super) fn paint_clipped_descendants(
        &mut self,
        clip: CssRoundedRect,
        paint: impl FnOnce(&mut SourcePainter<'_>) -> Option<()>,
    ) -> Option<()> {
        let mut group = crate::render::raster_pixels::PremultipliedRgba8::transparent(
            self.canvas.pixels.width(),
            self.canvas.pixels.height(),
        );
        let mut group_bounds = PaintBounds::default();
        {
            let canvas = RasterCanvas {
                pixels: &mut group,
                pixels_per_point: self.canvas.pixels_per_point,
                paint_bounds: &mut group_bounds,
            };
            let mut descendant_painter =
                SourcePainter::new(canvas, self.space, self.fonts, self.filter_dpi);
            paint(&mut descendant_painter)?;
        }
        self.canvas.composite_clipped_group(&group, clip);
        self.canvas
            .paint_bounds
            .include_clipped(group_bounds, clip.rect);
        Some(())
    }

    pub(super) fn paint_box(&mut self, element: &impl FilterBox) -> Option<DescendantPaintArea> {
        let model = element.box_model();
        let paint = element.paint();
        if !paint.visible || paint.outline.width > 0.0 {
            return None;
        }
        let rect = self.space.border_box;
        let background = FilterBackground::resolve(
            &paint.background,
            model,
            rect,
            paint.border_radii,
            paint.border_image.as_ref(),
        )?;
        self.canvas
            .paint_outset_shadows(rect, &paint.shadows, self.filter_dpi)?;
        background.paint(&mut self.canvas);
        let padding_box = rect.inset(model.border.widths());
        self.canvas
            .paint_inset_shadows(padding_box, &paint.shadows, self.filter_dpi)?;
        self.canvas
            .paint_border(rect, &model.border, paint.border_radii)?;
        let absolute_containing_block = if self.space.establishes_containing_block
            || element.positioning().scheme != Position::Static
        {
            Some(padding_box)
        } else {
            self.space.inherited_containing_block
        };
        Some(DescendantPaintArea {
            padding_box,
            content_box: rect.inset(model.border.widths() + model.padding),
            absolute_containing_block,
            direct_child_effects: RootEffectHandling::Paint,
        })
    }

    pub(super) fn paint_container_children(
        &mut self,
        children: &[crate::layout::elements::LayoutNode],
        area: DescendantPaintArea,
    ) -> Option<()> {
        self.paint_children(children, area)
    }

    pub(super) fn paint_children(
        &mut self,
        children: &[crate::layout::elements::LayoutNode],
        area: DescendantPaintArea,
    ) -> Option<()> {
        let frames = block_child_frames(
            children,
            BlockChildSpace::new(
                crate::types::Rect::new(area.content_box.origin, area.content_box.size),
                crate::types::Rect::new(area.padding_box.origin, area.padding_box.size),
                area.absolute_containing_block
                    .map(|rect| crate::types::Rect::new(rect.origin, rect.size)),
            ),
        )?;
        for (child, frame) in children.iter().zip(frames) {
            paint_element(
                &mut self.canvas,
                child.as_ref(),
                self.space.child(
                    SurfaceRect::new(frame.border_box.origin, frame.border_box.size),
                    area.absolute_containing_block,
                    area.direct_child_effects,
                ),
                self.fonts,
                self.filter_dpi,
            )?;
        }
        Some(())
    }
}

pub(super) fn paint_element(
    canvas: &mut RasterCanvas<'_>,
    element: &dyn LayoutElement,
    space: ElementPaintSpace,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
) -> Option<()> {
    let painter_canvas = RasterCanvas {
        pixels: canvas.pixels,
        pixels_per_point: canvas.pixels_per_point,
        paint_bounds: canvas.paint_bounds,
    };
    let mut painter = SourcePainter::new(painter_canvas, space, fonts, filter_dpi);
    if let Some(owner) = element.paint_group_owner() {
        return painter.paint_group(
            space,
            owner.paint_group(),
            element.box_reference_geometry(),
            |painter| {
                element.accept(painter);
                painter.result
            },
        );
    }
    element.accept(&mut painter);
    painter.result
}
