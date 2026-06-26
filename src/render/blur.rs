//! CSS `filter: blur()` and `filter: drop-shadow()` raster compositing.
//!
//! ironpress paints boxes and replaced images as vector content. The CSS
//! `filter` property (css-filter-effects-1 §2) instead operates on the
//! *rasterized* output of the element: a gaussian blur (or drop-shadow) is
//! applied to the painted pixels and feathers *outside* the element's border
//! box. To match Chrome we rasterize the element's paint into a pixel buffer
//! padded with transparency, gaussian-blur it (reusing `image::imageops::blur`,
//! which is a true separable gaussian with `sigma = stdDeviation`), and embed
//! the result as a PDF image XObject positioned so the padded buffer feathers
//! beyond the original box.
//!
//! Per css-filter-effects-1 §4.1, `blur(<length>)` uses a gaussian with
//! `stdDeviation` equal to that length. We rasterize at the parity device scale
//! so the embedded bitmap matches the final 300-DPI raster resolution, then the
//! sigma in *buffer* pixels is `radius_css_px * filter_dpi/96`.

use crate::layout::engine::{ImageFormat, LayoutBorder, RasterImageAsset};

/// Points per CSS pixel (1px = 0.75pt). `blur_radius` is stored in points.
const PT_PER_PX: f32 = 0.75;

fn filter_dpi_scale(filter_dpi: f32) -> f32 {
    filter_dpi.max(1.0) / 96.0
}

/// A blurred raster ready for embedding plus the overflow it adds outside the
/// element's border box (in points, applied symmetrically on every side).
pub(crate) struct BlurredRaster {
    pub asset: RasterImageAsset,
    /// Extra paint extent beyond each border-box edge, in points.
    pub overflow_pt: f32,
}

/// Number of padding pixels to add on each side so a gaussian with the given
/// sigma can feather without clipping (3σ captures ~99.7% of the kernel).
fn pad_pixels(sigma: f32) -> u32 {
    (sigma * 3.0).ceil().max(1.0) as u32
}

/// Gaussian-blur a straight-alpha RGBA buffer correctly: `image::imageops::blur`
/// blurs each channel independently, so transparent (0,0,0,0) padding would
/// bleed black into the feathered edge. Premultiply first, blur, then
/// un-premultiply so only visible colour contributes.
fn blur_premultiplied(img: &image::RgbaImage, sigma: f32) -> image::RgbaImage {
    let mut pre = img.clone();
    for px in pre.pixels_mut() {
        let a = px[3] as u16;
        px[0] = (px[0] as u16 * a / 255) as u8;
        px[1] = (px[1] as u16 * a / 255) as u8;
        px[2] = (px[2] as u16 * a / 255) as u8;
    }
    let mut blurred = image::imageops::blur(&pre, sigma);
    for px in blurred.pixels_mut() {
        let a = px[3] as u32;
        if a == 0 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
        } else {
            px[0] = ((px[0] as u32 * 255 / a).min(255)) as u8;
            px[1] = ((px[1] as u32 * 255 / a).min(255)) as u8;
            px[2] = ((px[2] as u32 * 255 / a).min(255)) as u8;
        }
    }
    blurred
}

/// Encode a (possibly padded) RGBA buffer as a full PNG file and wrap it in a
/// `PngAlpha` asset, whose embedding path decodes colour + soft-mask so the
/// transparent feathered border survives into the PDF.
fn rgba_to_png_alpha_asset(img: image::RgbaImage) -> Option<RasterImageAsset> {
    let (width, height) = (img.width(), img.height());
    let mut encoded = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some(RasterImageAsset {
        data: encoded,
        source_width: width,
        source_height: height,
        format: ImageFormat::PngAlpha,
        png_metadata: None,
    })
}

/// Rasterize a (rounded) `box-shadow` rectangle into a transparent, padded RGBA
/// buffer, gaussian-blur it, and return the embeddable asset plus the overflow
/// it adds beyond each edge of the shadow rect.
///
/// `width_pt`/`height_pt` are the shadow rect size in points (border box grown
/// by `spread`). `radius_pt` is the corner radius (0 for square). `blur_pt` is
/// the CSS `box-shadow` blur radius in points; css-backgrounds-3 §7.1.1 defines
/// the blur as a gaussian whose standard deviation is *half* the blur radius
/// (`sigma = blur / 2`). `color` is straight-alpha sRGB. The returned overflow
/// is the per-side padding in points: the buffer feathers symmetrically beyond
/// the shadow rect, so the caller positions the image at the shadow rect minus
/// `overflow_pt` on each side. Returns `None` when nothing would paint.
pub(crate) fn blur_shadow_rect(
    width_pt: f32,
    height_pt: f32,
    radius_pt: f32,
    blur_pt: f32,
    color: (f32, f32, f32, f32),
    filter_dpi: f32,
) -> Option<BlurredRaster> {
    let (_, _, _, a) = color;
    if width_pt <= 0.0 || height_pt <= 0.0 || a <= 0.0 {
        return None;
    }

    use resvg::tiny_skia;

    // css-backgrounds-3: blur radius is 2σ, so σ = blur/2. Map to buffer pixels.
    let s = filter_dpi_scale(filter_dpi);
    let sigma = (blur_pt / PT_PER_PX) * s / 2.0;
    let pad = pad_pixels(sigma);
    let box_w = (width_pt / PT_PER_PX * s).round().max(1.0) as u32;
    let box_h = (height_pt / PT_PER_PX * s).round().max(1.0) as u32;
    let buf_w = box_w + 2 * pad;
    let buf_h = box_h + 2 * pad;

    let mut pixmap = tiny_skia::Pixmap::new(buf_w, buf_h)?;
    let ox = pad as f32;
    let oy = pad as f32;

    let (r, g, b, _) = color;
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(color8(r, g, b, a));
    paint.anti_alias = true;

    let radius_px = (radius_pt / PT_PER_PX * s)
        .min(box_w as f32 / 2.0)
        .min(box_h as f32 / 2.0);
    if radius_px > 0.5 {
        let mut pb = tiny_skia::PathBuilder::new();
        let rr = radius_px;
        let (x0, y0) = (ox, oy);
        let (x1, y1) = (ox + box_w as f32, oy + box_h as f32);
        // Rounded rect via 4 quadratic-ish corners (use cubic-free arcs through
        // line + quad approximations is unnecessary; tiny-skia has no arc API,
        // so approximate corners with quad beziers — visually exact after blur).
        pb.move_to(x0 + rr, y0);
        pb.line_to(x1 - rr, y0);
        pb.quad_to(x1, y0, x1, y0 + rr);
        pb.line_to(x1, y1 - rr);
        pb.quad_to(x1, y1, x1 - rr, y1);
        pb.line_to(x0 + rr, y1);
        pb.quad_to(x0, y1, x0, y1 - rr);
        pb.line_to(x0, y0 + rr);
        pb.quad_to(x0, y0, x0 + rr, y0);
        pb.close();
        if let Some(path) = pb.finish() {
            pixmap.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
    } else if let Some(rect) = tiny_skia::Rect::from_xywh(ox, oy, box_w as f32, box_h as f32) {
        pixmap.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
    }

    let rgba = pixmap_to_rgba(&pixmap, buf_w, buf_h);
    let rgba = if sigma > 0.0 {
        blur_premultiplied(&rgba, sigma)
    } else {
        rgba
    };

    let overflow_pt = pad as f32 / s * PT_PER_PX;
    let asset = rgba_to_png_alpha_asset(rgba)?;
    Some(BlurredRaster { asset, overflow_pt })
}

/// Gaussian-blur a pre-rasterized straight-alpha coverage mask (e.g. shadow
/// glyphs), tinting with `color`, and return the embeddable asset plus the
/// per-side overflow in points.
///
/// `mask` is an RGBA buffer at `DEVICE_SCALE` whose **alpha** is the shadow
/// coverage (RGB ignored). `mask_origin_pt` is where the mask's top-left maps in
/// the unpadded device-pixel space; callers only need `overflow_pt` to know how
/// much the buffer grew. `blur_pt` is the CSS `text-shadow` blur radius in
/// points; like box-shadow, `sigma = blur / 2`. The mask is padded so the blur
/// feathers without clipping. Returns `None` when the mask is empty.
pub(crate) fn blur_shadow_alpha_mask(
    mask: &image::GrayImage,
    blur_pt: f32,
    color: (f32, f32, f32, f32),
    filter_dpi: f32,
) -> Option<(BlurredRaster, u32)> {
    let (mw, mh) = (mask.width(), mask.height());
    let (cr, cg, cb, ca) = color;
    if mw == 0 || mh == 0 || ca <= 0.0 {
        return None;
    }

    let s = filter_dpi_scale(filter_dpi);
    let sigma = (blur_pt / PT_PER_PX) * s / 2.0;
    let pad = pad_pixels(sigma);
    let buf_w = mw + 2 * pad;
    let buf_h = mh + 2 * pad;

    let (r8, g8, b8) = (
        (cr.clamp(0.0, 1.0) * 255.0).round() as u8,
        (cg.clamp(0.0, 1.0) * 255.0).round() as u8,
        (cb.clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    let mut tinted = image::RgbaImage::new(buf_w, buf_h);
    let mut any = false;
    for y in 0..mh {
        for x in 0..mw {
            let cov = mask.get_pixel(x, y)[0];
            if cov == 0 {
                continue;
            }
            any = true;
            let out_a = (cov as f32 * ca).round().clamp(0.0, 255.0) as u8;
            tinted.put_pixel(x + pad, y + pad, image::Rgba([r8, g8, b8, out_a]));
        }
    }
    if !any {
        return None;
    }
    let blurred = if sigma > 0.0 {
        blur_premultiplied(&tinted, sigma)
    } else {
        tinted
    };

    let overflow_pt = pad as f32 / s * PT_PER_PX;
    let asset = rgba_to_png_alpha_asset(blurred)?;
    Some((BlurredRaster { asset, overflow_pt }, pad))
}

/// A rasterized text run's alpha coverage plus where the text origin (baseline,
/// left edge) sits inside the mask, in device pixels from the mask's top-left.
pub(crate) struct GlyphRaster {
    pub mask: image::GrayImage,
    /// Device px from the mask's left edge to the text origin x.
    pub origin_x_px: f32,
    /// Device px from the mask's TOP edge down to the baseline.
    pub baseline_y_px: f32,
}

/// Rasterize a run's shaped glyph outlines into an 8-bit alpha coverage mask at
/// `DEVICE_SCALE`, for use as a `text-shadow` blur source. `font_data` is the
/// raw TTF/OTF bytes; `units_per_em` is the font's em scale; `font_size_pt` is
/// the run's font size in points; `glyphs` is the shaped run. Returns the mask
/// plus the text-origin position inside it, or `None` when the font can't be
/// parsed or nothing is drawn (so the caller falls back to a sharp copy).
pub(crate) fn rasterize_run_alpha(
    font_data: &[u8],
    units_per_em: u16,
    font_size_pt: f32,
    glyphs: &[crate::text::ShapedGlyph],
    filter_dpi: f32,
) -> Option<GlyphRaster> {
    use resvg::tiny_skia;

    if units_per_em == 0 || font_size_pt <= 0.0 || glyphs.is_empty() {
        return None;
    }
    let face = rustybuzz::ttf_parser::Face::parse(font_data, 0).ok()?;

    // Glyph font units -> device pixels: (units/upem) * font_size_pt(px-equiv)
    // * filter_dpi/96. font_size is in points; CSS px = pt / PT_PER_PX.
    let s = filter_dpi_scale(filter_dpi);
    let upem = units_per_em as f32;
    let px_per_unit = (font_size_pt / PT_PER_PX) * s / upem;
    // Advances/offsets from shaping are already in points; -> device px.
    let pt_to_px = s / PT_PER_PX;

    // Build one path for all glyphs, placed along the baseline. The path is in a
    // coordinate frame where the text origin (baseline, x=0) is at (0,0) and +y
    // is DOWN (device pixel convention). ttf outlines are +y UP, so negate y.
    struct Builder<'a> {
        pb: &'a mut tiny_skia::PathBuilder,
        pen_x: f32,
        baseline_y: f32,
        scale: f32,
    }
    impl rustybuzz::ttf_parser::OutlineBuilder for Builder<'_> {
        fn move_to(&mut self, x: f32, y: f32) {
            self.pb.move_to(
                self.pen_x + x * self.scale,
                self.baseline_y - y * self.scale,
            );
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.pb.line_to(
                self.pen_x + x * self.scale,
                self.baseline_y - y * self.scale,
            );
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.pb.quad_to(
                self.pen_x + x1 * self.scale,
                self.baseline_y - y1 * self.scale,
                self.pen_x + x * self.scale,
                self.baseline_y - y * self.scale,
            );
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.pb.cubic_to(
                self.pen_x + x1 * self.scale,
                self.baseline_y - y1 * self.scale,
                self.pen_x + x2 * self.scale,
                self.baseline_y - y2 * self.scale,
                self.pen_x + x * self.scale,
                self.baseline_y - y * self.scale,
            );
        }
        fn close(&mut self) {
            self.pb.close();
        }
    }

    // Provisional baseline at y=0; we measure bounds then re-anchor.
    let mut pb = tiny_skia::PathBuilder::new();
    let mut pen_x = 0.0f32;
    for g in glyphs {
        let gid = rustybuzz::ttf_parser::GlyphId(g.glyph_id);
        let mut b = Builder {
            pb: &mut pb,
            pen_x: pen_x + g.x_offset * pt_to_px,
            baseline_y: -g.y_offset * pt_to_px,
            scale: px_per_unit,
        };
        let _ = face.outline_glyph(gid, &mut b);
        pen_x += g.x_advance * pt_to_px;
    }
    let path = pb.finish()?;
    let bounds = path.bounds();

    // Margin so the outline anti-aliasing isn't clipped at the buffer edge.
    let margin = 2.0f32;
    let min_x = bounds.left() - margin;
    let min_y = bounds.top() - margin;
    let buf_w = (bounds.right() - bounds.left() + 2.0 * margin)
        .ceil()
        .max(1.0) as u32;
    let buf_h = (bounds.bottom() - bounds.top() + 2.0 * margin)
        .ceil()
        .max(1.0) as u32;

    let mut pixmap = tiny_skia::Pixmap::new(buf_w, buf_h)?;
    // Translate so the path's min corner lands at (margin, margin).
    let transform = tiny_skia::Transform::from_translate(-min_x, -min_y);
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::WHITE);
    paint.anti_alias = true;
    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None);

    // Convert to a grayscale alpha mask.
    let mut mask = image::GrayImage::new(buf_w, buf_h);
    for (i, px) in pixmap.pixels().iter().enumerate() {
        let a = px.alpha();
        let x = (i as u32) % buf_w;
        let y = (i as u32) / buf_w;
        mask.put_pixel(x, y, image::Luma([a]));
    }

    // The text origin (x=0, baseline y=0) maps to (-min_x, -min_y) in the mask.
    Some(GlyphRaster {
        mask,
        origin_x_px: -min_x,
        baseline_y_px: -min_y,
    })
}

/// Device pixels per point, for callers converting blur overflow / positions.
pub(crate) fn px_per_pt_at_filter_dpi(filter_dpi: f32) -> f32 {
    filter_dpi_scale(filter_dpi) / PT_PER_PX
}

/// Rasterize a solid-fill box (background colour + border) into a transparent,
/// padded RGBA buffer, gaussian-blur it, and return the embeddable asset plus
/// the overflow it adds outside the border box.
///
/// `width_pt`/`height_pt` are the border-box size in points. `blur_radius_pt`
/// is `ComputedStyle::blur_radius` (already in points). Returns `None` when the
/// element paints nothing (so the caller falls back to its normal path).
pub(crate) fn blur_box(
    width_pt: f32,
    height_pt: f32,
    background: Option<(f32, f32, f32, f32)>,
    border: &LayoutBorder,
    blur_radius_pt: f32,
    filter_dpi: f32,
) -> Option<BlurredRaster> {
    if blur_radius_pt <= 0.0 || width_pt <= 0.0 || height_pt <= 0.0 {
        return None;
    }
    let has_bg = background.is_some_and(|(_, _, _, a)| a > 0.0);
    if !has_bg && !border.has_visible() {
        return None;
    }

    use resvg::tiny_skia;

    // Buffer geometry: box at device scale plus transparent padding for the
    // gaussian to feather into.
    let s = filter_dpi_scale(filter_dpi);
    let sigma = (blur_radius_pt / PT_PER_PX) * s;
    let pad = pad_pixels(sigma);
    let box_w = (width_pt / PT_PER_PX * s).round().max(1.0) as u32;
    let box_h = (height_pt / PT_PER_PX * s).round().max(1.0) as u32;
    let buf_w = box_w + 2 * pad;
    let buf_h = box_h + 2 * pad;

    let mut pixmap = tiny_skia::Pixmap::new(buf_w, buf_h)?;
    let ox = pad as f32;
    let oy = pad as f32;

    // Background fill covers the whole border box.
    if let Some((r, g, b, a)) = background
        && a > 0.0
    {
        let mut paint = tiny_skia::Paint::default();
        paint.set_color(color8(r, g, b, a));
        paint.anti_alias = true;
        let rect = tiny_skia::Rect::from_xywh(ox, oy, box_w as f32, box_h as f32)?;
        pixmap.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
    }

    // Borders paint INSIDE the border box (the declared size is the border box).
    // Fill each visible side as a rectangle so a uniform solid frame matches the
    // vector painter; the gaussian then softens both fill and frame edge.
    paint_border_rects(&mut pixmap, border, ox, oy, box_w as f32, box_h as f32, s);

    let rgba = pixmap_to_rgba(&pixmap, buf_w, buf_h);
    let rgba = blur_premultiplied(&rgba, sigma);

    let overflow_pt = pad as f32 / s * PT_PER_PX;
    let asset = rgba_to_png_alpha_asset(rgba)?;
    Some(BlurredRaster { asset, overflow_pt })
}

/// Paint each visible border side as an inset rectangle, in device pixels.
fn paint_border_rects(
    pixmap: &mut resvg::tiny_skia::Pixmap,
    border: &LayoutBorder,
    ox: f32,
    oy: f32,
    box_w: f32,
    box_h: f32,
    scale: f32,
) {
    use resvg::tiny_skia;
    let s = scale / PT_PER_PX; // points -> device px
    let sides = [
        // (x, y, w, h, side)
        (
            0.0,
            0.0,
            box_w,
            (border.top.width * s).min(box_h),
            &border.top,
        ),
        (
            0.0,
            box_h - (border.bottom.width * s).min(box_h),
            box_w,
            (border.bottom.width * s).min(box_h),
            &border.bottom,
        ),
        (
            0.0,
            0.0,
            (border.left.width * s).min(box_w),
            box_h,
            &border.left,
        ),
        (
            box_w - (border.right.width * s).min(box_w),
            0.0,
            (border.right.width * s).min(box_w),
            box_h,
            &border.right,
        ),
    ];
    for (x, y, w, h, side) in sides {
        if !side.paints() || w <= 0.0 || h <= 0.0 {
            continue;
        }
        let mut paint = tiny_skia::Paint::default();
        let (r, g, b) = side.color;
        paint.set_color(color8(r, g, b, side.alpha));
        paint.anti_alias = true;
        if let Some(rect) = tiny_skia::Rect::from_xywh(ox + x, oy + y, w, h) {
            pixmap.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
        }
    }
}

/// Gaussian-blur an already-decoded source image with the CSS-correct sigma and
/// transparent padding so the blur feathers *outside* the element's content box
/// (css-filter-effects-1 §4.1: `blur(<length>)` → gaussian `stdDeviation =
/// length`).
///
/// Chrome composites the element at display resolution and blurs *there*, so we
/// upscale the source to the rendered content size at the device scale (nearest
/// neighbour, matching `image-rendering: pixelated`) and apply the gaussian with
/// `sigma = radius_css_px * DEVICE_SCALE` in that buffer. This reproduces the
/// full feather magnitude regardless of how small the source bitmap is.
/// Returns the blurred RGBA buffer (not yet encoded) plus the per-side overflow
/// in points, so callers can apply later filter-list functions (e.g.
/// `brightness` in `blur(...) brightness(...)`) to the blurred pixels —
/// including the feathered edge — before encoding, matching the CSS filter
/// pipeline order (css-filter-effects-1 §2: functions apply in order).
pub(crate) fn blur_image_buffer(
    source: &image::RgbaImage,
    display_w_pt: f32,
    display_h_pt: f32,
    blur_radius_pt: f32,
    filter_dpi: f32,
) -> Option<(image::RgbaImage, f32)> {
    let (sw, sh) = (source.width(), source.height());
    if sw == 0 || sh == 0 || blur_radius_pt <= 0.0 || display_w_pt <= 0.0 || display_h_pt <= 0.0 {
        return None;
    }
    // Render the image at device resolution (display CSS px × DEVICE_SCALE).
    let s = filter_dpi_scale(filter_dpi);
    let dev_w = (display_w_pt / PT_PER_PX * s).round().max(1.0) as u32;
    let dev_h = (display_h_pt / PT_PER_PX * s).round().max(1.0) as u32;
    let upscaled =
        image::imageops::resize(source, dev_w, dev_h, image::imageops::FilterType::Nearest);

    let sigma = (blur_radius_pt / PT_PER_PX) * s;
    let pad = pad_pixels(sigma);
    let mut padded = image::RgbaImage::new(dev_w + 2 * pad, dev_h + 2 * pad);
    image::imageops::replace(&mut padded, &upscaled, pad as i64, pad as i64);
    let blurred = blur_premultiplied(&padded, sigma);

    let overflow_pt = pad as f32 / s * PT_PER_PX;
    Some((blurred, overflow_pt))
}

/// Encode an already-built blurred RGBA buffer + overflow into a `BlurredRaster`.
pub(crate) fn raster_from_buffer(buf: image::RgbaImage, overflow_pt: f32) -> Option<BlurredRaster> {
    let asset = rgba_to_png_alpha_asset(buf)?;
    Some(BlurredRaster { asset, overflow_pt })
}

/// Build a `drop-shadow(dx dy blur color)` raster from an already-decoded source
/// image: take the source alpha, blur it, tint it with the shadow colour, and
/// composite the *original* image on top, offset within a padded buffer.
///
/// `display_w_pt`/`display_h_pt` are the rendered image-content size in points.
/// `dx_pt`/`dy_pt` are the shadow offsets (points; +y is downward). Returns the
/// blurred raster plus the overflow it adds beyond each border-box edge.
pub(crate) fn drop_shadow_image(
    source: &image::RgbaImage,
    display_w_pt: f32,
    display_h_pt: f32,
    dx_pt: f32,
    dy_pt: f32,
    blur_radius_pt: f32,
    color: (f32, f32, f32, f32),
) -> Option<BlurredRaster> {
    if display_w_pt <= 0.0 || display_h_pt <= 0.0 {
        return None;
    }
    // Work at the source resolution; map offsets/sigma from points into source
    // pixels using the display scale (source px per displayed point).
    let (sw, sh) = (source.width(), source.height());
    if sw == 0 || sh == 0 {
        return None;
    }
    let px_per_pt = sw as f32 / display_w_pt;
    let sigma = (blur_radius_pt / PT_PER_PX) * (sw as f32 / (display_w_pt / PT_PER_PX));
    let dx = dx_pt * px_per_pt;
    let dy = dy_pt * (sh as f32 / display_h_pt);

    // Padding must cover the blur feather AND the shadow offset so nothing clips.
    let pad = pad_pixels(sigma)
        .max(dx.abs().ceil() as u32)
        .max(dy.abs().ceil() as u32)
        + 1;
    let buf_w = sw + 2 * pad;
    let buf_h = sh + 2 * pad;

    // Shadow layer: source alpha, tinted, offset, then blurred.
    let mut shadow = image::RgbaImage::new(buf_w, buf_h);
    let (sr, sg, sb, sa) = color;
    let (cr, cg, cb) = (
        (sr * 255.0).round() as u8,
        (sg * 255.0).round() as u8,
        (sb * 255.0).round() as u8,
    );
    for y in 0..sh {
        for x in 0..sw {
            let a = source.get_pixel(x, y)[3];
            if a == 0 {
                continue;
            }
            let tx = x as i32 + pad as i32 + dx.round() as i32;
            let ty = y as i32 + pad as i32 + dy.round() as i32;
            if tx < 0 || ty < 0 || tx >= buf_w as i32 || ty >= buf_h as i32 {
                continue;
            }
            let out_a = (a as f32 * sa).round() as u8;
            shadow.put_pixel(tx as u32, ty as u32, image::Rgba([cr, cg, cb, out_a]));
        }
    }
    let mut composed = if sigma > 0.0 {
        blur_premultiplied(&shadow, sigma)
    } else {
        shadow
    };

    // Composite the original image over the shadow (source-over).
    for y in 0..sh {
        for x in 0..sw {
            let src = *source.get_pixel(x, y);
            if src[3] == 0 {
                continue;
            }
            let dx0 = x + pad;
            let dy0 = y + pad;
            let bg = *composed.get_pixel(dx0, dy0);
            composed.put_pixel(dx0, dy0, over(src, bg));
        }
    }

    let overflow_pt = pad as f32 / px_per_pt;
    let asset = rgba_to_png_alpha_asset(composed)?;
    Some(BlurredRaster { asset, overflow_pt })
}

/// Source-over composite of `src` onto `bg`, both straight-alpha RGBA8.
fn over(src: image::Rgba<u8>, bg: image::Rgba<u8>) -> image::Rgba<u8> {
    let sa = src[3] as f32 / 255.0;
    let ba = bg[3] as f32 / 255.0;
    let oa = sa + ba * (1.0 - sa);
    if oa <= 0.0 {
        return image::Rgba([0, 0, 0, 0]);
    }
    let blend = |s: u8, b: u8| {
        let s = s as f32;
        let b = b as f32;
        ((s * sa + b * ba * (1.0 - sa)) / oa)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    image::Rgba([
        blend(src[0], bg[0]),
        blend(src[1], bg[1]),
        blend(src[2], bg[2]),
        (oa * 255.0).round() as u8,
    ])
}

/// Convert an `f32` 0..1 RGBA to a tiny-skia non-premultiplied `Color`.
fn color8(r: f32, g: f32, b: f32, a: f32) -> resvg::tiny_skia::Color {
    resvg::tiny_skia::Color::from_rgba(
        r.clamp(0.0, 1.0),
        g.clamp(0.0, 1.0),
        b.clamp(0.0, 1.0),
        a.clamp(0.0, 1.0),
    )
    .unwrap_or(resvg::tiny_skia::Color::TRANSPARENT)
}

/// Convert a tiny-skia premultiplied pixmap into a straight-alpha RGBA image.
fn pixmap_to_rgba(pixmap: &resvg::tiny_skia::Pixmap, w: u32, h: u32) -> image::RgbaImage {
    let mut out = image::RgbaImage::new(w, h);
    for (i, px) in pixmap.pixels().iter().enumerate() {
        // tiny-skia stores premultiplied; demultiply to straight alpha.
        let c = px.demultiply();
        let x = (i as u32) % w;
        let y = (i as u32) / w;
        out.put_pixel(x, y, image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]));
    }
    out
}
