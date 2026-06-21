//! Combiner (spec §1.2/§1.3): fold per-verifier `SubVerdict`s into a single
//! `CombinedVerdict` by PER-CONCERN AUTHORITY, not by averaging or majority vote.
//!
//! For each concern the AUTHORITATIVE verifier's status decides that axis (the
//! authority table is §1.1); the combined status is WORST over the concerns using
//! the existing `Status::value()` severity order (Pass=1.0 > Partial=0.5 >
//! Fail=0.0; Unknown excluded). A non-authoritative verifier failing on an axis it
//! does NOT own can DOWNGRADE Pass→Partial (recorded as a `Disagreement`) but can
//! never force a Fail and never raises a status.
//!
//! PHASE 1: only `RasterVerifier` is present, and it owns all three concerns, so
//! there is exactly one verifier per axis, no challengers, no disagreements — and
//! WORST-of-its-three-axes == `verdict.rs`'s single status by construction (see
//! `raster.rs` for the equivalence argument, `goldens.rs` for the proof).

use super::super::config::FLOOR_PRESENCE_PCT;
use super::super::report::Status;
use super::{Concern, Disagreement, SubVerdict, VerifierKind};

/// The per-axis breakdown carried alongside the combined status. ADDITIVE — used
/// by the report; never feeds the gate beyond `status`.
#[derive(Clone, Debug)]
pub(crate) struct PerConcern {
    pub(crate) concern: Concern,
    pub(crate) status: Status,
    pub(crate) authority: VerifierKind,
}

/// The combiner's output. `status` is what `process_entry` maps onto
/// `FixtureResult.status` (exactly where `outcome.status` was used before).
#[derive(Clone, Debug)]
pub(crate) struct CombinedVerdict {
    pub(crate) status: Status,
    #[allow(dead_code)]
    pub(crate) per_concern: Vec<PerConcern>,
    pub(crate) disagreements: Vec<Disagreement>,
}

/// All three concern axes, in a fixed order so the per-concern list is
/// deterministic.
const CONCERNS: [Concern; 3] = [Concern::Geometry, Concern::Appearance, Concern::Presence];

/// The verifier that holds AUTHORITY over a concern when it is present and
/// applies (§1.1 table):
///   * Geometry  -> PdfGeometry when it applies, else RasterDiff (Phase 1: always
///                  RasterDiff, since PdfGeometry is not implemented yet).
///   * Appearance-> RasterDiff (ΔE/AA/blend over real pixels).
///   * Presence  -> RasterDiff (missing/extra coverage is a whole-area signal).
///
/// Returns the kind that should decide `concern`, given the kinds that produced a
/// sub-verdict for it. The order encodes precedence.
fn authority_for(concern: Concern, present: &[VerifierKind]) -> Option<VerifierKind> {
    let prefer = |order: &[VerifierKind]| -> Option<VerifierKind> {
        order.iter().copied().find(|k| present.contains(k))
    };
    match concern {
        Concern::Geometry => prefer(&[VerifierKind::PdfGeometry, VerifierKind::RasterDiff]),
        Concern::Appearance => prefer(&[VerifierKind::RasterDiff]),
        Concern::Presence => prefer(&[VerifierKind::RasterDiff]),
    }
}

/// Severity rank for WORST (lower is worse). Unknown has no value (excluded).
fn rank(s: Status) -> Option<f64> {
    s.value()
}

/// Pick the worse of two statuses by `Status::value()`. Unknown is excluded: if
/// one side is Unknown the other wins; both Unknown stays Unknown.
fn worse(a: Status, b: Status) -> Status {
    match (rank(a), rank(b)) {
        (Some(va), Some(vb)) => {
            if vb < va {
                b
            } else {
                a
            }
        }
        (Some(_), None) => a,
        (None, Some(_)) => b,
        (None, None) => Status::Unknown,
    }
}

/// Combine sub-verdicts into the final per-fixture verdict.
pub(crate) fn combine(subs: &[SubVerdict]) -> CombinedVerdict {
    let present: Vec<VerifierKind> = {
        let mut v: Vec<VerifierKind> = Vec::new();
        for s in subs {
            if !v.contains(&s.verifier) {
                v.push(s.verifier);
            }
        }
        v
    };

    // The floor-forgiveness PROOF: PdfGeometry has measured EVERY committed Chrome
    // box exact (Geometry=Pass within GEOM_TOL_PT). When present, a small residual
    // RasterDiff Appearance/Presence PARTIAL is the cross-rasterizer edge floor at
    // those VERIFIED boxes, not a real defect (§1.x / config FLOOR_* bounds).
    let pdf_geom_exact = subs.iter().any(|s| {
        s.verifier == VerifierKind::PdfGeometry
            && s.concern == Concern::Geometry
            && s.status == Status::Pass
    });

    let mut per_concern: Vec<PerConcern> = Vec::new();
    let mut disagreements: Vec<Disagreement> = Vec::new();
    // Start from the worst *known* axis status; Unknown axes are excluded (mirrors
    // Status::value()==None scoring). If EVERY axis is Unknown the result stays
    // Unknown.
    let mut combined: Option<Status> = None;

    for &concern in &CONCERNS {
        let owner = match authority_for(concern, &present) {
            Some(k) => k,
            None => continue, // no verifier produced this axis — skip it.
        };

        // The authoritative status for this axis.
        let mut auth_status = subs
            .iter()
            .find(|s| s.concern == concern && s.verifier == owner)
            .map(|s| s.status)
            .unwrap_or(Status::Unknown);

        // The image-confirmation temper (the symmetric counterpart of the
        // discarded-jitter rule below). RasterDiff is the visual ground truth — it
        // judges "matches the reference by image". When PdfGeometry holds Geometry
        // authority and reports FAIL, but RasterDiff's GEOMETRY opinion is NOT a FAIL
        // (the image matches well enough), the vector discrepancy is REAL but
        // SUB-VISUAL — e.g. a flex/grid container whose auto/explicit cross-axis is
        // ~3pt off Chrome at the vector level yet pixel-indistinguishable. A
        // sub-visual discrepancy must not become a hard FAIL (the brief's
        // no-false-fail-on-correct-geometry mandate), so PdfGeometry's FAIL is capped
        // to PARTIAL (the discrepancy is still surfaced, never hidden). When raster
        // ALSO fails geometry (the bug IS visible), the cap does NOT apply and the
        // FAIL stands — so genuinely-broken geometry still FAILs.
        if concern == Concern::Geometry
            && owner == VerifierKind::PdfGeometry
            && auth_status == Status::Fail
        {
            let raster_geom = subs
                .iter()
                .find(|s| s.concern == Concern::Geometry && s.verifier == VerifierKind::RasterDiff)
                .map(|s| s.status);
            let raster_confirms_visible = matches!(raster_geom, Some(Status::Fail));
            if !raster_confirms_visible {
                let note = subs
                    .iter()
                    .find(|s| s.concern == Concern::Geometry && s.verifier == owner)
                    .map(|s| s.headline.clone())
                    .unwrap_or_default();
                disagreements.push(Disagreement {
                    concern,
                    authoritative: auth_status,
                    authoritative_by: owner,
                    challenger: raster_geom.unwrap_or(Status::Unknown),
                    challenger_by: VerifierKind::RasterDiff,
                    note: format!("vector FAIL capped to PARTIAL (image not broken): {note}"),
                });
                auth_status = Status::Partial;
            }
        }

        // Floor-forgiveness (§1.x): once PdfGeometry has PROVEN the geometry exact,
        // a SMALL RasterDiff PRESENCE PARTIAL is the cross-rasterizer edge floor at
        // the verified box borders (Chrome and resvg cover slightly different pixels
        // along a 2px border ⇒ ~0.5-0.8% missing/extra), not a real defect. Temper
        // it to PASS, bounded by `config::FLOOR_PRESENCE_PCT` and PARTIAL-only.
        //
        // APPEARANCE is deliberately NOT forgiven: a real clip/fill difference at a
        // box boundary (e.g. overflow clipped at the border box vs Chrome's padding
        // box) shows up as a SMALL edge-band ColorErr indistinguishable by magnitude
        // from genuine border-AA — and PdfGeometry verifies box rects, NOT clip
        // regions or fill content. So Appearance must PASS on its own; only the
        // coverage (Presence) floor is forgiven. The discrepancy is still surfaced.
        if pdf_geom_exact
            && owner == VerifierKind::RasterDiff
            && auth_status == Status::Partial
        {
            let floor = match concern {
                Concern::Presence => Some(FLOOR_PRESENCE_PCT),
                Concern::Appearance | Concern::Geometry => None,
            };
            if let Some(bound) = floor {
                let mag = subs
                    .iter()
                    .find(|s| s.concern == concern && s.verifier == owner)
                    .map(|s| s.magnitude)
                    .unwrap_or(f64::INFINITY);
                if mag <= bound {
                    let note = subs
                        .iter()
                        .find(|s| s.concern == concern && s.verifier == owner)
                        .map(|s| s.headline.clone())
                        .unwrap_or_default();
                    disagreements.push(Disagreement {
                        concern,
                        authoritative: Status::Pass,
                        authoritative_by: VerifierKind::PdfGeometry,
                        challenger: auth_status,
                        challenger_by: VerifierKind::RasterDiff,
                        note: format!(
                            "edge floor forgiven to PASS (geometry vector-exact, {mag:.2} <= {bound:.1}): {note}"
                        ),
                    });
                    auth_status = Status::Pass;
                }
            }
        }

        // Cross-signal challengers (§1.3): a NON-authoritative verifier with a
        // WORSE opinion on this concern is recorded as a disagreement and can
        // SOFT-DOWNGRADE a PASS to PARTIAL — EXCEPT the one case the exact verifier
        // exists to override: RasterDiff's GEOMETRY opinion when PdfGeometry holds
        // authority. That raster geometry signal is the noisy `content_bbox` scan
        // (the structural ~1px false-fail); since PdfGeometry has measured the box
        // EXACTLY in pt, the raster jitter is DISCARDED for the verdict (no
        // downgrade) and only recorded as a disagreement. This is exactly how the
        // ~1px false-fail is fixed (PdfGeom PASS-exact + RasterGeom FAIL-jitter ->
        // PASS + disagreement). A challenger can never force a Fail and never raises
        // a status. PHASE 1/2a (no sidecar): only RasterDiff is present, so it owns
        // every axis, there are no challengers, and `axis_status == auth_status` —
        // the loop records nothing (no-op).
        let mut axis_status = auth_status;
        for s in subs {
            if s.concern != concern || s.verifier == owner {
                continue;
            }
            // A challenger opinion on an axis it does not own.
            let challenger_worse = matches!(
                (rank(s.status), rank(axis_status)),
                (Some(cs), Some(asx)) if cs < asx
            );
            if challenger_worse {
                disagreements.push(Disagreement {
                    concern,
                    authoritative: auth_status,
                    authoritative_by: owner,
                    challenger: s.status,
                    challenger_by: s.verifier,
                    note: s.headline.clone(),
                });
                // The discarded-jitter exception: RasterDiff challenging Geometry
                // owned by PdfGeometry is forgiven outright (no downgrade).
                let raster_geom_jitter = concern == Concern::Geometry
                    && owner == VerifierKind::PdfGeometry
                    && s.verifier == VerifierKind::RasterDiff;
                // Otherwise, soft-downgrade Pass->Partial only; never below Partial,
                // never raise.
                if !raster_geom_jitter && axis_status == Status::Pass {
                    axis_status = Status::Partial;
                }
            }
        }

        per_concern.push(PerConcern {
            concern,
            status: axis_status,
            authority: owner,
        });
        combined = Some(match combined {
            None => axis_status,
            Some(c) => worse(c, axis_status),
        });
    }

    CombinedVerdict {
        status: combined.unwrap_or(Status::Unknown),
        per_concern,
        disagreements,
    }
}
