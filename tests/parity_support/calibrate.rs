//! Page-origin calibration (spec §1.3) — the SOLE replacement for the deleted
//! per-fixture best-shift registration.
//!
//! ironpress anchors content at the spec-correct 28.8pt = 120px@300dpi margin;
//! Chrome's `--print-to-pdf` rounds the printable margin to ~116px, so ironpress
//! content sits a rigid +4,+4 device px vs the Chrome reference on EVERY fixture.
//! We shift the candidate by `-GLOBAL_OFFSET` once, uniformly — never a
//! per-fixture search (that masked real layout bugs). We AUDIT the offset once
//! per run from the deterministic rigid probes and abort loudly on drift, so a
//! genuine margin regression is announced, never silently re-absorbed.

use std::path::Path;

use super::config::{GLOBAL_OFFSET, PROBE_JITTER_PX};
use super::geom::{BBox, content_bbox, shift_image};
use super::manifest::ManifestEntry;
use super::render::{SharedFonts, check_pdf_valid, render_pdf};
use super::report::Calibration;

/// Apply the fixed page-origin correction to a candidate: shift by
/// `-GLOBAL_OFFSET`, filling the exposed edges with white. This is the ONLY
/// geometric correction in the V2 verdict path.
pub(crate) fn calibrate(cand: &image::RgbaImage) -> image::RgbaImage {
    shift_image(cand, -GLOBAL_OFFSET.0, -GLOBAL_OFFSET.1)
}

/// Pure offset check (spec §1.3) — factored out so it is unit-testable on
/// synthetic bboxes without rendering. Given the RAW (un-shifted) content bboxes
/// of a probe's candidate and reference, verify the offset is a pure translation
/// of `GLOBAL_OFFSET ± PROBE_JITTER_PX` (proves zero scale). Returns the max
/// per-axis deviation from the expected offset on success, or an error string.
pub(crate) fn check_probe_offset(cand_bb: BBox, ref_bb: BBox) -> Result<i32, String> {
    // d = cand - ref (ironpress content sits +GLOBAL_OFFSET PAST the Chrome ref —
    // 120px margin vs ~116px — so the candidate box is at the LARGER coordinate;
    // `calibrate` then shifts the candidate by -GLOBAL_OFFSET to land on the ref).
    let dtl_x = cand_bb.0 as i32 - ref_bb.0 as i32;
    let dtl_y = cand_bb.1 as i32 - ref_bb.1 as i32;
    let dbr_x = cand_bb.2 as i32 - ref_bb.2 as i32;
    let dbr_y = cand_bb.3 as i32 - ref_bb.3 as i32;

    // Per-AXIS bounds (review #5): X deltas check vs GLOBAL_OFFSET.0, Y deltas vs
    // GLOBAL_OFFSET.1, each with its own expected value in the error message. The old
    // code derived a single (lo,hi) from GLOBAL_OFFSET.0 and applied it to all four
    // deltas including the Y pair (with .0 hard-coded in the message) — masked only
    // because GLOBAL_OFFSET is the symmetric (4,4); an asymmetric origin would
    // mis-gate the very calibration audit meant to catch a margin regression.
    for (label, v, expected) in [
        ("dTL.x", dtl_x, GLOBAL_OFFSET.0),
        ("dTL.y", dtl_y, GLOBAL_OFFSET.1),
        ("dBR.x", dbr_x, GLOBAL_OFFSET.0),
        ("dBR.y", dbr_y, GLOBAL_OFFSET.1),
    ] {
        if v < expected - PROBE_JITTER_PX || v > expected + PROBE_JITTER_PX {
            return Err(format!(
                "{label}={v} outside expected {expected}±{PROBE_JITTER_PX} \
                 (raw offset is not the page-origin translation)"
            ));
        }
    }
    // Pure translation, zero scale: |dTL - dBR| <= 2 per axis.
    if (dtl_x - dbr_x).abs() > 2 || (dtl_y - dbr_y).abs() > 2 {
        return Err(format!(
            "non-uniform offset (dTL=({dtl_x},{dtl_y}) dBR=({dbr_x},{dbr_y})) — scale/skew, not a translation"
        ));
    }

    let dev = [
        (dtl_x - GLOBAL_OFFSET.0).abs(),
        (dtl_y - GLOBAL_OFFSET.1).abs(),
        (dbr_x - GLOBAL_OFFSET.0).abs(),
        (dbr_y - GLOBAL_OFFSET.1).abs(),
    ];
    Ok(*dev.iter().max().unwrap())
}

/// Audit the page-origin offset once per V2 run from the rigid probes. Renders +
/// rasterizes each `kind=="probe" && geometry=="rigid"` entry, measures the raw
/// (un-shifted) offset, and requires every probe to be a pure `GLOBAL_OFFSET`
/// translation. On any violation, returns an `Err` (the run aborts loudly); on
/// success, returns the audited `Calibration`.
///
/// This is CALLED from `run()` on every scoring run (pdftoppm available, not a
/// filtered dev run); the golden tests exercise the pure `check_probe_offset`
/// directly (they do not render).
pub(crate) fn assert_calibration(
    entries: &[ManifestEntry],
    parity_dir: &Path,
    refs_dir: &Path,
    tmp_dir: &Path,
    fonts: &SharedFonts,
) -> Result<Calibration, String> {
    let mut max_dev = 0i32;
    let mut measured = GLOBAL_OFFSET;
    let mut probed = 0u32;

    for entry in entries
        .iter()
        .filter(|e| e.kind == "probe" && e.geometry == "rigid")
    {
        let fixture = parity_dir.join(&entry.file);
        let html = std::fs::read_to_string(&fixture)
            .map_err(|e| format!("calibration: cannot read probe {}: {e}", entry.id))?;
        let pdf = render_pdf(&html, entry.sanitize, fonts, fixture.parent())
            .map_err(|e| format!("calibration: render probe {} failed: {e}", entry.id))?;
        check_pdf_valid(&pdf)
            .map_err(|e| format!("calibration: probe {} PDF invalid: {e}", entry.id))?;

        let pdf_path = tmp_dir.join(format!("calib-{}.pdf", entry.id));
        std::fs::write(&pdf_path, &pdf)
            .map_err(|e| format!("calibration: cannot write probe pdf: {e}"))?;
        let cand_png = tmp_dir.join(format!("calib-{}.png", entry.id));
        super::rasterize::rasterize(
            &pdf_path,
            &cand_png,
            tmp_dir,
            &format!("calib-{}", entry.id),
        )
        .map_err(|e| format!("calibration: rasterize probe {} failed: {e}", entry.id))?;
        let cand = image::open(&cand_png)
            .map_err(|e| format!("calibration: decode probe {} failed: {e}", entry.id))?
            .to_rgba8();

        let ref_path = refs_dir
            .join(&entry.category)
            .join(format!("{}.png", entry.id));
        let reference = image::open(&ref_path)
            .map_err(|e| {
                format!(
                    "calibration: probe {} has no/unreadable reference: {e}",
                    entry.id
                )
            })?
            .to_rgba8();

        let cand_bb = content_bbox(&cand)
            .ok_or_else(|| format!("calibration: probe {} candidate is blank", entry.id))?;
        let ref_bb = content_bbox(&reference)
            .ok_or_else(|| format!("calibration: probe {} reference is blank", entry.id))?;

        match check_probe_offset(cand_bb, ref_bb) {
            Ok(dev) => {
                // Record `measured_px` from the probe that produced the MAX deviation
                // (review #11) — the old code overwrote it every probe, so it reported
                // the LAST probe's offset, inconsistent with the aggregated `max_dev`.
                if dev >= max_dev {
                    max_dev = dev;
                    measured = (
                        cand_bb.0 as i32 - ref_bb.0 as i32,
                        cand_bb.1 as i32 - ref_bb.1 as i32,
                    );
                }
                probed += 1;
            }
            Err(why) => {
                return Err(format!(
                    "calibration drift: probe {} raw offset check failed: {why}; expected ({},{}); \
                     Chrome refs regenerated or ironpress margin regressed — refusing to re-absorb.",
                    entry.id, GLOBAL_OFFSET.0, GLOBAL_OFFSET.1
                ));
            }
        }
    }

    if probed == 0 {
        return Err(
            "calibration: no rigid probes found (need kind==\"probe\" && geometry==\"rigid\"); \
             cannot audit the page-origin offset."
                .to_string(),
        );
    }

    Ok(Calibration {
        offset_px: [GLOBAL_OFFSET.0, GLOBAL_OFFSET.1],
        offset_css: [
            GLOBAL_OFFSET.0 as f64 / super::config::CSS_PX,
            GLOBAL_OFFSET.1 as f64 / super::config::CSS_PX,
        ],
        measured_px: [measured.0, measured.1],
        residual_px: max_dev,
        drifted: false,
    })
}
