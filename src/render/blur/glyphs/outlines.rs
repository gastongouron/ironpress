//! Foundation-compatible glyph outlines for raster filter text.

use crate::parser::ttf::TtfFont;
use crate::text::ShapedGlyph;
use resvg::tiny_skia;
use skrifa::MetadataProvider as _;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{
    DrawSettings, Engine, GlyphStyles, HintingInstance, HintingOptions, OutlinePen, SmoothMode,
    Target,
};

use crate::render::raster_pixels::DevicePixelVector;

/// Maximum device-sample shortfall treated as arithmetic noise at a Skia
/// glyph-positioning tie.
const DEVICE_QUANTIZATION_TIE_TOLERANCE: f32 = 0.001;

struct GlyphPathPen<'a> {
    builder: &'a mut tiny_skia::PathBuilder,
    pen_x: f32,
    baseline_y: f32,
    shear: f32,
}

/// Skia's device-space positioning policy for an axis-aligned glyph run.
///
/// Chromium enables subpixel positioning while baseline snapping is active.
/// Skia consequently rounds each glyph's inline origin to a quarter sample and
/// its block-axis baseline to a whole sample. The run origin is retained so
/// shaped offsets and advances can be quantized independently without changing
/// layout-space metrics.
#[derive(Debug, Clone, Copy)]
pub(super) struct FoundationGlyphPositioning {
    requested_origin: DevicePixelVector,
    quantized_origin: DevicePixelVector,
}

impl FoundationGlyphPositioning {
    pub(super) fn new(requested_origin: DevicePixelVector) -> Option<Self> {
        if !requested_origin.x.is_finite() || !requested_origin.y.is_finite() {
            return None;
        }
        Some(Self {
            requested_origin,
            quantized_origin: DevicePixelVector::new(
                quantize_inline(requested_origin.x),
                quantize_baseline(requested_origin.y),
            ),
        })
    }

    pub(super) const fn origin(self) -> DevicePixelVector {
        self.quantized_origin
    }

    fn glyph_origin(self, inline_offset: f32, baseline_offset: f32) -> DevicePixelVector {
        DevicePixelVector::new(
            quantize_inline(self.requested_origin.x + inline_offset) - self.quantized_origin.x,
            quantize_baseline(self.requested_origin.y + baseline_offset) - self.quantized_origin.y,
        )
    }
}

fn quantize_inline(value: f32) -> f32 {
    ((value + 0.125 + DEVICE_QUANTIZATION_TIE_TOLERANCE) * 4.0).floor() * 0.25
}

fn quantize_baseline(value: f32) -> f32 {
    (value + 0.5 + DEVICE_QUANTIZATION_TIE_TOLERANCE).floor()
}

/// One Foundation-compatible outline in device space.
pub(super) struct FoundationGlyphOutline(tiny_skia::Path);

impl FoundationGlyphOutline {
    /// Apply Skia's frame-and-fill fake-bold geometry before scan conversion.
    pub(super) fn embolden(self, stroke_width: f32) -> Option<Self> {
        if stroke_width <= 0.0 {
            return Some(self);
        }
        let stroke = tiny_skia::Stroke {
            width: stroke_width,
            ..Default::default()
        };
        let stroked = tiny_skia::PathStroker::new().stroke(&self.0, &stroke, 1.0)?;
        let mut combined = tiny_skia::PathBuilder::new();
        combined.push_path(&self.0);
        combined.push_path(&stroked);
        combined.finish().map(Self)
    }

    pub(super) fn bounds(&self) -> crate::types::Rect {
        let bounds = self.0.bounds();
        crate::types::Rect::from_xywh(bounds.left(), bounds.top(), bounds.width(), bounds.height())
    }

    pub(super) fn rasterize(&self, frame: &super::GlyphMaskFrame) -> Option<image::GrayImage> {
        let width = frame.dimensions.width;
        let height = frame.dimensions.height;
        let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
        let transform = tiny_skia::Transform::from_translate(
            frame.baseline_in_mask.x,
            frame.baseline_in_mask.y,
        );
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(tiny_skia::Color::WHITE);
        paint.anti_alias = true;
        pixmap.fill_path(
            &self.0,
            &paint,
            tiny_skia::FillRule::Winding,
            transform,
            None,
        );
        let mut mask = image::GrayImage::new(width, height);
        for (index, pixel) in pixmap.pixels().iter().enumerate() {
            let x = index as u32 % width;
            let y = index as u32 / width;
            mask.put_pixel(x, y, image::Luma([pixel.alpha()]));
        }
        Some(mask)
    }
}

impl GlyphPathPen<'_> {
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (self.pen_x + x + self.shear * y, self.baseline_y - y)
    }
}

impl OutlinePen for GlyphPathPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        let (control_x, control_y) = self.point(control_x, control_y);
        let (x, y) = self.point(x, y);
        self.builder.quad_to(control_x, control_y, x, y);
    }

    fn curve_to(
        &mut self,
        first_x: f32,
        first_y: f32,
        second_x: f32,
        second_y: f32,
        x: f32,
        y: f32,
    ) {
        let (first_x, first_y) = self.point(first_x, first_y);
        let (second_x, second_y) = self.point(second_x, second_y);
        let (x, y) = self.point(x, y);
        self.builder
            .cubic_to(first_x, first_y, second_x, second_y, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

/// Build one Foundation-compatible device-space path while preserving shaped
/// run advances.
///
/// Chromium's slight hinting mode forces Skrifa's TrueType autohinter and uses
/// a light grayscale target. Skia disables hinting for sheared outlines because
/// the hint grid no longer represents their transformed geometry.
pub(super) fn foundation_run_outline(
    font: &TtfFont,
    size_px: f32,
    glyphs: &[ShapedGlyph],
    points_to_pixels: f32,
    shear: f32,
    positioning: FoundationGlyphPositioning,
) -> Option<FoundationGlyphOutline> {
    if !size_px.is_finite() || size_px <= 0.0 {
        return None;
    }
    let font_ref =
        skrifa::FontRef::from_index(font.program.bytes(), font.program.face_index().get()).ok()?;
    let outlines = font_ref.outline_glyphs();
    let hinting = (shear == 0.0)
        .then(|| {
            HintingInstance::new(
                &outlines,
                Size::new(size_px),
                LocationRef::default(),
                HintingOptions {
                    engine: Engine::Auto(Some(GlyphStyles::new(&outlines))),
                    target: Target::from(SmoothMode::Light),
                },
            )
            .ok()
        })
        .flatten();
    let mut builder = tiny_skia::PathBuilder::new();
    let mut pen_x = 0.0;
    for shaped in glyphs {
        let glyph = outlines.get(skrifa::GlyphId::new(u32::from(shaped.glyph_id)))?;
        let glyph_origin = positioning.glyph_origin(
            pen_x + shaped.x_offset * points_to_pixels,
            -shaped.y_offset * points_to_pixels,
        );
        let mut pen = GlyphPathPen {
            builder: &mut builder,
            pen_x: glyph_origin.x,
            baseline_y: glyph_origin.y,
            shear,
        };
        let settings = hinting.as_ref().map_or_else(
            || DrawSettings::unhinted(Size::new(size_px), LocationRef::default()),
            |instance| DrawSettings::hinted(instance, false),
        );
        glyph.draw(settings, &mut pen).ok()?;
        pen_x += shaped.x_advance * points_to_pixels;
    }
    builder.finish().map(FoundationGlyphOutline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foundation_positioning_quantizes_each_device_axis_like_skia() {
        let positioning = FoundationGlyphPositioning::new(DevicePixelVector::new(62.3, 212.2))
            .expect("finite device position");

        assert_eq!(positioning.origin(), DevicePixelVector::new(62.25, 212.0));
        assert_eq!(
            positioning.glyph_origin(10.2, 0.6),
            DevicePixelVector::new(10.25, 1.0)
        );
    }

    #[test]
    fn foundation_positioning_preserves_half_sample_ties_despite_float_noise() {
        let positioning =
            FoundationGlyphPositioning::new(DevicePixelVector::new(62.499_98, 212.499_98))
                .expect("finite device position");

        assert_eq!(positioning.origin(), DevicePixelVector::new(62.5, 213.0));
        assert_eq!(
            positioning.glyph_origin(0.0, 0.0),
            DevicePixelVector::new(0.0, 0.0)
        );
    }
}
