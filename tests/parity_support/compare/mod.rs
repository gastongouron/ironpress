//! The comparator (V2 — the only verdict path).
//!
//! A diagnostic multi-detector pipeline split into single-responsibility
//! submodules (spec §1.2): `color` (ΔE2000 + YIQ), `masks` (structural edge
//! bands), `classify` (per-pixel `PixelClass`), `segment` (connected diff
//! regions), `tally` (per-class aggregation), and `verdict` (multi-gate
//! PASS/PARTIAL/FAIL). `compare_v2` orchestrates them. The legacy best-shift /
//! close-match comparator was removed in C6 (its false-passing tolerances were
//! the whole point of the overhaul); the YIQ primitives `color_delta`/`rgb2y/i/q`
//! survive because the V2 `classify`/`segment` detectors still read them.

use image::{ImageBuffer, Rgba, RgbaImage};

use super::manifest::ManifestEntry;
use super::report::Status;

// V2 submodules (spec §4). Each is a plain owned-value stage of the pipeline.
pub(crate) mod classify;
pub(crate) mod color;
pub(crate) mod masks;
pub(crate) mod segment;
pub(crate) mod tally;
pub(crate) mod verdict;

#[cfg(test)]
mod goldens;

use classify::classify_pixels;
use segment::segment;
use tally::aggregate;
use verdict::verdict;

pub(crate) use classify::{ClassMap, PixelClass};
pub(crate) use segment::DiffRegion;
pub(crate) use tally::ClassTally;
pub(crate) use verdict::Verdict;

// ---------------------------------------------------------------------------
// YIQ colour primitives (pixelmatch port). KEPT post-C6 because the V2
// `classify`/`segment` detectors read `color_delta` (which depends on
// `rgb2y/i/q`); the legacy `diff_images` that also used them is gone.
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn rgb2y(p: &Rgba<u8>) -> f64 {
    p[0] as f64 * 0.298_895_31 + p[1] as f64 * 0.586_622_47 + p[2] as f64 * 0.114_482_23
}
#[inline]
pub(crate) fn rgb2i(p: &Rgba<u8>) -> f64 {
    p[0] as f64 * 0.595_977_99 - p[1] as f64 * 0.274_176_10 - p[2] as f64 * 0.321_801_89
}
#[inline]
pub(crate) fn rgb2q(p: &Rgba<u8>) -> f64 {
    p[0] as f64 * 0.211_470_17 - p[1] as f64 * 0.522_617_11 + p[2] as f64 * 0.311_146_94
}

/// Signed YIQ perceptual delta between two pixels (pixelmatch `colorDelta`).
/// `y_only` returns just the brightness delta (used for AA edge detection).
/// Sign encodes which pixel is brighter; callers use the magnitude for the
/// threshold and the sign for anti-aliasing classification.
pub(crate) fn color_delta(a: &Rgba<u8>, b: &Rgba<u8>, y_only: bool) -> f64 {
    if a == b {
        return 0.0;
    }
    let y1 = rgb2y(a);
    let y2 = rgb2y(b);
    if y_only {
        return y1 - y2;
    }
    let dy = y1 - y2;
    let di = rgb2i(a) - rgb2i(b);
    let dq = rgb2q(a) - rgb2q(b);
    let delta = 0.5053 * dy * dy + 0.299 * di * di + 0.1957 * dq * dq;
    if y1 > y2 { -delta } else { delta }
}

// ===========================================================================
// V2 ORCHESTRATION (spec §1.2)
// ===========================================================================

use super::geom::{content_bbox, content_mask, crop_rect, union_bbox};

/// Everything the V2 path produces for one fixture. `status`/`diff_pct` come from
/// the multi-gate `verdict`; `tally`/`regions`/`verdict` carry the diagnostic
/// detail (consumed later by `diagnose`/`overlay`/`report`); `overlay` is the
/// classed diff image written to disk. Owned values only — no borrows escape.
pub(crate) struct V2Outcome {
    pub(crate) status: Status,
    pub(crate) diff_pct: f64,
    pub(crate) tally: ClassTally,
    /// Per-region diagnosis. Consumed by `diagnose` (C4) and the HTML region
    /// table (C5); carried here now so the pipeline is complete end-to-end.
    #[allow(dead_code)]
    pub(crate) regions: Vec<DiffRegion>,
    pub(crate) verdict: Verdict,
    pub(crate) overlay: RgbaImage,
    /// The "why it failed" diagnosis (spec §2): computed here because this is the
    /// only place that holds the class map + aligned cand/ref the colour/alpha
    /// sub-classifiers need. ADDITIVE — it never feeds back into the verdict.
    pub(crate) diagnosis: super::diagnose::Diagnosis,
}

/// A loud UNKNOWN outcome for an unscoreable pair (e.g. a dimension mismatch). The
/// `note` is carried on the diagnosis headline so the report names the reason; every
/// magnitude is zero (we deliberately do NOT fabricate a score).
fn unknown_outcome(note: String) -> V2Outcome {
    let tally = ClassTally {
        color_pct: 0.0,
        missing_pct: 0.0,
        extra_pct: 0.0,
        edge_max_css: 0.0,
        edge_delta_css: [0.0; 4],
        shift_max_css: 0.0,
        aa_pct: 0.0,
        color_de: 0.0,
        interior_color_pct: 0.0,
        interior_color_de: 0.0,
        modal_drgb: [0, 0, 0],
        total_px: 0,
    };
    let verdict = Verdict {
        status: Status::Unknown,
        diff_pct: 0.0,
        dominant_class: PixelClass::Match,
    };
    let diagnosis = super::diagnose::Diagnosis {
        headline: note,
        ..Default::default()
    };
    V2Outcome {
        status: Status::Unknown,
        diff_pct: 0.0,
        tally,
        regions: Vec::new(),
        verdict,
        overlay: ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255])),
        diagnosis,
    }
}

/// Run the §1.2 V2 pipeline over a candidate and reference in shared page space.
///
/// CONTRACT: `cand` is ALREADY CALIBRATED — the caller (`process_entry`, and the
/// golden tests) applies `calibrate::calibrate` (the fixed `-GLOBAL_OFFSET`
/// shift) before calling this, so `compare_v2` sees content at the reference's
/// page origin and any surviving translation is a real residual. Both images are
/// assumed alpha-flattened over white (the rasterizer emits opaque RGBA, and the
/// golden builders use opaque fills, so this holds in practice).
///
/// Steps: content masks -> per-side bbox delta (the SIZE signal, read from
/// extents not the diluted pixel fraction) -> union-crop both for the pixel
/// compare -> structural edge bands -> per-pixel classify -> region segmentation
/// -> aggregate to per-class severities -> multi-gate verdict -> classed overlay.
pub(crate) fn compare_v2(
    cand: &RgbaImage,
    reference: &RgbaImage,
    entry: &ManifestEntry,
) -> V2Outcome {
    // Dimension reconciliation. Every fixture now sizes `@page` to its content
    // with margin:0, so content is anchored at the page ORIGIN (top-left) IDENTICALLY
    // in both engines. Chrome's `--print-to-pdf`, however, rounds the `@page` CSS
    // size UP to a slightly larger pt page (~0.5pt), so its raster can be a few px
    // taller/wider than ironpress's — the surplus is purely bottom/right WHITE page
    // margin, never content. Normalize both frames to the common MIN dimensions
    // (anchored at (0,0)) so the like-for-like pipeline (bbox deltas, union crop,
    // per-pixel classify) sees equal dims while only the white page-rounding band is
    // trimmed — no content is discarded (it lives at the origin-anchored top-left).
    //
    // A LARGE mismatch is NOT page rounding — it means ironpress computed a different
    // @page size (a real layout/parse bug) — so keep the loud UNKNOWN beyond a small
    // tolerance rather than silently absorbing a genuine page-size defect.
    const DIM_ROUND_TOL: u32 = 8; // device px; Chrome @page pt-rounding is <= ~3px
    let (cw, ch) = cand.dimensions();
    let (rw, rh) = reference.dimensions();
    if cw.abs_diff(rw) > DIM_ROUND_TOL || ch.abs_diff(rh) > DIM_ROUND_TOL {
        return unknown_outcome(format!(
            "dimension mismatch: cand {:?} != ref {:?} beyond page-rounding tolerance \
             ({DIM_ROUND_TOL}px) — ironpress @page size differs from Chrome; refusing to score",
            cand.dimensions(),
            reference.dimensions()
        ));
    }
    let cand_norm;
    let ref_norm;
    let (cand, reference): (&RgbaImage, &RgbaImage) = if (cw, ch) != (rw, rh) {
        let w = cw.min(rw);
        let h = ch.min(rh);
        cand_norm = crop_rect(cand, (0, 0, w - 1, h - 1));
        ref_norm = crop_rect(reference, (0, 0, w - 1, h - 1));
        (&cand_norm, &ref_norm)
    } else {
        (cand, reference)
    };

    let cand_bb = content_bbox(cand);
    let ref_bb = content_bbox(reference);

    // Per-side content-extent delta (device px): ref - cand, [L, R, T, B]. This is
    // the box-size verdict signal and is taken from the bbox corners, NOT from the
    // union-crop pixel fraction (so a 13px-too-tall box reads ~13px regardless of
    // body size). When a side is blank we leave its delta at 0 and let the
    // Missing/Extra coverage gates carry the verdict.
    let bbox_delta: [i32; 4] = match (cand_bb, ref_bb) {
        (Some(c), Some(r)) => [
            r.0 as i32 - c.0 as i32, // left
            r.2 as i32 - c.2 as i32, // right
            r.1 as i32 - c.1 as i32, // top
            r.3 as i32 - c.3 as i32, // bottom
        ],
        _ => [0, 0, 0, 0],
    };

    // Union bbox for the pixel compare. If one side is blank, crop to the other's
    // box so the missing/extra region is fully covered; if both blank, a 1x1 crop.
    let union = match (cand_bb, ref_bb) {
        (Some(c), Some(r)) => union_bbox(c, r),
        (Some(b), None) | (None, Some(b)) => b,
        (None, None) => (0, 0, 0, 0),
    };

    let cand_u = crop_rect(cand, union);
    let ref_u = crop_rect(reference, union);
    let mask_c = content_mask(&cand_u);
    let mask_r = content_mask(&ref_u);

    let masks = masks::structural_masks(&cand_u, &ref_u);
    let class_map = classify_pixels(&cand_u, &ref_u, &mask_c, &mask_r, &masks);
    let regions = segment(&class_map, &cand_u, &ref_u, &masks);
    let tally = aggregate(
        &class_map, &regions, bbox_delta, &mask_c, &mask_r, &cand_u, &ref_u,
    );
    let mut verdict = verdict(&tally, &regions, entry);
    // Exact derived back-compat scalar (% real-diff px, AA+Match excluded).
    let diff_pct = tally.diff_pct(&class_map);
    verdict.diff_pct = diff_pct;
    // The classed-diff overlay (spec §3.3 item 2): per-pixel class fill + region
    // bbox frames. Lives in the top-level `overlay` module (the C5 presentation
    // layer); see `crate::overlay::render_classed_overlay`.
    let overlay = super::overlay::render_classed_overlay(&class_map, &regions, &cand_u, &ref_u);

    // Diagnosis (spec §2). Computed here (the only stage with the class map +
    // aligned cand/ref) but PURELY additive: it reads the same owned products the
    // verdict already produced and can never change `status`/`diff_pct`.
    let diagnosis = super::diagnose::diagnose(&tally, &regions, &class_map, &cand_u, &ref_u);

    V2Outcome {
        status: verdict.status,
        diff_pct,
        tally,
        regions,
        verdict,
        overlay,
        diagnosis,
    }
}
