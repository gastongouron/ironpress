//! Multi-gate verdict (spec §1.10): PASS/PARTIAL/FAIL decided by per-class
//! severities, not a single `diff_pct`.
//!
//! PASS requires EVERY gate within its PASS bound. FAIL if ANY gate exceeds its
//! PARTIAL bound or hits a hard colour gate. Else PARTIAL. Crucially, the
//! geometry, coverage, and hard-colour gates are NOT overridable by manifest
//! thresholds — only `G_COLOR_PCT` and the derived total-area bound are — so no
//! per-fixture tuning can re-introduce a size/margin/recolour/missing false-pass
//! (amendment A6).

use super::super::config::{
    COLOR_DE_FAIL, COLOR_DE_PASS, G_COLOR_PCT, G_EDGE_CSS, G_EXTRA_PCT, G_MISSING_PCT, G_SHIFT_CSS,
};
use super::super::manifest::ManifestEntry;
use super::super::report::Status;
use super::classify::PixelClass;
use super::segment::DiffRegion;
use super::tally::ClassTally;

pub(crate) struct Verdict {
    pub(crate) status: Status,
    pub(crate) diff_pct: f64,
    pub(crate) dominant_class: PixelClass,
}

/// Apply the §1.10 gates. The returned `diff_pct` is a placeholder approximated
/// from the class percentages; `compare_v2` overwrites it with the exact
/// class-map-derived scalar (which needs the full class map, not just the tally).
pub(crate) fn verdict(t: &ClassTally, regions: &[DiffRegion], entry: &ManifestEntry) -> Verdict {
    // Manifest may RELAX only G_COLOR_PCT (and the derived total bound). Geometry,
    // coverage, and hard-colour gates are fixed.
    let color_pass = entry
        .pass_threshold_pct
        .map(|v| v.max(G_COLOR_PCT.0))
        .unwrap_or(G_COLOR_PCT.0);
    let color_partial = entry
        .partial_threshold_pct
        .map(|v| v.max(G_COLOR_PCT.1))
        .unwrap_or(G_COLOR_PCT.1);

    // A VISUALLY-VERIFIED cross-rasterizer floor (conic / repeating-gradient angular
    // banding, mask-edge band). It raises ONLY the PASS bounds — colour/missing/extra
    // up to `floor`, interior ΔE up to the (fixed) hard-colour bound — so a sub-
    // perceptual residual reads PASS instead of PARTIAL. `floor()` is clamped below
    // the coverage FAIL bound, and the FAIL gates below are untouched, so a real
    // large-area recolour/missing/extra still FAILs.
    let floor = entry.floor();
    let color_pass = color_pass.max(floor);
    let miss_pass = G_MISSING_PCT.0.max(floor);
    let extra_pass = G_EXTRA_PCT.0.max(floor);
    let de_pass = if floor > 0.0 {
        COLOR_DE_FAIL
    } else {
        COLOR_DE_PASS
    };

    let dominant_class = elect_dominant(regions);

    // --- FAIL gates -------------------------------------------------------
    // Hard colour: a SOLID INTERIOR recolour. We gate on the INTERIOR-ColorErr
    // signal (ColorErr px NOT in the structural edge band), area-relative to union
    // CONTENT (review #4 — `r.area_pct` was whole-FRAME, a unit mismatch with
    // G_COLOR_PCT's content-relative meaning), with the robust median interior ΔE
    // (review #17). This fires on a genuine fill/border recolour (solid interior
    // pixels) but NOT on:
    //   * scattered glyph-edge ColorErr from cross-rasterizer text AA (in the edge
    //     band -> not interior),
    //   * a shifted/resized element's fill abutting a different background (colours
    //     CORRECT, the boundary moved -> all ColorErr in the edge band, review §1-B),
    //   * a curved/coloured AA ring (border-radius-circle: interior byte-identical,
    //     only the perimeter AA differs -> interior_color_pct ~0).
    let hard_color = t.interior_color_de >= COLOR_DE_FAIL && t.interior_color_pct >= G_COLOR_PCT.0;

    let any_fail = hard_color
        || t.color_pct > color_partial
        || t.missing_pct > G_MISSING_PCT.1
        || t.extra_pct > G_EXTRA_PCT.1
        || t.edge_max_css > G_EDGE_CSS.1
        || t.shift_max_css > G_SHIFT_CSS.1;

    let status = if any_fail {
        Status::Fail
    } else if t.color_pct <= color_pass
        && t.interior_color_de <= de_pass
        && t.missing_pct <= miss_pass
        && t.extra_pct <= extra_pass
        && t.edge_max_css <= G_EDGE_CSS.0
        && t.shift_max_css <= G_SHIFT_CSS.0
    {
        // Colour PASS reads the INTERIOR ΔE, not the boundary-inclusive `color_de`:
        // a structural-boundary / curved-AA ColorErr (correct colours, moved
        // boundary — e.g. border-radius-circle, interior byte-identical) has a high
        // boundary `color_de` but zero interior ΔE, so it must not be denied PASS by
        // a phantom recolour. A genuine small interior recolour still has
        // interior_color_de > JND and is correctly held back to PARTIAL.
        Status::Pass
    } else {
        Status::Partial
    };

    // `diff_pct` placeholder (overwritten by compare_v2 with the exact value).
    let diff_pct = (t.color_pct + t.missing_pct + t.extra_pct).min(100.0);

    Verdict {
        status,
        diff_pct,
        dominant_class,
    }
}

/// The dominant class among real-diff regions: highest `area_pct`; ties broken by
/// severity (Missing > Extra > ColorErr > GeomShift).
fn elect_dominant(regions: &[DiffRegion]) -> PixelClass {
    let mut best: Option<&DiffRegion> = None;
    for r in regions {
        best = match best {
            None => Some(r),
            Some(b) => {
                if r.area_pct > b.area_pct + 1e-9
                    || ((r.area_pct - b.area_pct).abs() <= 1e-9
                        && severity(r.dominant) > severity(b.dominant))
                {
                    Some(r)
                } else {
                    Some(b)
                }
            }
        };
    }
    best.map(|r| r.dominant).unwrap_or(PixelClass::Match)
}

#[inline]
fn severity(c: PixelClass) -> u8 {
    match c {
        PixelClass::Missing => 5,
        PixelClass::Extra => 4,
        PixelClass::ColorErr => 3,
        PixelClass::GeomShift => 2,
        PixelClass::AaEdge => 1,
        PixelClass::Match => 0,
    }
}
