use crate::render::pdf::{ImageRef, PdfWriter};
use crate::render::svg_geometry::SvgViewportBox;
use crate::style::computed::{BackgroundPosition, BackgroundRepeat, BackgroundSize};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SvgVisualOverflow {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl SvgVisualOverflow {
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
    pub border_radius: f32,
    pub blur_radius: f32,
    pub size: BackgroundSize,
    pub position: BackgroundPosition,
    pub repeat: BackgroundRepeat,
}

impl BackgroundPaintContext {
    pub fn new(
        reference_box: SvgViewportBox,
        clip_box: SvgViewportBox,
        border_radius: f32,
        blur_radius: f32,
        size: BackgroundSize,
        position: BackgroundPosition,
        repeat: BackgroundRepeat,
    ) -> Self {
        Self {
            reference_box,
            clip_box,
            blur_canvas_box: None,
            border_radius,
            blur_radius,
            size,
            position,
            repeat,
        }
    }

    pub fn with_blur_canvas_box(mut self, blur_canvas_box: Option<SvgViewportBox>) -> Self {
        self.blur_canvas_box = blur_canvas_box;
        self
    }

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
        viewport.width + overflow.left + overflow.right,
        viewport.height + overflow.top + overflow.bottom,
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
}

pub(crate) struct RegisteredBackgroundImage {
    pub name: String,
    pub draw_box: Option<SvgViewportBox>,
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

fn blur_padding_pixels(blur_sigma_pixels: f32) -> u32 {
    (blur_sigma_pixels.max(0.0) * 2.5).ceil() as u32
}

const FILTERED_BACKGROUND_PPI: f32 = 300.0;

fn points_to_filtered_background_pixels(points: f32) -> u32 {
    ((points.max(0.0) * FILTERED_BACKGROUND_PPI / 72.0)
        .round()
        .max(1.0)) as u32
}

fn filtered_background_pixels_to_points(pixels: u32) -> f32 {
    pixels as f32 * 72.0 / FILTERED_BACKGROUND_PPI
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

fn premultiply_rgba(image: &image::RgbaImage) -> image::RgbaImage {
    let mut premultiplied = image::RgbaImage::new(image.width(), image.height());
    for (x, y, pixel) in image.enumerate_pixels() {
        let alpha = u16::from(pixel[3]);
        let premultiply = |channel: u8| -> u8 { ((u16::from(channel) * alpha + 127) / 255) as u8 };
        premultiplied.put_pixel(
            x,
            y,
            image::Rgba([
                premultiply(pixel[0]),
                premultiply(pixel[1]),
                premultiply(pixel[2]),
                pixel[3],
            ]),
        );
    }
    premultiplied
}

fn unpremultiply_rgba(image: &image::RgbaImage) -> image::RgbaImage {
    let mut unpremultiplied = image::RgbaImage::new(image.width(), image.height());
    for (x, y, pixel) in image.enumerate_pixels() {
        let alpha = u16::from(pixel[3]);
        let unpremultiply = |channel: u8| -> u8 {
            if alpha == 0 {
                0
            } else {
                ((u16::from(channel) * 255 + (alpha / 2)) / alpha).min(255) as u8
            }
        };
        unpremultiplied.put_pixel(
            x,
            y,
            image::Rgba([
                unpremultiply(pixel[0]),
                unpremultiply(pixel[1]),
                unpremultiply(pixel[2]),
                pixel[3],
            ]),
        );
    }
    unpremultiplied
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

    let canvas_width = points_to_filtered_background_pixels(request.canvas_box.width);
    let canvas_height = points_to_filtered_background_pixels(request.canvas_box.height);
    let image_width = points_to_filtered_background_pixels(request.image_box.width);
    let image_height = points_to_filtered_background_pixels(request.image_box.height);

    let mut canvas =
        image::RgbaImage::from_pixel(canvas_width, canvas_height, image::Rgba([0, 0, 0, 0]));
    let resized = image::imageops::resize(
        &decoded,
        image_width,
        image_height,
        image::imageops::FilterType::Lanczos3,
    );
    let image_x = ((request.image_box.x - request.canvas_box.x) * FILTERED_BACKGROUND_PPI / 72.0)
        .round() as i64;
    let image_y = ((request.image_box.y - request.canvas_box.y) * FILTERED_BACKGROUND_PPI / 72.0)
        .round() as i64;
    image::imageops::overlay(&mut canvas, &resized, image_x, image_y);

    let blur_pixels = (request.blur_radius * FILTERED_BACKGROUND_PPI / 72.0).max(0.0);
    let padding = blur_padding_pixels(blur_pixels);
    let premultiplied = premultiply_rgba(&canvas);
    let padded = pad_rgba_image(&premultiplied, padding)?;
    let blurred = image::imageops::blur(&image::DynamicImage::ImageRgba8(padded), blur_pixels);
    let encoded = encode_rgba_png(&unpremultiply_rgba(&blurred))?;
    let padding_points = filtered_background_pixels_to_points(padding);
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
    request: Option<RasterBackgroundRequest>,
) -> Option<RegisteredBackgroundImage> {
    let (raw, _mime) = crate::layout::engine::load_src_bytes(href)?;
    let (obj_id, draw_box) =
        if let Some(request) = request.filter(|request| request.blur_radius > 0.0) {
            let (encoded, draw_box) = encode_blurred_png_for_background(&raw, request)?;
            (
                pdf_writer.add_raw_png_image_object(&encoded)?,
                Some(draw_box),
            )
        } else if crate::parser::png::is_png(&raw) {
            (pdf_writer.add_raw_png_image_object(&raw)?, None)
        } else if raw.starts_with(&[0xFF, 0xD8]) {
            let decoded = crate::parser::jpeg::decode_jpeg_for_pdf(&raw)?;
            (
                pdf_writer.add_raw_rgb_image_object(
                    &decoded.rgb_data,
                    decoded.width,
                    decoded.height,
                    decoded.icc_profile.as_deref(),
                )?,
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
    Some(RegisteredBackgroundImage { name, draw_box })
}
