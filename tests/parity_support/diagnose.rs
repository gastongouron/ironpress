//! Per-fixture diagnosis (spec §2): the "why it failed" layer over the V2
//! comparator. ADDITIVE — it reads the tally/regions/aligned pixels the verdict
//! already produced and never changes a verdict.
//!
//! The output is one `Diagnosis` per scored fixture: a primary `ErrorClass`, a
//! human `headline` (a pure rule table keyed on the primary class + magnitude
//! signature, §2.3), all magnitudes in CSS px / 0..255 ΔRGB / ΔE (never raw
//! device px, §2.1), and a per-region breakdown. `compute_attribution` (kept
//! verbatim below) composes with it: a CONFOUNDED fixture's headline is prefixed
//! `via {dep}: …` so the report names the real culprit, not the surface feature
//! (honors the MEMORY.md failure-mode-attribution rule).
//!
//! Sub-classifier coverage (spec §2.2):
//!   - GeometrySize vs GeometryShift, AaOnly, ColorValue — implemented FULLY
//!     (cheap, robust; from `edge_delta_css` symmetry / the AA-only signal /
//!     the region modal ΔRGB+ΔE).
//!   - ColorSpace (gamma/sRGB-linear fit) and AlphaCompositing (α solve) —
//!     BEST-EFFORT: sampled from the aligned cand/ref pixels of the dominant
//!     ColorErr region. When the fit is inconclusive we fall back to ColorValue
//!     (never block a diagnosis on the refinement). See `fit_colorspace` /
//!     `recover_alpha`.

use std::collections::BTreeMap;

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use super::compare::{ClassMap, ClassTally, DiffRegion, PixelClass};
use super::config::{G_COLOR_PCT, G_EDGE_CSS, G_EXTRA_PCT, G_MISSING_PCT, G_SHIFT_CSS};
use super::report::{FixtureResult, Status};

// ===========================================================================
// Types (spec §2.1) — all Serialize/Deserialize/Clone/Debug/Default so the
// `diagnosis` field is additive (old baselines without it still parse) and the
// goldens can lock the shape.
// ===========================================================================

/// The kind of defect a region/fixture exhibits. Serialized as its name so the
/// report reads as text (e.g. `"GeometrySize"`).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ErrorClass {
    /// Reference paints, candidate is blank (feature absent / clipped).
    Missing,
    /// Candidate paints where the reference is blank.
    Extra,
    /// Asymmetric box-extent error (one/two sides) — a size bug (box-sizing).
    GeometrySize,
    /// All four sides displaced equally — a residual translation beyond calibration.
    GeometryShift,
    /// Flat recolour / wrong colour value (within sRGB).
    ColorValue,
    /// Gradient/blend drift consistent with an sRGB-vs-linear (gamma) mismatch.
    ColorSpace,
    /// Opacity not composited (an α∈(0,1) explains ref while cand is opaque).
    AlphaCompositing,
    /// The only non-Match pixels are shared-edge AA — a measurement ceiling.
    AaOnly,
}

impl ErrorClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ErrorClass::Missing => "Missing",
            ErrorClass::Extra => "Extra",
            ErrorClass::GeometrySize => "GeometrySize",
            ErrorClass::GeometryShift => "GeometryShift",
            ErrorClass::ColorValue => "ColorValue",
            ErrorClass::ColorSpace => "ColorSpace",
            ErrorClass::AlphaCompositing => "AlphaCompositing",
            ErrorClass::AaOnly => "AaOnly",
        }
    }
}

impl Default for ErrorClass {
    fn default() -> Self {
        ErrorClass::AaOnly
    }
}

/// All defect magnitudes for one fixture, in fixture-facing units: CSS px /
/// 0..255 signed ΔRGB / ΔE2000 — never raw device px (§2.1).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub(crate) struct Magnitude {
    /// Per-side box-extent delta [L, R, T, B], CSS px.
    pub(crate) edge_delta_css: [f64; 4],
    /// % of REF content area that is Missing.
    pub(crate) missing_area_pct: f64,
    /// % of CAND content area that is Extra.
    pub(crate) extra_area_pct: f64,
    /// Modal (median) signed per-channel cand−ref over ColorErr px (0..255).
    pub(crate) modal_drgb: [i16; 3],
    /// Area-weighted mean ΔE2000 over ColorErr regions.
    pub(crate) delta_e: f64,
    /// Recovered compositing α∈(0,1) when AlphaCompositing was diagnosed.
    pub(crate) recovered_alpha: Option<f64>,
    /// Residual whole-frame translation [dx, dy], CSS px.
    pub(crate) residual_shift_css: [f64; 2],
    /// A short tag for a detected colour-space fit (e.g. `"sRGB↔linear"`), if any.
    pub(crate) colorspace: Option<String>,
}

/// One diff region's diagnosis (top-N by area, worst-first). Magnitudes mirror
/// `Magnitude` at region scope. `selector` is Tier-2 (layout sidecar) and stays
/// `None` for now (spec §3.4 Tier 1).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub(crate) struct RegionDiag {
    pub(crate) class: String,
    pub(crate) bbox_css: [f64; 4],
    pub(crate) area_pct: f64,
    pub(crate) edge_delta_css: [f64; 4],
    pub(crate) modal_drgb: [i16; 3],
    pub(crate) delta_e: f64,
    pub(crate) recovered_alpha: Option<f64>,
    pub(crate) shift_css: [f64; 2],
    pub(crate) selector: Option<String>,
    pub(crate) headline: String,
}

/// The full diagnosis for one fixture (§2.1). `primary_class`/`secondary` are
/// `ErrorClass` names; `headline` is the human reason (§2.3, possibly attribution-
/// prefixed); `confidence` is the fraction of real-diff px in the primary class.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub(crate) struct Diagnosis {
    pub(crate) primary_class: String,
    pub(crate) secondary: Vec<String>,
    pub(crate) headline: String,
    pub(crate) magnitude: Magnitude,
    pub(crate) regions: Vec<RegionDiag>,
    pub(crate) residual_shift_css: [f64; 2],
    pub(crate) confidence: f64,
}

/// Top-N regions surfaced per fixture (worst-first; the rest are summarised by
/// the aggregate magnitude only).
const MAX_REGION_DIAGS: usize = 6;

// ===========================================================================
// diagnose() — the entry point (spec §2.2 sub-classifiers + §2.3 headline)
// ===========================================================================

/// Diagnose one fixture from the V2 comparator's owned products. `cm`/`cand`/`ref`
/// are the union-cropped class map and aligned images (the only pixel access the
/// colour-space / alpha sub-classifiers need). Pure: no I/O, no mutation of inputs.
pub(crate) fn diagnose(
    tally: &ClassTally,
    regions: &[DiffRegion],
    cm: &ClassMap,
    cand: &RgbaImage,
    reference: &RgbaImage,
) -> Diagnosis {
    // Real-diff pixel census across the whole frame — the basis for AaOnly and
    // for `confidence` (fraction of real-diff px in the primary class).
    let census = Census::of(cm);

    // --- AaOnly: the cheap measurement-ceiling case ----------------------
    // The fixture differs ONLY in shared-edge anti-aliasing — a cross-rasterizer
    // ceiling, not a bug (e.g. a text baseline). Read it as AaOnly so it never
    // reports a scary class. This requires BOTH: no real-diff pixels at all, AND
    // the geometry/coverage/colour signals all within their PASS bounds (so a real
    // size/shift bug carrying zero ColorErr/Missing/Extra pixels — which is
    // impossible by construction, the bbox delta has no pixels — cannot be
    // laundered, and a genuinely clean render with stray AA reads correctly).
    let geometry_quiet = tally.edge_max_css <= G_EDGE_CSS.0 && tally.shift_max_css <= G_SHIFT_CSS.0;
    let coverage_quiet = tally.missing_pct <= G_MISSING_PCT.0 && tally.extra_pct <= G_EXTRA_PCT.0;
    let color_quiet = tally.color_pct <= G_COLOR_PCT.0;
    if census.real == 0 && geometry_quiet && coverage_quiet && color_quiet {
        return aa_only_diagnosis(tally);
    }

    // --- Per-region diagnosis (worst-first, top-N) -----------------------
    let mut region_diags: Vec<(ErrorClass, RegionDiag, u32)> = Vec::new();
    for r in regions.iter() {
        let (class, alpha) = classify_region(r, tally, cand, reference);
        let rd = region_diag(r, class, alpha, tally);
        region_diags.push((class, rd, r.area_px));
    }

    // --- Elect the primary class -----------------------------------------
    // The dominant region (largest area) drives the primary class; ties already
    // resolved by `segment`'s worst-first ordering. If there are no surviving
    // regions (sub-speck diffs only) fall back to the strongest aggregate signal.
    let (primary, primary_region) = match region_diags.first() {
        Some((c, rd, _)) => (*c, Some(rd.clone())),
        None => (elect_from_tally(tally, &census), None),
    };

    // Secondary classes: every OTHER region class that is above its PASS bound,
    // de-duplicated, primary excluded.
    let mut secondary: Vec<String> = Vec::new();
    for (c, _, _) in region_diags.iter() {
        if *c != primary {
            let s = c.as_str().to_string();
            if !secondary.contains(&s) {
                secondary.push(s);
            }
        }
    }

    // Confidence: fraction of real-diff px that fall in the primary class.
    let primary_px = census.count_of(primary);
    let confidence = if census.real == 0 {
        0.0
    } else {
        (primary_px as f64 / census.real as f64).clamp(0.0, 1.0)
    };

    // Aggregate magnitude (§2.1): CSS px / ΔRGB / ΔE / α — straight from the tally
    // plus whatever the primary region recovered (colourspace / alpha).
    let recovered_alpha = primary_region.as_ref().and_then(|r| r.recovered_alpha);
    let colorspace = primary_region.as_ref().and_then(|r| {
        if r.class == ErrorClass::ColorSpace.as_str() {
            Some("sRGB↔linear".to_string())
        } else {
            None
        }
    });
    let residual_shift_css = whole_frame_shift(primary, tally, regions);
    let magnitude = Magnitude {
        edge_delta_css: tally.edge_delta_css,
        missing_area_pct: tally.missing_pct,
        extra_area_pct: tally.extra_pct,
        modal_drgb: tally.modal_drgb,
        delta_e: tally.color_de,
        recovered_alpha,
        residual_shift_css,
        colorspace,
    };

    // Headline (§2.3) for the whole fixture, keyed on the primary class + its
    // magnitude signature. Region headlines are filled per-region above.
    let headline = headline_for(primary, &magnitude, tally, &census);

    let regions_out: Vec<RegionDiag> =
        region_diags.into_iter().take(MAX_REGION_DIAGS).map(|(_, rd, _)| rd).collect();

    Diagnosis {
        primary_class: primary.as_str().to_string(),
        secondary,
        headline,
        magnitude,
        regions: regions_out,
        residual_shift_css,
        confidence,
    }
}

/// The AaOnly diagnosis: differences confined to glyph AA edges (§2.3 last row).
fn aa_only_diagnosis(tally: &ClassTally) -> Diagnosis {
    let magnitude = Magnitude {
        edge_delta_css: tally.edge_delta_css,
        ..Magnitude::default()
    };
    Diagnosis {
        primary_class: ErrorClass::AaOnly.as_str().to_string(),
        secondary: Vec::new(),
        headline: "differences confined to glyph AA edges — measurement ceiling, not a bug".to_string(),
        magnitude,
        regions: Vec::new(),
        residual_shift_css: [0.0, 0.0],
        confidence: 1.0,
    }
}

// ---------------------------------------------------------------------------
// Region-level classification (the per-region ErrorClass + its RegionDiag)
// ---------------------------------------------------------------------------

/// Map a `DiffRegion` to its `ErrorClass` (§2.2). Missing/Extra follow the
/// region's dominant pixel class directly. A GeomShift-dominant region (a
/// displaced boundary) is split SIZE vs SHIFT from the fixture's per-side extent
/// SYMMETRY — an asymmetric extent (one/two sides) is a box-size error even though
/// its pixels read as a shifted boundary, whereas an all-four-equal extent is a
/// true translation. A ColorErr-dominant region is refined into
/// ColorValue / ColorSpace / AlphaCompositing by sampling its pixels.
fn classify_region(
    r: &DiffRegion,
    tally: &ClassTally,
    cand: &RgbaImage,
    reference: &RgbaImage,
) -> (ErrorClass, Option<f64>) {
    match r.dominant {
        PixelClass::Missing => (ErrorClass::Missing, None),
        PixelClass::Extra => (ErrorClass::Extra, None),
        PixelClass::GeomShift => (geometry_signature(tally).unwrap_or(ErrorClass::GeometryShift), None),
        // A ColorErr-DOMINANT region whose ColorErr is entirely on the structural
        // boundary (interior_color_px ~0) is NOT a fill recolour — it is a
        // shifted/resized element's correct-colour fill abutting a different
        // background (the §1-B "fill recolour ΔRGB…" misattribution: the interiors
        // were byte-identical). When such a region also carries a real geometry
        // signal, name the geometry, not a phantom colour (review §1-B / F9).
        PixelClass::ColorErr if r.interior_color_px == 0 => {
            match geometry_signature(tally) {
                Some(geo) => (geo, None),
                None => refine_color(r, cand, reference),
            }
        }
        PixelClass::ColorErr => refine_color(r, cand, reference),
        // Match/AaEdge never dominate a real-diff region; treat as a colour value
        // fallback if one ever appears so the diagnosis stays total.
        PixelClass::Match | PixelClass::AaEdge => (ErrorClass::ColorValue, None),
    }
}

/// Refine a ColorErr-dominant region into ColorValue / ColorSpace / AlphaCompositing,
/// returning the class and (for AlphaCompositing) the recovered α. BEST-EFFORT for
/// the latter two: an inconclusive fit falls back to ColorValue.
fn refine_color(r: &DiffRegion, cand: &RgbaImage, reference: &RgbaImage) -> (ErrorClass, Option<f64>) {
    // AlphaCompositing — BEST-EFFORT and DELIBERATELY NON-CLASS-CHANGING here. A
    // uniform α∈(0,1) explaining ref≈α·cand+(1−α)·white is recovered as an
    // INFORMATIONAL magnitude (carried on `recovered_alpha`), but it does NOT
    // override the ColorValue class: from per-pixel modal colours alone a moved
    // SOLID box over a lighter SOLID background is pixel-indistinguishable from an
    // uncomposited opacity (both are "solid ink vs the same hue lightened"), so
    // firing the class produced layout false-positives on the real suite (e.g.
    // block-margin-* ) while MISSING the genuine alpha fixtures (opacity-half,
    // color-rgba-alpha — those have mixed glyph+fill content, not a clean modal
    // pair). The honest result is ColorValue + a recovered_alpha hint; a
    // spatially-coherent α solver (Tier 2, with the layout sidecar) can promote
    // the class later. Documented best-effort per the C4 brief.
    let alpha = recover_alpha(r, cand, reference);
    // ColorSpace (gamma/sRGB-linear) fit over the region's ColorErr pixels.
    if fit_colorspace(r, cand, reference) {
        return (ErrorClass::ColorSpace, alpha);
    }
    (ErrorClass::ColorValue, alpha)
}

/// Build the serialisable per-region diagnosis with its own headline.
fn region_diag(r: &DiffRegion, class: ErrorClass, recovered_alpha: Option<f64>, tally: &ClassTally) -> RegionDiag {
    let mut rd = RegionDiag {
        class: class.as_str().to_string(),
        bbox_css: r.bbox_css,
        area_pct: r.area_pct,
        // A region carries the fixture's per-side extent delta only when it is the
        // geometry-defining region; otherwise the bbox tells the local story and
        // the per-side delta stays zeroed (it is a whole-fixture quantity).
        edge_delta_css: if matches!(class, ErrorClass::GeometrySize | ErrorClass::GeometryShift) {
            tally.edge_delta_css
        } else {
            [0.0; 4]
        },
        modal_drgb: r.modal_drgb,
        delta_e: r.delta_e,
        recovered_alpha,
        shift_css: [r.shift_css.0, r.shift_css.1],
        selector: None,
        headline: String::new(),
    };
    // A region-scoped magnitude for its own headline.
    let mag = Magnitude {
        edge_delta_css: rd.edge_delta_css,
        missing_area_pct: if class == ErrorClass::Missing { r.area_pct } else { 0.0 },
        extra_area_pct: if class == ErrorClass::Extra { r.area_pct } else { 0.0 },
        modal_drgb: rd.modal_drgb,
        delta_e: rd.delta_e,
        recovered_alpha: rd.recovered_alpha,
        residual_shift_css: rd.shift_css,
        colorspace: if class == ErrorClass::ColorSpace { Some("sRGB↔linear".to_string()) } else { None },
    };
    let census = Census::default(); // region headlines do not need the census
    rd.headline = headline_for(class, &mag, tally, &census);
    rd
}

// ---------------------------------------------------------------------------
// GeometrySize vs GeometryShift (spec §2.2) — cheap, from edge symmetry.
// ---------------------------------------------------------------------------

/// Decide whether a geometry defect (above the PASS edge bound) reads as a SIZE
/// error (asymmetric: one/two sides) or a SHIFT (all four sides ~equal & opposite-
/// signed-by-pair, i.e. a pure translation). Returns `None` when the extent signal
/// is within the PASS bound (no geometry defect to name).
fn geometry_signature(tally: &ClassTally) -> Option<ErrorClass> {
    let d = tally.edge_delta_css;
    if tally.edge_max_css <= G_EDGE_CSS.0 && tally.shift_max_css <= G_SHIFT_CSS.0 {
        return None;
    }
    // Translation signature: ref−cand on opposite sides moves the SAME direction
    // in image space, so L≈R and T≈B in signed value, and all magnitudes ~equal.
    // (For a +dx,+dy translation of the candidate, every side's ref−cand delta is
    // the SAME signed value.) Size signature: at least one side dominates while
    // its opposite is ~0 (e.g. a box too tall only at the bottom).
    let mag = |v: f64| v.abs();
    let max = d.iter().cloned().fold(0.0_f64, |m, v| m.max(mag(v)));
    if max < f64::EPSILON {
        // Pure residual shift with no extent delta (whole-frame translation that
        // did not change the bbox corners measurably) — read as a shift.
        return Some(ErrorClass::GeometryShift);
    }
    // All four sides within 20% of the max AND the same sign on each axis pair =>
    // translation. Otherwise the asymmetry says size.
    let near = |a: f64, b: f64| (a - b).abs() <= 0.2 * max + 0.05;
    let all_equal = near(d[0], d[1]) && near(d[2], d[3]) && near(d[0], d[2]);
    if all_equal && d.iter().all(|v| mag(*v) > 0.4 * max) {
        Some(ErrorClass::GeometryShift)
    } else {
        Some(ErrorClass::GeometrySize)
    }
}

/// The side name carrying the dominant asymmetric extent delta (for the headline).
fn dominant_side(edge_delta_css: [f64; 4]) -> (&'static str, f64) {
    let names = ["left", "right", "top", "bottom"];
    let mut best = (names[0], edge_delta_css[0]);
    for i in 1..4 {
        if edge_delta_css[i].abs() > best.1.abs() {
            best = (names[i], edge_delta_css[i]);
        }
    }
    best
}

// ---------------------------------------------------------------------------
// ColorSpace / AlphaCompositing sub-classifiers (spec §2.2) — BEST-EFFORT.
// ---------------------------------------------------------------------------

/// Try to recover a uniform compositing α∈(0,1) explaining the region: the
/// candidate paints an OPAQUE colour `top`, the reference shows `top` composited
/// at α over white paper (`ref ≈ α·top + (1−α)·255`). We solve α per channel from
/// the region's modal cand/ref colours and accept only a CONSISTENT α well inside
/// (0,1). Best-effort: returns `None` (=> ColorValue) when the channels disagree
/// or α is degenerate.
fn recover_alpha(r: &DiffRegion, cand: &RgbaImage, reference: &RgbaImage) -> Option<f64> {
    let (top, bot, top_share, bot_share) = region_modal_colors(r, cand, reference)?;
    // Uniformity gate (the key false-positive guard): true uncomposited opacity is
    // a SOLID opaque ink (candidate) versus the SAME ink blended over paper
    // (reference) — BOTH sides are near-uniform. A moved/recoloured box instead
    // overlaps several background tones, so neither modal colour dominates. Require
    // both sides to be strongly modal so a layout/recolour region cannot pass.
    if top_share < 0.85 || bot_share < 0.85 {
        return None;
    }
    // The candidate ink must be a genuinely SATURATED/dark colour (far from white)
    // on at least two channels — otherwise "ref ≈ α·top + (1−α)·white" is ill-posed
    // and any colour difference fits a spurious α.
    let informative = (0..3).filter(|&ch| (top[ch] as i32 - 255).abs() >= 40).count();
    if informative < 2 {
        return None;
    }
    // Solve α from ref = α·top + (1−α)·white per channel where top != white.
    let mut alphas: Vec<f64> = Vec::new();
    for ch in 0..3 {
        let t = top[ch] as f64;
        let b = bot[ch] as f64;
        let denom = t - 255.0; // (1−α)·white term uses white=255
        if denom.abs() < 40.0 {
            continue; // this channel is ~white in the candidate -> uninformative
        }
        alphas.push((b - 255.0) / denom);
    }
    if alphas.len() < 2 {
        return None;
    }
    let mean = alphas.iter().sum::<f64>() / alphas.len() as f64;
    let spread = alphas.iter().map(|a| (a - mean).abs()).fold(0.0, f64::max);
    if spread > 0.06 || !(0.15..=0.85).contains(&mean) {
        return None; // channels disagree, or α is degenerate -> not a clean blend
    }
    // Reconstruction check: the recovered α must rebuild the reference modal from
    // the candidate modal to within a tight per-channel error. This is what a
    // recolour/layout region FAILS — its modal pair does not lie on a white-blend
    // line — so it falls back to ColorValue (best-effort, honest).
    let max_recon_err = (0..3)
        .map(|ch| {
            let recon = mean * top[ch] as f64 + (1.0 - mean) * 255.0;
            (recon - bot[ch] as f64).abs()
        })
        .fold(0.0, f64::max);
    if max_recon_err > 10.0 {
        return None;
    }
    Some((mean * 100.0).round() / 100.0)
}

/// Whether the region's ColorErr pixels fit an sRGB↔linear (gamma ~2.2/0.45)
/// transform markedly better than identity — the gradient/blend colour-space
/// drift. Best-effort: a coarse residual comparison on the region's sampled
/// pixels; returns false (=> ColorValue) when the gamma fit does not clearly win.
///
/// Hardened (review #6): the old test fired the gamma model on the GREEN channel
/// ALONE and over ANY region, so essentially any "fill rendered too light" neutral
/// recolour was mislabelled ColorSpace ("sRGB vs linear"), misdirecting triage. We
/// now require BOTH:
///   1. the >=3x residual reduction to hold JOINTLY on all of R, G, B (a true gamma
///      drift is a transfer-curve effect on every channel, not just luma), AND
///   2. non-trivial intra-region VARIANCE in the reference (a gamma/colour-space
///      drift is a GRADIENT/blend; a UNIFORM flat fill that merely came out lighter
///      is a ColorValue recolour, not a colour-space mismatch).
fn fit_colorspace(r: &DiffRegion, cand: &RgbaImage, reference: &RgbaImage) -> bool {
    let samples = sample_region_pairs(r, cand, reference, 4096);
    if samples.len() < 16 {
        return false;
    }
    // (1) Gradient gate: the reference must vary across the region. A flat recolour
    // has ~zero variance and is excluded (-> ColorValue). Measured as the per-channel
    // value spread (max−min) of the reference samples; require a meaningful ramp on
    // at least one channel.
    let mut lo = [255i32; 3];
    let mut hi = [0i32; 3];
    for (_, rr) in &samples {
        for ch in 0..3 {
            lo[ch] = lo[ch].min(rr[ch] as i32);
            hi[ch] = hi[ch].max(rr[ch] as i32);
        }
    }
    let ref_spread = (0..3).map(|ch| hi[ch] - lo[ch]).max().unwrap_or(0);
    if ref_spread < 24 {
        return false; // uniform-modal flat fill -> ColorValue, not ColorSpace
    }
    // (2) Per-channel identity vs gamma residual; the gamma model (inverse OETF on
    // the candidate toward linear) must collapse the residual by >=3x on EVERY
    // channel — a one-channel win is a coincidence, not a transfer-curve drift.
    for ch in 0..3 {
        let mut id_res = 0.0_f64;
        let mut gamma_res = 0.0_f64;
        for (c, rr) in &samples {
            let cv = c[ch] as f64 / 255.0;
            let rv = rr[ch] as f64 / 255.0;
            id_res += (cv - rv).powi(2);
            gamma_res += (srgb_eotf(cv) - rv).powi(2);
        }
        if !(gamma_res > 0.0 && id_res >= 3.0 * gamma_res) {
            return false;
        }
    }
    true
}

/// sRGB EOTF (display-encoded -> linear-light), the standard inverse transfer.
#[inline]
fn srgb_eotf(c: f64) -> f64 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The region's two dominant colours (candidate side `top`, reference side `bot`)
/// and each side's MODAL SHARE (the fraction of sampled pixels the modal bucket
/// holds — a uniformity measure). `None` when there are too few samples.
fn region_modal_colors(
    r: &DiffRegion,
    cand: &RgbaImage,
    reference: &RgbaImage,
) -> Option<([u8; 3], [u8; 3], f64, f64)> {
    let samples = sample_region_pairs(r, cand, reference, 4096);
    if samples.len() < 8 {
        return None;
    }
    let n = samples.len() as f64;
    let modal = |sel: fn(&([u8; 4], [u8; 4])) -> [u8; 3]| -> ([u8; 3], f64) {
        let mut counts: BTreeMap<[u8; 3], u32> = BTreeMap::new();
        for s in &samples {
            // Quantise to 8-step buckets so AA fringe doesn't fragment the mode.
            let q = sel(s).map(|c| (c / 8) * 8 + 4);
            *counts.entry(q).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(c, k)| (c, k as f64 / n))
            .unwrap_or(([0, 0, 0], 0.0))
    };
    let (top, top_share) = modal(|(c, _)| [c[0], c[1], c[2]]);
    let (bot, bot_share) = modal(|(_, rr)| [rr[0], rr[1], rr[2]]);
    Some((top, bot, top_share, bot_share))
}

/// Sample up to `cap` (cand,ref) RGBA pairs from the region's differing pixels.
/// The region's `bbox_css` is in CSS px relative to the union crop origin, so we
/// scan the device-px bbox. We do not have the class map here (a region carries
/// only its bbox/magnitude), so we accept any pixel whose cand/ref differ — for a
/// ColorErr-dominant region that is its defining condition (both ink, aligned,
/// colour differs), which keeps the colour sub-classifiers self-contained on the
/// owned region data.
fn sample_region_pairs(
    r: &DiffRegion,
    cand: &RgbaImage,
    reference: &RgbaImage,
    cap: usize,
) -> Vec<([u8; 4], [u8; 4])> {
    use super::config::CSS_PX;
    let (w, h) = cand.dimensions();
    let x0 = ((r.bbox_css[0] * CSS_PX).floor().max(0.0)) as u32;
    let y0 = ((r.bbox_css[1] * CSS_PX).floor().max(0.0)) as u32;
    let x1 = (((r.bbox_css[2] * CSS_PX).ceil()) as u32).min(w.saturating_sub(1));
    let y1 = (((r.bbox_css[3] * CSS_PX).ceil()) as u32).min(h.saturating_sub(1));
    let mut out = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = cand.get_pixel(x, y).0;
            let rr = reference.get_pixel(x, y).0;
            if c != rr {
                out.push((c, rr));
                if out.len() >= cap {
                    return out;
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Primary election & whole-frame helpers
// ---------------------------------------------------------------------------

/// When no region survived the speck filter, pick the primary class from the
/// strongest aggregate tally signal (so a thin-but-real defect still names itself).
fn elect_from_tally(tally: &ClassTally, census: &Census) -> ErrorClass {
    if let Some(geo) = geometry_signature(tally) {
        return geo;
    }
    if tally.missing_pct >= tally.extra_pct && tally.missing_pct > 0.0 {
        return ErrorClass::Missing;
    }
    if tally.extra_pct > 0.0 {
        return ErrorClass::Extra;
    }
    if tally.color_pct > 0.0 || census.color > 0 {
        return ErrorClass::ColorValue;
    }
    ErrorClass::AaOnly
}

/// Whole-frame residual translation (CSS px): the largest translation among the
/// regions flagged `is_translation`, reported as a signed [dx, dy].
///
/// When the primary class is GeometryShift but NO region carried a measured
/// translation peak (the shift came from the symmetric bbox-EXTENT signal, not a
/// per-region `best_local_shift`), fall back to the symmetric per-side extent delta
/// (review #19) so the reported magnitude AGREES with the headline (which already
/// uses that fallback) instead of disagreeing at [0,0].
fn whole_frame_shift(primary: ErrorClass, tally: &ClassTally, regions: &[DiffRegion]) -> [f64; 2] {
    let mut best = (0.0_f64, [0.0, 0.0]);
    for r in regions {
        if r.is_translation {
            let mag = (r.shift_css.0 * r.shift_css.0 + r.shift_css.1 * r.shift_css.1).sqrt();
            if mag > best.0 {
                best = (mag, [r.shift_css.0, r.shift_css.1]);
            }
        }
    }
    if best.0 == 0.0 && primary == ErrorClass::GeometryShift {
        // A pure translation moves all four sides by the same signed extent delta;
        // use that (L for x, T for y) so the magnitude matches the headline fallback.
        let d = tally.edge_delta_css;
        if d.iter().any(|v| v.abs() > 1e-6) {
            return [d[0], d[2]];
        }
    }
    best.1
}

// ---------------------------------------------------------------------------
// Pixel-class census (for AaOnly + confidence)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Census {
    color: u64,
    geom: u64,
    missing: u64,
    extra: u64,
    aa: u64,
    /// ColorErr+GeomShift+Missing+Extra (Match/AaEdge excluded).
    real: u64,
}

impl Census {
    fn of(cm: &ClassMap) -> Census {
        let mut c = Census::default();
        for px in &cm.px {
            match px {
                PixelClass::ColorErr => c.color += 1,
                PixelClass::GeomShift => c.geom += 1,
                PixelClass::Missing => c.missing += 1,
                PixelClass::Extra => c.extra += 1,
                PixelClass::AaEdge => c.aa += 1,
                PixelClass::Match => {}
            }
        }
        c.real = c.color + c.geom + c.missing + c.extra;
        c
    }

    /// Real-diff pixel count attributable to an ErrorClass (for `confidence`).
    /// Colour-family classes all draw from ColorErr px; a shift from GeomShift; a
    /// size error is a whole-extent property so all real-diff px are "in" it.
    fn count_of(&self, class: ErrorClass) -> u64 {
        match class {
            ErrorClass::Missing => self.missing,
            ErrorClass::Extra => self.extra,
            ErrorClass::GeometryShift => self.geom,
            ErrorClass::GeometrySize => self.real,
            ErrorClass::ColorValue | ErrorClass::ColorSpace | ErrorClass::AlphaCompositing => self.color,
            ErrorClass::AaOnly => self.aa,
        }
    }
}

// ===========================================================================
// Headline rule table (spec §2.3) — a PURE function of (class, magnitude).
// ===========================================================================

/// Human reason for a (class, magnitude) pair (§2.3). Pure: same inputs => same
/// string. The fixture-level and region-level headlines both go through here.
fn headline_for(primary: ErrorClass, mag: &Magnitude, tally: &ClassTally, census: &Census) -> String {
    match primary {
        ErrorClass::Missing => {
            if mag.missing_area_pct >= 50.0 || tally.missing_pct >= 50.0 {
                "feature not rendered — candidate blank where Chrome paints".to_string()
            } else {
                let pct = if mag.missing_area_pct > 0.0 { mag.missing_area_pct } else { tally.missing_pct };
                format!("content clipped/truncated ({pct:.1}% missing)")
            }
        }
        ErrorClass::Extra => {
            let pct = if mag.extra_area_pct > 0.0 { mag.extra_area_pct } else { tally.extra_pct };
            format!("extra paint where Chrome is blank ({pct:.1}%)")
        }
        ErrorClass::GeometrySize => {
            let (side, delta) = dominant_side(mag.edge_delta_css);
            let sign = if delta >= 0.0 { "+" } else { "−" };
            // Only NAME box-sizing when the signature actually matches it (review
            // #13): a content-box-vs-border-box mismatch grows BOTH the right and
            // bottom edges by a similar positive amount (the box is wider AND taller
            // by ~the border+padding). For any other asymmetric extent (one side, an
            // opposite-edge shift, a single-axis difference) we report the observed
            // signal WITHOUT asserting a single CSS cause.
            let d = mag.edge_delta_css; // [L, R, T, B]
            let right = d[1];
            let bottom = d[3];
            let border_box_pattern = right > 0.4
                && bottom > 0.4
                && (right - bottom).abs() <= 0.25 * right.max(bottom)
                && d[0].abs() < 0.4
                && d[2].abs() < 0.4;
            if border_box_pattern {
                format!(
                    "box +{right:.1}px right / +{bottom:.1}px bottom — box-sizing:border-box likely not applied"
                )
            } else {
                format!("box {sign}{:.1}px on {side} edge (size/box-model mismatch)", delta.abs())
            }
        }
        ErrorClass::GeometryShift => {
            let (dx, dy) = (mag.residual_shift_css[0], mag.residual_shift_css[1]);
            if dx.abs() < 1e-6 && dy.abs() < 1e-6 {
                // No per-region translation peak; use the symmetric extent delta.
                let m = mag.edge_delta_css.iter().cloned().fold(0.0_f64, |a, v| a.max(v.abs()));
                format!("content shifted ~{m:.1}px beyond page-origin calibration")
            } else {
                format!("content shifted ({dx:.1},{dy:.1})px beyond page-origin calibration")
            }
        }
        ErrorClass::ColorValue => {
            let cand_hex = drgb_to_note(mag.modal_drgb);
            format!("fill recolour {cand_hex} (ΔE {:.1})", mag.delta_e)
        }
        ErrorClass::ColorSpace => {
            "color-space mismatch (sRGB vs linear) — gradient/blend drift".to_string()
        }
        ErrorClass::AlphaCompositing => {
            let a = mag.recovered_alpha.unwrap_or(0.0);
            format!("opacity not composited (got α≈1.0, expected α≈{a:.2})")
        }
        ErrorClass::AaOnly => {
            let _ = census;
            "differences confined to glyph AA edges — measurement ceiling, not a bug".to_string()
        }
    }
}

/// Compact textual note for a modal ΔRGB triple (the per-channel cand−ref delta).
/// Reads as a signed RGB delta so the headline is self-describing without needing
/// the absolute hexes (which the aggregate tally does not retain).
fn drgb_to_note(d: [i16; 3]) -> String {
    let s = |v: i16| if v >= 0 { format!("+{v}") } else { format!("{v}") };
    format!("ΔRGB({},{},{})", s(d[0]), s(d[1]), s(d[2]))
}

// ===========================================================================
// Attribution composition (spec §2.3) — KEEP compute_attribution verbatim, and
// prefix CONFOUNDED fixtures' diagnosis headlines with `via {dep}: …`.
// ===========================================================================

/// For every non-PASS fixture, set `attribution`:
///   CONFOUNDED: <probe feature>  -> a depended substrate id is itself non-PASS
///   REAL                          -> all deps PASS (the target feature is wrong)
/// PASS fixtures get "" (no attribution).
///
/// Composition with diagnosis: a CONFOUNDED fixture's `diagnosis.headline` is
/// prefixed `via {dep}: …` so the report names the real culprit, not the surface
/// feature it renders through (honors the MEMORY.md failure-mode-attribution rule).
pub(crate) fn compute_attribution(results: &mut [FixtureResult]) {
    // id -> (status, feature) snapshot before mutation.
    let mut snap: BTreeMap<String, (Status, String)> = BTreeMap::new();
    for r in results.iter() {
        snap.insert(r.id.clone(), (r.status, r.feature.clone()));
    }
    for r in results.iter_mut() {
        if r.status == Status::Pass {
            r.attribution.clear();
            continue;
        }
        // Find the first non-PASS dependency (probe or base).
        let mut culprit: Option<(String, String)> = None; // (label, dep id)
        for d in r.depends_on.iter().chain(r.base_ids.iter()) {
            if let Some((st, feat)) = snap.get(d) {
                if *st != Status::Pass {
                    culprit = Some((format!("{feat} (`{d}`)"), d.clone()));
                    break;
                }
            }
        }
        r.attribution = match &culprit {
            Some((c, _)) => format!("CONFOUNDED: {c}"),
            None => "REAL".to_string(),
        };
        // Prefix the diagnosis headline for confounded fixtures so the human
        // reason leads with the upstream culprit, never the surface feature.
        if let (Some((_, dep)), Some(diag)) = (&culprit, r.diagnosis.as_mut()) {
            if !diag.headline.starts_with("via ") {
                diag.headline = format!("via {dep}: {}", diag.headline);
            }
        }
    }
}

// ===========================================================================
// Unit tests for diagnose() (spec deliverable): synthetic tallies/regions for
// the cheap-and-full sub-classifiers (size-vs-shift, AaOnly, colour value).
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba, RgbaImage};

    /// A trivial 1x1 class map of a single class — enough for the census-driven
    /// branches (AaOnly / confidence) that don't sample region pixels.
    fn class_map(w: u32, h: u32, fill: PixelClass) -> ClassMap {
        ClassMap { w, h, px: vec![fill; (w * h) as usize] }
    }

    fn white(w: u32, h: u32) -> RgbaImage {
        ImageBuffer::from_pixel(w, h, Rgba([255, 255, 255, 255]))
    }

    /// A tally with everything quiet (all gates within PASS) except the fields the
    /// caller sets — the common base for the synthetic cases.
    fn quiet_tally() -> ClassTally {
        ClassTally {
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
        }
    }

    /// A region with the given dominant class + magnitude knobs.
    fn region(dominant: PixelClass, area_pct: f64, de: f64, drgb: [i16; 3]) -> DiffRegion {
        DiffRegion {
            bbox_css: [0.0, 0.0, 1.0, 1.0],
            dominant,
            area_px: 100,
            area_pct,
            fill_ratio: 1.0,
            modal_drgb: drgb,
            delta_e: de,
            interior_color_px: if dominant == PixelClass::ColorErr { 100 } else { 0 },
            shift_css: (0.0, 0.0),
            is_translation: false,
        }
    }

    #[test]
    fn diagnose_aa_only_reads_as_measurement_ceiling() {
        // Only AaEdge pixels, everything else quiet -> AaOnly headline (not scary).
        let cm = class_map(4, 4, PixelClass::AaEdge);
        let tally = quiet_tally();
        let d = diagnose(&tally, &[], &cm, &white(4, 4), &white(4, 4));
        assert_eq!(d.primary_class, "AaOnly", "AA-only frame must read AaOnly");
        assert!(
            d.headline.contains("measurement ceiling"),
            "headline must name the measurement ceiling, got: {}",
            d.headline
        );
        assert!(d.regions.is_empty(), "AaOnly has no real-diff regions");
    }

    #[test]
    fn diagnose_geometry_size_names_box_sizing_only_on_the_border_box_signature() {
        // (a) The genuine box-sizing signature: BOTH right and bottom grew by ~the
        // same positive amount (content-box vs border-box) -> GeometrySize, and the
        // headline NAMES box-sizing (review #13: box-sizing is asserted only when the
        // delta pattern matches it).
        let cm = class_map(8, 8, PixelClass::GeomShift);
        let mut tally = quiet_tally();
        tally.edge_delta_css = [0.0, 4.0, 0.0, 4.0]; // right + bottom, equal
        tally.edge_max_css = 4.0;
        let d = diagnose(&tally, &[], &cm, &white(8, 8), &white(8, 8));
        assert_eq!(d.primary_class, "GeometrySize", "asymmetric extent => GeometrySize");
        assert!(
            d.headline.contains("box-sizing") && d.headline.contains("4.0px"),
            "border-box signature must name box-sizing + magnitude, got: {}",
            d.headline
        );

        // (b) A bottom-ONLY extent is a size error but NOT the box-sizing pattern, so
        // the headline must describe the side WITHOUT asserting box-sizing (review #13:
        // the old code hard-coded box-sizing for EVERY asymmetric delta).
        let mut tally2 = quiet_tally();
        tally2.edge_delta_css = [0.0, 0.0, 0.0, 4.0]; // bottom only
        tally2.edge_max_css = 4.0;
        let d2 = diagnose(&tally2, &[], &cm, &white(8, 8), &white(8, 8));
        assert_eq!(d2.primary_class, "GeometrySize", "asymmetric extent => GeometrySize");
        assert!(
            d2.headline.contains("bottom") && d2.headline.contains("4.0px"),
            "headline must name the bottom edge + magnitude, got: {}",
            d2.headline
        );
        assert!(
            !d2.headline.contains("box-sizing"),
            "a bottom-only delta must NOT assert box-sizing, got: {}",
            d2.headline
        );
    }

    #[test]
    fn diagnose_geometry_shift_reads_all_four_equal_as_translation() {
        // All four sides moved the SAME signed amount (a pure translation) and the
        // extent is beyond the PASS bound -> GeometryShift, not GeometrySize.
        let cm = class_map(8, 8, PixelClass::GeomShift);
        let mut tally = quiet_tally();
        tally.edge_delta_css = [1.6, 1.6, 1.6, 1.6];
        tally.edge_max_css = 1.6;
        let d = diagnose(&tally, &[], &cm, &white(8, 8), &white(8, 8));
        assert_eq!(d.primary_class, "GeometryShift", "equal four-side delta => GeometryShift");
        assert!(
            d.headline.contains("beyond page-origin calibration"),
            "shift headline must mention calibration, got: {}",
            d.headline
        );
    }

    #[test]
    fn diagnose_color_value_reports_drgb_and_delta_e() {
        // A ColorErr-dominant region with a flat modal ΔRGB + ΔE ~3.5 (the
        // #cc0000-vs-#dd0000 band) -> ColorValue with the ΔE in the headline.
        let cm = class_map(6, 6, PixelClass::ColorErr);
        let mut tally = quiet_tally();
        tally.color_pct = 100.0;
        tally.color_de = 3.5;
        tally.modal_drgb = [-17, 0, 0]; // cand darker red than ref
        // White images: the colour sub-classifiers sample no differing pixels, so
        // AlphaCompositing/ColorSpace stay None and we land on ColorValue.
        let r = region(PixelClass::ColorErr, 80.0, 3.5, [-17, 0, 0]);
        let d = diagnose(&tally, std::slice::from_ref(&r), &cm, &white(6, 6), &white(6, 6));
        assert_eq!(d.primary_class, "ColorValue", "a flat recolour => ColorValue");
        assert!(d.headline.contains("ΔE 3.5"), "headline must carry ΔE, got: {}", d.headline);
        assert!(d.headline.contains("ΔRGB(-17"), "headline must carry the modal ΔRGB, got: {}", d.headline);
        assert!((d.confidence - 1.0).abs() < 1e-9, "all real-diff px are ColorErr => confidence 1.0");
    }

    #[test]
    fn diagnose_colorspace_fit_detects_gamma_drift() {
        // A ColorErr-dominant region whose candidate is the sRGB-OETF (display)
        // encoding of a ramp the reference paints LINEARLY — the classic gamma /
        // colour-space drift. fit_colorspace must reduce the residual >=3x under the
        // inverse-OETF model and elect ColorSpace (the cheap full sub-classifier).
        let (w, h) = (64u32, 16u32);
        let mut cand = white(w, h);
        let mut reference = white(w, h);
        for x in 0..w {
            let t = x as f64 / (w as f64 - 1.0); // 0..1 ramp
            let lin = (t * 255.0).round() as u8; // reference: linear value
            // candidate: same intensity re-encoded through the sRGB OETF.
            let enc = if t <= 0.0031308 { t * 12.92 } else { 1.055 * t.powf(1.0 / 2.4) - 0.055 };
            let dis = (enc * 255.0).round().clamp(0.0, 255.0) as u8;
            for y in 0..h {
                reference.put_pixel(x, y, Rgba([lin, lin, lin, 255]));
                cand.put_pixel(x, y, Rgba([dis, dis, dis, 255]));
            }
        }
        let cm = class_map(w, h, PixelClass::ColorErr);
        let mut tally = quiet_tally();
        tally.color_pct = 100.0;
        tally.color_de = 8.0;
        // A region spanning the whole ramp (bbox in CSS px; sample_region_pairs maps
        // back to device px via CSS_PX).
        use super::super::config::CSS_PX;
        let r = DiffRegion {
            bbox_css: [0.0, 0.0, (w - 1) as f64 / CSS_PX, (h - 1) as f64 / CSS_PX],
            dominant: PixelClass::ColorErr,
            area_px: w * h,
            area_pct: 90.0,
            fill_ratio: 1.0,
            modal_drgb: [0, 0, 0],
            delta_e: 8.0,
            interior_color_px: w * h,
            shift_css: (0.0, 0.0),
            is_translation: false,
        };
        let d = diagnose(&tally, std::slice::from_ref(&r), &cm, &cand, &reference);
        assert_eq!(d.primary_class, "ColorSpace", "a gamma ramp must read ColorSpace");
        assert!(
            d.headline.contains("color-space mismatch"),
            "headline must name the colour-space mismatch, got: {}",
            d.headline
        );
        assert_eq!(d.magnitude.colorspace.as_deref(), Some("sRGB↔linear"), "magnitude must tag the fit");
    }

    #[test]
    fn diagnose_confounded_headline_is_prefixed_with_the_culprit() {
        // compute_attribution prefixes a CONFOUNDED fixture's diagnosis headline.
        let mut target = FixtureResult {
            id: "target".into(),
            category: "c".into(),
            feature: "f".into(),
            subfeature: String::new(),
            interaction_of: Vec::new(),
            base_ids: Vec::new(),
            status: Status::Fail,
            diff_pct: 50.0,
            weight: 1.0,
            description: String::new(),
            note: String::new(),
            kind: "feature".into(),
            depends_on: vec!["probe-x".into()],
            expected_support: "implemented".into(),
            attribution: String::new(),
            html_sha256: String::new(),
            diagnosis: Some(Diagnosis {
                primary_class: "Missing".into(),
                headline: "feature not rendered — candidate blank where Chrome paints".into(),
                ..Diagnosis::default()
            }),
        };
        let probe = FixtureResult {
            id: "probe-x".into(),
            category: "c".into(),
            feature: "probe".into(),
            subfeature: String::new(),
            interaction_of: Vec::new(),
            base_ids: Vec::new(),
            status: Status::Fail, // the substrate is itself broken
            diff_pct: 90.0,
            weight: 1.0,
            description: String::new(),
            note: String::new(),
            kind: "probe".into(),
            depends_on: Vec::new(),
            expected_support: "implemented".into(),
            attribution: String::new(),
            html_sha256: String::new(),
            diagnosis: None,
        };
        let mut results = vec![target.clone(), probe];
        compute_attribution(&mut results);
        let t = &results[0];
        assert!(t.attribution.starts_with("CONFOUNDED"), "target must be CONFOUNDED, got {}", t.attribution);
        let h = &t.diagnosis.as_ref().unwrap().headline;
        assert!(h.starts_with("via probe-x: "), "confounded headline must be prefixed, got: {h}");
        let _ = &mut target;
    }
}
