//! Classed-diff overlay rendering (spec §3.3 item 2 + §3.3 item 2 edge overlay).
//!
//! `render_classed_overlay` recolours every union-cropped pixel by its
//! `PixelClass` so the committed `.diff.png` shows WHAT differed and HOW, not a
//! flat red mask:
//!   Missing = magenta, Extra = green, ColorErr = blue, GeomShift = orange,
//!   AaEdge = faint-yellow, Match = faint-grey.
//! It additionally outlines each surviving `DiffRegion`'s bbox in its dominant-
//! class colour (the "edge/region overlay" of §3.3 item 2), so the single image
//! both shows the per-pixel classes AND frames the connected blobs the region
//! table lists. The legend in the HTML report (`report::render_legend`) maps the
//! exact same colours back to classes.
//!
//! The colour table is the single source of truth: `class_rgb` is consumed both
//! here (pixel fill + region frame) and by the HTML legend swatches, so the
//! overlay and its legend can never drift apart. No external deps.

use image::{ImageBuffer, Rgba, RgbaImage};

use super::compare::{ClassMap, DiffRegion, PixelClass};
use super::config::CSS_PX;

/// The overlay colour for a `PixelClass` (spec §3.3 item 2). The SINGLE source of
/// truth for both the rendered overlay and the HTML legend, so they stay in sync.
pub(crate) fn class_rgb(c: PixelClass) -> [u8; 3] {
    match c {
        // Faint grey: the matched substrate, kept visible so the shape reads.
        PixelClass::Match => [245, 245, 245],
        // Faint yellow: shared-edge AA — the measurement ceiling, never a bug.
        PixelClass::AaEdge => [255, 240, 150],
        // Blue: aligned recolour / wrong colour value (incl. colour-space drift).
        PixelClass::ColorErr => [40, 80, 255],
        // Orange: a boundary displaced <=1px (counted, not zeroed).
        PixelClass::GeomShift => [255, 150, 30],
        // Magenta: reference paints, candidate is paper-white.
        PixelClass::Missing => [230, 0, 230],
        // Green: candidate paints where the reference is blank.
        PixelClass::Extra => [0, 200, 60],
    }
}

/// Human label for a `PixelClass`, for the legend (spec §3.3 item 3).
pub(crate) fn class_label(c: PixelClass) -> &'static str {
    match c {
        PixelClass::Match => "Match",
        PixelClass::AaEdge => "AA edge",
        PixelClass::ColorErr => "ColorErr",
        PixelClass::GeomShift => "GeomShift",
        PixelClass::Missing => "Missing",
        PixelClass::Extra => "Extra",
    }
}

/// The legend rows, in the precedence order the classifier assigns them. Shared by
/// the overlay (it is exhaustive over `PixelClass`) and the HTML legend so both
/// describe the same palette.
pub(crate) const LEGEND_ORDER: [PixelClass; 6] = [
    PixelClass::Missing,
    PixelClass::Extra,
    PixelClass::ColorErr,
    PixelClass::GeomShift,
    PixelClass::AaEdge,
    PixelClass::Match,
];

/// Render the classed diff overlay (spec §3.3 item 2). Every pixel is filled by
/// its `PixelClass` colour; every surviving region's bounding box is then framed
/// in its dominant-class colour so the connected blobs the region table lists are
/// visible at a glance. `cand`/`reference` are accepted for signature parity with
/// the spec (a future heat overlay can sample them); the per-pixel classes already
/// encode the cand-vs-ref relationship, so they are not read here.
pub(crate) fn render_classed_overlay(
    cm: &ClassMap,
    regions: &[DiffRegion],
    _cand: &RgbaImage,
    _reference: &RgbaImage,
) -> RgbaImage {
    let w = cm.w.max(1);
    let h = cm.h.max(1);
    let mut out: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([255, 255, 255, 255]));

    // 1. Per-pixel class fill.
    for y in 0..cm.h {
        for x in 0..cm.w {
            let c = cm.px[(y as usize) * (cm.w as usize) + x as usize];
            let [r, g, b] = class_rgb(c);
            out.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    // 2. Region bbox frames in the dominant-class colour (the edge/region overlay).
    // bbox_css is CSS px relative to the union-crop origin -> back to device px.
    for region in regions {
        let [r, g, b] = class_rgb(region.dominant);
        let frame = Rgba([r, g, b, 255]);
        let x0 = (region.bbox_css[0] * CSS_PX)
            .round()
            .clamp(0.0, (w - 1) as f64) as u32;
        let y0 = (region.bbox_css[1] * CSS_PX)
            .round()
            .clamp(0.0, (h - 1) as f64) as u32;
        let x1 = (region.bbox_css[2] * CSS_PX)
            .round()
            .clamp(0.0, (w - 1) as f64) as u32;
        let y1 = (region.bbox_css[3] * CSS_PX)
            .round()
            .clamp(0.0, (h - 1) as f64) as u32;
        for x in x0..=x1 {
            out.put_pixel(x, y0, frame);
            out.put_pixel(x, y1, frame);
        }
        for y in y0..=y1 {
            out.put_pixel(x0, y, frame);
            out.put_pixel(x1, y, frame);
        }
    }

    out
}
