//! Per-pixel classification (spec §1.7) — assigns every union-cropped pixel a
//! `PixelClass` in a fixed precedence order, first match wins.
//!
//! This is where AA forgiveness is bounded: a differing pixel becomes `AaEdge`
//! ONLY inside the shared edge band (and within the wider `t_aa()` budget). Off
//! the shared band there is no AA mercy, so a wrong glyph/weight/recolour cannot
//! launder itself as anti-aliasing. A ≤1px boundary displacement becomes
//! `GeomShift` (counted, never zeroed); a genuine recolour becomes `ColorErr`.

use image::RgbaImage;

use super::super::config::{AA_RAMP_RADIUS_PX, COLOR_DE_PASS, EDGE_JITTER_PX, t_aa, t_match};
use super::super::geom::Mask;
use super::color::{ciede2000, srgb_to_lab};
use super::color_delta;
use super::masks::StructuralMasks;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PixelClass {
    /// Below the match budget — effectively identical.
    Match,
    /// Differing but on a shared structural edge within the AA budget — genuine
    /// cross-rasterizer glyph/edge anti-aliasing, never a defect.
    AaEdge,
    /// Both ink, aligned, colour differs beyond match with no 1px match — recolour.
    ColorErr,
    /// Both ink, differing, but the reference colour reappears within 1px in both
    /// directions — a boundary displaced ≤1px (counted, not zeroed).
    GeomShift,
    /// Reference paints, candidate is paper-white.
    Missing,
    /// Candidate paints, reference is paper-white.
    Extra,
}

/// A per-pixel class grid over the union-cropped frame (row-major).
pub(crate) struct ClassMap {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) px: Vec<PixelClass>,
}

/// Whether `target` colour reappears within `radius` of `(x,y)` in `img`, within
/// the match budget. Used for the ±1px `GeomShift` test (a real recolour or a
/// >1px shift has no such local match and is NOT forgiven).
fn color_present_near(
    img: &RgbaImage,
    target: &image::Rgba<u8>,
    x: u32,
    y: u32,
    radius: i32,
    budget: f64,
) -> bool {
    let (w, h) = img.dimensions();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || ny < 0 || nx as u32 >= w || ny as u32 >= h {
                continue;
            }
            if color_delta(target, img.get_pixel(nx as u32, ny as u32), false).abs() <= budget {
                return true;
            }
        }
    }
    false
}

/// Whether the SAME-COLOUR ink as `target` reappears within `radius` of `(x,y)` in
/// `img` (only pixels that are ink per `mask` count). "Same colour" = ΔE2000 within
/// the JND (`COLOR_DE_PASS`), with a cheap YIQ pre-filter so the ΔE is computed only
/// for near-colour neighbours. Used to forgive a displaced glyph/border edge: a
/// `Missing`/`Extra` fringe pixel whose ink merely moved a px or two (cross-rasterizer
/// jitter) — NOT a recolour (ΔE-gated) and NOT a consistent shift/size change (those
/// are caught by the bbox-extent gate, independent of this test).
fn ink_color_present_near(
    img: &RgbaImage,
    mask: &Mask,
    target: &image::Rgba<u8>,
    x: u32,
    y: u32,
    radius: i32,
) -> bool {
    let (w, h) = img.dimensions();
    let aa = t_aa();
    let tlab = srgb_to_lab([target[0], target[1], target[2]]);
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || ny < 0 || nx as u32 >= w || ny as u32 >= h {
                continue;
            }
            let (nx, ny) = (nx as u32, ny as u32);
            if !mask.get(nx, ny) {
                continue; // neighbour must itself be ink
            }
            let p = img.get_pixel(nx, ny);
            // Cheap YIQ pre-filter: skip the ΔE for clearly-different colours.
            if color_delta(target, p, false).abs() > aa {
                continue;
            }
            if ciede2000(tlab, srgb_to_lab([p[0], p[1], p[2]])) <= COLOR_DE_PASS {
                return true;
            }
        }
    }
    false
}

/// Classify every pixel in the aligned cand/ref frame (spec §1.7 precedence).
pub(crate) fn classify_pixels(
    cand: &RgbaImage,
    reference: &RgbaImage,
    mask_c: &Mask,
    mask_r: &Mask,
    masks: &StructuralMasks,
) -> ClassMap {
    let (w, h) = cand.dimensions();
    let mut px = Vec::with_capacity((w as usize) * (h as usize));
    let tm = t_match();
    let ta = t_aa();
    for y in 0..h {
        for x in 0..w {
            let c = cand.get_pixel(x, y);
            let r = reference.get_pixel(x, y);
            let d = color_delta(c, r, false).abs();
            let ink_c = mask_c.get(x, y);
            let ink_r = mask_r.get(x, y);

            // The YIQ `t_match` budget is COARSE: a perceptible recolour like
            // #cc0000 vs #dd0000 reads only ~46 YIQ (well under t_match ~352) yet
            // is ΔE2000 ~3.6 — above the JND. YIQ alone would launder it as Match,
            // defeating the whole point of the ΔE colour detector. So for two INK
            // pixels we additionally require the perceptual ΔE to be within the
            // JND (`COLOR_DE_PASS`) before calling it a match.
            //
            // CRITICAL one-side-ink case (`ink_c != ink_r`): a feature painted in
            // ONLY ONE render must NEVER be a Match. The coarse YIQ `d <= tm` budget
            // (~352) is far larger than the YIQ delta of a faint/pale fill against
            // paper-white, so a low-contrast feature present in only one image would
            // be laundered as Match before the Missing/Extra branches ever ran (this
            // was the root cause of the border-radius-per-corner / text-shadow-offset
            // false-passes). When exactly one side is ink we therefore force a
            // non-match and let the Missing/Extra/fringe branches below decide — the
            // ink mask already proved the pixel is content in one image and not the
            // other. (The Missing/Extra branches still forgive a same-colour displaced
            // edge via `ink_color_present_near`, so genuine glyph-AA jitter stays
            // forgiven.) Paper-white vs paper-white keeps the plain YIQ budget.
            let perceptual_match = if ink_c != ink_r {
                false
            } else if ink_c && ink_r {
                d <= tm
                    && ciede2000(
                        srgb_to_lab([r[0], r[1], r[2]]),
                        srgb_to_lab([c[0], c[1], c[2]]),
                    ) <= COLOR_DE_PASS
            } else {
                d <= tm
            };

            let class = if perceptual_match {
                // 1. Match (YIQ within budget AND, for ink pixels, ΔE within JND).
                PixelClass::Match
            } else if ink_c && ink_r && d <= tm {
                // YIQ matched but ΔE exceeded the JND on two ink pixels: a genuine
                // sub-YIQ recolour (not anti-aliasing — AA is intermediate values
                // on a contrast edge, which this is not). Score it as ColorErr.
                // GUARDED to both-ink: a one-side-ink faint pixel (which now has
                // perceptual_match=false even when its YIQ delta is sub-`tm`) must
                // NOT be sucked into ColorErr here — it has to fall through to the
                // Missing/Extra branches that the ink-mask disagreement implies.
                PixelClass::ColorErr
            } else if masks.in_shared_band(x, y) && d <= ta {
                // 2. AaEdge — only inside the shared edge band.
                PixelClass::AaEdge
            } else if ink_r && !ink_c {
                // 3. Missing — reference paints, candidate is paper-white. UNLESS
                // the same-colour ink reappears within EDGE_JITTER_PX in the
                // candidate: then it is a displaced glyph/border edge (cross-
                // rasterizer sub-px jitter), forgiven as AaEdge. A genuinely missing
                // feature has no nearby matching ink and stays Missing; a consistent
                // shift/size change is still caught by the bbox-extent gate.
                if ink_color_present_near(cand, mask_c, r, x, y, EDGE_JITTER_PX) {
                    PixelClass::AaEdge
                } else {
                    PixelClass::Missing
                }
            } else if ink_c && !ink_r {
                // 4. Extra — candidate paints, reference is paper-white. Same edge-
                // jitter forgiveness as Missing, mirrored (ref ink nearby => AaEdge).
                if ink_color_present_near(reference, mask_r, c, x, y, EDGE_JITTER_PX) {
                    PixelClass::AaEdge
                } else {
                    PixelClass::Extra
                }
            } else if ink_c
                && ink_r
                && color_present_near(cand, r, x, y, AA_RAMP_RADIUS_PX, tm)
                && color_present_near(reference, c, x, y, AA_RAMP_RADIUS_PX, tm)
            {
                // 5. GeomShift — an offset edge/AA-ramp: both ink, differing, but
                // each side's tone reappears within `AA_RAMP_RADIUS_PX` in the other
                // image (mutual match). This forgives cross-rasterizer/sub-pixel
                // glyph-edge AA whose intermediate tones land a few px apart (the #1
                // text false-FAIL — displaced glyph ramps on multi-line blocks reach
                // ~7px), WITHOUT laundering a solid recolour: a uniform recolour has
                // NO matching tone nearby (the bidirectional SAME-COLOUR match fails
                // in solid regions), so it falls through to ColorErr below; and a
                // consistent shift/size change is still caught by the bbox-extent
                // gate (`G_EDGE_CSS`). Wider here than the Missing/Extra branches
                // (which keep `EDGE_JITTER_PX`) precisely because this branch's
                // bidirectional same-colour test cannot absorb absent content.
                PixelClass::GeomShift
            } else {
                // 6. ColorErr — aligned recolour / wrong-value / colour-space.
                PixelClass::ColorErr
            };
            px.push(class);
        }
    }
    ClassMap { w, h, px }
}
