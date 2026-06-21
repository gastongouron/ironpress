//! Region segmentation (spec §1.8): a hand-rolled 4-connectivity flood fill over
//! all real-diff pixels (every class except `Match`/`AaEdge`), each labelled with
//! its dominant class, magnitude, and a per-region translation diagnosis.
//!
//! The shift diagnosis (`is_translation`/`shift_css`) runs AFTER the verdict
//! pixels are counted, so it can describe a displacement but can never reduce the
//! score.

use image::RgbaImage;

use super::super::config::{
    CSS_PX, EDGE_JITTER_PX, REGION_MIN_AREA_PX, RESIDUAL_JITTER_PX, SHIFT_SEARCH_PX,
};
use super::classify::{ClassMap, PixelClass};
use super::color::{ciede2000, srgb_to_lab};
use super::masks::StructuralMasks;

/// One connected blob of real-diff pixels with its diagnosis. Several fields
/// (`bbox_css`, `fill_ratio`, `modal_drgb`) are populated now but consumed by the
/// C4 diagnosis and C5 region-table/overlay — kept on the struct so the pipeline
/// is complete and the goldens lock the full shape.
#[allow(dead_code)]
pub(crate) struct DiffRegion {
    /// Bounding box in CSS px [x0, y0, x1, y1], relative to the union crop origin.
    pub(crate) bbox_css: [f64; 4],
    pub(crate) dominant: PixelClass,
    pub(crate) area_px: u32,
    pub(crate) area_pct: f64,
    /// `area_px / bbox_area` — discriminates a thin line from a solid blob.
    pub(crate) fill_ratio: f64,
    /// Median (cand − ref) over the region's `ColorErr` pixels (0..255 signed).
    pub(crate) modal_drgb: [i16; 3],
    /// Median CIEDE2000 over the region's INTERIOR (non-edge-band) `ColorErr`
    /// pixels — the solid-recolour signal. A robust statistic (median, not mean)
    /// so a low-ΔE AA fringe cannot dilute a hard recolour core below the FAIL
    /// bound (review #17), and INTERIOR-only so a geometry-boundary / curved-AA
    /// ring (correct colours, moved boundary) does not inflate it (review §1-B).
    pub(crate) delta_e: f64,
    /// Count of INTERIOR (non-edge-band) `ColorErr` pixels in this region — the
    /// area a solid recolour actually occupies (edge-band ColorErr excluded).
    pub(crate) interior_color_px: u32,
    /// Dominant translation (CSS px) for `GeomShift` regions.
    pub(crate) shift_css: (f64, f64),
    /// Whether a single consistent shift > `RESIDUAL_JITTER_PX` explains the region.
    pub(crate) is_translation: bool,
}

#[inline]
fn is_real_diff(c: PixelClass) -> bool {
    !matches!(c, PixelClass::Match | PixelClass::AaEdge)
}

/// Flood-fill the real-diff pixels into 4-connected regions, drop specks, and
/// diagnose each survivor.
pub(crate) fn segment(
    cm: &ClassMap,
    cand: &RgbaImage,
    reference: &RgbaImage,
    masks: &StructuralMasks,
) -> Vec<DiffRegion> {
    let w = cm.w as usize;
    let h = cm.h as usize;
    let total = w * h;
    if total == 0 {
        return Vec::new();
    }
    let total_px = total as f64;

    let mut visited = vec![false; total];
    let mut regions: Vec<DiffRegion> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for start in 0..total {
        if visited[start] || !is_real_diff(cm.px[start]) {
            continue;
        }
        // BFS/DFS flood fill collecting the connected blob's pixel indices.
        stack.clear();
        stack.push(start);
        visited[start] = true;
        let mut members: Vec<usize> = Vec::new();
        while let Some(i) = stack.pop() {
            members.push(i);
            let x = i % w;
            let y = i / w;
            let push = |nx: usize, ny: usize, stack: &mut Vec<usize>, visited: &mut [bool]| {
                let ni = ny * w + nx;
                if !visited[ni] && is_real_diff(cm.px[ni]) {
                    visited[ni] = true;
                    stack.push(ni);
                }
            };
            if x > 0 {
                push(x - 1, y, &mut stack, &mut visited);
            }
            if x + 1 < w {
                push(x + 1, y, &mut stack, &mut visited);
            }
            if y > 0 {
                push(x, y - 1, &mut stack, &mut visited);
            }
            if y + 1 < h {
                push(x, y + 1, &mut stack, &mut visited);
            }
        }

        if (members.len() as u32) < REGION_MIN_AREA_PX {
            continue;
        }

        regions.push(diagnose_region(
            &members, cm, cand, reference, masks, w, total_px,
        ));
    }

    // Worst-first by area for stable, useful ordering downstream.
    regions.sort_by(|a, b| {
        b.area_pct
            .partial_cmp(&a.area_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    regions
}

fn diagnose_region(
    members: &[usize],
    cm: &ClassMap,
    cand: &RgbaImage,
    reference: &RgbaImage,
    masks: &StructuralMasks,
    w: usize,
    total_px: f64,
) -> DiffRegion {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    let mut counts = [0u32; 6];
    // Per-channel deltas over ALL the region's ColorErr px (modal hue, for the
    // diagnosis headline) + per-pixel ΔE over INTERIOR (non-edge-band) ColorErr px
    // only (the solid-recolour signal that gates hard_color, robust via median).
    let mut dr: Vec<i16> = Vec::new();
    let mut dg: Vec<i16> = Vec::new();
    let mut db: Vec<i16> = Vec::new();
    let mut interior_de: Vec<f64> = Vec::new();
    // Shift estimate: average (ref-position - matching-cand-position) is implicit;
    // we instead accumulate the per-pixel displacement that best matched, plus the
    // sum of squares so we can measure AGREEMENT (a real translation has all pixels
    // shifting the SAME way -> low variance; a curved AA ring / scattered edge has
    // divergent per-pixel matches -> high variance, and must NOT read as a shift).
    let mut shift_dx_sum = 0.0;
    let mut shift_dy_sum = 0.0;
    let mut shift_dx_sq = 0.0;
    let mut shift_dy_sq = 0.0;
    let mut shift_n = 0u32;

    for &i in members {
        let x = i % w;
        let y = i / w;
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
        let cls = cm.px[i];
        counts[class_index(cls)] += 1;

        let c = cand.get_pixel(x as u32, y as u32).0;
        let r = reference.get_pixel(x as u32, y as u32).0;

        if cls == PixelClass::ColorErr {
            dr.push(c[0] as i16 - r[0] as i16);
            dg.push(c[1] as i16 - r[1] as i16);
            db.push(c[2] as i16 - r[2] as i16);
            // INTERIOR ColorErr only: a structural-boundary ColorErr (a moved/
            // resized fill abutting a different background — colours CORRECT, the
            // boundary moved — or a curved/coloured AA ring) lies in the edge band
            // and is NOT a recolour, so it is excluded from the hard-colour ΔE.
            if !masks.in_edge_band(x as u32, y as u32) {
                interior_de.push(ciede2000(
                    srgb_to_lab([r[0], r[1], r[2]]),
                    srgb_to_lab([c[0], c[1], c[2]]),
                ));
            }
        }
        if cls == PixelClass::GeomShift {
            if let Some((sx, sy)) = best_local_shift(cand, reference, x as u32, y as u32) {
                shift_dx_sum += sx as f64;
                shift_dy_sum += sy as f64;
                shift_dx_sq += (sx * sx) as f64;
                shift_dy_sq += (sy * sy) as f64;
                shift_n += 1;
            }
        }
    }

    let dominant = dominant_class(&counts);
    let area_px = members.len() as u32;
    let bbox_w = (x1 - x0 + 1) as f64;
    let bbox_h = (y1 - y0 + 1) as f64;
    let fill_ratio = area_px as f64 / (bbox_w * bbox_h);

    let modal_drgb = [median(&mut dr), median(&mut dg), median(&mut db)];
    let interior_color_px = interior_de.len() as u32;
    // MEDIAN ΔE over interior ColorErr (review #17): a low-ΔE fringe cannot drag a
    // hard recolour core under COLOR_DE_FAIL. Zero interior ColorErr => 0 (a region
    // whose only ColorErr is on the structural boundary is NOT a solid recolour).
    let delta_e = median_f64(&mut interior_de);

    // Phase-correlation lite: the average matched local displacement. A single
    // dominant shift > residual jitter marks a translation. For pure 1px boundary
    // noise the average displacement stays at/below the residual band.
    let (shift_x, shift_y) = if shift_n > 0 {
        (shift_dx_sum / shift_n as f64, shift_dy_sum / shift_n as f64)
    } else {
        (0.0, 0.0)
    };
    let shift_mag = (shift_x * shift_x + shift_y * shift_y).sqrt();
    // AGREEMENT gate (review #7 follow-on): only a CONSISTENT displacement is a
    // translation. The per-pixel shift standard deviation must be small — a real
    // residual translation moves every GeomShift pixel the SAME way (σ≈0), whereas a
    // curved AA ring (border-radius-circle) or a scattered glyph edge yields
    // divergent per-pixel best-matches (large σ) that the wider SHIFT_SEARCH_PX now
    // surfaces. Without this, the revived gate would false-FAIL curved/AA perimeters.
    let shift_std = if shift_n > 0 {
        let n = shift_n as f64;
        let var_x = (shift_dx_sq / n - shift_x * shift_x).max(0.0);
        let var_y = (shift_dy_sq / n - shift_y * shift_y).max(0.0);
        (var_x + var_y).sqrt()
    } else {
        0.0
    };
    // σ bound = the GeomShift admission radius (EDGE_JITTER_PX): per-pixel jitter up
    // to the classification radius is expected even for a true translation; beyond it
    // the matches are incoherent (a curve/scatter), not a single displacement.
    let consistent = shift_std <= EDGE_JITTER_PX as f64;
    let is_translation = shift_mag > RESIDUAL_JITTER_PX as f64 && consistent;
    let shift_css = (shift_x / CSS_PX, shift_y / CSS_PX);

    DiffRegion {
        bbox_css: [
            x0 as f64 / CSS_PX,
            y0 as f64 / CSS_PX,
            x1 as f64 / CSS_PX,
            y1 as f64 / CSS_PX,
        ],
        dominant,
        area_px,
        area_pct: 100.0 * area_px as f64 / total_px,
        fill_ratio,
        modal_drgb,
        delta_e,
        interior_color_px,
        shift_css,
        is_translation,
    }
}

/// The integer displacement (dx,dy) within ±`SHIFT_SEARCH_PX` that makes the
/// reference at `(x,y)` reappear in the candidate (i.e. how far the candidate
/// boundary moved).
///
/// The search radius is `SHIFT_SEARCH_PX` (device px), DECOUPLED from
/// `RESIDUAL_JITTER_PX` (review #2/#3/#6): the old radius `RESIDUAL_JITTER_PX = 1`
/// capped `shift_max_css` at `1/CSS_PX ≈ 0.32` per axis, BELOW the `G_SHIFT_CSS`
/// PASS bound (1.0), so the shift gate could never escalate — it was structurally
/// dead. `SHIFT_SEARCH_PX` is wide enough (≥ device-px equivalent of the 1.0/4.0
/// CSS-px bounds) that a real residual translation is measurable and the FAIL bound
/// is reachable. NOTE: this measures a residual translation AFTER the fixed -4,-4
/// page-origin calibration; it does NOT move any candidate pixel before scoring (no
/// best-shift masking) — the pixels were already classified; this only diagnoses how
/// far an already-counted GeomShift boundary moved.
///
/// Ties (equal colour delta) break toward the SMALLEST `|dx|+|dy|` (review #7): the
/// old strict-min kept the first-seen top-left offset, fabricating a (-1,-1)
/// directional shift on a symmetric AA ramp. Preferring the zero/centre offset means
/// a symmetric ramp yields ~(0,0), not a phantom diagonal shift.
fn best_local_shift(cand: &RgbaImage, reference: &RgbaImage, x: u32, y: u32) -> Option<(i32, i32)> {
    let (w, h) = cand.dimensions();
    let r = reference.get_pixel(x, y);
    // (colour delta, |dx|+|dy|, dx, dy) — lexicographic: lowest delta, then nearest.
    let mut best: Option<(f64, i32, i32, i32)> = None;
    let radius = SHIFT_SEARCH_PX;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || ny < 0 || nx as u32 >= w || ny as u32 >= h {
                continue;
            }
            let d = super::color_delta(r, cand.get_pixel(nx as u32, ny as u32), false).abs();
            let manhattan = dx.abs() + dy.abs();
            let cand_key = (d, manhattan, dx, dy);
            match best {
                // Replace only on a strictly-lower delta, or an equal delta with a
                // strictly-smaller |dx|+|dy| (tie-break toward the centre offset).
                Some((bd, bm, _, _)) if d > bd || (d == bd && manhattan >= bm) => {}
                _ => best = Some(cand_key),
            }
        }
    }
    best.map(|(_, _, dx, dy)| (dx, dy))
}

#[inline]
fn class_index(c: PixelClass) -> usize {
    match c {
        PixelClass::Match => 0,
        PixelClass::AaEdge => 1,
        PixelClass::ColorErr => 2,
        PixelClass::GeomShift => 3,
        PixelClass::Missing => 4,
        PixelClass::Extra => 5,
    }
}

/// The dominant real-diff class in a region (Match/AaEdge never dominate).
fn dominant_class(counts: &[u32; 6]) -> PixelClass {
    let candidates = [
        (PixelClass::ColorErr, counts[2]),
        (PixelClass::GeomShift, counts[3]),
        (PixelClass::Missing, counts[4]),
        (PixelClass::Extra, counts[5]),
    ];
    candidates
        .iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, _)| *c)
        .unwrap_or(PixelClass::ColorErr)
}

/// Median of a signed-channel sample, clamped to i16. Empty -> 0.
fn median(v: &mut [i16]) -> i16 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

/// Median of an f64 sample (used for the robust interior-ColorErr ΔE). Empty -> 0.
fn median_f64(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

// ===========================================================================
// Unit tests for the REVIVED shift gate (review #2/#3/#6/#7). These prove the
// shift estimator can now measure a real residual translation (the old radius=1
// capped it at ~0.32 CSS px, below every G_SHIFT_CSS bound — a dead gate) and that
// its tie-break no longer fabricates a directional shift on a symmetric ramp.
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba, RgbaImage};

    fn white(w: u32, h: u32) -> RgbaImage {
        ImageBuffer::from_pixel(w, h, Rgba([255, 255, 255, 255]))
    }

    /// best_local_shift must now FIND a 12-device-px displacement (it used to be
    /// capped at ±1). A unique dark mark sits 12px apart in cand vs ref over an
    /// otherwise white field; the estimator returns ~(12,12), which is 3.84 CSS px
    /// per axis -> magnitude 5.43 > the 4.0 CSS-px FAIL bound. With the OLD radius=1
    /// this could only ever return (±1,±1) ~ 0.45 CSS px. (Sign follows the
    /// convention: ref reappears in cand at +shift.)
    #[test]
    fn best_local_shift_measures_a_12px_displacement() {
        let (w, h) = (64u32, 64u32);
        let mut reference = white(w, h);
        let mut cand = white(w, h);
        // A 3x3 dark mark: ref at (20,20), cand at (32,32) (= +12,+12).
        for dy in 0..3 {
            for dx in 0..3 {
                reference.put_pixel(20 + dx, 20 + dy, Rgba([0, 0, 0, 255]));
                cand.put_pixel(32 + dx, 32 + dy, Rgba([0, 0, 0, 255]));
            }
        }
        // Sample the mark's top-left ref pixel so the nearest cand-black is the full
        // +12,+12 away (an interior ref pixel could match a nearer cand-black edge).
        let (sx, sy) = best_local_shift(&cand, &reference, 20, 20).expect("a shift must be found");
        assert_eq!(
            (sx, sy),
            (12, 12),
            "the estimator must measure the full 12px displacement"
        );
        let css = (sx as f64 / CSS_PX, sy as f64 / CSS_PX);
        let mag = (css.0 * css.0 + css.1 * css.1).sqrt();
        assert!(
            mag > 4.0,
            "12px displacement = {mag:.2} CSS px must exceed the 4.0 FAIL bound"
        );
    }

    /// Tie-break: on a perfectly SYMMETRIC field (every neighbour identical to the
    /// centre), all offsets tie on colour delta, so the estimator must return the
    /// CENTRE (0,0) — NOT the old strict-min (-radius,-radius) that fabricated a
    /// diagonal shift (review #7).
    #[test]
    fn best_local_shift_ties_resolve_to_centre() {
        let (w, h) = (40u32, 40u32);
        let grey = Rgba([128, 128, 128, 255]);
        let reference = ImageBuffer::from_pixel(w, h, grey);
        let cand = ImageBuffer::from_pixel(w, h, grey);
        let (sx, sy) = best_local_shift(&cand, &reference, 20, 20).expect("a shift must be found");
        assert_eq!(
            (sx, sy),
            (0, 0),
            "a symmetric field must yield the centre offset, not (-1,-1)"
        );
    }

    /// End-to-end: a gentle grey gradient translated by 12px produces GeomShift
    /// pixels (each ref tone reappears APPROXIMATELY within EDGE_JITTER_PX, so the
    /// pixel is admitted GeomShift, but its EXACT match lies 12px away). The revived
    /// estimator measures the 12px displacement, so `shift_max_css` clears the FAIL
    /// bound — the gate is ALIVE end-to-end (it was structurally unreachable before).
    #[test]
    fn shift_gate_fires_on_a_translated_gradient() {
        use super::super::super::geom::content_mask;
        use super::super::classify::classify_pixels;
        use super::super::masks::structural_masks;
        use super::super::tally::aggregate;

        let (w, h) = (160u32, 120u32);
        // A 2D grey ramp (varies in BOTH axes so a +12,+12 translation is recoverable
        // on both, not just x). Slope ≈1.2 raw/px/axis is the narrow window where a
        // SAME-position delta EXCEEDS t_match (so the pixel is not a Match) yet a
        // ±2px neighbour stays WITHIN it (so the pixel is ADMITTED as GeomShift):
        // same = 24·s ≈ 28.8 raw (YIQ ≈ 419 > t_match≈352); within-2 = 20·s ≈ 24 raw
        // (YIQ ≈ 291 < t_match). best_local_shift then finds the EXACT match 12px away.
        let val = |x: i32, y: i32| -> u8 {
            (60.0 + 1.2 * x as f64 + 1.2 * y as f64)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        let mut reference = white(w, h);
        let mut cand = white(w, h);
        for y in 10..h - 10 {
            for x in 10..w - 10 {
                let r = val(x as i32, y as i32);
                let c = val(x as i32 - 12, y as i32 - 12); // candidate translated +12,+12
                reference.put_pixel(x, y, Rgba([r, r, r, 255]));
                cand.put_pixel(x, y, Rgba([c, c, c, 255]));
            }
        }
        let mask_c = content_mask(&cand);
        let mask_r = content_mask(&reference);
        let masks = structural_masks(&cand, &reference);
        let cm = classify_pixels(&cand, &reference, &mask_c, &mask_r, &masks);
        let regions = segment(&cm, &cand, &reference, &masks);
        let tally = aggregate(&cm, &regions, [0; 4], &mask_c, &mask_r, &cand, &reference);
        assert!(
            tally.shift_max_css > 4.0,
            "a 12px-translated gradient must drive shift_max_css past the FAIL bound, got {:.3}",
            tally.shift_max_css
        );
    }
}
