//! Two-pass allocation for one recursively painted filter source.

use crate::layout::elements::LayoutElement;
use crate::render::raster_pixels::PremultipliedRgba8;
use crate::types::EdgeSizes;

use super::canvas::{PaintBounds, RasterCanvas, SurfaceRect};
use super::geometry::{SourceGraphic, SourceRasterGeometry, SourceRasterSpace, source_geometry};
use super::overflow::source_paint_overflow;
use super::painter::{ElementPaintSpace, RootEffectHandling, paint_element};

struct SourcePaintPass {
    pixels: PremultipliedRgba8,
    paint_bounds: Option<crate::types::Rect>,
}

impl SourcePaintPass {
    fn paint(
        element: &dyn LayoutElement,
        geometry: &SourceRasterGeometry,
        fonts: &dyn crate::font_registry::FontRegistry,
        filter_dpi: f32,
    ) -> Option<Self> {
        let dimensions = geometry.dimensions();
        let mut pixels = PremultipliedRgba8::transparent(dimensions.width, dimensions.height);
        let mut paint_bounds = PaintBounds::default();
        {
            let mut canvas = RasterCanvas {
                pixels: &mut pixels,
                pixels_per_point: crate::render::raster_scale::RasterScale::at_dpi(filter_dpi)
                    .pixels_per_point(),
                paint_bounds: &mut paint_bounds,
            };
            paint_element(
                &mut canvas,
                element,
                ElementPaintSpace::root(
                    SurfaceRect::new(geometry.border_origin(), geometry.layout.size),
                    RootEffectHandling::DeferToOwner,
                ),
                fonts,
                filter_dpi,
            )?;
        }
        Some(Self {
            pixels,
            paint_bounds: paint_bounds.resolve(),
        })
    }

    fn into_source(self, geometry: SourceRasterGeometry) -> SourceGraphic {
        SourceGraphic {
            pixels: self.pixels,
            geometry,
            paint_bounds: self.paint_bounds,
        }
    }
}

/// Paint the complete recursive source, expanding once when the first semantic
/// pass discovers positioned descendants outside the provisional allocation.
pub(crate) fn paint_source_graphic(
    element: &dyn LayoutElement,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
    raster_space: SourceRasterSpace,
) -> Option<SourceGraphic> {
    let layout = source_geometry(element)?;
    let authored_overflow = source_paint_overflow(element, layout.size, filter_dpi)?;
    let provisional =
        SourceRasterGeometry::resolve(layout.clone(), authored_overflow, filter_dpi, raster_space)?;
    let first_pass = SourcePaintPass::paint(element, &provisional, fonts, filter_dpi)?;
    let required_overflow = first_pass.paint_bounds.map_or(EdgeSizes::ZERO, |bounds| {
        provisional.required_overflow_for(bounds)
    });
    if provisional
        .paint_overflow()
        .contains_each(required_overflow)
    {
        return Some(first_pass.into_source(provisional));
    }

    let geometry = SourceRasterGeometry::resolve(
        layout,
        authored_overflow.max_each(required_overflow),
        filter_dpi,
        raster_space,
    )?;
    SourcePaintPass::paint(element, &geometry, fonts, filter_dpi)
        .map(|paint| paint.into_source(geometry))
}
