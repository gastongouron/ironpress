//! Pluggable multi-verifier architecture (spec §1) — Phase 1, a PROVABLE NO-OP.
//!
//! The directive: "a PDF-level check should also happen in the harness — allow
//! different ways to render things while requiring a result." Concretely, a
//! fixture's *required result* (correct rendering) is asserted by **one-or-more
//! pluggable verifiers**, decoupled from *how* the result is rendered/measured.
//! Each verifier produces `SubVerdict`s on the concerns it is authoritative over;
//! the combiner (`combine.rs`) folds them into a single `CombinedVerdict`.
//!
//! PHASE 1 SCOPE (no behaviour change): this introduces the trait + the
//! `RasterVerifier` adapter (maps the ALREADY-COMPUTED `V2Outcome` into three
//! `SubVerdict`s using the SAME `config.rs` gates — it never re-runs the
//! comparator) + the combiner. The `PdfGeometry` verifier (§2) is declared in the
//! `VerifierKind` enum but NOT implemented yet (Phase 2). With ONLY the
//! `RasterVerifier` present, `combine(...).status` reproduces `verdict.rs`'s
//! status EXACTLY for every fixture (see `combine.rs` + `goldens.rs` for the
//! proof), so the committed baseline does not move.

pub(crate) mod combine;
pub(crate) mod coords;
pub(crate) mod pdf_geom;
pub(crate) mod raster;

#[cfg(test)]
mod goldens;

use serde::{Deserialize, Serialize};

use super::manifest::ManifestEntry;
use super::report::Status;
use coords::CoordSidecar;

/// What a verifier is asked to look at: the per-fixture artifacts the harness
/// already produced in `process_entry`. A verifier reads only the fields it needs.
///
/// PHASE 1: only `entry` is consumed (by `RasterVerifier`). PHASE 2a: `pdf` +
/// `coords` are now read by the `PdfGeometry` verifier (it tokenizes the candidate
/// PDF bytes and asserts them against the committed sidecar). `cand`/`reference`
/// remain threaded for a future raster-side cross-check; they are
/// `#[allow(dead_code)]` until then so the seam in `process_entry` keeps the same
/// shape the spec (§1.4) prescribes.
pub(crate) struct VerifyCtx<'a> {
    #[allow(dead_code)]
    pub(crate) entry: &'a ManifestEntry,
    /// Candidate PDF bytes (already in memory in `process_entry`). Read by
    /// `PdfGeometry` (the vector geometry tokenizer).
    pub(crate) pdf: &'a [u8],
    /// CALIBRATED candidate raster. (Future raster cross-check.)
    #[allow(dead_code)]
    pub(crate) cand: &'a image::RgbaImage,
    /// Reference raster. (Future raster cross-check.)
    #[allow(dead_code)]
    pub(crate) reference: &'a image::RgbaImage,
    /// Expected pt geometry, if a sidecar is committed for this fixture. `None`
    /// makes `PdfGeometry` not apply (geometry authority stays with raster).
    /// PHASE 2a: always `None` (no sidecar files exist yet).
    pub(crate) coords: Option<&'a CoordSidecar>,
}

/// A single verifier's opinion on ONE fixture, on ONE concern (a "gate axis").
/// ADDITIVE report field (`FixtureResult.sub_verdicts`); `#[serde(default)]`
/// keeps old baselines parseable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SubVerdict {
    /// Which verifier produced this opinion.
    pub(crate) verifier: VerifierKind,
    /// The verdict on this concern (Pass | Partial | Fail | Unknown).
    pub(crate) status: Status,
    /// The axis this verdict is ALLOWED to decide.
    pub(crate) concern: Concern,
    /// Human reason (feeds the report).
    pub(crate) headline: String,
    /// Worst signal in the verifier's own unit (CSS px / ΔE / % / pt).
    pub(crate) magnitude: f64,
}

/// Which verifier emitted a `SubVerdict`. `PdfGeometry` is declared now but the
/// verifier itself lands in Phase 2 (§2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum VerifierKind {
    RasterDiff,
    PdfGeometry,
}

/// The axis a `SubVerdict` is authoritative over. This is the heart of "allow
/// different ways to render while REQUIRING a result": each axis of the required
/// result is owned by the verifier best able to measure it (combine.rs §1.1).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum Concern {
    /// Box/border rects, text baselines, sizes/positions (in pt / CSS px).
    Geometry,
    /// Fill/stroke/text COLOR, gradients, AA, blending.
    Appearance,
    /// Content exists at all (missing/extra ink).
    Presence,
}

/// A recorded cross-verifier disagreement (§1.3). A verifier failing on an axis
/// it does NOT own can pull `Pass→Partial` and is recorded here, but can never
/// force a `Fail` and never raises a status. ADDITIVE report field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Disagreement {
    /// The concern the verifiers disagree about.
    pub(crate) concern: Concern,
    /// The authoritative verifier's status on that concern.
    pub(crate) authoritative: Status,
    /// Which verifier holds authority.
    pub(crate) authoritative_by: VerifierKind,
    /// The dissenting (non-authoritative) verifier's status.
    pub(crate) challenger: Status,
    /// Which verifier dissents.
    pub(crate) challenger_by: VerifierKind,
    /// Human note (e.g. "raster edge jitter 0.32css discarded; pt-exact").
    pub(crate) note: String,
}

/// A verifier consumes the per-fixture artifacts it cares about and returns its
/// `SubVerdict`s. Verifiers never see each other.
pub(crate) trait Verifier {
    /// Identity of this verifier.
    fn kind(&self) -> VerifierKind;
    /// May return MULTIPLE sub-verdicts (one per concern it owns).
    fn verify(&self, ctx: &VerifyCtx) -> Vec<SubVerdict>;
    /// Does this verifier apply to this fixture? (e.g. `PdfGeometry` needs a
    /// sidecar — Phase 2.)
    fn applies(&self, ctx: &VerifyCtx) -> bool;
}
