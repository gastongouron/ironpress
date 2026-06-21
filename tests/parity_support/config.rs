//! All tunable constants for the parity engine.
//!
//! Extracted verbatim from the former monolithic `mod.rs` (constants block and
//! the comparator threshold pair). No values changed — this is a mechanical
//! split (C1).

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Rasterization DPI for both candidate and reference. High DPI so fine detail
/// (thin borders, small glyphs, gradient bands) is captured faithfully and any
/// anti-aliased edge is a smaller fraction of a region.
pub(crate) const DPI: u32 = 300;
/// Per-channel tolerance for the bbox white-detection (0..=255).
pub(crate) const WHITE_TOL: i32 = 10;
/// Overall-score regression epsilon (percentage points). Below this is noise.
pub(crate) const SCORE_EPSILON: f64 = 0.5;
/// Maximum possible YIQ color delta (pixelmatch constant). Read by the V2
/// `t_match()`/`t_aa()` budgets below (and, via `color_delta`, the V2 detectors).
pub(crate) const PM_MAX_DELTA: f64 = 35215.0;

// ===========================================================================
// V2 COMPARATOR CONSTANTS (spec §1.1)
//
// These drive the multi-gate V2 verdict path — the only verdict path after C6.
// The legacy best-shift/close-match comparator and its constants (MAX_REG,
// CHANNEL_TOL, the DEFAULT_/NOISE_FLOOR_ pass/partial floors, the legacy
// PM_THRESHOLD 0.12) were removed in C6; the V2 path uses its OWN tighter
// `t_match()` (PM_THRESHOLD_V2 0.10) and its own multi-gate verdict.
// ===========================================================================

/// Device px per CSS px @ 300 DPI (96 CSS px/in -> 300/96 = 3.125).
pub(crate) const CSS_PX: f64 = 3.125;
/// Fixed page-origin correction (device px): ironpress content sits +4,+4 vs the
/// Chrome reference because Chrome's `--print-to-pdf` rounds the printable margin.
/// We shift the candidate by `-GLOBAL_OFFSET` once, uniformly, and audit it — we
/// do NOT search per-fixture (that masked real layout bugs). See spec §0.1/§1.3.
pub(crate) const GLOBAL_OFFSET: (i32, i32) = (4, 4);
/// Allowed raw-probe deviation from `GLOBAL_OFFSET` during calibration audit.
pub(crate) const PROBE_JITTER_PX: i32 = 1;
/// Post-calibration sub-pixel rounding band: a residual displacement within this
/// radius is classed `GeomShift` (counted, never zeroed), not `ColorErr`. Also the
/// residual band that `DiffRegion::is_translation` uses (a measured per-region
/// shift magnitude must EXCEED this to count as a translation).
pub(crate) const RESIDUAL_JITTER_PX: i32 = 1;
/// Search radius (device px) for `best_local_shift` — DECOUPLED from
/// `RESIDUAL_JITTER_PX` (review #2/#3). The old radius (1) capped a measured shift
/// at `1/CSS_PX ≈ 0.32` CSS px per axis, below the `G_SHIFT_CSS` PASS bound (1.0),
/// so the shift gate was structurally dead. 16 device px ≈ 5.1 CSS px exceeds the
/// FAIL bound (4.0), so a real residual translation is measurable and the gate can
/// escalate. This does NOT reintroduce best-shift masking: it only diagnoses how far
/// an already-classified GeomShift boundary moved, AFTER scoring (no candidate pixel
/// is moved before the per-pixel classify).
pub(crate) const SHIFT_SEARCH_PX: i32 = 16;
/// Cross-rasterizer edge-jitter radius (device px). A `Missing`/`Extra` pixel whose
/// SAME-COLOUR ink reappears within this radius in the other image is a displaced
/// glyph/border edge (two rasterizers place the same stroke a px or two apart), not
/// real missing/extra content — it is forgiven as `AaEdge`. This is ΔE-gated (a
/// recoloured displaced edge is NOT forgiven) and CANNOT mask a consistent shift or
/// size change: those are caught independently by the bbox-extent gate
/// (`G_EDGE_CSS`, from `edge_delta_css`), which does not depend on this forgiveness.
/// ~2 device px = ~0.64 CSS px, below the 1.0 CSS-px edge PASS bound. Kept
/// conservative: widening to 3 only flipped one fixture (monospace FAIL->PARTIAL,
/// likely a real font-mapping difference) and did NOT reduce the residual ColorErr
/// on correctly-rendered text (that residual is genuine minor glyph-weight/border
/// difference, not forgivable AA — so it is honestly reported as PARTIAL, not masked).
/// (Distinct from the rejected global best-shift: this is a LOCAL, colour-gated,
/// bidirectional same-ink test that cannot mask a whole-element shift — that is the
/// bbox-extent gate's job.)
pub(crate) const EDGE_JITTER_PX: i32 = 2;

/// V2 per-pixel match threshold (pixelmatch `threshold`, 0..1), tighter than the
/// removed legacy 0.12. Used by the V2 path's `t_match()`.
pub(crate) const PM_THRESHOLD_V2: f64 = 0.10;
/// V2 "match" YIQ delta budget (~352). At/below this, a pixel is `Match`.
pub(crate) fn t_match() -> f64 {
    PM_MAX_DELTA * PM_THRESHOLD_V2 * PM_THRESHOLD_V2
}
/// Wider AA tolerance (0..1) — legal ONLY inside the shared edge band.
pub(crate) const AA_THRESHOLD: f64 = 0.18;
/// V2 anti-aliasing YIQ delta budget (~1141). A differing pixel inside the shared
/// edge band and within this budget is `AaEdge` (cross-rasterizer glyph AA).
pub(crate) fn t_aa() -> f64 {
    PM_MAX_DELTA * AA_THRESHOLD * AA_THRESHOLD
}

/// Per-channel 4-neighbour gradient threshold (0..255) for structural edges. A
/// pixel is an edge iff the max per-channel |Δ| to any 4-neighbour exceeds this.
pub(crate) const EDGE_GRAD: i32 = 24;

/// Drop diff regions smaller than this (ignore <3x3 device-px specks).
pub(crate) const REGION_MIN_AREA_PX: u32 = 9;

// --- verdict gates: (PASS bound, PARTIAL bound). FAIL if > PARTIAL bound. ---
/// % of union content pixels classed `ColorErr`.
pub(crate) const G_COLOR_PCT: (f64, f64) = (0.5, 8.0);
/// % of REF content area classed `Missing`.
pub(crate) const G_MISSING_PCT: (f64, f64) = (0.5, 6.0);
/// % of CAND content area classed `Extra`.
pub(crate) const G_EXTRA_PCT: (f64, f64) = (0.5, 6.0);
/// Max per-side content-extent delta, CSS px (the box-size signal).
pub(crate) const G_EDGE_CSS: (f64, f64) = (1.0, 3.0);
/// Residual translation beyond calibration, CSS px.
pub(crate) const G_SHIFT_CSS: (f64, f64) = (1.0, 4.0);
/// ΔE2000: at/below this a colour difference is not a defect even if pixels differ.
pub(crate) const COLOR_DE_PASS: f64 = 2.5;
/// ΔE2000: at/above this is a hard colour failure regardless of area.
pub(crate) const COLOR_DE_FAIL: f64 = 6.0;

// ===========================================================================
// PDF-GEOMETRY VERIFIER CONSTANTS (spec §2.3 / Phase 2a)
// ===========================================================================

/// Page height in pt (LETTER 612x792). The PDF geometry tokenizer normalizes
/// bottom-left-origin PDF y to a top-left-origin y via `y_tl = PAGE_H_PT - y`.
pub(crate) const PAGE_H_PT: f64 = 792.0;
/// Per-coordinate vector-geometry tolerance (pt) for the PDF geometry verifier.
/// 0.30 pt ≈ 0.40 CSS px ≈ 1.25 device px @300dpi — sub-0.5pt, so a real 1px shift
/// (0.72pt) fails, but f32 rounding in ironpress's `format_pdf_number` (≤ ~1e-3 pt)
/// and the calibration residual never trip it. PASS = every coord within this;
/// PARTIAL = within 2x on some coord, none beyond; FAIL = any coord beyond 2x (or a
/// missing primitive within 4x, or a gross page offset). See `verify/pdf_geom.rs`.
pub(crate) const GEOM_TOL_PT: f64 = 0.30;
