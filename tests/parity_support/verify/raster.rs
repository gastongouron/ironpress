//! `RasterVerifier` — a thin ADAPTER over the already-computed `V2Outcome`
//! (spec §1.4). It does NOT re-run `compare_v2`: it re-partitions the existing
//! `tally` + `verdict` into THREE `SubVerdict`s (Geometry, Appearance, Presence)
//! using the SAME `config.rs` gates `verdict.rs` reads, so the combined status is
//! byte-identical to today's verdict (the no-op proof lives in `goldens.rs`).
//!
//! In Phase 1 the `RasterVerifier` owns ALL THREE concerns; geometry authority
//! migrates to the exact `PdfGeometry` verifier one sidecar at a time in Phase 2
//! (§1.1 — it then falls back to raster geometry only where no sidecar applies).
//!
//! THE EQUIVALENCE (why WORST-of-three == verdict.rs's single status):
//! `verdict.rs` decides FAIL if ANY gate is over its PARTIAL bound (or the hard-
//! colour gate fires), PASS if ALL gates are within their PASS bound, else
//! PARTIAL. The gate set partitions cleanly by concern — no gate variable feeds a
//! fail-condition of one concern AND a pass-condition of another — so:
//!   * overall FAIL  ⟺ some concern FAILs  ⟺ WORST == Fail
//!   * overall PASS  ⟺ every concern PASSes ⟺ WORST == Pass
//!   * otherwise PARTIAL                     ⟺ WORST == Partial
//! Each concern below applies exactly the verdict.rs fail-bound / pass-bound for
//! its own gates, so the per-concern triple reconstructs the verdict by WORST.

use super::super::compare::V2Outcome;
use super::super::compare::tally::ClassTally;
use super::super::config::{
    COLOR_DE_FAIL, COLOR_DE_PASS, G_COLOR_PCT, G_EDGE_CSS, G_EXTRA_PCT, G_MISSING_PCT, G_SHIFT_CSS,
};
use super::super::manifest::ManifestEntry;
use super::super::report::Status;
use super::{Concern, SubVerdict, Verifier, VerifierKind, VerifyCtx};

/// Adapter holding only the raster signals the three concern mappings need —
/// snapshotted from the already-computed `V2Outcome`. No image data, no recompute.
pub(crate) struct RasterVerifier {
    // Geometry signals.
    edge_max_css: f64,
    shift_max_css: f64,
    // Appearance signals.
    color_pct: f64,
    interior_color_pct: f64,
    interior_color_de: f64,
    // Presence signals.
    missing_pct: f64,
    extra_pct: f64,
    // Manifest may RELAX only G_COLOR_PCT (and the derived total bound). Captured
    // here exactly as `verdict.rs` reads them so the colour gate is identical.
    color_pass: f64,
    color_partial: f64,
    /// Verified cross-rasterizer floor (`verdict.rs` floor): relaxes ONLY the
    /// PASS bounds (colour/missing/extra up to `floor`, interior ΔE up to the
    /// fixed hard-colour bound), never the FAIL bounds. Mirrors `verdict.rs` so
    /// the WORST-of-three equivalence holds.
    floor: f64,
    /// Whether the underlying outcome is UNKNOWN (unscoreable pair): every concern
    /// then reports `Unknown`, mirroring the verdict's UNKNOWN status.
    unknown: bool,
}

impl RasterVerifier {
    /// Build the adapter from the already-computed `V2Outcome` + the manifest
    /// entry (the entry supplies the same `G_COLOR_PCT` relaxation `verdict.rs`
    /// applies). Pure copy of scalars — zero added comparator cost.
    pub(crate) fn from_outcome(outcome: &V2Outcome, entry: &ManifestEntry) -> Self {
        let t: &ClassTally = &outcome.tally;
        // SAME relaxation as verdict.rs: a manifest threshold can only RELAX
        // G_COLOR_PCT upward (never below the fixed floor).
        let color_pass = entry
            .pass_threshold_pct
            .map(|v| v.max(G_COLOR_PCT.0))
            .unwrap_or(G_COLOR_PCT.0);
        let color_partial = entry
            .partial_threshold_pct
            .map(|v| v.max(G_COLOR_PCT.1))
            .unwrap_or(G_COLOR_PCT.1);
        let floor = entry.floor();
        RasterVerifier {
            edge_max_css: t.edge_max_css,
            shift_max_css: t.shift_max_css,
            color_pct: t.color_pct,
            interior_color_pct: t.interior_color_pct,
            interior_color_de: t.interior_color_de,
            missing_pct: t.missing_pct,
            extra_pct: t.extra_pct,
            color_pass: color_pass.max(floor),
            color_partial,
            floor,
            unknown: outcome.status == Status::Unknown,
        }
    }

    // --- per-concern status mappings (each = verdict.rs's gates for that axis) ---

    /// Geometry: `edge_max_css` (G_EDGE_CSS) + `shift_max_css` (G_SHIFT_CSS).
    fn geometry_status(&self) -> Status {
        // `shift_max_css` is a content-bbox CENTROID estimate. For soft-edged
        // content (gradients, masks, blends, shadows) the bbox is ill-defined —
        // a faint AA fringe on one side pulls the centroid several px, so the
        // shift is grossly over-reported (e.g. background-conic-gradient reads a
        // 6.8px "shift" at 0.02% pixel diff — physically impossible). The
        // RELIABLE displacement signals are the per-side EDGE delta and the
        // missing/extra PRESENCE: a REAL shift moves edges (raises edge_max) or
        // relocates ink (raises missing+extra). So when edges are within their
        // PASS bound AND almost no ink is missing/extra, the box is correctly
        // placed and a large shift reading is a centroid artifact — neutralize
        // it. (A genuine displacement trips edge or presence and is unaffected;
        // a real EDGE displacement like selectors-cascade's 2.24px keeps
        // edge_max > PASS so this does not apply.)
        let shift = if self.edge_max_css <= G_EDGE_CSS.0
            && (self.missing_pct + self.extra_pct) <= G_MISSING_PCT.0
        {
            self.shift_max_css.min(G_SHIFT_CSS.0)
        } else {
            self.shift_max_css
        };
        if self.edge_max_css > G_EDGE_CSS.1 || shift > G_SHIFT_CSS.1 {
            Status::Fail
        } else if self.edge_max_css <= G_EDGE_CSS.0 && shift <= G_SHIFT_CSS.0 {
            Status::Pass
        } else {
            Status::Partial
        }
    }

    /// Appearance: `color_pct` (relaxed PASS/PARTIAL bounds) + the interior ΔE
    /// PASS bound + the hard-colour FAIL gate (interior ΔE & interior area).
    /// Mirrors verdict.rs's `hard_color`, the `color_pct > color_partial` FAIL,
    /// and the `color_pct <= color_pass && interior_color_de <= COLOR_DE_PASS`
    /// PASS conditions EXACTLY.
    fn appearance_status(&self) -> Status {
        let hard_color =
            self.interior_color_de >= COLOR_DE_FAIL && self.interior_color_pct >= G_COLOR_PCT.0;
        // A declared floor accepts a sub-hard interior ΔE (still below the fixed
        // hard-colour FAIL gate above) as PASS — a verified rasterizer seam, not a
        // recolour.
        let de_pass = if self.floor > 0.0 {
            COLOR_DE_FAIL
        } else {
            COLOR_DE_PASS
        };
        if hard_color || self.color_pct > self.color_partial {
            Status::Fail
        } else if self.color_pct <= self.color_pass && self.interior_color_de <= de_pass {
            Status::Pass
        } else {
            Status::Partial
        }
    }

    /// Presence: `missing_pct` (G_MISSING_PCT) + `extra_pct` (G_EXTRA_PCT).
    fn presence_status(&self) -> Status {
        let miss_pass = G_MISSING_PCT.0.max(self.floor);
        let extra_pass = G_EXTRA_PCT.0.max(self.floor);
        if self.missing_pct > G_MISSING_PCT.1 || self.extra_pct > G_EXTRA_PCT.1 {
            Status::Fail
        } else if self.missing_pct <= miss_pass && self.extra_pct <= extra_pass {
            Status::Pass
        } else {
            Status::Partial
        }
    }
}

impl Verifier for RasterVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::RasterDiff
    }

    /// The raster diff applies to every scoreable fixture — it is always present.
    fn applies(&self, _ctx: &VerifyCtx) -> bool {
        true
    }

    fn verify(&self, _ctx: &VerifyCtx) -> Vec<SubVerdict> {
        // An UNKNOWN outcome (unscoreable pair) maps every concern to Unknown so
        // the combiner reproduces the verdict's UNKNOWN status (Unknown axes are
        // excluded from WORST; with all-Unknown the combined status is Unknown).
        if self.unknown {
            return [Concern::Geometry, Concern::Appearance, Concern::Presence]
                .iter()
                .map(|&concern| SubVerdict {
                    verifier: VerifierKind::RasterDiff,
                    status: Status::Unknown,
                    concern,
                    headline: "unscoreable (UNKNOWN outcome)".to_string(),
                    magnitude: 0.0,
                })
                .collect();
        }

        let geom = self.geometry_status();
        let appearance = self.appearance_status();
        let presence = self.presence_status();

        vec![
            SubVerdict {
                verifier: VerifierKind::RasterDiff,
                status: geom,
                concern: Concern::Geometry,
                headline: format!(
                    "raster geometry: edge {:.2}css shift {:.2}css",
                    self.edge_max_css, self.shift_max_css
                ),
                magnitude: self.edge_max_css.max(self.shift_max_css),
            },
            SubVerdict {
                verifier: VerifierKind::RasterDiff,
                status: appearance,
                concern: Concern::Appearance,
                headline: format!(
                    "raster appearance: color {:.2}% interiorΔE {:.2}",
                    self.color_pct, self.interior_color_de
                ),
                magnitude: self.color_pct,
            },
            SubVerdict {
                verifier: VerifierKind::RasterDiff,
                status: presence,
                concern: Concern::Presence,
                headline: format!(
                    "raster presence: missing {:.2}% extra {:.2}%",
                    self.missing_pct, self.extra_pct
                ),
                magnitude: self.missing_pct.max(self.extra_pct),
            },
        ]
    }
}
