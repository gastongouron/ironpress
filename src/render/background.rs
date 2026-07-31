use crate::render::pdf::{ImageRef, PdfWriter};
use crate::render::svg_geometry::SvgViewportBox;
use crate::style::computed::{BackgroundPosition, BackgroundRepeat, BackgroundSize};
use crate::types::CornerRadii;

mod bleed;
mod tiles;

pub(crate) use bleed::BackgroundBleed;
pub(crate) use tiles::{BackgroundRepeatModes, BackgroundTilePattern};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SvgVisualOverflow {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl SvgVisualOverflow {
    pub const fn horizontal(self) -> f32 {
        self.left + self.right
    }

    pub const fn vertical(self) -> f32 {
        self.top + self.bottom
    }

    pub fn scale(self, scale_x: f32, scale_y: f32) -> Self {
        Self {
            left: self.left * scale_x,
            top: self.top * scale_y,
            right: self.right * scale_x,
            bottom: self.bottom * scale_y,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BackgroundPaintContext {
    pub reference_box: SvgViewportBox,
    pub clip_box: SvgViewportBox,
    blur_canvas_box: Option<SvgViewportBox>,
    pub border_radii: CornerRadii,
    pub blur_radius: f32,
    pub size: BackgroundSize,
    pub position: BackgroundPosition,
    pub repeat: BackgroundRepeat,
}

impl BackgroundPaintContext {
    pub fn new(
        reference_box: SvgViewportBox,
        clip_box: SvgViewportBox,
        border_radii: CornerRadii,
        blur_radius: f32,
        size: BackgroundSize,
        position: BackgroundPosition,
        repeat: BackgroundRepeat,
    ) -> Self {
        Self {
            reference_box,
            clip_box,
            blur_canvas_box: None,
            border_radii,
            blur_radius,
            size,
            position,
            repeat,
        }
    }

    #[cfg(test)]
    pub fn with_blur_canvas_box(mut self, blur_canvas_box: Option<SvgViewportBox>) -> Self {
        self.blur_canvas_box = blur_canvas_box;
        self
    }

    #[cfg(test)]
    pub fn tile_origin(self, offset_x: f32, offset_y: f32) -> SvgViewportBox {
        self.reference_box.translate(offset_x, -offset_y)
    }

    pub fn local_reference_box(self) -> SvgViewportBox {
        SvgViewportBox::new(
            0.0,
            0.0,
            self.reference_box.width,
            self.reference_box.height,
        )
    }

    fn local_clip_reference_box(self) -> SvgViewportBox {
        self.blur_canvas_box
            .unwrap_or(self.reference_box)
            .translate(-self.reference_box.x, -self.reference_box.y)
    }

    pub fn local_blur_canvas_box(self) -> SvgViewportBox {
        self.local_reference_box()
            .union(self.local_clip_reference_box())
    }
}

pub(crate) fn viewport_box_from_overflow(
    viewport: SvgViewportBox,
    overflow: SvgVisualOverflow,
) -> SvgViewportBox {
    SvgViewportBox::new(
        viewport.x - overflow.left,
        viewport.y - overflow.bottom,
        viewport.width + overflow.horizontal(),
        viewport.height + overflow.vertical(),
    )
}

pub(crate) fn overflow_from_viewport_box(
    viewport: SvgViewportBox,
    draw_box: SvgViewportBox,
) -> SvgVisualOverflow {
    let viewport_right = viewport.x + viewport.width;
    let viewport_top = viewport.y + viewport.height;
    let draw_right = draw_box.x + draw_box.width;
    let draw_top = draw_box.y + draw_box.height;

    SvgVisualOverflow {
        left: (viewport.x - draw_box.x).max(0.0),
        top: (draw_top - viewport_top).max(0.0),
        right: (draw_right - viewport_right).max(0.0),
        bottom: (viewport.y - draw_box.y).max(0.0),
    }
}

pub(crate) fn svg_visual_overflow(tree: &crate::parser::svg::SvgTree) -> SvgVisualOverflow {
    let root_width = if tree.width > 0.0 {
        tree.width
    } else {
        tree.view_box
            .as_ref()
            .map_or(0.0, |view_box| view_box.width)
    };
    let root_height = if tree.height > 0.0 {
        tree.height
    } else {
        tree.view_box
            .as_ref()
            .map_or(0.0, |view_box| view_box.height)
    };
    if root_width <= 0.0 || root_height <= 0.0 {
        return SvgVisualOverflow::default();
    }

    let mut overflow = SvgVisualOverflow::default();
    collect_svg_visual_overflow(&tree.children, root_width, root_height, &mut overflow);
    overflow
}

fn collect_svg_visual_overflow(
    nodes: &[crate::parser::svg::SvgNode],
    root_width: f32,
    root_height: f32,
    overflow: &mut SvgVisualOverflow,
) {
    for node in nodes {
        match node {
            crate::parser::svg::SvgNode::Image {
                x,
                y,
                width,
                height,
                ..
            } => {
                overflow.left = overflow.left.max((-*x).max(0.0));
                overflow.top = overflow.top.max((-*y).max(0.0));
                overflow.right = overflow.right.max((x + width - root_width).max(0.0));
                overflow.bottom = overflow.bottom.max((y + height - root_height).max(0.0));
            }
            crate::parser::svg::SvgNode::Group {
                transform,
                children,
                ..
            } if transform.is_none() => {
                collect_svg_visual_overflow(children, root_width, root_height, overflow);
            }
            _ => {}
        }
    }
}

struct SyntheticRasterBackground<'a> {
    href: &'a str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct RasterBackgroundRequest {
    pub canvas_box: SvgViewportBox,
    pub image_box: SvgViewportBox,
    pub blur_radius: f32,
    pub filter_dpi: f32,
}

pub(crate) struct RegisteredBackgroundImage {
    pub name: String,
    pub draw_box: Option<SvgViewportBox>,
    pub pixel_dimensions: crate::util::RasterDimensions,
}

pub(crate) fn synthetic_raster_background(
    tree: &crate::parser::svg::SvgTree,
) -> Option<(&str, SvgViewportBox)> {
    if !tree.defs.gradients.is_empty() || !tree.defs.clip_paths.is_empty() {
        return None;
    }

    match tree.children.as_slice() {
        [
            crate::parser::svg::SvgNode::Image {
                x,
                y,
                width,
                height,
                href,
                ..
            },
        ] => {
            let background = SyntheticRasterBackground {
                href,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            };
            Some((
                background.href,
                SvgViewportBox::new(
                    background.x,
                    background.y,
                    background.width,
                    background.height,
                ),
            ))
        }
        _ => None,
    }
}

fn pad_rgba_image(image: &image::RgbaImage, padding: u32) -> Option<image::RgbaImage> {
    if padding == 0 {
        return Some(image.clone());
    }

    let padded_width = image.width().checked_add(padding.checked_mul(2)?)?;
    let padded_height = image.height().checked_add(padding.checked_mul(2)?)?;
    let mut padded =
        image::RgbaImage::from_pixel(padded_width, padded_height, image::Rgba([0, 0, 0, 0]));
    image::imageops::overlay(&mut padded, image, i64::from(padding), i64::from(padding));
    Some(padded)
}

fn encode_rgba_png(image: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut encoded = Vec::new();
    image::DynamicImage::ImageRgba8(image.clone())
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some(encoded)
}

fn encode_blurred_png_for_background(
    raw: &[u8],
    request: RasterBackgroundRequest,
) -> Option<(Vec<u8>, SvgViewportBox)> {
    let decoded = image::load_from_memory(raw).ok()?.to_rgba8();
    if request.canvas_box.width <= 0.0 || request.canvas_box.height <= 0.0 {
        return None;
    }
    if request.image_box.width <= 0.0 || request.image_box.height <= 0.0 {
        return None;
    }

    let scale = crate::render::raster_scale::RasterScale::at_dpi(request.filter_dpi);
    let canvas_width = scale.sample_count(request.canvas_box.width)?;
    let canvas_height = scale.sample_count(request.canvas_box.height)?;
    let image_width = scale.sample_count(request.image_box.width)?;
    let image_height = scale.sample_count(request.image_box.height)?;

    let mut canvas =
        image::RgbaImage::from_pixel(canvas_width, canvas_height, image::Rgba([0, 0, 0, 0]));
    let resized = image::imageops::resize(
        &decoded,
        image_width,
        image_height,
        image::imageops::FilterType::Lanczos3,
    );
    let image_x = scale.round(request.image_box.x - request.canvas_box.x)?;
    let image_y = scale.round(request.image_box.y - request.canvas_box.y)?;
    image::imageops::overlay(&mut canvas, &resized, image_x, image_y);

    let kernel =
        crate::render::blur::FilterBlurKernel::new(request.blur_radius, request.filter_dpi)?;
    let padding = kernel.padding_px;
    let padded = pad_rgba_image(&canvas, padding)?;
    let blurred = crate::render::blur::blur_css_filter(&padded, kernel)?;
    let encoded = encode_rgba_png(&blurred)?;
    let padding_points = scale.pixels_to_points(padding as f32);
    let draw_box = SvgViewportBox::new(
        request.canvas_box.x - padding_points,
        request.canvas_box.y - padding_points,
        request.canvas_box.width + padding_points * 2.0,
        request.canvas_box.height + padding_points * 2.0,
    );
    Some((encoded, draw_box))
}

pub(crate) fn register_background_image(
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    href: &str,
    display_box: SvgViewportBox,
    request: Option<RasterBackgroundRequest>,
) -> Option<RegisteredBackgroundImage> {
    let (raw, _mime) = crate::layout::images::load_resource(href, None)?;
    let (obj_id, draw_box) =
        if let Some(request) = request.filter(|request| request.blur_radius > 0.0) {
            let (encoded, draw_box) = encode_blurred_png_for_background(&raw, request)?;
            (
                pdf_writer.add_raw_png_image_object(&encoded)?,
                Some(draw_box),
            )
        } else if crate::parser::png::is_png(&raw) {
            let png = crate::parser::png::parse_png(&raw)?;
            let metadata = crate::layout::engine::PngMetadata {
                channels: png.channels,
                bit_depth: png.bit_depth,
            };
            let format = match png.channels {
                2 | 4 => crate::layout::engine::ImageFormat::PngAlpha,
                _ => crate::layout::engine::ImageFormat::Png,
            };
            (
                pdf_writer.add_decodable_source_image_object(
                    &raw,
                    png.width,
                    png.height,
                    format,
                    Some(&metadata),
                    display_box.width,
                    display_box.height,
                )?,
                None,
            )
        } else if raw.starts_with(&[0xFF, 0xD8]) {
            (
                {
                    let (width, height) = crate::parser::jpeg::parse_jpeg_dimensions(&raw)?;
                    pdf_writer.add_source_image_object(
                        &raw,
                        width,
                        height,
                        crate::layout::engine::ImageFormat::Jpeg,
                        None,
                        display_box.width,
                        display_box.height,
                    )
                },
                None,
            )
        } else {
            return None;
        };

    let name = format!("Im{obj_id}");
    page_images.push(ImageRef {
        name: name.clone(),
        obj_id,
    });
    let pixel_dimensions = pdf_writer.image_dimensions(obj_id)?;
    Some(RegisteredBackgroundImage {
        name,
        draw_box,
        pixel_dimensions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::svg_geometry::SvgViewportBox;
    use crate::style::computed::{BackgroundPosition, BackgroundRepeat, BackgroundSize};
    use crate::types::CornerRadius;

    // ── SvgVisualOverflow::scale ─────────────────────────────────────────────

    #[test]
    fn svg_visual_overflow_scale_uniform() {
        let overflow = SvgVisualOverflow {
            left: 2.0,
            top: 3.0,
            right: 4.0,
            bottom: 5.0,
        };
        let scaled = overflow.scale(2.0, 3.0);
        assert_eq!(scaled.left, 4.0);
        assert_eq!(scaled.top, 9.0);
        assert_eq!(scaled.right, 8.0);
        assert_eq!(scaled.bottom, 15.0);
    }

    #[test]
    fn svg_visual_overflow_scale_zero() {
        let overflow = SvgVisualOverflow {
            left: 10.0,
            top: 20.0,
            right: 30.0,
            bottom: 40.0,
        };
        let scaled = overflow.scale(0.0, 0.0);
        assert_eq!(scaled.left, 0.0);
        assert_eq!(scaled.top, 0.0);
        assert_eq!(scaled.right, 0.0);
        assert_eq!(scaled.bottom, 0.0);
    }

    // ── BackgroundPaintContext helpers ────────────────────────────────────────

    fn make_context(ref_x: f32, ref_y: f32, w: f32, h: f32) -> BackgroundPaintContext {
        let reference_box = SvgViewportBox::new(ref_x, ref_y, w, h);
        let clip_box = SvgViewportBox::new(ref_x, ref_y, w, h);
        BackgroundPaintContext::new(
            reference_box,
            clip_box,
            CornerRadii::ZERO,
            0.0,
            BackgroundSize::Auto,
            BackgroundPosition::default(),
            BackgroundRepeat::NoRepeat,
        )
    }

    #[test]
    fn background_paint_context_tile_origin_no_offset() {
        let ctx = make_context(10.0, 20.0, 100.0, 50.0);
        let origin = ctx.tile_origin(0.0, 0.0);
        assert_eq!(origin.x, 10.0);
        assert_eq!(origin.y, 20.0);
        assert_eq!(origin.width, 100.0);
        assert_eq!(origin.height, 50.0);
    }

    #[test]
    fn background_paint_context_tile_origin_with_offset() {
        let ctx = make_context(10.0, 20.0, 100.0, 50.0);
        // tile_origin translates by (offset_x, -offset_y)
        let origin = ctx.tile_origin(5.0, 3.0);
        assert_eq!(origin.x, 15.0);
        assert_eq!(origin.y, 17.0); // 20 - 3
    }

    #[test]
    fn background_paint_context_local_reference_box() {
        let ctx = make_context(50.0, 80.0, 200.0, 100.0);
        let local = ctx.local_reference_box();
        assert_eq!(local.x, 0.0);
        assert_eq!(local.y, 0.0);
        assert_eq!(local.width, 200.0);
        assert_eq!(local.height, 100.0);
    }

    #[test]
    fn background_paint_context_keeps_resolved_corner_radii_together() {
        let radii = CornerRadii::new(
            CornerRadius::new(1.0, 2.0),
            CornerRadius::new(3.0, 4.0),
            CornerRadius::new(5.0, 6.0),
            CornerRadius::new(7.0, 8.0),
        );
        let reference_box = SvgViewportBox::new(0.0, 0.0, 100.0, 50.0);
        let ctx = BackgroundPaintContext::new(
            reference_box,
            reference_box,
            radii,
            0.0,
            BackgroundSize::Auto,
            BackgroundPosition::default(),
            BackgroundRepeat::NoRepeat,
        );

        assert_eq!(ctx.border_radii, radii);
    }

    // ── viewport_box_from_overflow / overflow_from_viewport_box symmetry ─────

    #[test]
    fn viewport_box_overflow_roundtrip() {
        let viewport = SvgViewportBox::new(10.0, 20.0, 100.0, 80.0);
        let overflow = SvgVisualOverflow {
            left: 5.0,
            top: 8.0,
            right: 12.0,
            bottom: 3.0,
        };

        let draw_box = viewport_box_from_overflow(viewport, overflow);
        let recovered = overflow_from_viewport_box(viewport, draw_box);

        assert!((recovered.left - overflow.left).abs() < 1e-4);
        assert!((recovered.top - overflow.top).abs() < 1e-4);
        assert!((recovered.right - overflow.right).abs() < 1e-4);
        assert!((recovered.bottom - overflow.bottom).abs() < 1e-4);
    }

    #[test]
    fn overflow_from_viewport_box_no_overflow() {
        // draw_box equal to viewport → zero overflow
        let viewport = SvgViewportBox::new(0.0, 0.0, 100.0, 100.0);
        let overflow = overflow_from_viewport_box(viewport, viewport);
        assert_eq!(overflow.left, 0.0);
        assert_eq!(overflow.top, 0.0);
        assert_eq!(overflow.right, 0.0);
        assert_eq!(overflow.bottom, 0.0);
    }

    #[test]
    fn filtered_background_uses_the_shared_physical_scale() {
        let scale = crate::render::raster_scale::RasterScale::at_dpi(300.0);
        let original_points = 72.0;
        let pixels = scale
            .sample_count(original_points)
            .expect("positive test extent has a sample count");
        let recovered = scale.pixels_to_points(pixels as f32);

        assert_eq!(pixels, 300);
        assert!((recovered - original_points).abs() < 0.5);
    }

    #[test]
    fn filtered_background_rejects_empty_extents() {
        let scale = crate::render::raster_scale::RasterScale::at_dpi(300.0);

        assert_eq!(scale.sample_count(0.0), None);
        assert_eq!(scale.sample_count(-100.0), None);
    }

    fn make_solid_image(r: u8, g: u8, b: u8, a: u8) -> image::RgbaImage {
        let mut img = image::RgbaImage::new(2, 2);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([r, g, b, a]);
        }
        img
    }

    // ── pad_rgba_image ───────────────────────────────────────────────────────

    #[test]
    fn pad_rgba_image_zero_padding_same_dimensions() {
        let original = make_solid_image(10, 20, 30, 255);
        let padded = pad_rgba_image(&original, 0).expect("pad_rgba_image returned None");
        assert_eq!(padded.width(), original.width());
        assert_eq!(padded.height(), original.height());
    }

    #[test]
    fn pad_rgba_image_nonzero_padding_expands_dimensions() {
        let original = make_solid_image(10, 20, 30, 255);
        let padding = 5u32;
        let padded = pad_rgba_image(&original, padding).expect("pad_rgba_image returned None");
        assert_eq!(padded.width(), original.width() + padding * 2);
        assert_eq!(padded.height(), original.height() + padding * 2);
    }

    #[test]
    fn pad_rgba_image_border_is_transparent() {
        let original = image::RgbaImage::from_pixel(4, 4, image::Rgba([255u8, 0, 0, 255]));
        let padded = pad_rgba_image(&original, 2).expect("pad_rgba_image returned None");
        // Top-left corner pixel should be transparent (part of the padding).
        let corner = padded.get_pixel(0, 0);
        assert_eq!(corner[3], 0);
    }
}
