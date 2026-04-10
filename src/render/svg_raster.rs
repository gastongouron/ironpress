use image::RgbaImage;
use resvg::{tiny_skia, usvg};

pub(crate) fn points_to_pixels(points: f32, ppi: f32) -> u32 {
    ((points.max(0.0) * ppi / 72.0).round().max(1.0)) as u32
}

pub(crate) fn pixels_to_points(pixels: u32, ppi: f32) -> f32 {
    pixels as f32 * 72.0 / ppi
}

pub(crate) fn rasterize_svg_to_rgba(
    svg_source: &str,
    width_points: f32,
    height_points: f32,
    ppi: f32,
) -> Option<RgbaImage> {
    if width_points <= 0.0 || height_points <= 0.0 || ppi <= 0.0 {
        return None;
    }

    let width = points_to_pixels(width_points, ppi);
    let height = points_to_pixels(height_points, ppi);

    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg_source, &options).ok()?;

    let source_size = tree.size();
    if source_size.width() <= 0.0 || source_size.height() <= 0.0 {
        return None;
    }

    let scale_x = width as f32 / source_size.width();
    let scale_y = height as f32 / source_size.height();
    let transform = tiny_skia::Transform::from_scale(scale_x, scale_y);
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    RgbaImage::from_raw(width, height, pixmap.take())
}
