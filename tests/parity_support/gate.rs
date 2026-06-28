//! Scoring, report assembly, breadth/coverage + fix-first metrics, the surfaced
//! freshness/unsupported guards, and the CI regression gate.
//!
//! Extracted verbatim from the former monolithic `mod.rs` (C1 mechanical split).

use std::collections::BTreeMap;
use std::path::Path;

use super::config::{DPI, SCORE_EPSILON, WHITE_TOL};
use super::report::{
    CategoryReport, Counts, Coverage, EnvBlock, FeatureReport, FixFirst, FixtureResult, Overall,
    Report, StaleRef, Status,
};
use super::util::round2;

// ---------------------------------------------------------------------------
// Scoring helpers
// ---------------------------------------------------------------------------

pub(crate) fn weighted_score(results: &[&FixtureResult]) -> f64 {
    let mut num = 0.0;
    let mut den = 0.0;
    for r in results {
        if let Some(v) = r.status.value() {
            num += r.weight * v;
            den += r.weight;
        }
    }
    if den == 0.0 {
        0.0
    } else {
        round2(100.0 * num / den)
    }
}

/// Collect ids of fixtures tagged `expected_support == "unsupported"` that
/// nonetheless scored PASS. Surfaced (not gated) so the run still completes.
pub(crate) fn collect_suspect_unsupported_pass(results: &[FixtureResult]) -> Vec<String> {
    let mut v: Vec<String> = results
        .iter()
        .filter(|r| r.expected_support == "unsupported" && r.status == Status::Pass)
        .map(|r| r.id.clone())
        .collect();
    v.sort();
    v
}

/// "Fix these first": rank substrate probes / base ids by how many non-PASS
/// downstream fixtures depend on them (and which are themselves non-PASS).
pub(crate) fn compute_fix_first(results: &[FixtureResult]) -> Vec<FixFirst> {
    let mut status_of: BTreeMap<String, Status> = BTreeMap::new();
    let mut feature_of: BTreeMap<String, String> = BTreeMap::new();
    for r in results {
        status_of.insert(r.id.clone(), r.status);
        feature_of.insert(r.id.clone(), r.feature.clone());
    }
    // probe/base id -> non-PASS dependents.
    let mut confound: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in results {
        if r.status == Status::Pass {
            continue;
        }
        for d in r.depends_on.iter().chain(r.base_ids.iter()) {
            if matches!(status_of.get(d), Some(s) if *s != Status::Pass) {
                confound.entry(d.clone()).or_default().push(r.id.clone());
            }
        }
    }
    let mut ranked: Vec<FixFirst> = confound
        .into_iter()
        .map(|(id, mut deps)| {
            deps.sort();
            FixFirst {
                feature: feature_of.get(&id).cloned().unwrap_or_default(),
                status: status_of
                    .get(&id)
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| "?".to_string()),
                confounded_count: deps.len() as u32,
                confounded_ids: deps,
                id,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.confounded_count
            .cmp(&a.confounded_count)
            .then(a.id.cmp(&b.id))
    });
    ranked
}

// ---------------------------------------------------------------------------
// Breadth metrics (honest — no fabricated "% of all CSS" denominator)
// ---------------------------------------------------------------------------

/// Breadth, not score: how many distinct (category/feature) pairs we even probe,
/// plus a fixture-count breakdown by expected_support. There is intentionally no
/// `coverage_pct` against a whole-CSS total — that denominator does not exist.
pub(crate) fn compute_coverage(report: &Report) -> Coverage {
    let mut covered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut implemented = 0u32;
    let mut partial = 0u32;
    let mut unsupported = 0u32;
    for c in &report.categories {
        for f in &c.features {
            // Probes are substrate canaries, not taxonomy surface entries.
            if c.category != "probes" {
                covered.insert(format!("{}/{}", c.category, f.feature));
            }
            for fx in &f.fixtures {
                match fx.expected_support.as_str() {
                    "partial" => partial += 1,
                    "unsupported" => unsupported += 1,
                    _ => implemented += 1,
                }
            }
        }
    }
    let n = covered.len() as u32;
    Coverage {
        features_with_fixture: n,
        covered: covered.into_iter().collect(),
        implemented,
        partial,
        unsupported,
    }
}

pub(crate) fn build_report(mut results: Vec<FixtureResult>, pdftoppm_available: bool) -> Report {
    results.sort_by(|a, b| {
        (a.category.clone(), a.feature.clone(), a.id.clone()).cmp(&(
            b.category.clone(),
            b.feature.clone(),
            b.id.clone(),
        ))
    });

    // Group category -> feature -> [results]
    let mut cat_map: BTreeMap<String, BTreeMap<String, Vec<FixtureResult>>> = BTreeMap::new();
    for r in &results {
        cat_map
            .entry(r.category.clone())
            .or_default()
            .entry(r.feature.clone())
            .or_default()
            .push(r.clone());
    }

    let mut categories = Vec::new();
    let mut overall_counts = Counts::default();
    for (cat, feats) in &cat_map {
        let mut feat_reports = Vec::new();
        let mut cat_counts = Counts::default();
        let mut cat_results: Vec<&FixtureResult> = Vec::new();
        for (feat, fxs) in feats {
            let mut counts = Counts::default();
            for fx in fxs {
                counts.add(fx.status);
                cat_counts.add(fx.status);
                overall_counts.add(fx.status);
            }
            let refs: Vec<&FixtureResult> = fxs.iter().collect();
            cat_results.extend(refs.iter().copied());
            feat_reports.push(FeatureReport {
                feature: feat.clone(),
                score_pct: weighted_score(&refs),
                counts,
                fixtures: fxs.clone(),
            });
        }
        categories.push(CategoryReport {
            category: cat.clone(),
            score_pct: weighted_score(&cat_results),
            counts: cat_counts,
            features: feat_reports,
        });
    }

    let all_refs: Vec<&FixtureResult> = results.iter().collect();
    let total = results.len() as u32;
    let scored = overall_counts.pass + overall_counts.partial + overall_counts.fail;
    let scored_ratio = if total == 0 {
        0.0
    } else {
        round2(100.0 * scored as f64 / total as f64)
    };

    Report {
        schema_version: 4,
        env: EnvBlock {
            dpi: DPI,
            // Legacy channel tolerance removed in C6; retained as 0 for schema
            // back-compat (see EnvBlock::channel_tol).
            channel_tol: 0,
            white_tol: WHITE_TOL,
            pdftoppm_available,
        },
        overall: Overall {
            score_pct: weighted_score(&all_refs),
            pass: overall_counts.pass,
            partial: overall_counts.partial,
            fail: overall_counts.fail,
            unknown: overall_counts.unknown,
            total,
            scored_ratio_pct: scored_ratio,
        },
        categories,
        coverage: Coverage::default(),
        fix_first: Vec::new(),
        ref_mismatches: Vec::new(),
        suspect_unsupported_pass: Vec::new(),
        stale_refs: Vec::new(),
        refs_lock_present: false,
        stale_coords: Vec::new(),
        coords_lock_present: false,
        calibration: None,
    }
}

// ---------------------------------------------------------------------------
// Regression gate
// ---------------------------------------------------------------------------

pub(crate) fn enforce_gate(baseline: Option<&Report>, current: &Report) -> Result<(), String> {
    let Some(base) = baseline else {
        eprintln!(
            "parity: no committed baseline report.json — first run, writing baseline and passing."
        );
        return Ok(());
    };

    let base_by_id = base.by_id();
    let cur_by_id = current.by_id();

    let mut problems: Vec<String> = Vec::new();

    // 1. Named PASS -> FAIL regressions.
    for (id, cur) in &cur_by_id {
        if cur.status == Status::Fail {
            if let Some(b) = base_by_id.get(id) {
                if b.status == Status::Pass {
                    let sub = if !cur.interaction_of.is_empty() {
                        format!("interaction {}", cur.interaction_of.join("×"))
                    } else {
                        cur.subfeature.clone()
                    };
                    problems.push(format!(
                        "PASS->FAIL regression: {} [{}/{}/{}] diff={:.2}% {}",
                        id, cur.category, cur.feature, sub, cur.diff_pct, cur.note
                    ));
                }
            }
        }
    }

    // 2. Overall-score regression beyond epsilon — measured over the COMMON
    // fixture set (ids in BOTH baseline and current). ADDING fixtures (e.g.
    // tracked-unsupported coverage gaps from the spec audit) legitimately lowers
    // the whole-corpus score without being a regression, so the delta must ignore
    // newly-added ids; only a genuine degradation of pre-existing fixtures trips
    // this. (Named PASS->FAIL above already catches per-fixture breakage.)
    let base_common: Vec<&FixtureResult> = base_by_id
        .iter()
        .filter_map(|(id, fx)| cur_by_id.contains_key(id).then_some(fx))
        .collect();
    let cur_common: Vec<&FixtureResult> = cur_by_id
        .iter()
        .filter_map(|(id, fx)| base_by_id.contains_key(id).then_some(fx))
        .collect();
    let base_score = weighted_score(&base_common);
    let cur_score = weighted_score(&cur_common);
    let delta = base_score - cur_score;
    if delta > SCORE_EPSILON {
        problems.push(format!(
            "overall score regression (common fixtures): {:.2}% -> {:.2}% (drop {:.2}pp > epsilon {:.2})",
            base_score, cur_score, delta, SCORE_EPSILON
        ));
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "parity gate FAILED ({} issue(s)):\n  - {}",
            problems.len(),
            problems.join("\n  - ")
        ))
    }
}

/// Read the committed `refs.lock` (a flat JSON map `{ "<id>": "<sha256>" }`) and
/// compare each scored fixture's current HTML hash against it. Returns
/// `(stale_refs, lock_present)`. A fixture is STALE when its id is absent from
/// the lock (no recorded ref) or the recorded hash differs (fixture changed since
/// the ref was generated). Fixtures with no computed hash (skipped/error before
/// the read) are ignored. Non-gating here: CI enforces, this only surfaces.
pub(crate) fn check_refs_freshness(
    parity_dir: &Path,
    results: &[FixtureResult],
) -> (Vec<StaleRef>, bool) {
    let lock_path = parity_dir.join("refs.lock");
    let lock: Option<BTreeMap<String, String>> = std::fs::read_to_string(&lock_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let Some(lock) = lock else {
        return (Vec::new(), false);
    };
    let mut stale: Vec<StaleRef> = Vec::new();
    for r in results {
        // Only meaningful when we actually hashed the fixture.
        if r.html_sha256.is_empty() {
            continue;
        }
        match lock.get(&r.id) {
            Some(locked) if *locked == r.html_sha256 => {} // fresh
            Some(locked) => stale.push(StaleRef {
                id: r.id.clone(),
                category: r.category.clone(),
                reason: "hash-mismatch".to_string(),
                current_sha256: r.html_sha256.clone(),
                locked_sha256: locked.clone(),
            }),
            None => stale.push(StaleRef {
                id: r.id.clone(),
                category: r.category.clone(),
                reason: "absent-from-lock".to_string(),
                current_sha256: r.html_sha256.clone(),
                locked_sha256: String::new(),
            }),
        }
    }
    stale.sort_by(|a, b| {
        (a.category.as_str(), a.id.as_str()).cmp(&(b.category.as_str(), b.id.as_str()))
    });
    (stale, true)
}

/// Read the committed `coords.lock` (a flat JSON map `{ "<id>": "<sha256>" }`,
/// mirroring `refs.lock`) and compare each scored fixture's current HTML hash
/// against it. Returns `(stale_coords, lock_present)`. Mirrors
/// `check_refs_freshness` BUT only fixtures that actually have a committed
/// coordinate SIDECAR are tracked: unlike refs (one per fixture), sidecars exist
/// only for the curated geometry-clean starter set (probes/block/grid/flex in
/// Phase 2b), so a fixture absent from the lock is expected (raster-only) and
/// must NOT be flagged. A fixture that IS in the lock but whose HTML hash differs
/// is STALE — its sidecar describes an older fixture and must be regenerated with
/// `scripts/parity-gen-coords.sh`. Non-gating here: CI enforces, this surfaces.
pub(crate) fn check_coords_freshness(
    parity_dir: &Path,
    results: &[FixtureResult],
) -> (Vec<StaleRef>, bool) {
    let lock_path = parity_dir.join("coords.lock");
    let lock: Option<BTreeMap<String, String>> = std::fs::read_to_string(&lock_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let Some(lock) = lock else {
        return (Vec::new(), false);
    };
    let mut stale: Vec<StaleRef> = Vec::new();
    for r in results {
        if r.html_sha256.is_empty() {
            continue;
        }
        // Only fixtures with a committed sidecar are in the lock; absence is
        // expected (raster-only) and never a staleness defect.
        match lock.get(&r.id) {
            Some(locked) if *locked == r.html_sha256 => {} // fresh
            Some(locked) => stale.push(StaleRef {
                id: r.id.clone(),
                category: r.category.clone(),
                reason: "hash-mismatch".to_string(),
                current_sha256: r.html_sha256.clone(),
                locked_sha256: locked.clone(),
            }),
            None => {} // no sidecar for this fixture — fine.
        }
    }
    stale.sort_by(|a, b| {
        (a.category.as_str(), a.id.as_str()).cmp(&(b.category.as_str(), b.id.as_str()))
    });
    (stale, true)
}
