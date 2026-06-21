//! Raster geometry: content detection, bounding boxes, union/crop, content masks,
//! and image translation.
//!
//! Extracted verbatim from the former monolithic `mod.rs` (C1 mechanical split).
//! The clamped best-shift registration search (`best_registration_offset`) was
//! removed in C6 — the V2 path uses a single fixed page-origin calibration and
//! never searches per-fixture (that masked real layout bugs).

use image::{ImageBuffer, Rgba, RgbaImage};

use super::config::WHITE_TOL;

pub(crate) fn is_content(px: &Rgba<u8>) -> bool {
    let [r, g, b, _] = px.0;
    let dr = (r as i32 - 255).abs();
    let dg = (g as i32 - 255).abs();
    let db = (b as i32 - 255).abs();
    dr.max(dg).max(db) > WHITE_TOL
}

/// Inclusive content bounding box `(min_x, min_y, max_x, max_y)` in the image's
/// own (== shared page) pixel coordinates, or `None` if the image is entirely
/// white (no content pixels). Coordinates are NOT re-anchored, so a box at the
/// same page position in two images yields the same numbers -> positional
/// offsets survive into the union/diff.
pub(crate) type BBox = (u32, u32, u32, u32);

pub(crate) fn content_bbox(img: &RgbaImage) -> Option<BBox> {
    let (w, h) = img.dimensions();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            if is_content(img.get_pixel(x, y)) {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if found {
        Some((min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

/// Translate `img` by `(dx, dy)` pixels on a white background (same dimensions),
/// so calibrated content lands at the reference's page position before cropping.
/// Out-of-frame source pixels become white. Used by the V2 calibration step
/// (`calibrate::calibrate` applies the fixed `-GLOBAL_OFFSET` shift), so only a
/// few px is lost per edge.
pub(crate) fn shift_image(img: &RgbaImage, dx: i32, dy: i32) -> RgbaImage {
    if dx == 0 && dy == 0 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let mut out: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([255, 255, 255, 255]));
    for y in 0..h {
        for x in 0..w {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx >= 0 && ny >= 0 && (nx as u32) < w && (ny as u32) < h {
                out.put_pixel(nx as u32, ny as u32, *img.get_pixel(x, y));
            }
        }
    }
    out
}

/// Union of two inclusive bboxes (min of mins, max of maxes).
pub(crate) fn union_bbox(a: BBox, b: BBox) -> BBox {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

// ---------------------------------------------------------------------------
// Content mask (V2; spec §1.4)
// ---------------------------------------------------------------------------

/// A 1-bit-per-pixel content mask in row-major order: bit set iff the pixel is
/// ink (`is_content`). Packed into `u64` words so the per-pixel classifier and
/// the structural-edge dilation can test membership in O(1) without re-running
/// `is_content`. Used only by the V2 comparator path.
pub(crate) struct Mask {
    pub(crate) w: u32,
    pub(crate) h: u32,
    bits: Vec<u64>,
}

impl Mask {
    #[inline]
    fn idx(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.w as usize) + (x as usize)
    }
    /// Whether the pixel at `(x, y)` is ink. Out-of-bounds reads as `false`.
    #[inline]
    pub(crate) fn get(&self, x: u32, y: u32) -> bool {
        if x >= self.w || y >= self.h {
            return false;
        }
        let i = self.idx(x, y);
        (self.bits[i >> 6] >> (i & 63)) & 1 == 1
    }
    #[inline]
    fn set(&mut self, x: u32, y: u32) {
        let i = self.idx(x, y);
        self.bits[i >> 6] |= 1u64 << (i & 63);
    }
}

/// Build the content mask of `img`: one set bit per ink pixel (`is_content`).
pub(crate) fn content_mask(img: &RgbaImage) -> Mask {
    let (w, h) = img.dimensions();
    let words = ((w as usize * h as usize) + 63) / 64;
    let mut m = Mask {
        w,
        h,
        bits: vec![0u64; words.max(1)],
    };
    for y in 0..h {
        for x in 0..w {
            if is_content(img.get_pixel(x, y)) {
                m.set(x, y);
            }
        }
    }
    m
}

/// Crop `img` to the inclusive rectangle `bb` in `img`'s OWN coordinate space,
/// padding with white where the rectangle extends past the image bounds. Both
/// ref and candidate are cropped to the SAME rectangle, so output dims match and
/// every pixel compares like-for-like at the same page position.
pub(crate) fn crop_rect(img: &RgbaImage, bb: BBox) -> RgbaImage {
    let (min_x, min_y, max_x, max_y) = bb;
    let w = max_x - min_x + 1;
    let h = max_y - min_y + 1;
    let mut out: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([255, 255, 255, 255]));
    for oy in 0..h {
        for ox in 0..w {
            let sx = min_x + ox;
            let sy = min_y + oy;
            if sx < img.width() && sy < img.height() {
                out.put_pixel(ox, oy, *img.get_pixel(sx, sy));
            }
        }
    }
    out
}
