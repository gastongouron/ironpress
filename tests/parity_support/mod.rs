//! ironpress feature-parity engine (core).
//!
//! This module is included by `tests/feature_parity.rs` via `#[path]`. It is the
//! implementation of the parity test driver. It is deliberately self-contained:
//! it shells out only to `pdftoppm` (poppler) at test time and never invokes
//! Chrome (references are pre-generated and committed by
//! `scripts/parity-gen-refs.sh`).
//!
//! Pipeline per fixture:
//!   render in-process (Letter + 28.8pt margins) -> validity check -> temp PDF
//!   -> `pdftoppm` -> decode candidate + committed reference (image crate)
//!   -> apply the fixed page-origin calibration to the candidate -> run the V2
//!      multi-detector pipeline (`compare::compare_v2`): content masks, per-side
//!      bbox-extent deltas, per-pixel class map, region segmentation, per-class
//!      tally, and the multi-gate PASS/PARTIAL/FAIL verdict
//!   -> write the classed-diff overlay -> aggregate weighted scores.
//!
//! The legacy best-shift/close-match comparator was removed in C6: the V2
//! multi-gate verdict is now the ONLY scoring path (revert safety net = the git
//! tag `harness-pre-v2`).
//!
//! The engine ALWAYS writes `report.json` + `REPORT.md`, then enforces the
//! regression gate against the committed baseline `report.json` (loaded before
//! any write): it fails the test only on an overall-score regression beyond
//! EPSILON, or a named PASS->FAIL transition. Missing baseline => first run
//! (write baseline, pass). A missing reference or a missing `pdftoppm` yields
//! UNKNOWN and never fails CI. A single fixture error never aborts the run.
//!
//! The engine is split into single-responsibility submodules (C1 mechanical
//! split). This `mod.rs` is the thin orchestrator: it wires `run()`'s top-level
//! flow and the per-fixture pipeline; all algorithms live in the submodules.

mod calibrate;
mod compare;
mod config;
mod diagnose;
mod gate;
mod geom;
mod manifest;
mod overlay;
mod rasterize;
mod render;
mod report;
mod util;
mod verify;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;

use calibrate::{assert_calibration, calibrate};
use compare::compare_v2;
use diagnose::compute_attribution;
use gate::{
    build_report, check_coords_freshness, check_refs_freshness, collect_suspect_unsupported_pass,
    compute_coverage, compute_fix_first, enforce_gate,
};
use manifest::{ManifestEntry, find_ref_mismatches, load_manifests};
use render::{SharedFonts, check_pdf_valid, load_bundled_fonts, render_pdf};
use report::{
    FixtureResult, Report, Status, fixture_fail, fixture_unknown, write_html_reports,
    write_report_json, write_report_md,
};
use util::{sha256_hex, which};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let parity_dir = root.join("tests").join("parity");
    let manifest_dir = parity_dir.join("manifest");
    let cases_dir = parity_dir.join("cases");
    let refs_dir = parity_dir.join("refs");
    let diffs_dir = parity_dir.join("diffs");
    let out_dir = parity_dir.join("out");
    let reports_dir = parity_dir.join("reports");
    let tmp_dir = root.join("target").join("parity-tmp");
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("cannot create temp dir {}: {e}", tmp_dir.display()))?;

    // Load committed baseline BEFORE writing anything.
    let baseline_path = parity_dir.join("report.json");
    let baseline: Option<Report> = std::fs::read_to_string(&baseline_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());

    let pdftoppm_available = which("pdftoppm");
    if !pdftoppm_available {
        eprintln!(
            "parity: WARNING pdftoppm not found on PATH; all fixtures will be UNKNOWN (not gating)."
        );
    }

    // Discover + parse manifests.
    let mut entries = match load_manifests(&manifest_dir, &parity_dir) {
        Ok(e) => e,
        Err(e) => return Err(e),
    };
    if entries.is_empty() {
        eprintln!(
            "parity: no manifest entries found under {} (nothing to do).",
            manifest_dir.display()
        );
    }

    // Verdict path: the V2 multi-gate comparator is the ONLY path (C6). The legacy
    // best-shift/close-match comparator and its `PARITY_VERDICT` escape hatch were
    // removed — the git tag `harness-pre-v2` is the revert safety net.

    // Fast dev loop (amendment A3): `PARITY_ONLY` is a comma list of substrings;
    // when set, process only fixtures whose `<category>/<id>` contains any
    // substring, and SKIP the regression gate (these are partial runs and must
    // never inform the baseline). Empty/unset => normal full run.
    let only_filter: Vec<String> = std::env::var("PARITY_ONLY")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let filtered_run = !only_filter.is_empty();
    if filtered_run {
        entries.retain(|e| {
            let key = format!("{}/{}", e.category, e.id);
            only_filter.iter().any(|f| key.contains(f.as_str()))
        });
        eprintln!(
            "parity: PARITY_ONLY={:?} -> {} fixture(s); regression gate SKIPPED (dev run).",
            only_filter,
            entries.len()
        );
    }

    // Load the bundled font bytes ONCE into shared immutable data so the heavy
    // per-fixture work can run in parallel without re-reading the faces from disk
    // per render and without sharing any mutable converter across threads.
    let shared_fonts: SharedFonts = Arc::new(load_bundled_fonts());

    // V2 calibration audit: render the rigid probes once and verify the page-origin
    // offset is the expected fixed translation, BEFORE scoring any fixture. Drift
    // aborts the run loudly. Skipped when pdftoppm is unavailable (nothing renders)
    // or on a filtered dev run (probes may not be selected).
    let calibration = if pdftoppm_available && !filtered_run {
        match assert_calibration(&entries, &parity_dir, &refs_dir, &tmp_dir, &shared_fonts) {
            Ok(c) => Some(c),
            Err(e) => return Err(e),
        }
    } else {
        None
    };

    // Heavy per-fixture work (ironpress render -> pdftoppm raster -> image decode
    // -> bbox/diff -> classify) is embarrassingly parallel: each fixture builds
    // its OWN HtmlConverter and shells pdftoppm to a per-fixture-UNIQUE temp path
    // (keyed on `entry.id`), so jobs never collide. We size the rayon pool to
    // min(nproc-2, 8) to leave headroom, keep `catch_unwind` per fixture, then
    // SORT the collected results by (category, id) so report.json / REPORT.md are
    // byte-identical regardless of thread scheduling. All scoring / attribution /
    // guard / gate logic downstream is unchanged.
    let pool_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .saturating_sub(2)
        .clamp(1, 8);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(pool_threads)
        .build()
        .map_err(|e| format!("cannot build rayon pool: {e}"))?;

    let mut results: Vec<FixtureResult> = pool.install(|| {
        entries
            .par_iter()
            .map(|entry| {
                let fonts = Arc::clone(&shared_fonts);
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    process_entry(
                        entry,
                        &parity_dir,
                        &cases_dir,
                        &refs_dir,
                        &diffs_dir,
                        &out_dir,
                        &reports_dir,
                        &tmp_dir,
                        pdftoppm_available,
                        &fonts,
                    )
                }))
                .unwrap_or_else(|_| {
                    fixture_fail(entry, 100.0, "panic during processing".to_string())
                });
                eprintln!(
                    "parity: {:8} {:>7.4}%  {}/{}  {}",
                    res.status.as_str(),
                    res.diff_pct,
                    res.category,
                    res.id,
                    res.note
                );
                res
            })
            .collect()
    });

    // Determinism: fix a stable order independent of thread scheduling before any
    // scoring / reporting. `build_report` re-sorts too, but attribution / fix_first
    // / guards iterate `results` directly, so normalize here first.
    results.sort_by(|a, b| {
        (a.category.as_str(), a.id.as_str()).cmp(&(b.category.as_str(), b.id.as_str()))
    });

    // Substrate-probe attribution: name the root cause of each non-PASS fixture.
    compute_attribution(&mut results);
    let fix_first = compute_fix_first(&results);

    // Guards (surfaced, non-gating): id!=ref-filename mismatches and
    // unsupported-but-PASS suspects.
    let ref_mismatches = find_ref_mismatches(&entries, &refs_dir);
    let suspect_unsupported_pass = collect_suspect_unsupported_pass(&results);

    // refs.lock freshness check (reads the lock written by gen-refs; we only
    // READ + verify). Non-gating here — surfaced in report.json + REPORT.md + a
    // loud WARNING line; CI enforces the hard fail.
    let (stale_refs, refs_lock_present) = check_refs_freshness(&parity_dir, &results);
    // Sidecar (coords.lock) freshness — same machinery, only sidecar-bearing
    // fixtures tracked (Phase 2b ships the starter set). Non-gating; surfaced.
    let (stale_coords, coords_lock_present) = check_coords_freshness(&parity_dir, &results);

    let mut report = build_report(results, pdftoppm_available);
    report.coverage = compute_coverage(&report);
    report.fix_first = fix_first;
    report.ref_mismatches = ref_mismatches;
    report.suspect_unsupported_pass = suspect_unsupported_pass;
    report.stale_refs = stale_refs;
    report.refs_lock_present = refs_lock_present;
    report.stale_coords = stale_coords;
    report.coords_lock_present = coords_lock_present;
    report.calibration = calibration;

    // A filtered dev run (`PARITY_ONLY`) scores only a subset, so it must NOT
    // overwrite the committed baseline `report.json` / `REPORT.md` (that would
    // corrupt the baseline) and must NOT enforce the gate. Print the summary and
    // return early.
    if filtered_run {
        if let Err(e) = write_html_reports(&reports_dir, &cases_dir, &report) {
            eprintln!("parity: WARNING could not write HTML reports: {e}");
        }
        println!(
            "parity (PARTIAL/dev): {}P/{}p/{}F/{}U over {} filtered fixture(s) — baseline NOT written, gate SKIPPED.",
            report.overall.pass,
            report.overall.partial,
            report.overall.fail,
            report.overall.unknown,
            report.overall.total
        );
        return Ok(());
    }

    // ALWAYS write report.json + REPORT.md.
    write_report_json(&baseline_path, &report)?;
    write_report_md(&parity_dir.join("REPORT.md"), &report)?;

    // Generate the in-repo per-theme visual HTML reports (diagnostic quad cards).
    if let Err(e) = write_html_reports(&reports_dir, &cases_dir, &report) {
        eprintln!("parity: WARNING could not write HTML reports: {e}");
    }

    println!(
        "parity: {:.2}% ({}P/{}p/{}F/{}U) · scored {:.2}% · report at {}",
        report.overall.score_pct,
        report.overall.pass,
        report.overall.partial,
        report.overall.fail,
        report.overall.unknown,
        report.overall.scored_ratio_pct,
        parity_dir.join("REPORT.md").display()
    );

    if !report.ref_mismatches.is_empty() {
        eprintln!(
            "parity: WARNING {} ref lookup mismatch(es) (id != ref-filename) — see REPORT.md.",
            report.ref_mismatches.len()
        );
    }
    if !report.suspect_unsupported_pass.is_empty() {
        eprintln!(
            "parity: WARNING {} unsupported-but-PASS suspect(s): {} — see REPORT.md.",
            report.suspect_unsupported_pass.len(),
            report.suspect_unsupported_pass.join(", ")
        );
    }
    if !report.refs_lock_present {
        eprintln!(
            "parity: WARNING no refs.lock committed — reference freshness is UNVERIFIED. \
             Run scripts/parity-gen-refs.sh to write the lock."
        );
    } else if !report.stale_refs.is_empty() {
        let ids: Vec<&str> = report.stale_refs.iter().map(|s| s.id.as_str()).collect();
        eprintln!(
            "parity: WARNING {} STALE reference(s) (fixture changed since ref was generated) — \
             regenerate with scripts/parity-gen-refs.sh: {}",
            report.stale_refs.len(),
            ids.join(", ")
        );
    }
    if report.coords_lock_present && !report.stale_coords.is_empty() {
        let ids: Vec<&str> = report.stale_coords.iter().map(|s| s.id.as_str()).collect();
        eprintln!(
            "parity: WARNING {} STALE coordinate sidecar(s) (fixture changed since sidecar was \
             generated) — regenerate with scripts/parity-gen-coords.sh: {}",
            report.stale_coords.len(),
            ids.join(", ")
        );
    }

    // Regression gate.
    enforce_gate(baseline.as_ref(), &report)
}

// ---------------------------------------------------------------------------
// Per-fixture processing
// ---------------------------------------------------------------------------

/// Reference PNG path for page `page` of a fixture. Page 1 is `<id>.png` (the
/// legacy single-page name, so the entire existing corpus stays valid); pages
/// 2.. are `<id>.pN.png`.
fn ref_page_path(refs_dir: &Path, category: &str, id: &str, page: usize) -> std::path::PathBuf {
    let name = if page <= 1 {
        format!("{id}.png")
    } else {
        format!("{id}.p{page}.png")
    };
    refs_dir.join(category).join(name)
}

/// Count committed reference pages for a fixture: 0 when there is no `<id>.png`,
/// otherwise 1 plus the run of consecutive `<id>.pN.png` (N = 2, 3, …) present.
fn count_ref_pages(refs_dir: &Path, category: &str, id: &str) -> usize {
    if !ref_page_path(refs_dir, category, id, 1).is_file() {
        return 0;
    }
    let mut n = 1;
    while ref_page_path(refs_dir, category, id, n + 1).is_file() {
        n += 1;
    }
    n
}

#[allow(clippy::too_many_arguments)]
fn process_entry(
    entry: &ManifestEntry,
    parity_dir: &Path,
    _cases_dir: &Path,
    refs_dir: &Path,
    diffs_dir: &Path,
    out_dir: &Path,
    reports_dir: &Path,
    tmp_dir: &Path,
    pdftoppm_available: bool,
    fonts: &[(&'static str, Vec<u8>)],
) -> FixtureResult {
    let fixture = parity_dir.join(&entry.file);
    let html = match std::fs::read_to_string(&fixture) {
        Ok(h) => h,
        Err(e) => return fixture_fail(entry, 100.0, format!("cannot read fixture: {e}")),
    };
    // SHA-256 of the fixture HTML for the refs.lock freshness check. Computed
    // once here so every result (even UNKNOWN/error paths via the `with_sha`
    // closure below) carries it.
    let html_sha = sha256_hex(html.as_bytes());
    // Helper: stamp the sha onto any result we return from this function.
    let with_sha = |mut r: FixtureResult| -> FixtureResult {
        r.html_sha256 = html_sha.clone();
        r
    };

    // In-process render at Chrome-matching geometry. The fixture's own directory
    // is the base for resolving relative resource URLs (e.g. `@font-face` `src`).
    let base_path = fixture.parent();
    let pdf = match render_pdf(&html, entry.sanitize, fonts, base_path) {
        Ok(p) => p,
        Err(e) => return with_sha(fixture_fail(entry, 100.0, format!("render error: {e}"))),
    };

    // PDF validity guard (mirror pdf_smoke_tests).
    if let Err(e) = check_pdf_valid(&pdf) {
        return with_sha(fixture_fail(entry, 100.0, format!("malformed PDF: {e}")));
    }

    let pdf_path = tmp_dir.join(format!("{}.pdf", entry.id));
    if let Err(e) = std::fs::write(&pdf_path, &pdf) {
        return with_sha(fixture_fail(
            entry,
            100.0,
            format!("cannot write temp pdf: {e}"),
        ));
    }

    // The committed candidate raster path (LFS). Written below whenever we
    // successfully rasterize, so the in-repo visual reports always have an
    // ironpress image to show even for non-scored (UNKNOWN-ref) fixtures.
    let out_png = out_dir
        .join(&entry.category)
        .join(format!("{}.png", entry.id));

    if !pdftoppm_available {
        return with_sha(fixture_unknown(entry, "pdftoppm unavailable".to_string()));
    }

    // Rasterize ALL candidate pages (pagination support). Page 1 drives the
    // existing single-page comparison + verify pipeline below; the page COUNT and
    // pages 2.. are folded into the verdict afterward (see the multi-page block).
    let cand_page_paths = match rasterize::rasterize_all_pages(&pdf_path, tmp_dir, &entry.id) {
        Ok(p) => p,
        Err(e) => return with_sha(fixture_fail(entry, 100.0, format!("pdftoppm failed: {e}"))),
    };
    let cand_page_count = cand_page_paths.len();

    // Decode page 1 (the primary candidate) and persist every page to the committed
    // `out/` tree (page 1 -> <id>.png, page N>=2 -> <id>.pN.png).
    let cand = match image::open(&cand_page_paths[0]) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            return with_sha(fixture_fail(
                entry,
                100.0,
                format!("decode candidate failed: {e}"),
            ));
        }
    };
    if let Some(parent) = out_png.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = cand.save(&out_png);
    for (i, p) in cand_page_paths.iter().enumerate().skip(1) {
        if let Ok(img) = image::open(p) {
            let extra = out_dir
                .join(&entry.category)
                .join(format!("{}.p{}.png", entry.id, i + 1));
            let _ = img.to_rgba8().save(&extra);
        }
    }

    // Reference lookup. Absent => UNKNOWN (never gates). Candidate already
    // committed above so the report still shows the ironpress render.
    let ref_path = refs_dir
        .join(&entry.category)
        .join(format!("{}.png", entry.id));
    if !ref_path.is_file() {
        return with_sha(fixture_unknown(
            entry,
            "no reference (run scripts/parity-gen-refs.sh)".to_string(),
        ));
    }

    let reference = match image::open(&ref_path) {
        Ok(i) => i.to_rgba8(),
        // A corrupt/truncated reference (e.g. a gen-refs run killed mid-rasterize)
        // must NOT be scored as a 100% FAIL — that would gate CI and pollute the
        // baseline on a tooling glitch. Treat it as UNKNOWN (non-gating);
        // re-running gen-refs regenerates it.
        Err(e) => {
            return with_sha(fixture_unknown(
                entry,
                format!("reference unreadable (regenerate): {e}"),
            ));
        }
    };

    // V2 PATH (the only verdict path after C6): apply the fixed page-origin
    // calibration, then run the §1.2 multi-detector pipeline. The verdict's
    // status/diff_pct are the score; the classed overlay is the committed diff.
    let cand_cal = calibrate(&cand);
    let outcome = compare_v2(&cand_cal, &reference, entry);
    let diff_pct = util::round4(outcome.diff_pct);

    // Per-class breakdown for tuning (set PARITY_DEBUG_TALLY=1). Non-gating.
    if std::env::var("PARITY_DEBUG_TALLY").is_ok() {
        let t = &outcome.tally;
        eprintln!(
            "tally {}/{}: color={:.2}% (ΔE {:.2}) missing={:.2}% extra={:.2}% edge_max={:.2}css shift_max={:.2}css aa={:.2}% dom={:?}",
            entry.category,
            entry.id,
            t.color_pct,
            t.color_de,
            t.missing_pct,
            t.extra_pct,
            t.edge_max_css,
            t.shift_max_css,
            t.aa_pct,
            outcome.verdict.dominant_class
        );
        eprintln!(
            "DIAG {}/{}: STATUS={} [{}] {}  (conf {:.2}) interior_color%={:.3} interior_de={:.2}",
            entry.category,
            entry.id,
            outcome.status.as_str(),
            outcome.diagnosis.primary_class,
            outcome.diagnosis.headline,
            outcome.diagnosis.confidence,
            outcome.tally.interior_color_pct,
            outcome.tally.interior_color_de
        );
    }

    let reports_diff = reports_dir
        .join(&entry.category)
        .join(format!("{}.diff.png", entry.id));
    if let Some(parent) = reports_diff.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = outcome.overlay.save(&reports_diff);
    if outcome.status != Status::Pass {
        let out = diffs_dir
            .join(&entry.category)
            .join(format!("{}.png", entry.id));
        if let Some(parent) = out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = outcome.overlay.save(&out);
    }
    // Pluggable multi-verifier seam (spec §1.4). PHASE 1 = a PROVABLE NO-OP: the
    // only verifier present is the `RasterVerifier` adapter, which re-partitions
    // the ALREADY-COMPUTED `outcome` (its `tally`/`verdict`) into three concern
    // sub-verdicts using the SAME `config.rs` gates — it does NOT re-run the
    // comparator. With only the RasterVerifier, the combiner's WORST-of-concern
    // status reproduces `outcome.status` exactly (see `verify/raster.rs` for the
    // equivalence and `verify/goldens.rs` for the proof), so the committed
    // baseline does not move. The Phase-2 `PdfGeometry` verifier (which reads the
    // PDF bytes + sidecar) plugs into this same `verifiers` list; the ctx already
    // carries the artifacts it will need.
    // PHASE 2a: load the committed coordinate sidecar (if any). NO sidecar files
    // exist yet, so this is `None` for every fixture -> `PdfGeomVerifier.applies()`
    // is false everywhere and the combined status is still byte-identical to today
    // (proven no-op). Sidecar generation is Phase 2b.
    let coords = verify::coords::load_coords_sidecar(parity_dir, entry);
    let ctx = verify::VerifyCtx {
        entry,
        pdf: &pdf,
        cand: &cand_cal,
        reference: &reference,
        coords: coords.as_ref(),
    };
    let raster_verifier = verify::raster::RasterVerifier::from_outcome(&outcome, entry);
    let pdf_geom_verifier = verify::pdf_geom::PdfGeomVerifier;
    let verifiers: [&dyn verify::Verifier; 2] = [&raster_verifier, &pdf_geom_verifier];
    let mut subs: Vec<verify::SubVerdict> = Vec::new();
    for v in verifiers {
        if v.applies(&ctx) {
            subs.extend(v.verify(&ctx));
        }
    }
    let combined = verify::combine::combine(&subs);

    // Surface the per-verifier sub-verdicts (incl. the new PdfGeometry axis) under
    // the same PARITY_DEBUG_TALLY flag the raster tally uses — non-gating, dev only.
    if std::env::var("PARITY_DEBUG_TALLY").is_ok() {
        for s in &subs {
            eprintln!(
                "subverdict {}/{}: {:?} {:?}={} mag={:.3} :: {}",
                entry.category,
                entry.id,
                s.verifier,
                s.concern,
                s.status.as_str(),
                s.magnitude,
                s.headline
            );
        }
        for d in &combined.disagreements {
            eprintln!(
                "disagree   {}/{}: {:?} auth={}({:?}) chal={}({:?}) :: {}",
                entry.category,
                entry.id,
                d.concern,
                d.authoritative.as_str(),
                d.authoritative_by,
                d.challenger.as_str(),
                d.challenger_by,
                d.note
            );
        }
    }

    // STRICT MODE: a PARTIAL verdict means the render is WRONG (a real diff above
    // the PASS gates, not a forgiven sub-pixel match), so collapse it to FAIL at
    // the fixture level. The comparator/combiner still computes the PARTIAL band
    // internally (its goldens depend on it) — only the per-fixture parity verdict
    // is hardened, so the gate demands every fixture be a genuine PASS.
    let mut fixture_status = if combined.status == report::Status::Partial {
        report::Status::Fail
    } else {
        combined.status
    };

    // MULTI-PAGE PAGINATION (additive). Assert ironpress's page COUNT matches the
    // reference's, and compare pages 2.. against their `<id>.pN.png` references,
    // folding the WORST result into the fixture verdict. The entire legacy corpus
    // is single-page (ref_page_count == 1): the count matches and the loop runs
    // zero extra times, so those fixtures are byte-for-byte unaffected. This is
    // what makes a real page break testable — and is exactly the check that would
    // have caught the `page-break-after` trailing-blank-page bug (ironpress 2 pages
    // vs Chrome 1) that the page-1-only comparison silently passed.
    let mut diff_pct = diff_pct;
    let mut page_note = String::new();
    let ref_page_count = count_ref_pages(refs_dir, &entry.category, &entry.id);
    if ref_page_count >= 1 && cand_page_count != ref_page_count {
        fixture_status = report::Status::Fail;
        diff_pct = 100.0;
        page_note = format!(
            "page-count mismatch: ironpress {cand_page_count} vs reference {ref_page_count}"
        );
    } else if ref_page_count >= 2 {
        for page in 2..=ref_page_count {
            let ref_p = ref_page_path(refs_dir, &entry.category, &entry.id, page);
            let (rimg, cimg) = match (image::open(&ref_p), image::open(&cand_page_paths[page - 1]))
            {
                (Ok(r), Ok(c)) => (r.to_rgba8(), c.to_rgba8()),
                _ => {
                    fixture_status = report::Status::Fail;
                    page_note = format!("page {page} unreadable");
                    break;
                }
            };
            let page_outcome = compare_v2(&calibrate(&cimg), &rimg, entry);
            diff_pct = diff_pct.max(util::round4(page_outcome.diff_pct));
            let pdiff = reports_dir
                .join(&entry.category)
                .join(format!("{}.p{}.diff.png", entry.id, page));
            if let Some(parent) = pdiff.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = page_outcome.overlay.save(&pdiff);
            if page_outcome.status != report::Status::Pass {
                fixture_status = report::Status::Fail;
                if page_note.is_empty() {
                    page_note = format!("page {page}: {}", page_outcome.diagnosis.headline);
                }
            }
        }
    }

    // ADDITIVE: attach the V2 diagnosis (spec §2). The attribution prefix
    // (`via {dep}: …` for confounded fixtures) is applied later in `run()` by
    // `compute_attribution`, once every fixture's status is known.
    let mut result = report::fixture_base(entry, fixture_status, diff_pct, page_note);
    result.diagnosis = Some(outcome.diagnosis);
    result.sub_verdicts = subs;
    result.disagreements = combined.disagreements;
    with_sha(result)
}
