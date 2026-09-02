//! ironpress feature-parity engine (core).
//!
//! This module is the implementation of the parity test driver. It is
//! deliberately self-contained:
//! it shells out only to `pdftoppm` (poppler) at test time and never invokes
//! Chrome (oracle PDFs are pre-generated and committed by
//! `scripts/parity-gen-refs.sh`).
//!
//! Pipeline per fixture:
//!   render in-process (Letter + 28.8pt margins) -> validity check -> temp PDF
//!   -> rasterize candidate and committed oracle PDFs through the SAME runtime
//!      `pdftoppm` executable -> decode both (image crate) -> run the V2
//!      pipeline (`compare::compare_page_v2`): same-coordinate pixel classes, complete
//!      region aggregates, direct per-class tallies, raw exact evidence, and a
//!      same-coordinate human-visibility PASS/FAIL verdict
//!      verdict
//!   -> write the classed-diff overlay -> aggregate weighted scores.
//!
//! Exact raw raster equality is always measured and reported. The pass/fail
//! decision applies its fixed policy directly to those coordinates. Diagnostic
//! classes never search nearby pixels, filter either page, or infer a
//! translation.
//!
//! A full run validates the current corpus, publishes the honest current
//! `report.json`, and enforces the regression gate against the separate
//! committed `baseline.json`. Missing/malformed baselines, missing references,
//! every non-PASS fixture, disappeared fixtures, and every status downgrade fail
//! closed.
//! `PARITY_UPDATE_BASELINE=1` is the explicit, visible path for intentionally
//! accepting a new baseline; it still enforces corpus/reference integrity.
//!
//! The engine is split into single-responsibility submodules (C1 mechanical
//! split). This `mod.rs` is the thin orchestrator: it wires `run()`'s top-level
//! flow and the per-fixture pipeline; all algorithms live in the submodules.

mod compare;
mod config;
mod diagnose;
mod fontations_launcher;
mod gate;
mod geom;
mod integrity;
mod interaction_coverage;
mod manifest;
mod overlay;
mod rasterize;
mod refs_lock;
mod render;
mod report;
mod util;

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use diagnose::compute_dependency_context;
use gate::{
    BaselineState, baseline_is_compatible, build_report, check_refs_freshness,
    collect_suspect_unsupported_pass, compute_coverage, compute_fix_first, enforce_baseline_update,
    enforce_current_health, enforce_gate, load_baseline,
};
use integrity::{audit_corpus, audit_oracle_semantics, audit_raster_signals, raster_fingerprints};
use manifest::{ManifestEntry, find_ref_mismatches, load_manifests};
use rasterize::Rasterizer;
use render::{FontBundle, PinnedUaStylesheet, check_pdf_valid, load_bundled_fonts, render_pdf};
use report::{
    FixtureResult, Report, Status, fixture_fail, write_html_reports, write_report_artifacts,
    write_report_json, write_report_md,
};
use util::sha256_hex;

#[derive(Debug)]
struct RunPaths {
    refs: PathBuf,
    diffs: PathBuf,
    out: PathBuf,
    reports: PathBuf,
    tmp: PathBuf,
    diagnostic_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct InvocationId(String);

impl InvocationId {
    fn parse(value: String) -> Result<Self, String> {
        if !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            Ok(Self(value))
        } else {
            Err(format!(
                "PARITY_INVOCATION_ID must be 1-128 ASCII letters, digits, `.`, `_`, or `-`, got {value:?}"
            ))
        }
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
struct FixtureFilters(Vec<String>);

impl FixtureFilters {
    fn new(filters: Vec<String>) -> Result<Self, String> {
        if filters.is_empty() {
            Err("a filtered parity run requires at least one fixture filter".to_string())
        } else {
            Ok(Self(filters))
        }
    }

    fn as_slice(&self) -> &[String] {
        &self.0
    }
}

#[derive(Debug)]
enum RunConfig {
    Full {
        update_baseline: bool,
        invocation_id: InvocationId,
    },
    Filtered {
        filters: FixtureFilters,
    },
}

impl RunConfig {
    fn is_filtered(&self) -> bool {
        matches!(self, Self::Filtered { .. })
    }

    fn update_baseline(&self) -> bool {
        matches!(
            self,
            Self::Full {
                update_baseline: true,
                ..
            }
        )
    }

    fn invocation_id(&self) -> &str {
        match self {
            Self::Full { invocation_id, .. } => invocation_id.as_str(),
            Self::Filtered { .. } => "",
        }
    }

    fn filters(&self) -> &[String] {
        match self {
            Self::Full { .. } => &[],
            Self::Filtered { filters } => filters.as_slice(),
        }
    }
}

fn invocation_id_from_env() -> Result<Option<InvocationId>, String> {
    match std::env::var("PARITY_INVOCATION_ID") {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(format!("cannot read PARITY_INVOCATION_ID: {error}")),
        Ok(value) => InvocationId::parse(value).map(Some),
    }
}

fn only_filter_from_env() -> Result<Option<FixtureFilters>, String> {
    match std::env::var("PARITY_ONLY") {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(format!("cannot read PARITY_ONLY: {error}")),
        Ok(value) => FixtureFilters::new(parse_only_filter(&value)?).map(Some),
    }
}

fn parse_only_filter(value: &str) -> Result<Vec<String>, String> {
    let filters: Vec<_> = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    if filters.is_empty() {
        Err("PARITY_ONLY must contain at least one non-empty fixture filter".to_string())
    } else {
        Ok(filters)
    }
}

#[derive(Debug)]
struct ParityLayout {
    root: PathBuf,
    parity: PathBuf,
    manifests: PathBuf,
    cases: PathBuf,
    oracles: PathBuf,
    reports: PathBuf,
    baseline: PathBuf,
    report_json: PathBuf,
    report_markdown: PathBuf,
}

struct RunExecution<'a> {
    config: &'a RunConfig,
    layout: &'a ParityLayout,
    paths: &'a RunPaths,
    baseline: &'a BaselineState,
    refs_lock_sha256: &'a str,
}

impl ParityLayout {
    fn new(root: &Path) -> Self {
        let root = root.to_path_buf();
        let parity = root.join("tests").join("parity");
        Self {
            manifests: parity.join("manifest"),
            cases: parity.join("cases"),
            oracles: parity.join("oracles"),
            reports: parity.join("reports"),
            baseline: parity.join("baseline.json"),
            report_json: parity.join("report.json"),
            report_markdown: parity.join("REPORT.md"),
            root,
            parity,
        }
    }
}

trait ReportPublisher {
    fn publish(&mut self, report: &Report) -> Result<(), String>;
    fn discard(&mut self) -> Result<(), String>;
}

struct DurableReportPublisher<'a> {
    layout: &'a ParityLayout,
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl ReportPublisher for DurableReportPublisher<'_> {
    fn publish(&mut self, report: &Report) -> Result<(), String> {
        write_report_artifacts(
            &self.layout.report_json,
            &self.layout.report_markdown,
            &self.layout.reports,
            &self.layout.cases,
            report,
        )
    }

    fn discard(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        for path in [
            &self.layout.report_json,
            &self.layout.report_markdown,
            &self.layout.reports,
        ] {
            if let Err(error) = remove_path(path) {
                failures.push(format!("cannot remove stale {}: {error}", path.display()));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("\n"))
        }
    }
}

fn terminal_failure_report(source: &Report, cause: &str) -> Report {
    // Start clean instead of cloning the failed payload: malformed fixture data
    // or a category-specific HTML path may be the reason publication failed.
    // Retain only trustworthy run identity/provenance needed to diagnose it.
    let mut report = build_report(Vec::new(), false);
    report.invocation_id = source.invocation_id.clone();
    report.env = source.env.clone();
    report.refs_lock_present = source.refs_lock_present;
    report.refs_lock_sha256 = source.refs_lock_sha256.clone();
    report.corpus_issues = source.corpus_issues.clone();
    report.baseline_present = source.baseline_present;
    report.run_complete = false;
    report.gate_failure = Some(cause.to_string());
    report
}

fn publish_failure<P: ReportPublisher>(
    publisher: &mut P,
    report: &Report,
    cause: String,
) -> Result<(), String> {
    let failure = terminal_failure_report(report, &cause);
    if let Err(publication_error) = publisher.publish(&failure) {
        let cleanup_error = publisher.discard().err();
        eprintln!(
            "parity: could not publish terminal failure report: {publication_error}{}",
            cleanup_error
                .as_deref()
                .map(|error| format!("; could not clear partial durable evidence: {error}"))
                .unwrap_or_default()
        );
    }
    Err(cause)
}

fn publish_or_record_failure<P: ReportPublisher>(
    publisher: &mut P,
    report: &Report,
) -> Result<(), String> {
    match publisher.publish(report) {
        Ok(()) => Ok(()),
        Err(cause) => publish_failure(publisher, report, cause),
    }
}

impl Drop for RunPaths {
    fn drop(&mut self) {
        // The durable/full previews and filtered diagnostic previews live in
        // their own roots. Raw PDFs and pdftoppm pages are only per-run scratch
        // and must not accumulate across the repeated parity loop.
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

/// Process-wide ownership of the durable full-run report paths. The tracked
/// lock inode is never created or deleted by the runner.
struct FullRunLock(std::fs::File);

impl FullRunLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        // This tracked inode deliberately lives outside `target/`. Creating a
        // lock file on demand under cargo-cleanable scratch lets another process
        // delete and recreate the pathname, then lock a different inode while
        // the first run still owns the old one.
        let path = root.join("tests/parity/.full-run.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "cannot open tracked parity lock {}: {error}; restore the repository file",
                    path.display()
                )
            })?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
            format!(
                "another full parity run owns {} ({error}); refusing concurrent report publication",
                path.display()
            )
        })?;
        Ok(Self(file))
    }
}

impl Drop for FullRunLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn prepare_run_paths(
    root: &Path,
    parity_dir: &Path,
    filtered_run: bool,
) -> Result<RunPaths, String> {
    // A process-specific scratch directory prevents concurrent full/filtered
    // invocations from deleting or consuming one another's pdftoppm pages.
    let run_name = format!("run-{}", std::process::id());
    let tmp = root.join("target").join("parity-tmp").join(&run_name);
    match std::fs::remove_dir_all(&tmp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot clear scratch {}: {error}", tmp.display())),
    }
    std::fs::create_dir_all(&tmp)
        .map_err(|error| format!("cannot create scratch {}: {error}", tmp.display()))?;

    if filtered_run {
        // Diagnostic runs must not mutate the images belonging to the last full
        // report. Keep their previews and overlays under target instead.
        let diagnostic_root = root
            .join("target")
            .join("parity-diagnostics")
            .join(run_name);
        match std::fs::remove_dir_all(&diagnostic_root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot clear diagnostic output {}: {error}",
                    diagnostic_root.display()
                ));
            }
        }
        std::fs::create_dir_all(&diagnostic_root).map_err(|error| {
            format!(
                "cannot create diagnostic output {}: {error}",
                diagnostic_root.display()
            )
        })?;
        Ok(RunPaths {
            refs: diagnostic_root.join("refs"),
            diffs: diagnostic_root.join("diffs"),
            out: diagnostic_root.join("out"),
            reports: diagnostic_root.join("reports"),
            tmp,
            diagnostic_root: Some(diagnostic_root),
        })
    } else {
        Ok(RunPaths {
            refs: parity_dir.join("refs"),
            diffs: parity_dir.join("diffs"),
            out: parity_dir.join("out"),
            reports: parity_dir.join("reports"),
            tmp,
            diagnostic_root: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Parse the scope first. A diagnostic run cannot acquire durable-report
    // authority, while a full run cannot exist without the wrapper-supplied
    // invocation identity checked across JSON, Markdown, and HTML.
    let filters = only_filter_from_env()?;
    let update_baseline = match std::env::var("PARITY_UPDATE_BASELINE") {
        Err(std::env::VarError::NotPresent) => false,
        Ok(value) if value == "1" => true,
        Ok(value) => {
            return Err(format!(
                "PARITY_UPDATE_BASELINE must be exactly `1` when set, got {value:?}"
            ));
        }
        Err(error) => return Err(format!("cannot read PARITY_UPDATE_BASELINE: {error}")),
    };
    let config = match filters {
        Some(_) if update_baseline => {
            return Err(
                "PARITY_ONLY and PARITY_UPDATE_BASELINE=1 are mutually exclusive; a partial run can never become the baseline"
                    .to_string(),
            );
        }
        Some(filters) => RunConfig::Filtered { filters },
        None => {
            let invocation_id = invocation_id_from_env()?.ok_or_else(|| {
                "a full parity run requires a fresh report identity; use scripts/parity.sh"
                    .to_string()
            })?;
            RunConfig::Full {
                update_baseline,
                invocation_id,
            }
        }
    };
    run_at(root, config)
}

fn publish_configuration_failure(
    root: &Path,
    filtered_run: bool,
    invocation_id: &str,
    cause: String,
) -> Result<(), String> {
    if filtered_run {
        return Err(cause);
    }
    let layout = ParityLayout::new(root);
    let _lock = FullRunLock::acquire(root)?;
    let refs_lock_sha256 = std::fs::read(layout.parity.join("refs.lock"))
        .ok()
        .map(|bytes| sha256_hex(&bytes))
        .unwrap_or_default();
    let report = preflight_report(refs_lock_sha256, invocation_id);
    let mut publisher = DurableReportPublisher { layout: &layout };
    if let Err(cause) = publisher.discard() {
        return publish_failure(&mut publisher, &report, cause);
    }
    publish_failure(&mut publisher, &report, cause)
}

fn preflight_report(refs_lock_sha256: String, invocation_id: &str) -> Report {
    let mut report = build_report(Vec::new(), false);
    report.invocation_id = invocation_id.to_string();
    report.run_complete = false;
    report.env.rasterizer_source_path = "PENDING".to_string();
    report.env.rasterizer_executed_path = "PENDING".to_string();
    report.env.rasterizer_arguments = Rasterizer::argument_contract();
    report.env.rasterizer_version = "parity run has not completed".to_string();
    report.refs_lock_sha256 = refs_lock_sha256;
    report
}

fn write_baseline_snapshot(path: &Path, report: &mut Report) -> Result<(), String> {
    let invocation_id = std::mem::take(&mut report.invocation_id);
    let baseline_present = std::mem::replace(&mut report.baseline_present, true);
    let gate_failure = report.gate_failure.take();
    let result = write_report_json(path, report);
    report.gate_failure = gate_failure;
    report.baseline_present = baseline_present;
    report.invocation_id = invocation_id;
    result
}

fn run_at(root: &Path, config: RunConfig) -> Result<(), String> {
    let layout = ParityLayout::new(root);
    let filtered_run = config.is_filtered();
    let _full_run_lock = (!filtered_run)
        .then(|| FullRunLock::acquire(&layout.root))
        .transpose()?;

    let refs_lock_sha256 = std::fs::read(layout.parity.join("refs.lock"))
        .ok()
        .map(|bytes| sha256_hex(&bytes))
        .unwrap_or_default();
    let mut failure_report = preflight_report(refs_lock_sha256.clone(), config.invocation_id());

    if filtered_run {
        let paths = prepare_run_paths(&layout.root, &layout.parity, true)?;
        let mut checkpoint = |_report: &Report| Ok(());
        return execute_run(
            RunExecution {
                config: &config,
                layout: &layout,
                paths: &paths,
                baseline: &BaselineState::Missing,
                refs_lock_sha256: &refs_lock_sha256,
            },
            &mut failure_report,
            &mut checkpoint,
        )
        .map(|_| ());
    }

    let mut publisher = DurableReportPublisher { layout: &layout };
    if let Err(cause) = publisher.discard() {
        return publish_failure(&mut publisher, &failure_report, cause);
    }
    let paths = match prepare_run_paths(&layout.root, &layout.parity, false) {
        Ok(paths) => paths,
        Err(cause) => return publish_failure(&mut publisher, &failure_report, cause),
    };
    let baseline = load_baseline(&layout.baseline);
    failure_report.baseline_present = baseline_is_compatible(&baseline, &failure_report);
    publish_or_record_failure(&mut publisher, &failure_report)?;

    let execution = {
        let mut checkpoint = |report: &Report| publish_or_record_failure(&mut publisher, report);
        execute_run(
            RunExecution {
                config: &config,
                layout: &layout,
                paths: &paths,
                baseline: &baseline,
                refs_lock_sha256: &refs_lock_sha256,
            },
            &mut failure_report,
            &mut checkpoint,
        )
    };
    let mut report = match execution {
        Ok(report) => report,
        Err(cause) => return publish_failure(&mut publisher, &failure_report, cause),
    };
    report.invocation_id = config.invocation_id().to_string();

    if config.update_baseline() && enforce_baseline_update(&baseline, &report).is_ok() {
        // Commit the structurally validated regression snapshot before any
        // complete report claims it is present. Current FAIL verdicts remain in
        // the report and still make the wrapper exit nonzero. A cutoff leaves
        // the already-published incomplete checkpoint, never a report referring
        // to a snapshot that was not written.
        if let Err(cause) = write_baseline_snapshot(&layout.baseline, &mut report) {
            return publish_failure(&mut publisher, &report, cause);
        }
        report.baseline_present = true;
    }

    // Gate evaluation happens in `execute_run`; final publication happens once
    // afterward. A writer failure is converted into an incomplete terminal
    // report before returning.
    publish_or_record_failure(&mut publisher, &report)?;
    if let Some(cause) = report.gate_failure.clone() {
        return Err(cause);
    }

    if config.update_baseline() {
        eprintln!(
            "parity: EXPLICIT BASELINE UPDATE accepted via PARITY_UPDATE_BASELINE=1 after structural and retained-ID validation; current FAIL verdicts remain gate failures"
        );
    }

    print_final_summary(&layout, &report);
    Ok(())
}

fn execute_run(
    execution: RunExecution<'_>,
    failure_report: &mut Report,
    checkpoint: &mut dyn FnMut(&Report) -> Result<(), String>,
) -> Result<Report, String> {
    let RunExecution {
        config,
        layout,
        paths,
        baseline,
        refs_lock_sha256,
    } = execution;
    // Parse repository-owned inputs before external-tool setup so their exact
    // failures are deterministic and immediately reportable.
    let mut entries = load_manifests(&layout.root, &layout.manifests, &layout.parity)?;
    interaction_coverage::validate_manifest_product(&entries)?;
    if entries.is_empty() {
        eprintln!(
            "parity: no manifest entries found under {} (nothing to do).",
            layout.manifests.display()
        );
    }

    if config.is_filtered() {
        entries.retain(|entry| {
            let key = format!("{}/{}", entry.category, entry.id);
            config
                .filters()
                .iter()
                .any(|filter| key.contains(filter.as_str()))
        });
        eprintln!(
            "parity: PARITY_ONLY={:?} -> {} fixture(s); diagnostic run cannot satisfy the gate.",
            config.filters(),
            entries.len()
        );
        if entries.is_empty() {
            return Err(format!(
                "PARITY_ONLY matched zero fixtures; the full-corpus parity gate did not run (filters: {:?})",
                config.filters()
            ));
        }
    }

    let mut corpus_issues =
        audit_corpus(&layout.root, &layout.manifests, &layout.parity, &entries)?;
    failure_report.corpus_issues.clone_from(&corpus_issues);
    checkpoint(failure_report)?;

    let shared_fonts = load_bundled_fonts(&layout.root)?;
    let ua_stylesheet = PinnedUaStylesheet::load(&layout.parity)?;
    let rasterizer = match Rasterizer::discover() {
        Ok(rasterizer) => rasterizer,
        Err(error) => {
            failure_report.env.rasterizer_source_path = "MISSING".to_string();
            failure_report.env.rasterizer_executed_path = "MISSING".to_string();
            failure_report.env.rasterizer_arguments = Rasterizer::argument_contract();
            failure_report.env.rasterizer_version = error.clone();
            failure_report.env.rasterizer_sha256.clear();
            return Err(format!("parity requires pdftoppm: {error}"));
        }
    };
    failure_report.env.pdftoppm_available = true;
    failure_report.env.rasterizer_source_path =
        rasterizer.source_executable().display().to_string();
    failure_report.env.rasterizer_executed_path =
        rasterizer.executed_snapshot().display().to_string();
    failure_report.env.rasterizer_arguments = Rasterizer::argument_contract();
    failure_report.env.rasterizer_version = rasterizer.version().to_string();
    failure_report.env.rasterizer_sha256 = rasterizer.sha256().to_string();
    checkpoint(failure_report)?;
    eprintln!(
        "parity: rasterizer source={} snapshot={} argv={} ({}) dpi={}",
        rasterizer.source_executable().display(),
        rasterizer.executed_snapshot().display(),
        Rasterizer::argument_contract(),
        rasterizer.version(),
        config::DPI
    );

    let pool_threads = std::thread::available_parallelism()
        .map(|threads| threads.get())
        .unwrap_or(1)
        .saturating_sub(2)
        .clamp(1, 8);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(pool_threads)
        .build()
        .map_err(|error| format!("cannot build rayon pool: {error}"))?;

    let mut results: Vec<FixtureResult> = pool.install(|| {
        entries
            .par_iter()
            .map(|entry| {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    process_entry(
                        entry,
                        &layout.parity,
                        &layout.cases,
                        &layout.oracles,
                        &paths.refs,
                        &paths.diffs,
                        &paths.out,
                        &paths.reports,
                        &paths.tmp,
                        &rasterizer,
                        &shared_fonts,
                        &ua_stylesheet,
                    )
                }))
                .unwrap_or_else(|_| {
                    fixture_fail(entry, 100.0, "panic during processing".to_string())
                });
                eprintln!(
                    "parity: {:8} above-floor {:>11.8}% · raw {:>11.8}%  {}/{}  {}",
                    result.status.as_str(),
                    result.semantic_diff_pct,
                    result.diff_pct,
                    result.category,
                    result.id,
                    result.note
                );
                result
            })
            .collect()
    });
    results.sort_by(|left, right| {
        (left.category.as_str(), left.id.as_str())
            .cmp(&(right.category.as_str(), right.id.as_str()))
    });

    corpus_issues.extend(audit_raster_signals(&results));
    corpus_issues.extend(audit_oracle_semantics(&entries, &results));

    compute_dependency_context(&mut results);
    let fix_first = compute_fix_first(&results);
    let ref_mismatches = find_ref_mismatches(&entries, &layout.oracles);
    let suspect_unsupported_pass = collect_suspect_unsupported_pass(&results);
    let (stale_refs, refs_lock_present) =
        check_refs_freshness(&layout.root, &layout.parity, &results);

    let mut report = build_report(results, true);
    report.env = failure_report.env.clone();
    report.corpus_issues = corpus_issues;
    report.coverage = compute_coverage(&report);
    report.fix_first = fix_first;
    report.ref_mismatches = ref_mismatches;
    report.suspect_unsupported_pass = suspect_unsupported_pass;
    report.stale_refs = stale_refs;
    report.refs_lock_present = refs_lock_present;
    report.refs_lock_sha256 = refs_lock_sha256.to_string();
    report.baseline_present = baseline_is_compatible(baseline, &report);

    if config.is_filtered() {
        let Some(diagnostic_root) = paths.diagnostic_root.as_ref() else {
            return Err("filtered parity run has no diagnostic root".to_string());
        };
        return Err(format!(
            "parity diagnostic only: {}P/{}F/{} reference-disputed over {} filtered fixture(s); current full report, its images, and baseline were not modified; diagnostic images: {}; the full-corpus gate did not run",
            report.overall.pass,
            report.overall.fail,
            report.overall.reference_disputed,
            report.overall.total,
            diagnostic_root.display(),
        ));
    }

    let gate_result = if config.update_baseline() {
        enforce_baseline_update(baseline, &report).and_then(|()| enforce_current_health(&report))
    } else {
        enforce_gate(baseline, &report)
    };
    report.gate_failure = gate_result.err().map(|error| {
        format!(
            "{error}\nCurrent report: {}\nCurrent visual report: {}",
            layout.report_markdown.display(),
            layout.reports.join("index.html").display()
        )
    });
    Ok(report)
}

fn print_final_summary(layout: &ParityLayout, report: &Report) {
    println!(
        "parity: {:.2}% verified ({}P/{}F/{} reference-disputed) · report at {}",
        report.overall.score_pct,
        report.overall.pass,
        report.overall.fail,
        report.overall.reference_disputed,
        layout.report_markdown.display()
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
        let ids: Vec<&str> = report
            .stale_refs
            .iter()
            .map(|stale| stale.id.as_str())
            .collect();
        eprintln!(
            "parity: WARNING {} STALE reference(s) (fixture changed since ref was generated) — \
             regenerate with scripts/parity-gen-refs.sh: {}",
            report.stale_refs.len(),
            ids.join(", ")
        );
    }
}

// ---------------------------------------------------------------------------
// Per-fixture processing
// ---------------------------------------------------------------------------

fn decode_pages(paths: &[PathBuf], label: &str) -> Result<Vec<image::RgbaImage>, String> {
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            image::open(path)
                .map(|image| image.to_rgba8())
                .map_err(|error| format!("decode {label} page {} failed: {error}", index + 1))
        })
        .collect()
}

fn preview_path(root: &Path, category: &str, id: &str, page: usize) -> PathBuf {
    let name = if page == 1 {
        format!("{id}.png")
    } else {
        format!("{id}.p{page}.png")
    };
    root.join(category).join(name)
}

fn clear_previews(root: &Path, category: &str, id: &str) -> Result<(), String> {
    let directory = root.join(category);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let first = directory.join(format!("{id}.png"));
    if first.is_file() {
        std::fs::remove_file(&first)
            .map_err(|error| format!("cannot remove stale {}: {error}", first.display()))?;
    }
    if let Ok(entries) = std::fs::read_dir(&directory) {
        let numbered_prefix = format!("{id}.p");
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name
                .strip_prefix(&numbered_prefix)
                .and_then(|rest| rest.strip_suffix(".png"))
                .is_some_and(|digits| {
                    !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
                })
            {
                std::fs::remove_file(entry.path()).map_err(|error| {
                    format!(
                        "cannot remove stale preview in {}: {error}",
                        directory.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn clear_diffs(root: &Path, category: &str, id: &str) -> Result<(), String> {
    let directory = root.join(category);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    for entry in std::fs::read_dir(&directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .flatten()
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let first = name == format!("{id}.diff.png");
        let numbered = name
            .strip_prefix(&format!("{id}.p"))
            .and_then(|rest| rest.strip_suffix(".diff.png"))
            .is_some_and(|digits| {
                !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
            });
        if first || numbered {
            std::fs::remove_file(entry.path()).map_err(|error| {
                format!(
                    "cannot remove stale diff in {}: {error}",
                    directory.display()
                )
            })?;
        }
    }
    Ok(())
}

fn save_image_atomic(image: &image::RgbaImage, path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("image path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid image path: {}", path.display()))?;
    let temporary = parent.join(format!(".{name}.tmp.png"));
    image
        .save(&temporary)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .map_err(|error| format!("cannot publish {}: {error}", path.display()))
}

fn save_previews(
    images: &[image::RgbaImage],
    root: &Path,
    category: &str,
    id: &str,
) -> Result<(), String> {
    clear_previews(root, category, id)?;
    for (index, image) in images.iter().enumerate() {
        if let Err(error) = save_image_atomic(image, &preview_path(root, category, id, index + 1)) {
            let _ = clear_previews(root, category, id);
            return Err(error);
        }
    }
    Ok(())
}

fn later_page_is_more_informative_diagnosis(
    current_status: Status,
    current_diff_pct: f64,
    later_status: Status,
    later_diff_pct: f64,
) -> bool {
    match (current_status.is_failure(), later_status.is_failure()) {
        // A failing page always explains a visual failure better than a passing
        // page; among failures retain the page with the larger raw signal.
        (false, true) => true,
        (true, true) => later_diff_pct > current_diff_pct,
        // A fully passing fixture can still have raw PDF representation residue.
        // Keep the most different page's diagnosis so the report never presents
        // a nonzero maximum-page diff as an exact match.
        (false, false) => later_diff_pct > current_diff_pct,
        (true, false) => false,
    }
}

/// A visible mismatch against a reference that standard review has rejected is
/// compatibility evidence, not an Ironpress rendering verdict. Processing and
/// raster diagnostics still run exactly as for a failure; only the final report
/// classification changes after all pages have been compared.
fn classify_reference_dispute(entry: &ManifestEntry, status: Status) -> Status {
    if status == Status::Fail && entry.reference.is_disputed() {
        Status::ReferenceDisputed
    } else {
        status
    }
}

/// Emit the same diagnostic evidence for every page of a filtered fixture.
///
/// The normal report stays compact; this is available only while explicitly
/// investigating an individual case with `PARITY_DEBUG_TALLY=1`.
fn log_debug_tally(entry: &ManifestEntry, page: usize, outcome: &compare::V2Outcome) {
    if std::env::var("PARITY_DEBUG_TALLY").is_err() {
        return;
    }
    let t = &outcome.tally;
    eprintln!(
        "tally {}/{} page {}: different={} color={:.2}% (Delta-E {:.2}, bias {:.3}, modal {:?}, anchors {}, hue_preserved {}) missing={:.2}% extra={:.2}% mixed_phase={} dom={:?}",
        entry.category,
        entry.id,
        page,
        t.different_px,
        t.color_pct,
        t.color_de,
        t.color_coverage_bias,
        t.modal_drgba,
        t.color_errors_have_css_anchors,
        t.color_errors_preserve_hue,
        t.missing_pct,
        t.extra_pct,
        compare::visibility::is_mixed_coverage_phase(t, &outcome.regions),
        outcome.verdict.dominant_class
    );
    eprintln!(
        "DIAG {}/{} page {}: STATUS={} [{}] {} (conf {:.2}) interior_color%={:.3} interior_de={:.2}",
        entry.category,
        entry.id,
        page,
        outcome.status.as_str(),
        outcome.diagnosis.primary_class,
        outcome.diagnosis.headline,
        outcome.diagnosis.confidence,
        outcome.tally.interior_color_pct,
        outcome.tally.interior_color_de
    );
    let visible = &outcome.visibility.tally;
    eprintln!(
        "VISIBILITY {}/{} page {}: color={:.2}% (Delta-E {:.2}, bias {:.3}, anchors {}) missing={:.2}% extra={:.2}% shared_content={:.3} outside_edges={} mixed_phase={} shared_presence={} shared_color={} predominant_color={} presence={:?}",
        entry.category,
        entry.id,
        page,
        visible.color_pct,
        visible.color_de,
        visible.color_coverage_bias,
        visible.color_errors_have_css_anchors,
        visible.missing_pct,
        visible.extra_pct,
        visible.shared_content_ratio,
        visible.presence_outside_edge_band_px,
        compare::visibility::is_mixed_coverage_phase(visible, &outcome.visibility.regions),
        outcome
            .visibility
            .regions
            .only_sub_css_coverage_presence_residues(),
        outcome
            .visibility
            .regions
            .shared_coverage_color_with_compact_remainder(),
        compare::visibility::is_predominantly_shared_coverage_phase(
            visible,
            &outcome.visibility.regions,
        ),
        compare::visibility::visible_presence_class(visible, &outcome.visibility.regions),
    );
    for aggregate in &outcome.visibility.regions.aggregates {
        eprintln!(
            "VISIBILITY_REGION {}/{} page {}: class={:?} components={} total={} largest={} span={} ramp={}/{} large_color_balanced={} interior={}",
            entry.category,
            entry.id,
            page,
            aggregate.class,
            aggregate.region_count,
            aggregate.total_area_px,
            aggregate.largest_area_px,
            aggregate.largest_span_px,
            aggregate.color_ramp_proven_px,
            aggregate.color_ramp_total_px,
            aggregate.all_large_color_components_balanced,
            aggregate.interior_color_px,
        );
    }
    for aggregate in &outcome.regions.aggregates {
        let remainder = aggregate.non_long_edge_presence;
        eprintln!(
            "REGION {}/{} page {}: class={:?} components={} total={} largest={} span={} after_long_edge={}/{} largest={} span={} edge_fringe={} sub_css_residue={} shared_1px_residue={} shared_coverage_color={} ramp={}/{} large_color_balanced={} unproven_ramp={} compact_remainder={} max_direct_de={:.2}",
            entry.category,
            entry.id,
            page,
            aggregate.class,
            aggregate.region_count,
            aggregate.total_area_px,
            aggregate.largest_area_px,
            aggregate.largest_span_px,
            remainder.region_count,
            remainder.total_area_px,
            remainder.largest_area_px,
            remainder.largest_span_px,
            aggregate.coverage.all_outer_device_edge_fringes,
            aggregate.coverage.all_sub_css_presence_residues,
            aggregate.coverage.all_one_device_pixel_presence_residues,
            aggregate.coverage.all_shared_color_ramps,
            aggregate.color_ramp_proven_px,
            aggregate.color_ramp_total_px,
            aggregate.all_large_color_components_balanced,
            aggregate.unproven_color_ramp_px,
            aggregate.all_unproven_color_ramps_compact,
            aggregate.max_direct_delta_e,
        );
    }
    for region in &outcome.regions.examples {
        eprintln!(
            "REGION_EXAMPLE {}/{} page {}: class={:?} area={} bbox_css={:?} span={} interior_color={} edge_fringe={} sub_css_residue={} shared_1px_residue={} shared_coverage_color={} ramp={}/{} max_direct_de={:.2}",
            entry.category,
            entry.id,
            page,
            region.class,
            region.area_px,
            region.bbox_css,
            region.longest_span_px,
            region.interior_color_px,
            region.coverage.outer_device_edge_fringe,
            region.coverage.sub_css_presence_residue,
            region.coverage.one_device_pixel_presence_residue,
            region.coverage.shared_color_ramp,
            region.coverage.color_ramp_proven_px,
            region.coverage.color_ramp_total_px,
            region.max_direct_delta_e,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn process_entry(
    entry: &ManifestEntry,
    parity_dir: &Path,
    _cases_dir: &Path,
    oracles_dir: &Path,
    refs_dir: &Path,
    diffs_dir: &Path,
    out_dir: &Path,
    reports_dir: &Path,
    tmp_dir: &Path,
    rasterizer: &Rasterizer,
    fonts: &FontBundle,
    ua_stylesheet: &PinnedUaStylesheet,
) -> FixtureResult {
    let fixture = parity_dir.join(&entry.file);
    let html = match std::fs::read_to_string(&fixture) {
        Ok(h) => h,
        Err(e) => return fixture_fail(entry, 100.0, format!("cannot read fixture: {e}")),
    };
    // SHA-256 of the fixture HTML for the refs.lock freshness check. Computed
    // once here so every result (including error paths via the `with_sha`
    // closure below) carries it.
    let html_sha = sha256_hex(html.as_bytes());
    // Helper: stamp the sha onto any result we return from this function.
    let with_sha = |mut r: FixtureResult| -> FixtureResult {
        r.html_sha256 = html_sha.clone();
        r
    };

    for root in [refs_dir, out_dir] {
        if let Err(error) = clear_previews(root, &entry.category, &entry.id) {
            return with_sha(fixture_fail(entry, 100.0, error));
        }
    }
    for root in [reports_dir, diffs_dir] {
        if let Err(error) = clear_diffs(root, &entry.category, &entry.id) {
            return with_sha(fixture_fail(entry, 100.0, error));
        }
    }

    // In-process render at Chrome-matching geometry. The fixture's own directory
    // is the base for resolving relative resource URLs (e.g. `@font-face` `src`).
    let base_path = fixture.parent();
    let pdf = match render_pdf(
        &html,
        entry.sanitize,
        fonts,
        ua_stylesheet,
        base_path,
        Some(parity_dir),
    ) {
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
    if let Err(error) = retain_debug_candidate_pdf(out_dir, entry, &pdf) {
        return with_sha(fixture_fail(entry, 100.0, error));
    }
    let candidate_paths = match rasterizer.rasterize_all_pages(
        &pdf_path,
        tmp_dir,
        &format!("candidate-{}", entry.id),
    ) {
        Ok(paths) => paths,
        Err(error) => {
            return with_sha(fixture_fail(
                entry,
                100.0,
                format!("candidate pdftoppm failed: {error}"),
            ));
        }
    };
    let candidate_pages = match decode_pages(&candidate_paths, "candidate") {
        Ok(pages) => pages,
        Err(error) => return with_sha(fixture_fail(entry, 100.0, error)),
    };
    let candidate_fingerprints = raster_fingerprints(&candidate_pages);
    if let Err(error) = save_previews(&candidate_pages, out_dir, &entry.category, &entry.id) {
        return with_sha(fixture_fail(entry, 100.0, error));
    }

    let oracle_pdf = oracles_dir
        .join(&entry.category)
        .join(format!("{}.pdf", entry.id));
    if !oracle_pdf.is_file() {
        let mut result = fixture_fail(
            entry,
            100.0,
            "no oracle PDF (run scripts/parity-gen-refs.sh)".to_string(),
        );
        result.raster.candidate = candidate_fingerprints;
        return with_sha(result);
    }
    let oracle_paths =
        match rasterizer.rasterize_all_pages(&oracle_pdf, tmp_dir, &format!("oracle-{}", entry.id))
        {
            Ok(paths) => paths,
            Err(error) => {
                let mut result =
                    fixture_fail(entry, 100.0, format!("oracle pdftoppm failed: {error}"));
                result.raster.candidate = candidate_fingerprints;
                return with_sha(result);
            }
        };
    let reference_pages = match decode_pages(&oracle_paths, "oracle") {
        Ok(pages) => pages,
        Err(error) => {
            let mut result = fixture_fail(entry, 100.0, error);
            result.raster.candidate = candidate_fingerprints;
            return with_sha(result);
        }
    };
    let oracle_fingerprints = raster_fingerprints(&reference_pages);
    if let Err(error) = save_previews(&reference_pages, refs_dir, &entry.category, &entry.id) {
        let mut result = fixture_fail(entry, 100.0, error);
        result.raster.candidate = candidate_fingerprints;
        result.raster.oracle = oracle_fingerprints;
        return with_sha(result);
    }

    let cand = &candidate_pages[0];
    let reference = &reference_pages[0];

    // V2 PATH (the only verdict path): compare raw rasters in shared page space.
    // `status` is the fixed human-visibility verdict; `diff_pct` remains the raw
    // exact evidence, while `semantic_diff_pct` and the classed overlay apply
    // only the global per-pixel RGB tolerance.
    let outcome = compare::compare_page_v2(cand, reference);
    // Keep the raw exact scalar. Presentation code is responsible for compact
    // formatting and must not make a nonzero signal look like zero.
    let diff_pct = outcome.diff_pct;

    log_debug_tally(entry, 1, &outcome);

    let reports_diff = reports_dir
        .join(&entry.category)
        .join(format!("{}.diff.png", entry.id));
    if let Err(error) = save_image_atomic(&outcome.overlay, &reports_diff) {
        let mut result = fixture_fail(entry, 100.0, error);
        result.raster.candidate = candidate_fingerprints;
        result.raster.oracle = oracle_fingerprints;
        return with_sha(result);
    }
    if outcome.status != Status::Pass {
        let out = diffs_dir
            .join(&entry.category)
            .join(format!("{}.png", entry.id));
        if let Err(error) = save_image_atomic(&outcome.overlay, &out) {
            let mut result = fixture_fail(entry, 100.0, error);
            result.raster.candidate = candidate_fingerprints;
            result.raster.oracle = oracle_fingerprints;
            return with_sha(result);
        }
    }
    let mut fixture_status = outcome.status;

    // Assert the candidate/oracle PDF page counts match and fold every later page
    // through the same comparator. Page identity comes from the two runtime
    // rasterization vectors, never from committed PNG filename runs.
    let mut diff_pct = diff_pct;
    let mut semantic_diff_pct = outcome.semantic_diff_pct;
    let mut diagnosis = outcome.diagnosis.clone();
    let mut diagnosis_status = outcome.status;
    let mut diagnosis_diff_pct = diff_pct;
    let mut page_note = String::new();
    let page_count_mismatch = candidate_pages.len() != reference_pages.len();
    if page_count_mismatch {
        fixture_status = report::Status::Fail;
        diff_pct = 100.0;
        semantic_diff_pct = 100.0;
        page_note = format!(
            "page-count mismatch: ironpress {} vs oracle {}",
            candidate_pages.len(),
            reference_pages.len()
        );
        diagnosis = diagnose::Diagnosis {
            primary_class: "PageCount".to_string(),
            headline: page_note.clone(),
            ..Default::default()
        };
    }
    // Compare every page that exists on both sides even when the counts differ.
    // The count mismatch remains the terminal 100% PageCount failure, while the
    // overlapping page overlays stay available for inspection.
    for page in 2..=candidate_pages.len().min(reference_pages.len()) {
        let page_outcome =
            compare::compare_page_v2(&candidate_pages[page - 1], &reference_pages[page - 1]);
        log_debug_tally(entry, page, &page_outcome);
        let page_diff_pct = page_outcome.diff_pct;
        if !page_count_mismatch {
            diff_pct = diff_pct.max(page_diff_pct);
            semantic_diff_pct = semantic_diff_pct.max(page_outcome.semantic_diff_pct);
        }
        let pdiff = reports_dir
            .join(&entry.category)
            .join(format!("{}.p{}.diff.png", entry.id, page));
        if let Err(error) = save_image_atomic(&page_outcome.overlay, &pdiff) {
            fixture_status = report::Status::Fail;
            if page_note.is_empty() {
                page_note = error;
            } else {
                page_note.push_str(&format!("; {error}"));
            }
            break;
        }
        if !page_count_mismatch {
            if page_outcome.status != report::Status::Pass {
                fixture_status = report::Status::Fail;
            }
            if later_page_is_more_informative_diagnosis(
                diagnosis_status,
                diagnosis_diff_pct,
                page_outcome.status,
                page_diff_pct,
            ) {
                diagnosis = page_outcome.diagnosis.clone();
                diagnosis.headline = format!("page {page}: {}", diagnosis.headline);
                diagnosis_status = page_outcome.status;
                diagnosis_diff_pct = page_diff_pct;
                if fixture_status != report::Status::Pass {
                    page_note = diagnosis.headline.clone();
                }
            }
        }
    }

    // Attach the measured diagnosis. Failing dependency context is computed
    // later without rewriting this headline or claiming an unproven cause.
    let fixture_status = classify_reference_dispute(entry, fixture_status);
    let mut result = report::fixture_base(entry, fixture_status, diff_pct, page_note);
    result.semantic_diff_pct = semantic_diff_pct;
    result.raster.candidate = candidate_fingerprints;
    result.raster.oracle = oracle_fingerprints;
    result.diagnosis = Some(diagnosis);
    with_sha(result)
}

/// Keep a candidate PDF beside a diagnostic run only when explicitly asked.
///
/// Normal parity runs keep PDFs in per-run scratch and remove them afterwards.
/// Retention is deliberately opt-in so the durable report remains a compact
/// PNG-diff inventory while a rendering investigation can inspect both PDFs.
fn retain_debug_candidate_pdf(
    out_dir: &Path,
    entry: &ManifestEntry,
    pdf: &[u8],
) -> Result<(), String> {
    if std::env::var_os("PARITY_KEEP_PDFS").is_none() {
        return Ok(());
    }
    let Some(root) = out_dir.parent() else {
        return Err(format!(
            "cannot retain candidate PDF for {}/{}: no output parent",
            entry.category, entry.id
        ));
    };
    let directory = root.join("pdfs").join(&entry.category);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let path = directory.join(format!("{}.pdf", entry.id));
    std::fs::write(&path, pdf).map_err(|error| format!("cannot retain {}: {error}", path.display()))
}

#[cfg(test)]
mod status_tests {
    use super::{
        FixtureFilters, FullRunLock, InvocationId, Report, ReportPublisher, RunConfig, Status,
        build_report, later_page_is_more_informative_diagnosis, parse_only_filter,
        prepare_run_paths, publish_configuration_failure, publish_or_record_failure, run_at,
        write_baseline_snapshot,
    };

    fn temp_root(label: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("ironpress-parity-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tests/parity/manifest")).unwrap();
        std::fs::create_dir_all(root.join("tests/parity/cases")).unwrap();
        std::fs::write(
            root.join("tests/parity/ua-pins.css"),
            ":where(body){margin:0}",
        )
        .unwrap();
        std::fs::write(
            root.join("tests/parity/.full-run.lock"),
            "test lock inode\n",
        )
        .unwrap();
        root
    }

    fn full_config() -> RunConfig {
        RunConfig::Full {
            update_baseline: false,
            invocation_id: InvocationId::parse("test-invocation".to_string()).unwrap(),
        }
    }

    fn assert_terminal_formats(root: &std::path::Path, cause: &str) {
        let parity = root.join("tests/parity");
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(parity.join("report.json")).expect("terminal JSON report"),
        )
        .unwrap();
        let markdown =
            std::fs::read_to_string(parity.join("REPORT.md")).expect("terminal Markdown report");
        let html = std::fs::read_to_string(parity.join("reports/index.html"))
            .expect("terminal HTML report");

        assert_eq!(json["run_complete"], false);
        assert_eq!(json["invocation_id"], "test-invocation");
        assert_eq!(json["gate_failure"], cause);
        assert!(markdown.contains(cause), "Markdown omitted {cause:?}");
        assert!(html.contains(cause), "HTML omitted {cause:?}");
        assert!(markdown.contains("<!-- parity-invocation-id: test-invocation -->"));
        assert!(html.contains("<meta name=\"parity-invocation-id\" content=\"test-invocation\">"));
        assert!(markdown.contains("**RUN FAILURE — FAILED.**"));
        assert!(html.contains("<strong>RUN FAILURE — FAILED</strong>"));
    }

    fn report_matches_invocation(root: &std::path::Path, invocation_id: &str) -> bool {
        std::process::Command::new("bash")
            .arg(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("scripts/parity-check-report.sh"),
            )
            .arg(root)
            .arg(invocation_id)
            .status()
            .is_ok_and(|status| status.success())
    }

    struct FailOncePublisher {
        reports: Vec<Report>,
        cause: String,
        failed: bool,
    }

    impl ReportPublisher for FailOncePublisher {
        fn publish(&mut self, report: &Report) -> Result<(), String> {
            if !self.failed {
                self.failed = true;
                return Err(self.cause.clone());
            }
            self.reports.push(report.clone());
            Ok(())
        }

        fn discard(&mut self) -> Result<(), String> {
            self.reports.clear();
            Ok(())
        }
    }

    #[test]
    fn later_page_keeps_the_most_informative_diagnosis() {
        assert!(later_page_is_more_informative_diagnosis(
            Status::Pass,
            0.0,
            Status::Fail,
            0.01,
        ));
        assert!(later_page_is_more_informative_diagnosis(
            Status::Fail,
            0.01,
            Status::Fail,
            2.0,
        ));
        assert!(!later_page_is_more_informative_diagnosis(
            Status::Fail,
            2.0,
            Status::Fail,
            0.01,
        ));
        assert!(later_page_is_more_informative_diagnosis(
            Status::Pass,
            0.01,
            Status::Pass,
            2.0,
        ));
        assert!(!later_page_is_more_informative_diagnosis(
            Status::Pass,
            2.0,
            Status::Pass,
            0.01,
        ));
    }

    #[test]
    fn filtered_run_artifacts_are_isolated_from_the_durable_report() {
        let root =
            std::env::temp_dir().join(format!("ironpress-parity-run-paths-{}", std::process::id()));
        let parity = root.join("tests/parity");
        std::fs::create_dir_all(&parity).unwrap();

        let filtered = prepare_run_paths(&root, &parity, true).unwrap();
        let diagnostic = filtered.diagnostic_root.as_ref().unwrap();
        for path in [
            &filtered.refs,
            &filtered.out,
            &filtered.diffs,
            &filtered.reports,
        ] {
            assert!(path.starts_with(diagnostic));
            assert!(!path.starts_with(&parity));
        }
        let diagnostic = diagnostic.clone();
        let filtered_tmp = filtered.tmp.clone();
        drop(filtered);
        assert!(!filtered_tmp.exists());
        assert!(diagnostic.exists());

        let full = prepare_run_paths(&root, &parity, false).unwrap();
        assert_eq!(full.refs, parity.join("refs"));
        assert_eq!(full.out, parity.join("out"));
        assert_eq!(full.diffs, parity.join("diffs"));
        assert_eq!(full.reports, parity.join("reports"));
        assert!(full.diagnostic_root.is_none());
        let full_tmp = full.tmp.clone();
        drop(full);
        assert!(!full_tmp.exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_full_runs_cannot_share_durable_report_paths() {
        let root = temp_root("full-run-lock");

        let first = FullRunLock::acquire(&root).unwrap();
        let error = FullRunLock::acquire(&root).err().unwrap();
        assert!(error.contains("refusing concurrent report publication"));

        drop(first);
        let second = FullRunLock::acquire(&root).unwrap();
        drop(second);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cargo_cleanable_target_cannot_replace_the_locked_inode() {
        let root = temp_root("stable-full-run-lock");
        std::fs::create_dir_all(root.join("target/parity-tmp")).unwrap();
        std::fs::write(root.join("target/parity-tmp/full-run.lock"), "obsolete\n").unwrap();

        let first = FullRunLock::acquire(&root).unwrap();
        std::fs::remove_dir_all(root.join("target")).unwrap();
        let error = FullRunLock::acquire(&root).err().unwrap();
        assert!(error.contains("refusing concurrent report publication"));

        drop(first);
        let second = FullRunLock::acquire(&root).unwrap();
        drop(second);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejected_concurrent_full_run_preserves_the_current_durable_evidence() {
        let root = temp_root("concurrent-evidence");
        let parity = root.join("tests/parity");
        std::fs::create_dir_all(parity.join("reports")).unwrap();
        std::fs::write(parity.join("report.json"), "json sentinel").unwrap();
        std::fs::write(parity.join("REPORT.md"), "markdown sentinel").unwrap();
        std::fs::write(parity.join("reports/index.html"), "html sentinel").unwrap();

        let first = FullRunLock::acquire(&root).unwrap();
        let error = run_at(&root, full_config()).unwrap_err();

        assert!(error.contains("refusing concurrent report publication"));
        assert_eq!(
            std::fs::read_to_string(parity.join("report.json")).unwrap(),
            "json sentinel"
        );
        assert_eq!(
            std::fs::read_to_string(parity.join("REPORT.md")).unwrap(),
            "markdown sentinel"
        );
        assert_eq!(
            std::fs::read_to_string(parity.join("reports/index.html")).unwrap(),
            "html sentinel"
        );

        drop(first);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invocation_identity_is_not_part_of_the_deterministic_baseline() {
        let mut report = build_report(Vec::new(), true);
        report.invocation_id = "wrapper-invocation".to_string();
        report.gate_failure = Some("current fixture remains FAIL".to_string());
        let path = std::env::temp_dir().join(format!(
            "ironpress-parity-baseline-token-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        write_baseline_snapshot(&path, &mut report).unwrap();
        let baseline_json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();

        assert_eq!(report.invocation_id, "wrapper-invocation");
        assert!(!report.baseline_present);
        assert_eq!(
            report.gate_failure.as_deref(),
            Some("current fixture remains FAIL")
        );
        assert!(baseline_json.get("invocation_id").is_none());
        assert_eq!(baseline_json["baseline_present"], true);
        assert!(baseline_json["gate_failure"].is_null());
        let _ = std::fs::remove_file(path);

        let missing_parent = std::env::temp_dir().join(format!(
            "ironpress-parity-missing-baseline-parent-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&missing_parent);
        assert!(
            write_baseline_snapshot(&missing_parent.join("baseline.json"), &mut report).is_err()
        );
        assert_eq!(report.invocation_id, "wrapper-invocation");
        assert!(!report.baseline_present);
    }

    #[test]
    fn mixed_generation_report_surfaces_never_satisfy_wrapper_freshness() {
        let root = temp_root("mixed-report-generations");
        let layout = super::ParityLayout::new(&root);
        let invocation = "fresh-invocation";
        let mut first = build_report(Vec::new(), false);
        first.invocation_id = invocation.to_string();
        first.gate_failure = Some("checkpoint A".to_string());
        let mut publisher = super::DurableReportPublisher { layout: &layout };
        publisher.publish(&first).unwrap();
        assert!(report_matches_invocation(&root, invocation));

        let mut second = first;
        second.gate_failure = Some("checkpoint B".to_string());
        super::write_report_md(&layout.report_markdown, &second).unwrap();
        assert!(!report_matches_invocation(&root, invocation));
        super::write_html_reports(&layout.reports, &layout.cases, &second).unwrap();
        assert!(!report_matches_invocation(&root, invocation));
        super::write_report_json(&layout.report_json, &second).unwrap();
        assert!(report_matches_invocation(&root, invocation));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_parity_filter_cannot_collapse_into_a_full_run() {
        assert!(parse_only_filter("").is_err());
        assert!(parse_only_filter(" , , ").is_err());
        assert_eq!(
            parse_only_filter(" alpha, beta ,, ").unwrap(),
            ["alpha", "beta"]
        );
    }

    #[test]
    fn invalid_full_run_invocation_identity_replaces_stale_evidence() {
        let root = temp_root("invalid-invocation");
        let parity = root.join("tests/parity");
        std::fs::create_dir_all(parity.join("reports")).unwrap();
        std::fs::write(parity.join("report.json"), "stale JSON").unwrap();
        std::fs::write(parity.join("REPORT.md"), "stale Markdown").unwrap();
        std::fs::write(parity.join("reports/index.html"), "stale HTML").unwrap();
        let cause = "PARITY_INVOCATION_ID contains invalid characters".to_string();

        assert_eq!(
            publish_configuration_failure(&root, false, "", cause.clone()),
            Err(cause.clone())
        );

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(parity.join("report.json")).unwrap()).unwrap();
        let markdown = std::fs::read_to_string(parity.join("REPORT.md")).unwrap();
        let html = std::fs::read_to_string(parity.join("reports/index.html")).unwrap();
        assert_eq!(json["run_complete"], false);
        assert_eq!(json["gate_failure"], cause);
        assert!(json.get("invocation_id").is_none());
        assert!(markdown.contains(&cause));
        assert!(html.contains(&cause));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_parse_failure_is_final_in_all_durable_formats() {
        let root = temp_root("terminal-manifest");
        std::fs::write(root.join("tests/parity/manifest/broken.json"), "[").unwrap();

        let cause = run_at(&root, full_config()).unwrap_err();

        assert!(cause.contains("invalid manifest JSON"));
        assert_terminal_formats(&root, &cause);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn font_setup_failure_is_final_in_all_durable_formats() {
        let root = temp_root("terminal-fonts");

        let cause = run_at(&root, full_config()).unwrap_err();

        assert!(cause.contains("required parity font"));
        assert_terminal_formats(&root, &cause);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scratch_setup_failure_is_final_in_all_durable_formats() {
        let root = temp_root("terminal-scratch");
        let scratch_parent = root.join("target/parity-tmp");
        std::fs::create_dir_all(&scratch_parent).unwrap();
        std::fs::write(
            scratch_parent.join(format!("run-{}", std::process::id())),
            "not a directory",
        )
        .unwrap();

        let cause = run_at(&root, full_config()).unwrap_err();

        assert!(cause.contains("cannot clear scratch"));
        assert_terminal_formats(&root, &cause);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn report_writer_failure_is_replaced_by_its_exact_terminal_report() {
        let cause = "injected report artifact write failure".to_string();
        let mut publisher = FailOncePublisher {
            reports: Vec::new(),
            cause: cause.clone(),
            failed: false,
        };
        let report = build_report(Vec::new(), true);

        assert_eq!(
            publish_or_record_failure(&mut publisher, &report),
            Err(cause.clone())
        );
        assert_eq!(publisher.reports.len(), 1);
        assert!(!publisher.reports[0].run_complete);
        assert_eq!(
            publisher.reports[0].gate_failure.as_deref(),
            Some(cause.as_str())
        );
    }

    #[test]
    fn filtered_setup_failure_does_not_mutate_durable_reports() {
        let root = temp_root("filtered-terminal");
        let parity = root.join("tests/parity");
        std::fs::write(parity.join("manifest/broken.json"), "[").unwrap();
        std::fs::create_dir_all(parity.join("reports")).unwrap();
        std::fs::write(parity.join("report.json"), "json sentinel").unwrap();
        std::fs::write(parity.join("REPORT.md"), "markdown sentinel").unwrap();
        std::fs::write(parity.join("baseline.json"), "baseline sentinel").unwrap();
        std::fs::write(parity.join("reports/index.html"), "html sentinel").unwrap();

        let cause = run_at(
            &root,
            RunConfig::Filtered {
                filters: FixtureFilters::new(vec!["fixture".to_string()]).unwrap(),
            },
        )
        .unwrap_err();

        assert!(cause.contains("invalid manifest JSON"));
        assert_eq!(
            std::fs::read_to_string(parity.join("report.json")).unwrap(),
            "json sentinel"
        );
        assert_eq!(
            std::fs::read_to_string(parity.join("REPORT.md")).unwrap(),
            "markdown sentinel"
        );
        assert_eq!(
            std::fs::read_to_string(parity.join("baseline.json")).unwrap(),
            "baseline sentinel"
        );
        assert_eq!(
            std::fs::read_to_string(parity.join("reports/index.html")).unwrap(),
            "html sentinel"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
