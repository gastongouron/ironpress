//! Phase 1 no-op proof (spec §4.5 item 2 + the CRITICAL acceptance): synthetic
//! `V2Outcome`s are fed through the `RasterVerifier` adapter + combiner, and the
//! combined status is asserted EQUAL to the raster `verdict.rs` status for every
//! case (all-pass, color-fail, edge-fail, missing-fail, mixed, plus boundary and
//! UNKNOWN cases). These run WITHOUT pdftoppm/Chrome (sub-second) and are the
//! standing guard that the multi-verifier seam does not move any verdict while
//! only `RasterVerifier` is present.

use image::{ImageBuffer, Rgba};

use super::super::compare::tally::ClassTally;
use super::super::compare::verdict::verdict;
use super::super::compare::{PixelClass, V2Outcome, Verdict};
use super::super::config::{
    COLOR_DE_FAIL, COLOR_DE_PASS, G_COLOR_PCT, G_EDGE_CSS, G_EXTRA_PCT, G_MISSING_PCT, G_SHIFT_CSS,
};
use super::super::manifest::ManifestEntry;
use super::super::report::Status;
use super::combine::combine;
use super::raster::RasterVerifier;
use super::{Verifier, VerifierKind, VerifyCtx};

// ----------------------------------------------------------------------------
// Builders
// ----------------------------------------------------------------------------

/// A zeroed tally; tests set only the fields they exercise (the rest default to a
/// PASS-clean value).
fn tally() -> ClassTally {
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
        total_px: 1,
    }
}

/// A minimal free-geometry manifest entry (no threshold relaxation), so the
/// verdict + the adapter both read the fixed `config.rs` gates.
fn entry() -> ManifestEntry {
    ManifestEntry {
        id: "g".into(),
        category: "g".into(),
        feature: "g".into(),
        subfeature: String::new(),
        description: String::new(),
        file: "cases/g/g.html".into(),
        interaction_of: Vec::new(),
        base_ids: Vec::new(),
        weight: 1.0,
        pass_threshold_pct: None,
        partial_threshold_pct: None,
        floor_pct: None,
        sanitize: true,
        kind: "feature".into(),
        depends_on: Vec::new(),
        expected_support: "implemented".into(),
        geometry: "free".into(),
        oracle: "chrome".into(),
    }
}

/// Wrap a tally into a `V2Outcome` whose `status` is the REAL `verdict.rs` status
/// for that tally — exactly as `compare_v2` would have produced it. This exercises
/// the production `RasterVerifier::from_outcome` path, not a test shortcut.
fn outcome_for(t: ClassTally, e: &ManifestEntry) -> (V2Outcome, Status) {
    let v: Verdict = verdict(&t, &[], e);
    let status = v.status;
    let outcome = V2Outcome {
        status,
        diff_pct: 0.0,
        tally: t,
        regions: Vec::new(),
        verdict: v,
        overlay: ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255])),
        diagnosis: Default::default(),
    };
    (outcome, status)
}

/// Assert that the combiner (with ONLY the RasterVerifier present) reproduces the
/// raster verdict status for this tally, and return the combined sub-verdict count
/// for the structural checks below.
fn assert_noop(t: ClassTally) -> (Status, usize) {
    let e = entry();
    let (outcome, verdict_status) = outcome_for(t, &e);
    let rv = RasterVerifier::from_outcome(&outcome, &e);

    // The Phase-1 ctx: only `entry` is read by RasterVerifier; the image/pdf
    // fields are placeholders for Phase 2.
    let px = ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255]));
    let pdf: &[u8] = b"";
    let ctx = VerifyCtx {
        entry: &e,
        pdf,
        cand: &px,
        reference: &px,
        coords: None,
    };

    assert!(rv.applies(&ctx), "RasterVerifier must always apply");
    let subs = rv.verify(&ctx);
    let combined = combine(&subs);

    assert_eq!(
        combined.status, verdict_status,
        "combined status must equal verdict.rs status (tally produced {verdict_status:?})"
    );
    // Phase 1: only RasterDiff present -> never any disagreements.
    assert!(
        combined.disagreements.is_empty(),
        "Phase 1 has a single verifier; no disagreements expected"
    );
    (combined.status, subs.len())
}

// ----------------------------------------------------------------------------
// Tests — synthetic V2Outcomes, the §4.5 cases
// ----------------------------------------------------------------------------

#[test]
fn all_pass_is_noop() {
    // Everything clean -> PASS.
    let (status, n) = assert_noop(tally());
    assert_eq!(status, Status::Pass);
    assert_eq!(n, 3, "RasterVerifier emits one sub-verdict per concern");
}

#[test]
fn color_fail_is_noop() {
    // color_pct over the PARTIAL bound -> Appearance FAIL -> verdict FAIL.
    let mut t = tally();
    t.color_pct = G_COLOR_PCT.1 + 1.0;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Fail);
}

#[test]
fn hard_color_fail_is_noop() {
    // Interior recolour (hard-colour gate) -> Appearance FAIL even at small area.
    let mut t = tally();
    t.interior_color_de = COLOR_DE_FAIL + 1.0;
    t.interior_color_pct = G_COLOR_PCT.0 + 0.1;
    t.color_pct = 0.0; // under the area gate; hard-colour alone must FAIL.
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Fail);
}

#[test]
fn edge_fail_is_noop() {
    // edge_max_css over the PARTIAL bound -> Geometry FAIL -> verdict FAIL.
    let mut t = tally();
    t.edge_max_css = G_EDGE_CSS.1 + 1.0;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Fail);
}

#[test]
fn shift_fail_is_noop() {
    // A REAL shift relocates ink, so missing/extra rise alongside it. With
    // presence confirming the displacement, the shift FAIL stands and the
    // RasterVerifier still reproduces verdict.rs (a no-op). (A shift with NO
    // edge AND NO missing/extra is physically impossible for a real defect — see
    // `centroid_shift_artifact_forgiven`.)
    let mut t = tally();
    t.shift_max_css = G_SHIFT_CSS.1 + 1.0;
    t.missing_pct = G_MISSING_PCT.0 + 0.5; // relocated ink => real displacement
    t.extra_pct = G_EXTRA_PCT.0 + 0.5;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Fail);
}

#[test]
fn centroid_shift_artifact_forgiven() {
    // A large `shift_max_css` with edges within their PASS bound and ~no
    // missing/extra ink is a content-bbox CENTROID artifact (a soft-edged
    // gradient/mask/blend fringe pulls the centroid several px) — NOT a real
    // displacement, which would move edges or relocate ink. The RasterVerifier
    // neutralizes it to Geometry PASS. This is a DELIBERATE divergence from
    // verdict.rs's naive shift gate (which would FAIL), so it is asserted
    // directly rather than via the no-op equivalence helper.
    let mut t = tally();
    t.shift_max_css = G_SHIFT_CSS.1 + 2.0; // huge centroid-only "shift"
    // edge_max_css / missing_pct / extra_pct default to 0 (within PASS).
    let e = entry();
    let (outcome, _) = outcome_for(t, &e);
    let rv = RasterVerifier::from_outcome(&outcome, &e);
    let px = ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255]));
    let ctx = VerifyCtx {
        entry: &e,
        pdf: b"",
        cand: &px,
        reference: &px,
        coords: None,
    };
    let combined = combine(&rv.verify(&ctx));
    assert_eq!(
        combined.status,
        Status::Pass,
        "centroid-only shift (edges & presence clean) must be forgiven to PASS"
    );
}

#[test]
fn missing_fail_is_noop() {
    // missing_pct over the PARTIAL bound -> Presence FAIL -> verdict FAIL.
    let mut t = tally();
    t.missing_pct = G_MISSING_PCT.1 + 1.0;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Fail);
}

#[test]
fn extra_fail_is_noop() {
    let mut t = tally();
    t.extra_pct = G_EXTRA_PCT.1 + 1.0;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Fail);
}

#[test]
fn partial_band_is_noop() {
    // A single gate in the PARTIAL band (over PASS, under FAIL) -> PARTIAL.
    let mut t = tally();
    t.edge_max_css = (G_EDGE_CSS.0 + G_EDGE_CSS.1) / 2.0;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Partial);
}

#[test]
fn color_partial_band_is_noop() {
    // color_pct between PASS and PARTIAL bound -> PARTIAL.
    let mut t = tally();
    t.color_pct = (G_COLOR_PCT.0 + G_COLOR_PCT.1) / 2.0;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Partial);
}

#[test]
fn interior_de_partial_is_noop() {
    // Interior ΔE between PASS and FAIL bounds, area clean -> PARTIAL (denied PASS
    // by the interior-ΔE PASS condition, but below the hard-colour FAIL gate).
    let mut t = tally();
    t.interior_color_de = (COLOR_DE_PASS + COLOR_DE_FAIL) / 2.0;
    t.interior_color_pct = G_COLOR_PCT.0 + 0.1;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Partial);
}

#[test]
fn mixed_partial_and_fail_is_noop() {
    // One axis PARTIAL, another FAIL -> WORST == FAIL == verdict.
    let mut t = tally();
    t.edge_max_css = (G_EDGE_CSS.0 + G_EDGE_CSS.1) / 2.0; // Geometry PARTIAL
    t.missing_pct = G_MISSING_PCT.1 + 1.0; // Presence FAIL
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Fail);
}

#[test]
fn mixed_two_partials_is_noop() {
    // Two axes PARTIAL, none FAIL -> WORST == PARTIAL == verdict.
    let mut t = tally();
    t.edge_max_css = (G_EDGE_CSS.0 + G_EDGE_CSS.1) / 2.0; // Geometry PARTIAL
    t.color_pct = (G_COLOR_PCT.0 + G_COLOR_PCT.1) / 2.0; // Appearance PARTIAL
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Partial);
}

#[test]
fn pass_boundary_is_noop() {
    // Every gate exactly at its PASS bound -> still PASS (verdict uses `<=`).
    let mut t = tally();
    t.edge_max_css = G_EDGE_CSS.0;
    t.shift_max_css = G_SHIFT_CSS.0;
    t.color_pct = G_COLOR_PCT.0;
    t.missing_pct = G_MISSING_PCT.0;
    t.extra_pct = G_EXTRA_PCT.0;
    t.interior_color_de = COLOR_DE_PASS;
    let (status, _) = assert_noop(t);
    assert_eq!(status, Status::Pass);
}

#[test]
fn fail_boundary_just_over_is_noop() {
    // Just over a PARTIAL bound -> FAIL (verdict uses `>`); at the bound -> PARTIAL.
    let mut t = tally();
    t.edge_max_css = G_EDGE_CSS.1; // exactly at PARTIAL bound -> PARTIAL
    assert_eq!(assert_noop(t).0, Status::Partial);
    let mut t2 = tally();
    t2.edge_max_css = G_EDGE_CSS.1 + f64::EPSILON.max(0.001);
    assert_eq!(assert_noop(t2).0, Status::Fail);
}

#[test]
fn unknown_outcome_is_noop() {
    // An UNKNOWN outcome (unscoreable pair) maps every concern to Unknown; the
    // combiner reproduces UNKNOWN. Build it directly (verdict() never returns
    // Unknown for a tally — only the dimension guard does).
    let e = entry();
    let outcome = V2Outcome {
        status: Status::Unknown,
        diff_pct: 0.0,
        tally: tally(),
        regions: Vec::new(),
        verdict: Verdict {
            status: Status::Unknown,
            diff_pct: 0.0,
            dominant_class: PixelClass::Match,
        },
        overlay: ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255])),
        diagnosis: Default::default(),
    };
    let rv = RasterVerifier::from_outcome(&outcome, &e);
    let px = ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255]));
    let pdf: &[u8] = b"";
    let ctx = VerifyCtx {
        entry: &e,
        pdf,
        cand: &px,
        reference: &px,
        coords: None,
    };
    let subs = rv.verify(&ctx);
    assert!(subs.iter().all(|s| s.status == Status::Unknown));
    assert_eq!(combine(&subs).status, Status::Unknown);
}

#[test]
fn all_subverdicts_are_raster_in_phase1() {
    // Structural: in Phase 1 every sub-verdict is RasterDiff and the combiner
    // assigns RasterDiff authority to all three concerns.
    let e = entry();
    let (outcome, _) = outcome_for(tally(), &e);
    let rv = RasterVerifier::from_outcome(&outcome, &e);
    let px = ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255]));
    let pdf: &[u8] = b"";
    let ctx = VerifyCtx {
        entry: &e,
        pdf,
        cand: &px,
        reference: &px,
        coords: None,
    };
    assert_eq!(rv.kind(), VerifierKind::RasterDiff);
    let subs = rv.verify(&ctx);
    assert!(subs.iter().all(|s| s.verifier == VerifierKind::RasterDiff));
    let combined = combine(&subs);
    assert_eq!(combined.per_concern.len(), 3);
    assert!(
        combined
            .per_concern
            .iter()
            .all(|p| p.authority == VerifierKind::RasterDiff)
    );
    // Each axis status equals the corresponding raster sub-verdict (no challenger
    // can move it in Phase 1).
    for p in &combined.per_concern {
        let sub = subs.iter().find(|s| s.concern == p.concern).unwrap();
        assert_eq!(
            p.status, sub.status,
            "axis {:?} must mirror its sub-verdict",
            p.concern
        );
    }
}

// ============================================================================
// PHASE 2a — PDF-geometry tokenizer + verifier + coords goldens.
//
// All DORMANT in production (no sidecar files exist), exercised here directly.
// PAGE_H_PT = 792; top-left y = 792 - pdf_top.
// ============================================================================

use super::Concern;
use super::coords::{CoordBox, CoordSidecar, CoordText, spec_fill_rect_pt};
use super::pdf_geom::{
    BorderRect, FillRect, PdfGeometry, TextRun, extract_from_body, extract_geometry,
    verify_geometry_for_test,
};

/// Helper: assert two f64 are within `eps`.
fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

// ---------------------------------------------------------------------------
// Tokenizer goldens
// ---------------------------------------------------------------------------

#[test]
fn tokenizer_extracts_fill_border_text_clip_in_topleft_pt() {
    // A hand-written content stream covering every primitive ironpress emits.
    //   - fill:   120.96 120.96 240 160 re f   (PDF top = 280.96 -> y_tl 511.04)
    //   - border: 4 centered segments around [100,300]x[400,500] (PDF) at 2pt width
    //   - text:   Tm baseline at (120.96, 134.4) PDF, /F1 12 Tf  (-> y_tl 657.6)
    //   - clip:   45.3 671.7 75 75 re W n  (the validated grid red cell; y_tl 45.3)
    let body = b"\
1 0 0 1 0 0 cm\n\
0 0 1 rg\n\
120.96 120.96 240 160 re\nf\n\
0 0 0 RG\n2 w\n\
100 500 m 300 500 l S\n\
300 500 m 300 400 l S\n\
300 400 m 100 400 l S\n\
100 400 m 100 500 l S\n\
45.3 671.7 75 75 re W n\n\
BT\n/F1 12 Tf\n1 0 0 1 120.96 134.4 Tm\n<0048> Tj\nET\n";

    let g = extract_from_body(body);

    // Fill.
    assert_eq!(g.fills.len(), 1, "one fill rect");
    let f = &g.fills[0];
    assert!(near(f.rect_pt[0], 120.96, 1e-6), "fill x");
    assert!(near(f.rect_pt[1], 511.04, 1e-6), "fill y_tl = 792-280.96");
    assert!(near(f.rect_pt[2], 240.0, 1e-6), "fill w");
    assert!(near(f.rect_pt[3], 160.0, 1e-6), "fill h");
    assert!(near(f.fill[2], 1.0, 1e-6), "fill blue channel from rg");

    // Border (4-segment reconstruction) — bbox [100,400_tl]..200x100, width 2.
    assert_eq!(g.borders.len(), 1, "one border rect from the segment run");
    let b = &g.borders[0];
    assert!(near(b.rect_pt[0], 100.0, 1e-6), "border x");
    assert!(near(b.rect_pt[1], 292.0, 1e-6), "border y_tl = 792-500");
    assert!(near(b.rect_pt[2], 200.0, 1e-6), "border w");
    assert!(near(b.rect_pt[3], 100.0, 1e-6), "border h");
    assert!(near(b.width_pt, 2.0, 1e-6), "stroke width");
    assert!(
        b.from_segments,
        "a `m..l..S` run reconstructs the OUTER bbox (from_segments)"
    );

    // Clip.
    assert_eq!(g.clips.len(), 1, "one clip rect");
    let c = &g.clips[0];
    assert!(near(c.rect_pt[0], 45.3, 1e-6), "clip x");
    assert!(
        near(c.rect_pt[1], 45.3, 1e-6),
        "clip y_tl = 792-746.7 (validated grid cell)"
    );
    assert!(
        near(c.rect_pt[2], 75.0, 1e-6) && near(c.rect_pt[3], 75.0, 1e-6),
        "clip size"
    );

    // Text run — baseline origin + size only (no glyph advances).
    assert_eq!(g.text_runs.len(), 1, "one text run");
    let t = &g.text_runs[0];
    assert!(near(t.origin_pt[0], 120.96, 1e-6), "text origin x = Tm tx");
    assert!(
        near(t.origin_pt[1], 657.6, 1e-6),
        "text origin y_tl = 792-134.4"
    );
    assert!(near(t.size_pt, 12.0, 1e-6), "font size from Tf");
}

#[test]
fn tokenizer_tracks_cm_translation_for_text() {
    // A `cm` translate must shift the text baseline origin (transform-aware).
    // cm translates by (+10,+20); Tm at (100,100) -> page (110,120) PDF.
    let body = b"\
10 0 0 1 0 0 cm\n\
1 0 0 1 0 0 cm\n\
q\n1 0 0 1 10 20 cm\n\
BT\n/F1 9 Tf\n1 0 0 1 100 100 Tm\n<00> Tj\nET\n\
Q\n\
0 0 0 rg\n5 5 5 5 re f\n";
    // NOTE: the first `10 0 0 1 ... cm` scales x by 10 but is then NOT inside the
    // q/Q that wraps the text; it persists. So text x = (100*10)+10 = 1010? No —
    // cm composes: after `10 0 0 1 cm` the CTM scales x*10; the q/Q `1 0 0 1 10 20`
    // prepends a translate, mapping (100,100)->(110,120) in the scaled frame, then
    // x*10 -> 1100. We only assert the rect (clean identity at emit) to keep this
    // test focused; the cm-stack composition is covered by the rect path below.
    let g = extract_from_body(body);
    // The trailing fill is emitted AFTER the q/Q closed but the outer `10 0 0 1 cm`
    // scale is still active: 5x5 at (5,5) -> x in [50,100] (w 50), y unscaled.
    assert_eq!(g.fills.len(), 1);
    let f = &g.fills[0];
    assert!(
        near(f.rect_pt[0], 50.0, 1e-6),
        "fill x scaled by cm a=10: 5*10"
    );
    assert!(
        near(f.rect_pt[2], 50.0, 1e-6),
        "fill w scaled by cm a=10: 5*10"
    );
    assert!(near(f.rect_pt[3], 5.0, 1e-6), "fill h unscaled (d=1)");
    // Text origin x reflects the composed scale*translate.
    assert_eq!(g.text_runs.len(), 1);
    assert!(
        near(g.text_runs[0].origin_pt[0], 1100.0, 1e-6),
        "text x = (100+10)*10"
    );
}

#[test]
fn tokenizer_returns_none_on_filtered_stream() {
    // A FlateDecode'd content stream must NOT be guessed at -> None (degrade to the
    // raster fallback). We build a minimal PDF whose only stream is filtered.
    let pdf = b"%PDF-1.7\n\
1 0 obj\n<< /Length 20 /Filter /FlateDecode >>\nstream\n\
xx re xx rg xx m xxxx\nendstream\nendobj\n\
%%EOF\n";
    assert!(
        extract_geometry(pdf).is_none(),
        "a /Filter content stream must yield None (no flate guessing)"
    );
}

#[test]
fn tokenizer_finds_uncompressed_stream_in_minimal_pdf() {
    // The positive complement: an UNCOMPRESSED stream (no /Filter) is located.
    let pdf = b"%PDF-1.7\n\
1 0 obj\n<< /Length 40 >>\nstream\n\
0 0 1 rg\n10 10 20 30 re\nf\nendstream\nendobj\n\
%%EOF\n";
    let g = extract_geometry(pdf).expect("uncompressed stream must be found");
    assert_eq!(g.fills.len(), 1);
    assert!(near(g.fills[0].rect_pt[0], 10.0, 1e-6));
    assert!(near(g.fills[0].rect_pt[2], 20.0, 1e-6));
}

// ---------------------------------------------------------------------------
// Verifier goldens — synthetic candidate PdfGeometry vs synthetic sidecar.
// ---------------------------------------------------------------------------

/// A 2-fill + 1-text sidecar used as the "correct" contract across cases.
fn sidecar_2box_1text() -> CoordSidecar {
    CoordSidecar {
        schema: 1,
        frame: "chrome-ref-pt".into(),
        page_pt: [612.0, 792.0],
        boxes: vec![
            CoordBox {
                role: "fill".into(),
                rect_pt: [100.0, 100.0, 200.0, 150.0],
                selector: None,
            },
            CoordBox {
                role: "fill".into(),
                rect_pt: [400.0, 300.0, 80.0, 60.0],
                selector: None,
            },
        ],
        borders: vec![],
        text_runs: vec![CoordText {
            role: "baseline".into(),
            origin_pt: [120.0, 134.0],
            size_pt: 12.0,
            selector: None,
        }],
    }
}

/// Build a candidate geometry mirroring the sidecar, with optional per-element
/// position perturbations (dx,dy applied to each fill) and a uniform offset.
fn cand_from(boxes: &[[f64; 4]], text: &[([f64; 2], f64)], uniform: (f64, f64)) -> PdfGeometry {
    PdfGeometry {
        fills: boxes
            .iter()
            .map(|r| FillRect {
                rect_pt: [r[0] + uniform.0, r[1] + uniform.1, r[2], r[3]],
                fill: [0.0, 0.0, 1.0],
            })
            .collect(),
        borders: vec![],
        clips: vec![],
        text_runs: text
            .iter()
            .map(|(o, s)| TextRun {
                origin_pt: [o[0] + uniform.0, o[1] + uniform.1],
                size_pt: *s,
            })
            .collect(),
    }
}

#[test]
fn verifier_border_centerline_normalization_matches_chrome() {
    // Phase-2b border-convention fix (the brief's border-segment-grouping caveat):
    // Chrome's --print-to-pdf strokes a border as ONE centerline `re S` (inset
    // half the border width from the outer edge), so the sidecar records the
    // CENTERLINE rect. ironpress draws the four sides as separate centered strokes,
    // which the tokenizer reconstructs to the OUTER border-box bbox + width. The
    // verifier insets the candidate's outer bbox by half the width to recover the
    // centerline; the two then match EXACTLY.
    //
    // Probe-border-box ground truth (200x140px border-box, 12px border):
    //   ironpress outer bbox = [28.8, 28.8, 150, 105] pt, width 9pt
    //   Chrome centerline    = [32.25, 32.25, 141, 96] pt  (= outer inset 4.5pt;
    //                          ~1.05pt frame offset cancelled by the aligner)
    let sc = CoordSidecar {
        schema: 1,
        frame: "chrome-ref-pt".into(),
        page_pt: [612.0, 792.0],
        boxes: vec![],
        borders: vec![CoordBox {
            role: "border".into(),
            rect_pt: [32.25, 32.25, 141.0, 96.0],
            selector: None,
        }],
        text_runs: vec![],
    };
    let cand = PdfGeometry {
        fills: vec![],
        borders: vec![BorderRect {
            rect_pt: [28.8, 28.8, 150.0, 105.0],
            width_pt: 9.0,
            from_segments: true,
        }],
        clips: vec![],
        text_runs: vec![],
    };
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Pass,
        "outer-bbox border insets to Chrome centerline -> PASS (mag {}, {})",
        v.magnitude,
        v.headline
    );

    // A border genuinely the wrong size (size is frame-INDEPENDENT) must still
    // FAIL: shrink the candidate outer box by 12pt of width with the SAME stroke
    // width, so the centerline w/h are ~12pt off -> FAIL (not aligned away).
    let cand_wrong = PdfGeometry {
        fills: vec![],
        borders: vec![BorderRect {
            rect_pt: [28.8, 28.8, 138.0, 93.0],
            width_pt: 9.0,
            from_segments: true,
        }],
        clips: vec![],
        text_runs: vec![],
    };
    let vw = verify_geometry_for_test(&cand_wrong, &sc);
    assert_eq!(
        vw.status,
        Status::Fail,
        "12pt-undersized border -> FAIL ({})",
        vw.headline
    );
}

#[test]
fn verifier_re_s_centerline_border_not_double_inset() {
    // REGRESSION GUARD for the multi-border reconstruction bug. A grid/flex CHILD
    // draws its uniform border as a self-contained `re S` whose `re` rect is ALREADY
    // the centerline (= the element's OUTER background fill inset by the border
    // width). Ground truth: the validated grid red cell — outer fill [45.3,45.3,75,75]
    // pt, border `re S` [46.05,46.05,73.5,73.5] pt at 1.5pt width. The centerline is
    // the cell's OWN value, 73.5x73.5; the prior verifier inset the `re` a SECOND
    // time -> 72.0x72.0 (Δ1.5pt FALSE-fail). With the fill cross-check the co-located
    // outer fill (75) defines the box and centerline = 75 - 1.5 = 73.5 == the `re`.
    // The centerline TOP-LEFT sits half a width inside the fill corner: 45.3 + 0.75.
    let sc = CoordSidecar {
        schema: 1,
        frame: "chrome-ref-pt".into(),
        page_pt: [612.0, 792.0],
        boxes: vec![CoordBox {
            role: "fill".into(),
            rect_pt: [45.3, 45.3, 75.0, 75.0],
            selector: None,
        }],
        borders: vec![CoordBox {
            role: "border".into(),
            rect_pt: [46.05, 46.05, 73.5, 73.5],
            selector: None,
        }],
        text_runs: vec![],
    };
    let cand = PdfGeometry {
        fills: vec![FillRect {
            rect_pt: [45.3, 45.3, 75.0, 75.0],
            fill: [1.0, 0.0, 0.0],
        }],
        // The `re S` border at the centerline (from_segments == false).
        borders: vec![BorderRect {
            rect_pt: [45.3, 45.3, 73.5, 73.5],
            width_pt: 1.5,
            from_segments: false,
        }],
        clips: vec![],
        text_runs: vec![],
    };
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Pass,
        "centerline `re S` border is NOT inset twice -> PASS (mag {}, {})",
        v.magnitude,
        v.headline
    );
    assert!(
        v.magnitude <= 0.30 + 1e-9,
        "size matches exactly (mag {})",
        v.magnitude
    );
}

#[test]
fn verifier_does_not_match_box_to_full_page_background() {
    // REGRESSION GUARD for the primitive-matching bug (`fill#0 h Δ629pt`). ironpress
    // paints an opaque full-page background rect sharing the page's top-left corner
    // with the first real box. Matching by POSITION alone picked the page bg for a
    // small expected box -> a ~page-height size delta. The verifier (a) drops the
    // page-background fill by area, and (b) matches by SIZE-then-position, so the
    // real box wins. Ground truth shapes: LETTER printable bg + a 300x105 parent box.
    let sc = CoordSidecar {
        schema: 1,
        frame: "chrome-ref-pt".into(),
        page_pt: [612.0, 792.0],
        boxes: vec![CoordBox {
            role: "fill".into(),
            rect_pt: [27.75, 27.75, 300.0, 105.0],
            selector: None,
        }],
        borders: vec![],
        text_runs: vec![],
    };
    let cand = PdfGeometry {
        fills: vec![
            // The full-page background (printable area), same top-left corner.
            FillRect {
                rect_pt: [28.8, 28.8, 554.4, 734.4],
                fill: [1.0, 1.0, 1.0],
            },
            // The real parent box (~1.05pt frame offset, exact size).
            FillRect {
                rect_pt: [28.8, 28.8, 300.0, 105.0],
                fill: [0.8, 0.85, 0.9],
            },
        ],
        borders: vec![],
        clips: vec![],
        text_runs: vec![],
    };
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Pass,
        "the 300x105 box matches the real box, not the page bg -> PASS (mag {}, {})",
        v.magnitude,
        v.headline
    );
    assert!(
        v.magnitude < 1.0,
        "no ~629pt phantom size delta (mag {})",
        v.magnitude
    );
}

#[test]
fn verifier_concentric_nested_border_uses_own_background() {
    // REGRESSION GUARD for the nested-box fill cross-check. Three concentric fills
    // share the SAME center as the innermost element's border. The border's outer box
    // must be the element's OWN (tightest enclosing) background, NOT an ancestor's
    // larger one. Ground truth: block-nested-containment — fills 225/189/153, the
    // l3 border `re`-bbox 153 (segments) at 3pt; centerline = 153 - 3 = 150.
    let sc = CoordSidecar {
        schema: 1,
        frame: "chrome-ref-pt".into(),
        page_pt: [612.0, 792.0],
        boxes: vec![
            CoordBox {
                role: "fill".into(),
                rect_pt: [27.75, 27.75, 225.0, 225.0],
                selector: None,
            },
            CoordBox {
                role: "fill".into(),
                rect_pt: [45.75, 45.75, 189.0, 189.0],
                selector: None,
            },
            CoordBox {
                role: "fill".into(),
                rect_pt: [63.75, 63.75, 153.0, 153.0],
                selector: None,
            },
        ],
        borders: vec![CoordBox {
            role: "border".into(),
            rect_pt: [65.25, 65.25, 150.0, 150.0],
            selector: None,
        }],
        text_runs: vec![],
    };
    let cand = PdfGeometry {
        fills: vec![
            FillRect {
                rect_pt: [28.8, 28.8, 225.0, 225.0],
                fill: [0.18, 0.42, 0.87],
            },
            FillRect {
                rect_pt: [46.8, 46.8, 189.0, 189.0],
                fill: [0.85, 0.31, 0.31],
            },
            FillRect {
                rect_pt: [64.8, 64.8, 153.0, 153.0],
                fill: [0.94, 0.89, 0.29],
            },
        ],
        // Segment-reconstructed outer bbox of the innermost border (153 outer).
        borders: vec![BorderRect {
            rect_pt: [64.8, 64.8, 153.0, 153.0],
            width_pt: 3.0,
            from_segments: true,
        }],
        clips: vec![],
        text_runs: vec![],
    };
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Pass,
        "concentric border insets to 150 using its OWN bg, not the 225 ancestor (mag {}, {})",
        v.magnitude,
        v.headline
    );
}

#[test]
fn verifier_pass_exact() {
    let sc = sidecar_2box_1text();
    let cand = cand_from(
        &[[100.0, 100.0, 200.0, 150.0], [400.0, 300.0, 80.0, 60.0]],
        &[([120.0, 134.0], 12.0)],
        (0.0, 0.0),
    );
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Pass,
        "exact match -> PASS (mag {})",
        v.magnitude
    );
    assert_eq!(v.concern, Concern::Geometry);
}

#[test]
fn verifier_pass_after_uniform_1pt_offset_cancel() {
    // A UNIFORM +1pt,+1pt frame offset on every element (Chrome-frame vs
    // ironpress-frame margin rounding) must be cancelled exactly -> PASS, deltas 0.
    let sc = sidecar_2box_1text();
    let cand = cand_from(
        &[[100.0, 100.0, 200.0, 150.0], [400.0, 300.0, 80.0, 60.0]],
        &[([120.0, 134.0], 12.0)],
        (1.0, 1.0),
    );
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Pass,
        "uniform 1pt offset cancelled -> PASS"
    );
    assert!(
        v.magnitude <= 0.30 + 1e-9,
        "post-cancel worst delta within tol (got {})",
        v.magnitude
    );
}

#[test]
fn verifier_partial_half_pt() {
    // One coordinate 0.5pt off (in (TOL, 2*TOL]) and NOT a uniform offset (only one
    // box moves) -> the median offset stays 0, so the 0.5pt survives -> PARTIAL.
    let sc = sidecar_2box_1text();
    let cand = cand_from(
        &[[100.5, 100.0, 200.0, 150.0], [400.0, 300.0, 80.0, 60.0]],
        &[([120.0, 134.0], 12.0)],
        (0.0, 0.0),
    );
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Partial,
        "0.5pt on one box -> PARTIAL (mag {})",
        v.magnitude
    );
    assert!(
        near(v.magnitude, 0.5, 1e-6),
        "magnitude is the 0.5pt worst delta"
    );
}

#[test]
fn verifier_fail_3pt_on_one_box() {
    // 3pt on ONE box (others exact) -> median offset 0 -> 3pt > 2*TOL -> FAIL.
    let sc = sidecar_2box_1text();
    let cand = cand_from(
        &[[103.0, 100.0, 200.0, 150.0], [400.0, 300.0, 80.0, 60.0]],
        &[([120.0, 134.0], 12.0)],
        (0.0, 0.0),
    );
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(v.status, Status::Fail, "3pt on one box -> FAIL");
}

#[test]
fn verifier_fail_missing_box() {
    // Sidecar expects 2 fills; candidate has only 1 -> the second is unmatched
    // (nearest candidate is the first, far away) -> FAIL.
    let sc = sidecar_2box_1text();
    let cand = cand_from(
        &[[100.0, 100.0, 200.0, 150.0]],
        &[([120.0, 134.0], 12.0)],
        (0.0, 0.0),
    );
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(v.status, Status::Fail, "a missing expected box -> FAIL");
}

#[test]
fn verifier_fail_gross_global_offset() {
    // A UNIFORM but GROSS offset (every element +10pt) exceeds MAX_ALIGN_PT (3pt)
    // -> the page is misplaced, not merely frame-shifted -> FAIL (not cancelled).
    let sc = sidecar_2box_1text();
    let cand = cand_from(
        &[[100.0, 100.0, 200.0, 150.0], [400.0, 300.0, 80.0, 60.0]],
        &[([120.0, 134.0], 12.0)],
        (10.0, 10.0),
    );
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Fail,
        "gross global offset must FAIL (not aligned away)"
    );
    assert!(
        v.headline.contains("gross page offset"),
        "headline names the gross offset"
    );
}

#[test]
fn verifier_per_element_9pt_bug_not_aligned_away() {
    // THE ROBUSTNESS PROOF: one box 9pt wrong, the OTHER box + text exact. A
    // whole-page single translation cannot fix only one element without breaking
    // the others, so the median offset stays ~0 and the 9pt error survives -> FAIL.
    // (If alignment were per-element, this would be wrongly cancelled.)
    let sc = sidecar_2box_1text();
    let cand = cand_from(
        &[[109.0, 100.0, 200.0, 150.0], [400.0, 300.0, 80.0, 60.0]],
        &[([120.0, 134.0], 12.0)],
        (0.0, 0.0),
    );
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Fail,
        "a per-element 9pt bug must NOT be aligned away -> FAIL (mag {})",
        v.magnitude
    );
    assert!(
        v.magnitude >= 8.9,
        "the 9pt error survives the alignment (mag {})",
        v.magnitude
    );
}

#[test]
fn verifier_size_error_not_offset_cancellable() {
    // A SIZE error (w 9pt too wide) is frame-INDEPENDENT: it is compared WITHOUT
    // the offset, so no whole-page translation can hide it -> FAIL.
    let sc = sidecar_2box_1text();
    let cand = cand_from(
        &[[100.0, 100.0, 209.0, 150.0], [400.0, 300.0, 80.0, 60.0]],
        &[([120.0, 134.0], 12.0)],
        (0.0, 0.0),
    );
    let v = verify_geometry_for_test(&cand, &sc);
    assert_eq!(
        v.status,
        Status::Fail,
        "a 9pt width error -> FAIL (size is exact, no offset)"
    );
}

// ---------------------------------------------------------------------------
// coords spec cross-check golden
// ---------------------------------------------------------------------------

#[test]
fn coords_spec_fill_rect_pt_matches_validated_grid_cell() {
    // The validated grid red cell: content origin 28.8pt = 38.4 CSS px... no — the
    // brief gives it directly in px terms: 28.8pt margin + 20px(=15pt) + 2px(=1.5pt)
    // border inset, 100px(=75pt) box. Express in CSS px: content_origin = 38.4 css
    // (28.8pt/0.75), inset = 20+2 = 22 css, size = 100 css. Expect pt
    // [45.3, 45.3, 75, 75] (the spec value), top-left.
    let rect = spec_fill_rect_pt([38.4, 38.4], [22.0, 22.0], [100.0, 100.0]);
    assert!(near(rect[0], 45.3, 1e-6), "x = (38.4+22)*0.75 = 45.3");
    assert!(near(rect[1], 45.3, 1e-6), "y = 45.3");
    assert!(near(rect[2], 75.0, 1e-6), "w = 100*0.75");
    assert!(near(rect[3], 75.0, 1e-6), "h = 75");
}

// ---------------------------------------------------------------------------
// Combiner goldens (Phase 2a authority migration).
// ---------------------------------------------------------------------------

use super::{SubVerdict, VerifierKind as VK};

fn sv(verifier: VK, concern: Concern, status: Status) -> SubVerdict {
    SubVerdict {
        verifier,
        status,
        concern,
        headline: format!("{verifier:?}/{concern:?}={status:?}"),
        magnitude: 0.0,
    }
}

/// A full RasterDiff triple at the given statuses.
fn raster_triple(geom: Status, appearance: Status, presence: Status) -> Vec<SubVerdict> {
    vec![
        sv(VK::RasterDiff, Concern::Geometry, geom),
        sv(VK::RasterDiff, Concern::Appearance, appearance),
        sv(VK::RasterDiff, Concern::Presence, presence),
    ]
}

#[test]
fn combiner_pdfgeom_pass_raster_geom_fail_jitter_is_pass_plus_disagreement() {
    // PdfGeom PASS (exact pt) + RasterGeom FAIL (sub-px jitter) -> the raster
    // geometry opinion is DISCARDED (PdfGeometry has authority) -> combined PASS,
    // and the disagreement is recorded. This is the ~1px false-fail fix.
    let mut subs = raster_triple(Status::Fail, Status::Pass, Status::Pass);
    subs.push(sv(VK::PdfGeometry, Concern::Geometry, Status::Pass));
    let c = combine(&subs);
    assert_eq!(
        c.status,
        Status::Pass,
        "geom authority is PdfGeometry; jitter discarded"
    );
    assert_eq!(
        c.disagreements.len(),
        1,
        "the raster geometry jitter is recorded"
    );
    let d = &c.disagreements[0];
    assert_eq!(d.concern, Concern::Geometry);
    assert_eq!(d.authoritative_by, VK::PdfGeometry);
    assert_eq!(d.challenger_by, VK::RasterDiff);
    // The Geometry axis is owned by PdfGeometry and stays PASS.
    let geom_axis = c
        .per_concern
        .iter()
        .find(|p| p.concern == Concern::Geometry)
        .unwrap();
    assert_eq!(geom_axis.authority, VK::PdfGeometry);
    assert_eq!(geom_axis.status, Status::Pass);
}

#[test]
fn combiner_pdfgeom_fail_capped_when_image_not_broken() {
    // THE IMAGE-CONFIRMATION TEMPER (no-false-fail-on-correct-geometry): PdfGeom
    // FAIL (a real but SUB-VISUAL vector discrepancy, e.g. a flex/grid container
    // ~3pt off in its cross-axis) while RasterDiff's GEOMETRY opinion is NOT a FAIL
    // (the image matches). The vector FAIL is capped to PARTIAL and the cap is
    // recorded as a disagreement (the discrepancy is surfaced, never hidden) ->
    // combined PARTIAL, not a hard FAIL.
    let mut subs = raster_triple(Status::Pass, Status::Pass, Status::Pass);
    subs.push(sv(VK::PdfGeometry, Concern::Geometry, Status::Fail));
    let c = combine(&subs);
    assert_eq!(
        c.status,
        Status::Partial,
        "vector FAIL on visually-correct geometry is capped to PARTIAL, not FAIL"
    );
    let geom = c
        .per_concern
        .iter()
        .find(|p| p.concern == Concern::Geometry)
        .unwrap();
    assert_eq!(geom.authority, VK::PdfGeometry);
    assert_eq!(
        geom.status,
        Status::Partial,
        "geometry axis tempered to PARTIAL"
    );
    assert!(
        c.disagreements
            .iter()
            .any(|d| d.note.contains("capped to PARTIAL")),
        "the cap is recorded as a disagreement"
    );
}

#[test]
fn combiner_pdfgeom_fail_stands_when_raster_geom_also_fails() {
    // The temper does NOT apply when the bug is VISIBLE: RasterDiff also FAILs
    // Geometry, so PdfGeom's FAIL is confirmed by the image and the combined verdict
    // is FAIL. (Genuinely-broken geometry still FAILs — the verifier keeps teeth.)
    let mut subs = raster_triple(Status::Fail, Status::Pass, Status::Pass);
    subs.push(sv(VK::PdfGeometry, Concern::Geometry, Status::Fail));
    let c = combine(&subs);
    assert_eq!(
        c.status,
        Status::Fail,
        "vector FAIL confirmed by raster geometry FAIL -> FAIL"
    );
}

#[test]
fn combiner_pdfgeom_fail_capped_when_raster_geom_partial() {
    // The common container case: PdfGeom FAIL (3pt) + RasterDiff Geometry PARTIAL
    // (mild visible edge) -> the image is not BROKEN (only mildly off) -> cap to
    // PARTIAL. Raster's other axes pass, so combined PARTIAL (no regression from the
    // raster baseline, which was PARTIAL).
    let mut subs = raster_triple(Status::Partial, Status::Pass, Status::Pass);
    subs.push(sv(VK::PdfGeometry, Concern::Geometry, Status::Fail));
    let c = combine(&subs);
    assert_eq!(
        c.status,
        Status::Partial,
        "PdfGeom FAIL + raster PARTIAL geometry -> PARTIAL"
    );
}

#[test]
fn combiner_pdfgeom_pass_raster_appearance_fail_is_fail() {
    // PdfGeom PASS (Geometry) + RasterDiff Appearance FAIL -> Appearance is owned by
    // RasterDiff -> WORST = FAIL. The vector check cannot mask a colour bug.
    let mut subs = raster_triple(Status::Pass, Status::Fail, Status::Pass);
    subs.push(sv(VK::PdfGeometry, Concern::Geometry, Status::Pass));
    let c = combine(&subs);
    assert_eq!(
        c.status,
        Status::Fail,
        "Appearance authority stays with raster -> FAIL"
    );
}

#[test]
fn combiner_pdfgeom_pass_raster_presence_fail_is_fail() {
    // Symmetric: a missing glyph (Presence FAIL, owned by raster) -> FAIL even with
    // perfect geometry.
    let mut subs = raster_triple(Status::Pass, Status::Pass, Status::Fail);
    subs.push(sv(VK::PdfGeometry, Concern::Geometry, Status::Pass));
    let c = combine(&subs);
    assert_eq!(
        c.status,
        Status::Fail,
        "Presence authority stays with raster -> FAIL"
    );
}

// ---------------------------------------------------------------------------
// applies() no-op confirmation (Phase 2a).
// ---------------------------------------------------------------------------

#[test]
fn pdfgeom_applies_is_false_without_sidecar() {
    use super::pdf_geom::PdfGeomVerifier;
    let e = entry();
    let px = ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255]));
    // Even with a perfectly tokenizable PDF, NO sidecar => does not apply.
    let pdf = b"%PDF-1.7\n1 0 obj\n<< /Length 20 >>\nstream\n0 0 1 rg\n1 1 2 2 re\nf\nendstream\nendobj\n%%EOF\n";
    let ctx = VerifyCtx {
        entry: &e,
        pdf: pdf.as_slice(),
        cand: &px,
        reference: &px,
        coords: None,
    };
    let v = PdfGeomVerifier;
    assert!(
        !v.applies(&ctx),
        "Phase 2a: no sidecar => PdfGeometry never applies (no-op)"
    );
}

#[test]
fn pdfgeom_applies_true_only_with_sidecar_and_tokenizable_pdf() {
    use super::pdf_geom::PdfGeomVerifier;
    let e = entry();
    let px = ImageBuffer::from_pixel(1, 1, Rgba([255, 255, 255, 255]));
    let sc = sidecar_2box_1text();
    // With a sidecar AND a tokenizable PDF -> applies. (Proves the gate is the
    // sidecar presence, so the production no-op is purely the absence of files.)
    let pdf = b"%PDF-1.7\n1 0 obj\n<< /Length 20 >>\nstream\n0 0 1 rg\n1 1 2 2 re\nf\nendstream\nendobj\n%%EOF\n";
    let ctx = VerifyCtx {
        entry: &e,
        pdf: pdf.as_slice(),
        cand: &px,
        reference: &px,
        coords: Some(&sc),
    };
    let v = PdfGeomVerifier;
    assert!(v.applies(&ctx), "sidecar + tokenizable PDF -> applies");
    // And with a sidecar but a FILTERED PDF -> Unknown (does NOT apply).
    let filtered = b"%PDF-1.7\n1 0 obj\n<< /Length 20 /Filter /FlateDecode >>\nstream\nxx re xx rg xx m\nendstream\nendobj\n%%EOF\n";
    let ctx2 = VerifyCtx {
        entry: &e,
        pdf: filtered.as_slice(),
        cand: &px,
        reference: &px,
        coords: Some(&sc),
    };
    assert!(
        !v.applies(&ctx2),
        "filtered PDF -> does not apply (degrade to raster)"
    );
}
