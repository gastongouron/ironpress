//! The V2 golden contract (spec §5.1, amendment A4): synthetic in-memory image
//! pairs that pin the comparator's class + verdict. They run WITHOUT pdftoppm or
//! Chrome (sub-second) and are the standing honesty guard — any future change
//! that lets a wrong colour/size/margin/alpha/missing render PASS, or false-fails
//! clean glyph anti-aliasing, breaks a golden here.
//!
//! Each test asserts the verdict STATUS, the dominant `PixelClass`, and the named
//! magnitude band where the spec gives one. Where the spec's stated magnitude or
//! verdict is provably inconsistent with the spec's own constants/construction,
//! the test asserts the HONEST measured result and the discrepancy is documented
//! inline (and reported to the orchestrator) rather than fudged (amendment A6).

use image::{ImageBuffer, Rgba, RgbaImage};

use super::super::calibrate::{calibrate, check_probe_offset};
use super::super::config::CSS_PX;
use super::super::manifest::ManifestEntry;
use super::super::report::Status;
use super::{PixelClass, V2Outcome, compare_v2};

// ----------------------------------------------------------------------------
// Synthetic image builders
// ----------------------------------------------------------------------------

const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);

/// A white canvas of the given size.
fn canvas(w: u32, h: u32) -> RgbaImage {
    ImageBuffer::from_pixel(w, h, WHITE)
}

/// Fill the inclusive rect [x0,x1]x[y0,y1] with `c`.
fn fill(img: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, c: Rgba<u8>) {
    for y in y0..=y1.min(img.height() - 1) {
        for x in x0..=x1.min(img.width() - 1) {
            img.put_pixel(x, y, c);
        }
    }
}

/// Translate `img` by (dx,dy) on a white background (same dims).
fn translate(img: &RgbaImage, dx: i32, dy: i32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let mut out = canvas(w, h);
    for y in 0..h {
        for x in 0..w {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx >= 0 && ny >= 0 && (nx as u32) < w && (ny as u32) < h {
                out.put_pixel(nx as u32, ny as u32, *img.get_pixel(x, y));
            }
        }
    }
    out
}

/// A minimal "free"-geometry feature entry (no manifest overrides), so the verdict
/// runs the fixed gates only.
fn entry() -> ManifestEntry {
    ManifestEntry {
        id: "golden".into(),
        category: "golden".into(),
        feature: "golden".into(),
        subfeature: String::new(),
        description: String::new(),
        file: "cases/golden/golden.html".into(),
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

/// Run the SAME pipeline `process_entry` runs in a live v2 run: the caller has
/// already calibrated, so the goldens that model a real (non-origin) defect pass
/// the candidate straight through; the origin-offset golden calibrates first.
fn run(cand: &RgbaImage, reference: &RgbaImage) -> V2Outcome {
    compare_v2(cand, reference, &entry())
}

/// A 120x120 solid black box centred in a 200x200 frame (the common substrate).
fn box_frame() -> RgbaImage {
    let mut img = canvas(200, 200);
    fill(&mut img, 40, 40, 159, 159, BLACK);
    img
}

fn dump(name: &str, o: &V2Outcome) {
    eprintln!(
        "golden {name:24} status={:7} dom={:?} diff={:.3}% color%={:.3} miss%={:.3} extra%={:.3} \
         edge_max={:.3} shift_max={:.3} ΔE={:.3} aa%={:.3}",
        o.status.as_str(),
        o.verdict.dominant_class,
        o.diff_pct,
        o.tally.color_pct,
        o.tally.missing_pct,
        o.tally.extra_pct,
        o.tally.edge_max_css,
        o.tally.shift_max_css,
        o.tally.color_de,
        o.tally.aa_pct,
    );
}

// ----------------------------------------------------------------------------
// Golden rows (spec §5.1)
// ----------------------------------------------------------------------------

#[test]
fn golden_identical() {
    let a = box_frame();
    let o = run(&a, &a);
    dump("identical", &o);
    assert_eq!(o.status, Status::Pass, "identical must PASS");
    assert!(
        o.diff_pct < 1e-9,
        "identical diff must be 0, got {}",
        o.diff_pct
    );
    assert!(o.tally.color_pct == 0.0 && o.tally.missing_pct == 0.0 && o.tally.extra_pct == 0.0);
    assert!(o.tally.edge_max_css == 0.0 && o.tally.shift_max_css == 0.0);
}

#[test]
fn golden_origin_offset_zero() {
    // Every fixture now declares `@page { margin: 0 }`, so content sits at the page
    // ORIGIN in BOTH engines: the page-origin offset is GLOBAL_OFFSET=(0,0) and
    // `calibrate()` is the identity. An IDENTICAL candidate therefore PASSes with a
    // ~0 residual. (Historically this offset was (4,4) from Chrome's 28.8pt print-
    // margin rounding under uniform-LETTER fixtures; a 4px displacement is now a
    // GENUINE shift — see `golden_real_shift_5px`.)
    let reference = box_frame();
    let cand = calibrate(&reference); // identity under GLOBAL_OFFSET=(0,0)
    let o = run(&cand, &reference);
    dump("origin_offset_zero", &o);
    assert_eq!(
        o.status,
        Status::Pass,
        "0px (origin-aligned) offset must PASS"
    );
    assert!(
        o.tally.edge_max_css < 1e-9 && o.tally.shift_max_css < 1e-9,
        "residual edge/shift must be ~0, got edge {} shift {}",
        o.tally.edge_max_css,
        o.tally.shift_max_css
    );

    // A 4px displacement is no longer absorbed by calibration -> NOT PASS.
    let off4 = run(&calibrate(&translate(&reference, 4, 4)), &reference);
    assert_ne!(
        off4.status,
        Status::Pass,
        "a 4px shift must NOT pass under (0,0) calibration"
    );
}

#[test]
fn golden_real_shift_5px() {
    // A genuine 5px translation beyond calibration. Spec §5.1 requires "NOT PASS"
    // with a ~1.6 CSS px GeometryShift signature. A pure translation moves all
    // four bbox corners equally, so the SIZE signal reads ~1.6 CSS px on every
    // side (the all-four-equal translation signature) AND exposes ~8% of the box
    // area as Missing+Extra strips (5px of a 120px box). 1.6 CSS px is in the edge
    // PARTIAL band, but the 8% coverage exceeds the Missing/Extra PARTIAL bound
    // (6%), so the HONEST verdict is FAIL — which satisfies the spec's "NOT PASS".
    let reference = box_frame();
    let cand = translate(&reference, 5, 5);
    let o = run(&cand, &reference);
    dump("real_shift_5px", &o);
    assert_ne!(
        o.status,
        Status::Pass,
        "a 5px shift must NOT pass (spec: NOT PASS)"
    );
    let band = (5.0 / CSS_PX).abs(); // 1.6
    assert!(
        (o.tally.edge_max_css - band).abs() < 0.2,
        "edge_max_css ~= {band:.2} CSS px (the GeometryShift magnitude), got {:.3}",
        o.tally.edge_max_css
    );
    // All four sides moved equally (the translation signature -> GeometryShift).
    let d = o.tally.edge_delta_css;
    assert!(
        (d[0] - d[1]).abs() < 0.05 && (d[2] - d[3]).abs() < 0.05 && (d[0] - d[2]).abs() < 0.05,
        "translation must move all 4 sides equally, got {d:?}"
    );
}

#[test]
fn golden_real_shift_12px() {
    // 12px translation: edge signal ~3.84 CSS px > the edge PARTIAL bound (3.0)
    // -> FAIL.
    let reference = box_frame();
    let cand = translate(&reference, 12, 12);
    let o = run(&cand, &reference);
    dump("real_shift_12px", &o);
    assert_eq!(o.status, Status::Fail, "a 12px shift must FAIL");
    let band = 12.0 / CSS_PX; // 3.84
    assert!(
        (o.tally.edge_max_css - band).abs() < 0.3,
        "edge_max_css ~= {band:.2} CSS px, got {:.3}",
        o.tally.edge_max_css
    );
}

#[test]
fn golden_box_too_tall_13px() {
    // Candidate box is 13px taller at the bottom only (box-sizing not applied).
    // bottom-side extent delta ~= 4.16 CSS px > edge PARTIAL bound (3.0) -> FAIL,
    // asymmetric (one side) -> the GeometrySize signature.
    let reference = box_frame(); // box [40..159] in y
    let mut cand = canvas(200, 200);
    fill(&mut cand, 40, 40, 159, 172, BLACK); // +13px on the bottom
    let o = run(&cand, &reference);
    dump("box_too_tall_13px", &o);
    assert_eq!(o.status, Status::Fail, "a +13px box must FAIL");
    let band = 13.0 / CSS_PX; // 4.16
    assert!(
        (o.tally.edge_max_css - band).abs() < 0.3,
        "edge_max_css ~= {band:.2}, got {:.3}",
        o.tally.edge_max_css
    );
    // Asymmetric: bottom side dominates, others ~0.
    let d = o.tally.edge_delta_css;
    assert!(d[3].abs() > 3.0, "bottom delta must dominate, got {d:?}");
    assert!(
        d[0].abs() < 0.5 && d[1].abs() < 0.5 && d[2].abs() < 0.5,
        "other sides ~0, got {d:?}"
    );
}

#[test]
fn golden_box_offby1() {
    // 501x501 vs 500x500 — a 1-DEVICE-px size difference == 0.32 CSS px, which is
    // BELOW the edge PASS bound (1.0 CSS px = 3.125 device px). The gate is
    // deliberately set there to forgive cross-rasterizer corner rounding, so the
    // HONEST verdict for a 1-device-px diff is PASS (spec §1.10's "PASS if truly
    // sub-CSS-px" branch).
    //
    // DISCREPANCY (reported): the §5.1 row annotates this "(PARTIAL)". That is the
    // OTHER branch of the spec's own either/or and is unreachable for a 1-device-px
    // difference under G_EDGE_CSS.0 = 1.0. We assert the reachable, honest PASS and
    // additionally prove (in golden_box_offby1_css) that a real ONE-CSS-PX error
    // IS caught -> the off-by-one *detection* the harness exists for is intact.
    let mut reference = canvas(520, 520);
    fill(&mut reference, 5, 5, 504, 504, BLACK); // 500x500
    let mut cand = canvas(520, 520);
    fill(&mut cand, 5, 5, 505, 505, BLACK); // 501x501 (+1 device px R & B)
    let o = run(&cand, &reference);
    dump("box_offby1", &o);
    let band = 1.0 / CSS_PX; // 0.32
    assert!(
        o.tally.edge_max_css <= band + 0.05,
        "edge_max_css ~= {band:.2}, got {:.3}",
        o.tally.edge_max_css
    );
    assert_eq!(
        o.status,
        Status::Pass,
        "a 1-device-px (0.32 CSS px) diff is below the edge gate -> PASS"
    );
}

#[test]
fn golden_box_offby1_css() {
    // Companion to box_offby1: a real ONE-CSS-PX (≈3.125 device px, rounded to 4)
    // size error. edge_max ~= 1.28 CSS px -> above the PASS bound, within PARTIAL
    // -> NOT PASS. This is the off-by-one error the harness MUST catch (and the
    // sub-device-px case above MUST forgive).
    let mut reference = canvas(520, 520);
    fill(&mut reference, 5, 5, 504, 504, BLACK);
    let mut cand = canvas(520, 520);
    fill(&mut cand, 5, 5, 508, 508, BLACK); // +4 device px R & B ~= 1.28 CSS px
    let o = run(&cand, &reference);
    dump("box_offby1_css", &o);
    assert_ne!(
        o.status,
        Status::Pass,
        "a 1-CSS-px size error must NOT pass"
    );
    assert!(
        o.tally.edge_max_css > 1.0,
        "edge_max_css must clear the PASS bound, got {:.3}",
        o.tally.edge_max_css
    );
}

#[test]
fn golden_recolor_c00_d00() {
    // Fill #cc0000 vs #dd0000. The whole box is recoloured -> ~100% ColorErr ->
    // color_pct >> the ColorErr PARTIAL bound (8%) -> FAIL, dominant ColorErr.
    //
    // DISCREPANCY (reported): the §5.1 row says "ΔE>6". The ACTUAL ΔE2000 between
    // #cc0000 and #dd0000 is ~3.56 (verified), NOT >6 — so the hard-colour gate
    // (COLOR_DE_FAIL=6) does NOT fire; the FAIL comes from the ColorErr AREA gate
    // instead. We assert the real ΔE band (~3.0..5.0). NB: this also exposes that
    // the YIQ t_match budget (~46 for this pair) would have laundered the recolour
    // as Match; the classifier's added per-pixel ΔE>JND check is what makes the
    // colour error visible at all.
    let mut reference = box_frame_colored(Rgba([0xdd, 0, 0, 255]));
    let cand = box_frame_colored(Rgba([0xcc, 0, 0, 255]));
    let _ = &mut reference;
    let o = run(&cand, &reference);
    dump("recolor_c00_d00", &o);
    assert_eq!(o.status, Status::Fail, "a full-area recolour must FAIL");
    assert_eq!(
        o.verdict.dominant_class,
        PixelClass::ColorErr,
        "dominant must be ColorErr"
    );
    assert!(
        o.tally.color_de > 2.5 && o.tally.color_de < 5.0,
        "ΔE for #cc0000 vs #dd0000 is ~3.56, got {:.3}",
        o.tally.color_de
    );
}

#[test]
fn golden_colorspace_gamma() {
    // sRGB-encoded vs linear-light grey gradient. The mid-tones diverge enough to
    // exceed the JND -> ColorErr over a large area -> NOT PASS. (The ColorSpace
    // sub-classification itself is C4; at C3 we assert status != PASS + ColorErr.)
    let w = 200u32;
    let h = 120u32;
    let mut srgb = canvas(w, h);
    let mut lin = canvas(w, h);
    for x in 0..w {
        let t = x as f64 / (w as f64 - 1.0); // 0..1 ramp
        // sRGB reference: value == t (already display-encoded).
        let s = (t * 255.0).round().clamp(0.0, 255.0) as u8;
        // linear candidate: same light intensity but display-encoded differently
        // (apply the sRGB OETF to the linear value) -> the classic gamma drift.
        let enc = if t <= 0.0031308 {
            t * 12.92
        } else {
            1.055 * t.powf(1.0 / 2.4) - 0.055
        };
        let l = (enc * 255.0).round().clamp(0.0, 255.0) as u8;
        for y in 10..h - 10 {
            srgb.put_pixel(x, y, Rgba([s, s, s, 255]));
            lin.put_pixel(x, y, Rgba([l, l, l, 255]));
        }
    }
    let o = run(&lin, &srgb);
    dump("colorspace_gamma", &o);
    assert_ne!(
        o.status,
        Status::Pass,
        "a gamma/colour-space drift must NOT pass"
    );
    assert_eq!(
        o.verdict.dominant_class,
        PixelClass::ColorErr,
        "dominant must be ColorErr"
    );
}

#[test]
fn golden_opacity_half() {
    // Candidate paints an OPAQUE red box; reference paints the SAME red at 50%
    // over white (= pink #ff8080). Both ink, aligned, large ΔE -> ColorErr over
    // the whole box -> FAIL. (Recovering α≈0.5 is the C4 AlphaCompositing
    // sub-classifier; at C3 we assert FAIL + ColorErr, the honest C3 signal.)
    let cand = box_frame_colored(Rgba([255, 0, 0, 255])); // opaque red
    let reference = box_frame_colored(Rgba([255, 128, 128, 255])); // red @ 0.5 over white
    let o = run(&cand, &reference);
    dump("opacity_half", &o);
    assert_eq!(o.status, Status::Fail, "uncomposited opacity must FAIL");
    assert_eq!(
        o.verdict.dominant_class,
        PixelClass::ColorErr,
        "dominant must be ColorErr"
    );
    assert!(
        o.tally.color_de >= 6.0,
        "0.5-blend ΔE must be large, got {:.3}",
        o.tally.color_de
    );
}

#[test]
fn golden_missing_box() {
    // Reference paints a box; candidate is blank -> 100% Missing -> FAIL.
    let reference = box_frame();
    let cand = canvas(200, 200);
    let o = run(&cand, &reference);
    dump("missing_box", &o);
    assert_eq!(o.status, Status::Fail, "a blank candidate must FAIL");
    assert_eq!(
        o.verdict.dominant_class,
        PixelClass::Missing,
        "dominant must be Missing"
    );
    assert!(
        o.tally.missing_pct >= 50.0,
        "missing_pct must be ~100, got {:.2}",
        o.tally.missing_pct
    );
}

#[test]
fn golden_extra_box() {
    // Candidate paints a box; reference is blank -> Extra -> FAIL (well over the
    // extra PARTIAL bound).
    let cand = box_frame();
    let reference = canvas(200, 200);
    let o = run(&cand, &reference);
    dump("extra_box", &o);
    assert!(
        o.status == Status::Fail || o.status == Status::Partial,
        "extra paint must NOT pass"
    );
    assert_eq!(
        o.verdict.dominant_class,
        PixelClass::Extra,
        "dominant must be Extra"
    );
    assert!(
        o.tally.extra_pct > 6.0,
        "extra_pct must exceed the partial bound, got {:.2}",
        o.tally.extra_pct
    );
}

#[test]
fn golden_pure_glyph_aa() {
    // The SAME anti-aliased edge in both images, jittered by a sub-pixel amount:
    // both have a soft (intermediate-value) boundary column, and the candidate's
    // ramp value differs by a small AA-budget amount (grey96 vs grey128, YIQ ~517:
    // above t_match but within t_aa). Those pixels sit IN the shared edge band ->
    // AaEdge only -> PASS (the AaOnly measurement ceiling, not a bug). A FULL
    // black->grey jump (YIQ ~8279) would correctly EXCEED the AA budget and score;
    // genuine rasterizer jitter is a small perturbation of already-soft edges.
    let mut a = canvas(200, 200);
    fill(&mut a, 40, 40, 159, 159, BLACK);
    let mut b = a.clone();
    // Soft (anti-aliased) left boundary in BOTH, jittered between them.
    for y in 41..=158 {
        a.put_pixel(40, y, Rgba([96, 96, 96, 255]));
        b.put_pixel(40, y, Rgba([128, 128, 128, 255]));
    }
    let o = run(&b, &a);
    dump("pure_glyph_aa", &o);
    assert_eq!(o.status, Status::Pass, "pure shared-edge AA must PASS");
    assert!(
        o.tally.color_pct == 0.0,
        "no ColorErr on pure AA, got {:.3}",
        o.tally.color_pct
    );
    assert!(
        o.tally.aa_pct > 0.0,
        "the AA pixels must be counted as AaEdge, got {:.3}",
        o.tally.aa_pct
    );
}

#[test]
fn golden_wrong_font() {
    // Wrong font/weight proxy: a THICK black stroke (reference, "bold") vs a THIN
    // black stroke (candidate, "regular") at the same baseline. The extra stroke
    // thickness is Missing ink OFF the shared edge band (interior pixels, not a
    // 1px AA ramp) -> NOT PASS via Missing.
    let mut reference = canvas(200, 80);
    fill(&mut reference, 20, 30, 179, 49, BLACK); // 20px-thick bar
    let mut cand = canvas(200, 80);
    fill(&mut cand, 20, 36, 179, 43, BLACK); // 8px-thick bar (same centre)
    let o = run(&cand, &reference);
    dump("wrong_font", &o);
    assert_ne!(o.status, Status::Pass, "wrong stroke weight must NOT pass");
    assert!(
        matches!(
            o.verdict.dominant_class,
            PixelClass::Missing | PixelClass::ColorErr
        ),
        "wrong weight surfaces as Missing/ColorErr, got {:?}",
        o.verdict.dominant_class
    );
    assert!(
        o.tally.missing_pct > 6.0,
        "stroke-thickness Missing must exceed the partial bound, got {:.2}",
        o.tally.missing_pct
    );
}

#[test]
fn golden_miter_vs_square_corner() {
    // A border corner: reference is a MITERED (filled triangle) join; candidate is
    // a BUTT join (the triangle absent). The corner triangle is Missing ink that
    // does NOT lie in the shared edge band (the two corner geometries' edges do
    // not coincide) -> NOT PASS. This is the case the old 1px tolerance laundered.
    let mut reference = canvas(120, 120);
    // Two thick arms forming an L.
    fill(&mut reference, 10, 10, 30, 109, BLACK); // vertical arm
    fill(&mut reference, 10, 10, 109, 30, BLACK); // horizontal arm
    // Mitered fill of the outer corner triangle.
    for y in 31..=60 {
        for x in 31..=60 {
            if (x - 31) + (y - 31) <= 29 {
                reference.put_pixel(x, y, BLACK);
            }
        }
    }
    let mut cand = canvas(120, 120);
    fill(&mut cand, 10, 10, 30, 109, BLACK);
    fill(&mut cand, 10, 10, 109, 30, BLACK); // no miter triangle (butt join)
    let o = run(&cand, &reference);
    dump("miter_vs_square_corner", &o);
    assert_ne!(
        o.status,
        Status::Pass,
        "a mitered-vs-butt corner must NOT pass"
    );
}

#[test]
fn golden_calibration_drift() {
    // assert_calibration's pure offset check under the @page{margin:0} geometry:
    // content is origin-aligned in both engines, so the expected page-origin offset
    // is GLOBAL_OFFSET=(0,0). A probe whose RAW cand-vs-ref offset is (0,0)±1 passes;
    // any larger offset is a real margin/origin regression -> Err (the live run
    // aborts). Tested on synthetic bboxes (no rendering).
    let cand_bb = (10u32, 10u32, 110u32, 110u32);
    let good_ref = (10u32, 10u32, 110u32, 110u32); // cand - ref = (0,0)
    assert!(
        check_probe_offset(cand_bb, good_ref).is_ok(),
        "a clean (0,0) probe offset must pass calibration"
    );

    // Drifted probe: cand - ref == +4,+4 -> outside (0,0)±1 -> Err.
    let drift_ref = (6u32, 6u32, 106u32, 106u32);
    let res = check_probe_offset(cand_bb, drift_ref);
    assert!(
        res.is_err(),
        "a (4,4) raw offset must be reported as drift now, got {res:?}"
    );

    // A scale (not a pure translation) -> Err even if TL is in band.
    // cand - ref: TL (0,0), BR +10 -> non-uniform.
    let scaled_ref = (10u32, 10u32, 100u32, 100u32);
    assert!(
        check_probe_offset(cand_bb, scaled_ref).is_err(),
        "a non-uniform (scale) offset must be reported as drift"
    );
}

/// A 120x120 box of colour `c` centred in a 200x200 frame.
fn box_frame_colored(c: Rgba<u8>) -> RgbaImage {
    let mut img = canvas(200, 200);
    fill(&mut img, 40, 40, 159, 159, c);
    img
}
