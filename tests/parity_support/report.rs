//! Report data model (also the regression baseline schema), the per-fixture
//! result constructors, and all artifact writers: `report.json`, `REPORT.md`,
//! and the in-repo visual HTML galleries.
//!
//! Extracted verbatim from the former monolithic `mod.rs` (C1 mechanical split).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::diagnose::Diagnosis;
use super::manifest::ManifestEntry;
use super::overlay::{LEGEND_ORDER, class_label, class_rgb};

// ---------------------------------------------------------------------------
// Report schema (also the regression baseline)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Status {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "PARTIAL")]
    Partial,
    #[serde(rename = "FAIL")]
    Fail,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl Status {
    pub(crate) fn value(self) -> Option<f64> {
        match self {
            Status::Pass => Some(1.0),
            Status::Partial => Some(0.5),
            Status::Fail => Some(0.0),
            Status::Unknown => None,
        }
    }
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Partial => "PARTIAL",
            Status::Fail => "FAIL",
            Status::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct FixtureResult {
    pub(crate) id: String,
    pub(crate) category: String,
    pub(crate) feature: String,
    #[serde(default)]
    pub(crate) subfeature: String,
    #[serde(default)]
    pub(crate) interaction_of: Vec<String>,
    #[serde(default)]
    pub(crate) base_ids: Vec<String>,
    pub(crate) status: Status,
    pub(crate) diff_pct: f64,
    pub(crate) weight: f64,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) note: String,
    // ---- new substrate-attribution fields ----
    #[serde(default = "super::manifest::default_kind")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) depends_on: Vec<String>,
    #[serde(default = "super::manifest::default_expected_support")]
    pub(crate) expected_support: String,
    /// Reference ORACLE that produced `refs/<cat>/<id>.png` ("chrome" default,
    /// "weasyprint" for CSS GCPM features Chrome's print path renders blank,
    /// "none" = no oracle). Surfaced in the report so a non-Chrome comparison is
    /// clearly labelled (Chrome+Paged.js marked unsupported for that fixture).
    #[serde(default = "super::manifest::default_oracle")]
    pub(crate) oracle: String,
    /// Root-cause attribution for non-PASS fixtures. "" for PASS / not computed.
    /// "REAL" -> the named feature is itself wrong; "CONFOUNDED: <probe feature>"
    /// -> a depended substrate probe is non-PASS so the failure is likely there.
    #[serde(default)]
    pub(crate) attribution: String,
    /// SHA-256 of the fixture HTML (`cases/<cat>/<id>.html`), lowercase hex. Used
    /// to verify the committed reference is still fresh against `refs.lock`. Not
    /// part of the regression baseline comparison; carried for the freshness check.
    #[serde(default)]
    pub(crate) html_sha256: String,
    /// Per-fixture V2 diagnosis (spec §2): the "why it failed" layer — primary
    /// error class, human headline, magnitudes, per-region breakdown. ADDITIVE and
    /// non-gating: set on every scored fixture; `None` only on UNKNOWN/error
    /// results and on pre-V2 baselines (the `serde(default)` keeps them parseable).
    #[serde(default)]
    pub(crate) diagnosis: Option<super::diagnose::Diagnosis>,
    /// Per-concern sub-verdicts from the pluggable multi-verifier layer (spec §1).
    /// ADDITIVE and non-gating: in Phase 1 these are the `RasterVerifier`'s three
    /// concern opinions (Geometry/Appearance/Presence) that combine to `status`.
    /// `#[serde(default)]` keeps pre-verifier baselines parseable.
    #[serde(default)]
    pub(crate) sub_verdicts: Vec<super::verify::SubVerdict>,
    /// Recorded cross-verifier disagreements (spec §1.3). Empty in Phase 1 (a
    /// single verifier cannot disagree with itself). ADDITIVE, non-gating.
    #[serde(default)]
    pub(crate) disagreements: Vec<super::verify::Disagreement>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub(crate) struct Counts {
    pub(crate) pass: u32,
    pub(crate) partial: u32,
    pub(crate) fail: u32,
    pub(crate) unknown: u32,
}

impl Counts {
    pub(crate) fn add(&mut self, s: Status) {
        match s {
            Status::Pass => self.pass += 1,
            Status::Partial => self.partial += 1,
            Status::Fail => self.fail += 1,
            Status::Unknown => self.unknown += 1,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct FeatureReport {
    pub(crate) feature: String,
    pub(crate) score_pct: f64,
    pub(crate) counts: Counts,
    pub(crate) fixtures: Vec<FixtureResult>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct CategoryReport {
    pub(crate) category: String,
    pub(crate) score_pct: f64,
    pub(crate) counts: Counts,
    pub(crate) features: Vec<FeatureReport>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Overall {
    pub(crate) score_pct: f64,
    pub(crate) pass: u32,
    pub(crate) partial: u32,
    pub(crate) fail: u32,
    pub(crate) unknown: u32,
    pub(crate) total: u32,
    pub(crate) scored_ratio_pct: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct EnvBlock {
    pub(crate) dpi: u32,
    /// Legacy per-channel pixel-diff tolerance. The legacy comparator that used it
    /// was removed in C6; the field is retained (serde-default, always 0) only to
    /// keep `report.json` schema-compatible with older baselines.
    #[serde(default)]
    pub(crate) channel_tol: i32,
    pub(crate) white_tol: i32,
    pub(crate) pdftoppm_available: bool,
}

/// One entry in the "Fix these first" ranked list: a substrate probe / base id
/// ordered by how many non-PASS downstream fixtures it confounds.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct FixFirst {
    pub(crate) id: String,
    pub(crate) feature: String,
    pub(crate) status: String,
    pub(crate) confounded_count: u32,
    pub(crate) confounded_ids: Vec<String>,
}

/// Honest breadth metrics. Deliberately NOT a percentage of "all of CSS": there
/// is no credible denominator for that, so any "X/199 = 100%" figure is a
/// tautology. Instead we report (a) how many distinct category/feature pairs
/// have at least one fixture, and (b) the fixture count by expected_support.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub(crate) struct Coverage {
    /// Number of distinct (category/feature) pairs with >= 1 fixture.
    pub(crate) features_with_fixture: u32,
    /// Those distinct (category/feature) labels.
    pub(crate) covered: Vec<String>,
    /// Fixture counts grouped by `expected_support`.
    pub(crate) implemented: u32,
    pub(crate) partial: u32,
    pub(crate) unsupported: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Report {
    pub(crate) schema_version: u32,
    pub(crate) env: EnvBlock,
    pub(crate) overall: Overall,
    pub(crate) categories: Vec<CategoryReport>,
    #[serde(default)]
    pub(crate) coverage: Coverage,
    #[serde(default)]
    pub(crate) fix_first: Vec<FixFirst>,
    /// Manifest ids whose expected ref PNG (`refs/<cat>/<id>.png`) is absent
    /// while the category dir DOES contain ref PNG(s) not claimed by any id —
    /// i.e. an id != ref-filename mismatch (a permanent UNKNOWN footgun), as
    /// opposed to a ref that was simply never generated.
    #[serde(default)]
    pub(crate) ref_mismatches: Vec<RefMismatch>,
    /// Fixtures that are tagged `expected_support == "unsupported"` yet scored
    /// PASS — the tag or the feature implementation is suspect.
    #[serde(default)]
    pub(crate) suspect_unsupported_pass: Vec<String>,
    /// Fixtures whose committed reference is STALE relative to `refs.lock`: the
    /// fixture HTML's SHA-256 differs from the locked hash, or the id is absent
    /// from the lock entirely. Surfaced (not gated here) so CI can enforce and a
    /// human can regenerate refs. Empty + `refs_lock_present == false` means no
    /// lock was committed yet.
    #[serde(default)]
    pub(crate) stale_refs: Vec<StaleRef>,
    /// Whether a `refs.lock` file was present and parsed. When false, no freshness
    /// claim can be made (every fixture is implicitly "unverified").
    #[serde(default)]
    pub(crate) refs_lock_present: bool,
    /// Fixtures whose committed COORDINATE SIDECAR (`coords/<cat>/<id>.json`) is
    /// STALE relative to `coords.lock`: the fixture HTML's SHA-256 differs from the
    /// locked hash. Only sidecar-bearing fixtures are tracked (Phase 2b ships
    /// sidecars for the starter set only); a fixture with no sidecar is never
    /// flagged. Surfaced (not gated here) so CI can enforce regeneration with
    /// `scripts/parity-gen-coords.sh`.
    #[serde(default)]
    pub(crate) stale_coords: Vec<StaleRef>,
    /// Whether a `coords.lock` file was present and parsed.
    #[serde(default)]
    pub(crate) coords_lock_present: bool,
    /// V2 page-origin calibration audit (spec §1.3). `Some` on every scoring run;
    /// `None` only when pdftoppm is unavailable or on a filtered dev run.
    #[serde(default)]
    pub(crate) calibration: Option<Calibration>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct StaleRef {
    pub(crate) id: String,
    pub(crate) category: String,
    /// "absent-from-lock" or "hash-mismatch".
    pub(crate) reason: String,
    /// Current SHA-256 of `cases/<cat>/<id>.html`.
    pub(crate) current_sha256: String,
    /// The hash recorded in refs.lock (empty when absent).
    pub(crate) locked_sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct RefMismatch {
    pub(crate) id: String,
    pub(crate) category: String,
    pub(crate) expected_ref: String,
    /// Unclaimed ref PNG file names present in the same category dir.
    pub(crate) orphan_refs: Vec<String>,
}

/// V2 page-origin calibration audit (spec §1.3). Emitted once per V2 run from the
/// deterministic rigid probes; a drift from `(4,4)±1` aborts the run loudly so a
/// genuine margin regression is announced, never silently re-absorbed.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Calibration {
    /// The fixed correction applied to every candidate (device px), [dx, dy].
    pub(crate) offset_px: [i32; 2],
    /// Same correction in CSS px.
    pub(crate) offset_css: [f64; 2],
    /// The raw offset actually measured from the probes (device px), [dx, dy].
    pub(crate) measured_px: [i32; 2],
    /// Max per-axis deviation of any probe from the expected offset (device px).
    pub(crate) residual_px: i32,
    /// Whether calibration drifted beyond tolerance (run aborts when true).
    pub(crate) drifted: bool,
}

impl Report {
    /// Flat id -> result lookup across the whole report.
    pub(crate) fn by_id(&self) -> BTreeMap<String, FixtureResult> {
        let mut m = BTreeMap::new();
        for c in &self.categories {
            for f in &c.features {
                for fx in &f.fixtures {
                    m.insert(fx.id.clone(), fx.clone());
                }
            }
        }
        m
    }
}

// ---------------------------------------------------------------------------
// Result constructors
// ---------------------------------------------------------------------------

pub(crate) fn fixture_base(
    entry: &ManifestEntry,
    status: Status,
    diff_pct: f64,
    note: String,
) -> FixtureResult {
    FixtureResult {
        id: entry.id.clone(),
        category: entry.category.clone(),
        feature: entry.feature.clone(),
        subfeature: entry.subfeature.clone(),
        interaction_of: entry.interaction_of.clone(),
        base_ids: entry.base_ids.clone(),
        status,
        diff_pct,
        weight: entry.weight,
        description: entry.description.clone(),
        note,
        kind: entry.kind.clone(),
        depends_on: entry.depends_on.clone(),
        expected_support: entry.expected_support.clone(),
        oracle: entry.oracle.clone(),
        attribution: String::new(),
        html_sha256: String::new(),
        diagnosis: None,
        sub_verdicts: Vec::new(),
        disagreements: Vec::new(),
    }
}

pub(crate) fn fixture_fail(entry: &ManifestEntry, diff_pct: f64, note: String) -> FixtureResult {
    fixture_base(entry, Status::Fail, diff_pct, note)
}

pub(crate) fn fixture_unknown(entry: &ManifestEntry, note: String) -> FixtureResult {
    fixture_base(entry, Status::Unknown, 0.0, note)
}

// ---------------------------------------------------------------------------
// Writers
// ---------------------------------------------------------------------------

pub(crate) fn write_report_json(path: &Path, report: &Report) -> Result<(), String> {
    let mut s = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    s.push('\n');
    std::fs::write(path, s).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub(crate) fn write_report_md(path: &Path, report: &Report) -> Result<(), String> {
    let mut o = String::new();
    let ov = &report.overall;
    o.push_str("# ironpress Feature Parity Report\n\n");
    o.push_str(&format!(
        "Overall: {:.2}%  (PASS {} · PARTIAL {} · FAIL {} · UNKNOWN {} · total {})\n",
        ov.score_pct, ov.pass, ov.partial, ov.fail, ov.unknown, ov.total
    ));
    o.push_str(&format!(
        "Scored coverage: {:.2}% ({} / {} fixtures have a reference)\n",
        ov.scored_ratio_pct,
        ov.pass + ov.partial + ov.fail,
        ov.total
    ));
    o.push_str(&format!(
        "Env: DPI {} · white-tol {} · V2 multi-gate verdict · pdftoppm {}\n",
        report.env.dpi,
        report.env.white_tol,
        if report.env.pdftoppm_available {
            "yes"
        } else {
            "MISSING"
        }
    ));
    o.push_str(&format!(
        "Breadth: {} distinct category/feature pairs have a fixture (NOT a % of all CSS).\n",
        report.coverage.features_with_fixture
    ));
    o.push_str(&format!(
        "By expected_support: implemented {} · partial {} · unsupported {}\n",
        report.coverage.implemented, report.coverage.partial, report.coverage.unsupported
    ));
    o.push_str("Generated by `cargo test --test feature_parity`.\n\n");

    // Calibration audit (spec §3.2). The V2 page-origin correction + drift state.
    // Emitted on every scoring run; absent only when pdftoppm is unavailable or on
    // a filtered dev run (nothing to calibrate against).
    o.push_str("## Calibration\n");
    o.push_str("> The fixed page-origin correction applied to every candidate ");
    o.push_str("(spec §1.3). ironpress's 28.8pt margin is spec-correct; Chrome's ");
    o.push_str("`--print-to-pdf` rounds it, leaving a rigid offset that is ");
    o.push_str("subtracted once and AUDITED — a drift from the deterministic ");
    o.push_str("probes aborts the run rather than re-absorbing a real regression.\n\n");
    match &report.calibration {
        None => {
            o.push_str("Not applied (pdftoppm unavailable or a filtered dev run — ");
            o.push_str("no probes to calibrate against).\n\n");
        }
        Some(cal) => {
            o.push_str(&format!(
                "- offset: **{}, {} device px** ({:.2}, {:.2} CSS px)\n",
                cal.offset_px[0], cal.offset_px[1], cal.offset_css[0], cal.offset_css[1]
            ));
            o.push_str(&format!(
                "- measured (from probes): {}, {} device px · residual {} device px\n",
                cal.measured_px[0], cal.measured_px[1], cal.residual_px
            ));
            o.push_str(&format!(
                "- drifted: **{}**\n\n",
                if cal.drifted {
                    "YES — see WARNING above"
                } else {
                    "no"
                }
            ));
        }
    }

    let by_id = report.by_id();

    // Guard: ref lookup mismatches (id != ref-filename). Loud, near the top.
    o.push_str("## Ref lookup mismatches (id != ref-filename)\n");
    o.push_str("> A manifest id whose expected `refs/<cat>/<id>.png` is missing ");
    o.push_str("WHILE the category dir holds unclaimed ref PNG(s) -> the ref was ");
    o.push_str("committed under the wrong name and the fixture is a permanent ");
    o.push_str("silent UNKNOWN. Rename the ref to `<id>.png` to fix.\n\n");
    if report.ref_mismatches.is_empty() {
        o.push_str("None.\n\n");
    } else {
        o.push_str(&format!(
            "**{} mismatch(es) found:**\n\n",
            report.ref_mismatches.len()
        ));
        o.push_str("| category | id | expected ref (not found) | orphan ref(s) present |\n");
        o.push_str("|----------|----|--------------------------|-----------------------|\n");
        for m in &report.ref_mismatches {
            o.push_str(&format!(
                "| {} | `{}` | `{}` | {} |\n",
                m.category,
                m.id,
                m.expected_ref,
                m.orphan_refs.join(", ")
            ));
        }
        o.push('\n');
    }

    // Guard: unsupported-but-PASS suspects (tag or feature is wrong).
    o.push_str("## Suspect: unsupported-but-PASS (re-check tag or feature)\n");
    o.push_str("> Fixtures tagged `expected_support == \"unsupported\"` that ");
    o.push_str("nonetheless PASSed. Either the feature IS implemented (fix the ");
    o.push_str("tag) or the fixture/ref is not exercising it. Surfaced, not gated.\n\n");
    if report.suspect_unsupported_pass.is_empty() {
        o.push_str("None.\n\n");
    } else {
        o.push_str(&format!(
            "**{} suspect(s):** {}\n\n",
            report.suspect_unsupported_pass.len(),
            report
                .suspect_unsupported_pass
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Stale references — prominent, near the top: a fixture's HTML changed since
    // its committed ref PNG was generated (hash != refs.lock), so the ref no
    // longer matches what we score. Regenerate before trusting these rows.
    o.push_str("## Stale references (regenerate)\n");
    o.push_str("> A fixture whose HTML SHA-256 differs from `refs.lock` (or is ");
    o.push_str("absent from it): the committed reference PNG was generated from an ");
    o.push_str("older fixture and is STALE. Run `scripts/parity-gen-refs.sh` to ");
    o.push_str("regenerate refs + the lock. (Surfaced here; CI enforces the gate.)\n\n");
    if !report.refs_lock_present {
        o.push_str("**No `refs.lock` committed** — reference freshness is UNVERIFIED. ");
        o.push_str("Run `scripts/parity-gen-refs.sh` to write the lock.\n\n");
    } else if report.stale_refs.is_empty() {
        o.push_str("None — every fixture's HTML matches `refs.lock`.\n\n");
    } else {
        o.push_str(&format!(
            "**{} stale reference(s) found:**\n\n",
            report.stale_refs.len()
        ));
        o.push_str("| category | id | reason | current sha256 | locked sha256 |\n");
        o.push_str("|----------|----|--------|----------------|---------------|\n");
        for s in &report.stale_refs {
            let short = |h: &str| -> String {
                if h.len() >= 12 {
                    h[..12].to_string()
                } else {
                    h.to_string()
                }
            };
            let locked = if s.locked_sha256.is_empty() {
                "—".to_string()
            } else {
                short(&s.locked_sha256)
            };
            o.push_str(&format!(
                "| {} | `{}` | {} | `{}` | `{}` |\n",
                s.category,
                s.id,
                s.reason,
                short(&s.current_sha256),
                locked
            ));
        }
        o.push('\n');
    }

    // Regressions / Failures section first. KNOWN GAPS (expected_support !=
    // implemented) are intentionally excluded here and listed separately below.
    o.push_str("## Regressions / Failures\n");
    o.push_str("> Real regressions only (known gaps are in their own section). ");
    o.push_str("`attribution` = REAL (the named feature is wrong) vs ");
    o.push_str("CONFOUNDED (a substrate it depends on is broken).\n\n");
    let mut fails: Vec<&FixtureResult> = Vec::new();
    for c in &report.categories {
        for f in &c.features {
            for fx in &f.fixtures {
                if fx.status == Status::Fail && fx.expected_support == "implemented" {
                    fails.push(fx);
                }
            }
        }
    }
    if fails.is_empty() {
        o.push_str("No failures or regressions.\n\n");
    } else {
        // `class` (diagnosis.primary_class) and `reason` (diagnosis.headline)
        // surface the V2 "why it failed" layer inline (spec §3.2). Both are "—" on
        // the legacy path / old baselines where no diagnosis was computed.
        o.push_str("| status | attribution | class | diff% | category | feature | subfeature | id | reason |\n");
        o.push_str("|--------|-------------|-------|------:|----------|---------|-----------|----|--------|\n");
        for fx in &fails {
            let sub = if !fx.interaction_of.is_empty() {
                let kind = interaction_kind(fx, &by_id);
                format!("(interaction: {}) {}", fx.interaction_of.join("×"), kind)
            } else {
                fx.subfeature.clone()
            };
            let attr = if fx.attribution.is_empty() {
                "REAL".to_string()
            } else {
                fx.attribution.clone()
            };
            // Prefer the diagnosis headline as the reason; fall back to the note.
            let reason = diag_reason(fx).unwrap_or_else(|| fx.note.clone());
            o.push_str(&format!(
                "| FAIL | {} | {} | {:.2} | {} | {} | {} | {} | {} |\n",
                attr,
                diag_class(fx),
                fx.diff_pct,
                fx.category,
                fx.feature,
                sub,
                fx.id,
                reason
            ));
        }
        o.push('\n');
    }

    // Fix these first — substrate probes/bases ranked by downstream confounds.
    o.push_str("## Fix these first\n");
    o.push_str("> Substrate probes / base fixtures ranked by how many non-PASS ");
    o.push_str("downstream fixtures they confound. Fixing the top of this list ");
    o.push_str("should unblock the most dependents.\n\n");
    if report.fix_first.is_empty() {
        o.push_str("Nothing confounded — every depended substrate PASSes.\n\n");
    } else {
        o.push_str("| rank | id | feature | status | confounds | dependents |\n");
        o.push_str("|-----:|----|---------|--------|----------:|------------|\n");
        for (i, ff) in report.fix_first.iter().enumerate() {
            let deps = if ff.confounded_ids.len() > 6 {
                format!(
                    "{} …(+{})",
                    ff.confounded_ids[..6].join(", "),
                    ff.confounded_ids.len() - 6
                )
            } else {
                ff.confounded_ids.join(", ")
            };
            o.push_str(&format!(
                "| {} | `{}` | {} | {} | {} | {} |\n",
                i + 1,
                ff.id,
                ff.feature,
                ff.status,
                ff.confounded_count,
                deps
            ));
        }
        o.push('\n');
    }

    // Coverage by category.
    o.push_str("## Coverage by Category\n");
    o.push_str("| category | score | pass | partial | fail | unknown |\n");
    o.push_str("|----------|------:|-----:|--------:|-----:|--------:|\n");
    for c in &report.categories {
        o.push_str(&format!(
            "| {} | {:.2}% | {} | {} | {} | {} |\n",
            c.category,
            c.score_pct,
            c.counts.pass,
            c.counts.partial,
            c.counts.fail,
            c.counts.unknown
        ));
    }
    o.push('\n');

    // Known gaps (expected_support != implemented) — distinct from regressions.
    let mut gaps: Vec<&FixtureResult> = Vec::new();
    for c in &report.categories {
        for f in &c.features {
            for fx in &f.fixtures {
                if fx.expected_support != "implemented" {
                    gaps.push(fx);
                }
            }
        }
    }
    o.push_str("## Known gaps (expected_support != implemented)\n");
    o.push_str("> Fixtures targeting features ironpress is NOT expected to fully ");
    o.push_str("support. These are tracked for breadth, not counted as regressions.\n\n");
    if gaps.is_empty() {
        o.push_str("None.\n\n");
    } else {
        o.push_str("| expected | status | diff% | category | feature | id | description |\n");
        o.push_str("|----------|--------|------:|----------|---------|----|-------------|\n");
        for fx in &gaps {
            o.push_str(&format!(
                "| {} | {} | {:.2} | {} | {} | {} | {} |\n",
                fx.expected_support,
                fx.status.as_str(),
                fx.diff_pct,
                fx.category,
                fx.feature,
                fx.id,
                fx.description
            ));
        }
        o.push('\n');
    }

    // Detail tree.
    o.push_str("## Detail\n");
    for c in &report.categories {
        o.push_str(&format!("### {} — {:.2}%\n", c.category, c.score_pct));
        for f in &c.features {
            o.push_str(&format!("- **{}** — {:.2}%\n", f.feature, f.score_pct));
            for fx in &f.fixtures {
                let label = if !fx.subfeature.is_empty() {
                    format!("{}={}", fx.feature, fx.subfeature)
                } else {
                    fx.feature.clone()
                };
                o.push_str(&format!(
                    "  - {} {:.2}% {} — `{}` — {}\n",
                    fx.status.as_str(),
                    fx.diff_pct,
                    label,
                    fx.id,
                    fx.description
                ));
            }
        }
        o.push('\n');
    }

    // Untested.
    let mut unknowns: Vec<&FixtureResult> = Vec::new();
    for c in &report.categories {
        for f in &c.features {
            for fx in &f.fixtures {
                if fx.status == Status::Unknown {
                    unknowns.push(fx);
                }
            }
        }
    }
    if !unknowns.is_empty() {
        o.push_str("## Untested (UNKNOWN — no reference yet)\n");
        for fx in &unknowns {
            o.push_str(&format!(
                "- {} / {} — `{}` ({})\n",
                fx.category, fx.feature, fx.id, fx.note
            ));
        }
        o.push('\n');
    }

    std::fs::write(path, o).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// In-repo visual HTML reports (triptych galleries)
// ---------------------------------------------------------------------------

/// The V2 diagnosis primary class for a fixture, or "—" when none was computed
/// (legacy verdict path / old baseline). Used by the Markdown `class` column and
/// the HTML header chip.
pub(crate) fn diag_class(fx: &FixtureResult) -> String {
    fx.diagnosis
        .as_ref()
        .filter(|d| !d.primary_class.is_empty())
        .map(|d| d.primary_class.clone())
        .unwrap_or_else(|| "—".to_string())
}

/// The V2 diagnosis headline (human reason) for a fixture, if one was computed.
pub(crate) fn diag_reason(fx: &FixtureResult) -> Option<String> {
    fx.diagnosis
        .as_ref()
        .map(|d| d.headline.clone())
        .filter(|h| !h.is_empty())
}

/// Minimal HTML-attribute/text escaper (no external deps).
pub(crate) fn html_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

/// Sort rank: FAIL (0) < PARTIAL (1) < PASS (2) < UNKNOWN (3) so failures float
/// to the top of every gallery.
pub(crate) fn status_rank(s: Status) -> u8 {
    match s {
        Status::Fail => 0,
        Status::Partial => 1,
        Status::Pass => 2,
        Status::Unknown => 3,
    }
}

pub(crate) fn status_color(s: Status) -> &'static str {
    match s {
        Status::Pass => "#1a7f37",
        Status::Partial => "#9a6700",
        Status::Fail => "#cf222e",
        Status::Unknown => "#57606a",
    }
}

/// Shared inline stylesheet + a tiny client-side sort hook for the per-category
/// tables. Fully self-contained (no external/CDN assets).
pub(crate) fn report_css() -> &'static str {
    "<style>\
:root{--bg:#fff;--fg:#1f2328;--muted:#57606a;--line:#d0d7de;--card:#f6f8fa;}\
*{box-sizing:border-box}\
body{margin:0;padding:24px;font:14px/1.5 -apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif;color:var(--fg);background:var(--bg)}\
h1{font-size:22px;margin:0 0 4px}h2{font-size:17px;margin:28px 0 8px}\
a{color:#0969da;text-decoration:none}a:hover{text-decoration:underline}\
.meta{color:var(--muted);font-size:13px;margin:0 0 16px}\
.badge{display:inline-block;padding:1px 8px;border-radius:999px;color:#fff;font-weight:600;font-size:12px}\
table{border-collapse:collapse;width:100%;margin:8px 0 24px;font-size:13px}\
th,td{border:1px solid var(--line);padding:6px 8px;text-align:left;vertical-align:top}\
th{background:var(--card);position:sticky;top:0;cursor:pointer;user-select:none}\
tr:nth-child(even) td{background:#fafbfc}\
.num{text-align:right;font-variant-numeric:tabular-nums}\
.trip{display:flex;gap:4px;flex-wrap:nowrap}\
.trip figure{margin:0;flex:1;min-width:0}\
.trip figcaption{font-size:11px;color:var(--muted);text-align:center;margin-top:2px}\
.trip img{width:100%;max-width:240px;height:auto;border:1px solid var(--line);background:\
repeating-conic-gradient(#eee 0% 25%,#fff 0% 50%) 50%/16px 16px;display:block}\
.desc{color:var(--muted);max-width:34ch}\
.real{color:#cf222e;font-weight:600}.confound{color:#9a6700}\
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:8px}\
.jump{font-size:13px;margin:4px 0 18px;line-height:2.1}\
.jump a{display:inline-block;padding:2px 8px;margin:2px 2px 2px 0;border:1px solid var(--line);border-radius:999px;background:var(--card)}\
.jump .sc{color:var(--muted);font-variant-numeric:tabular-nums}\
section.feat{margin:0 0 26px}\
section.feat h2{display:flex;align-items:baseline;gap:10px;border-bottom:2px solid var(--line);padding-bottom:4px}\
section.feat h2 .top{margin-left:auto;font-size:12px;font-weight:400}\
.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(360px,1fr));gap:12px;margin-top:10px}\
.card{border:1px solid var(--line);border-radius:8px;padding:8px;background:#fff}\
.chead{font-size:13px;margin-bottom:6px;display:flex;align-items:center;gap:6px;flex-wrap:wrap}\
details{margin:4px 0;border:1px solid var(--line);border-radius:6px;padding:6px 10px;background:var(--card)}\
summary{cursor:pointer;font-weight:600}\
.featlist{margin:6px 0 4px;columns:2;font-size:13px;list-style:none;padding:0}\
.featlist li{break-inside:avoid;margin:2px 0}\
.chip{display:inline-block;padding:1px 7px;border-radius:6px;font-size:11px;font-weight:600;border:1px solid var(--line);background:var(--card);font-variant-numeric:tabular-nums}\
.chip.cls{color:#fff;border:0}\
.quad{display:flex;gap:4px;flex-wrap:nowrap;margin:4px 0}\
.pglabel{font-size:11px;font-weight:600;color:var(--muted);margin:6px 0 0}\
.quad figure{margin:0;flex:1;min-width:0}\
.quad figcaption{font-size:11px;color:var(--muted);text-align:center;margin-top:2px}\
.quad img{width:100%;max-width:240px;height:auto;border:1px solid var(--line);background:\
repeating-conic-gradient(#eee 0% 25%,#fff 0% 50%) 50%/16px 16px;display:block}\
.legend{display:flex;flex-wrap:wrap;gap:4px 12px;font-size:11px;color:var(--muted);margin:4px 0;padding:4px 6px;border:1px dashed var(--line);border-radius:6px}\
.legend .lg{display:inline-flex;align-items:center;gap:4px}\
.legend .sw{display:inline-block;width:11px;height:11px;border-radius:2px;border:1px solid rgba(0,0,0,.15)}\
.legend .note{flex-basis:100%;color:#9a6700;font-style:italic}\
.regtbl{font-size:12px;margin:6px 0 2px}.regtbl th{position:static;cursor:default}\
.src{margin:4px 0}.src pre{margin:0;max-height:280px;overflow:auto;background:#0d1117;color:#e6edf3;border-radius:6px;padding:8px 10px;font:12px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}\
.src code{white-space:pre}.src .ln{color:#6e7681;display:inline-block;width:3ch;text-align:right;margin-right:12px;user-select:none}\
.filterbar{display:flex;gap:8px;align-items:center;flex-wrap:wrap;margin:8px 0 14px;font-size:13px}\
.filterbar label{color:var(--muted)}.filterbar select,.filterbar input{font:13px inherit;padding:2px 6px;border:1px solid var(--line);border-radius:6px}\
.fxrow.hidden{display:none}\
</style>\
<script>\
function sortTable(t,col){var tb=t.tBodies[0];var rows=[].slice.call(tb.rows);\
var asc=t.getAttribute('data-sort')!=col+'a';\
rows.sort(function(a,b){var x=a.cells[col].getAttribute('data-k')||a.cells[col].innerText;\
var y=b.cells[col].getAttribute('data-k')||b.cells[col].innerText;\
var nx=parseFloat(x),ny=parseFloat(y);\
if(!isNaN(nx)&&!isNaN(ny)){return asc?nx-ny:ny-nx;}\
return asc?x.localeCompare(y):y.localeCompare(x);});\
rows.forEach(function(r){tb.appendChild(r);});\
t.setAttribute('data-sort',col+(asc?'a':'d'));}\
function filterCards(){\
var st=(document.getElementById('f-status')||{}).value||'';\
var cl=(document.getElementById('f-class')||{}).value||'';\
var md=parseFloat((document.getElementById('f-diff')||{}).value)||0;\
var cards=document.querySelectorAll('.card.fxrow');var shown=0;\
cards.forEach(function(c){\
var ok=(!st||c.getAttribute('data-status')===st)&&(!cl||c.getAttribute('data-class')===cl)&&(parseFloat(c.getAttribute('data-diff'))>=md);\
c.classList.toggle('hidden',!ok);if(ok)shown++;});\
var fc=document.getElementById('f-count');if(fc)fc.textContent=shown+' / '+cards.length+' fixtures';}\
function sortCards(){\
var mode=(document.getElementById('f-sort')||{}).value||'fail';\
var rank={FAIL:0,PARTIAL:1,PASS:2,UNKNOWN:3};\
document.querySelectorAll('.cards').forEach(function(grid){\
var cards=[].slice.call(grid.querySelectorAll('.card.fxrow'));\
cards.sort(function(a,b){\
var da=parseFloat(a.getAttribute('data-diff'))||0,db=parseFloat(b.getAttribute('data-diff'))||0;\
if(mode==='diffd')return db-da;if(mode==='diffa')return da-db;\
var ra=rank[a.getAttribute('data-status')],rb=rank[b.getAttribute('data-status')];\
return ra!==rb?ra-rb:db-da;});\
cards.forEach(function(c){grid.appendChild(c);});});}\
function filterWorklist(){\
var cl=(document.getElementById('wl-class')||{}).value||'';\
document.querySelectorAll('tr.wl').forEach(function(r){\
r.style.display=(!cl||r.getAttribute('data-class')===cl)?'':'none';});}\
</script>"
}

pub(crate) fn status_badge(s: Status) -> String {
    format!(
        "<span class=\"badge\" style=\"background:{}\">{}</span>",
        status_color(s),
        s.as_str()
    )
}

/// CSS colour for a diagnosis `ErrorClass` chip. Mirrors the overlay palette where
/// a 1:1 class exists (Missing/Extra/GeometryShift/ColorValue), and gives the
/// derived colour sub-classes (ColorSpace/AlphaCompositing) and GeometrySize their
/// own readable hues so the chip is self-describing. AaOnly is the muted ceiling.
pub(crate) fn diag_class_color(class: &str) -> &'static str {
    match class {
        "Missing" => "#e600e6",          // magenta (overlay Missing)
        "Extra" => "#1a9e3c",            // green (overlay Extra)
        "ColorValue" => "#2850ff",       // blue (overlay ColorErr)
        "ColorSpace" => "#0a8a8a",       // teal (gradient/blend drift)
        "AlphaCompositing" => "#7a3cc0", // purple (opacity)
        "GeometryShift" => "#d2691e",    // orange (overlay GeomShift)
        "GeometrySize" => "#b8860b",     // dark goldenrod (box-size)
        "AaOnly" => "#9a6700",           // amber (measurement ceiling)
        _ => "#57606a",                  // grey (unknown / none)
    }
}

/// The always-visible legend (spec §3.3 item 3): overlay colour -> class, plus the
/// "AA = measurement ceiling, not a bug" note. Shares the overlay palette
/// (`overlay::class_rgb`/`class_label`) so the swatches match the rendered diff
/// exactly.
pub(crate) fn render_legend() -> String {
    let mut o = String::from("<div class=\"legend\"><strong>diff colours:</strong>");
    for c in LEGEND_ORDER {
        let [r, g, b] = class_rgb(c);
        o.push_str(&format!(
            "<span class=\"lg\"><span class=\"sw\" style=\"background:rgb({r},{g},{b})\"></span>{}</span>",
            html_escape(class_label(c))
        ));
    }
    o.push_str(
        "<span class=\"note\">AA / edge-jitter = cross-rasterizer measurement ceiling, not a bug.</span>",
    );
    o.push_str("</div>");
    o
}

/// The magnitude chips for a fixture's header bar (spec §3.3 item 1), read from the
/// diagnosis: a `class` chip plus per-signal magnitude chips (edge delta, missing/
/// extra %, ΔE, α, residual shift) — only the ones that are non-trivial. Empty
/// when there is no diagnosis (legacy path).
fn render_diag_chips(diag: &Diagnosis) -> String {
    let mut o = String::new();
    if !diag.primary_class.is_empty() {
        o.push_str(&format!(
            "<span class=\"chip cls\" style=\"background:{}\">{}</span>",
            diag_class_color(&diag.primary_class),
            html_escape(&diag.primary_class)
        ));
    }
    let m = &diag.magnitude;
    // Dominant per-side box-extent delta (CSS px) + the side it sits on.
    let sides = ["L", "R", "T", "B"];
    let mut worst = (0usize, 0.0f64);
    for (i, v) in m.edge_delta_css.iter().enumerate() {
        if v.abs() > worst.1.abs() {
            worst = (i, *v);
        }
    }
    if worst.1.abs() >= 0.1 {
        let sign = if worst.1 >= 0.0 { "+" } else { "−" };
        o.push_str(&format!(
            "<span class=\"chip\">{sign}{:.1}px {}</span>",
            worst.1.abs(),
            sides[worst.0]
        ));
    }
    if m.missing_area_pct >= 0.1 {
        o.push_str(&format!(
            "<span class=\"chip\">missing {:.1}%</span>",
            m.missing_area_pct
        ));
    }
    if m.extra_area_pct >= 0.1 {
        o.push_str(&format!(
            "<span class=\"chip\">extra {:.1}%</span>",
            m.extra_area_pct
        ));
    }
    if m.delta_e >= 0.1 {
        o.push_str(&format!("<span class=\"chip\">ΔE {:.1}</span>", m.delta_e));
    }
    if let Some(a) = m.recovered_alpha {
        o.push_str(&format!("<span class=\"chip\">α {a:.2}↛</span>"));
    }
    let (dx, dy) = (m.residual_shift_css[0], m.residual_shift_css[1]);
    if dx.abs() >= 0.1 || dy.abs() >= 0.1 {
        o.push_str(&format!(
            "<span class=\"chip\">shift ({dx:.1},{dy:.1})px</span>"
        ));
    }
    o
}

/// The per-region table (spec §3.3 item 5): one row per `RegionDiag`, worst-first
/// (the diagnosis already orders them). Empty string when there are no regions.
fn render_region_table(diag: &Diagnosis) -> String {
    if diag.regions.is_empty() {
        return String::new();
    }
    let mut o = String::from(
        "<details><summary>regions ({n})</summary>\
<table class=\"regtbl\"><thead><tr>\
<th>class</th><th>bbox (CSS px)</th><th class=\"num\">area%</th>\
<th>magnitude</th><th>reason</th></tr></thead><tbody>",
    );
    o = o.replace("{n}", &diag.regions.len().to_string());
    for r in &diag.regions {
        let bbox = format!(
            "{:.0},{:.0} → {:.0},{:.0}",
            r.bbox_css[0], r.bbox_css[1], r.bbox_css[2], r.bbox_css[3]
        );
        // A compact per-region magnitude: ΔE / ΔRGB for colour, shift for shifts,
        // edge delta for geometry — whichever the class carries.
        let mut mag = String::new();
        if r.delta_e >= 0.1 {
            mag.push_str(&format!("ΔE {:.1} ", r.delta_e));
        }
        if r.modal_drgb.iter().any(|v| *v != 0) {
            mag.push_str(&format!(
                "ΔRGB({},{},{}) ",
                r.modal_drgb[0], r.modal_drgb[1], r.modal_drgb[2]
            ));
        }
        if r.shift_css[0].abs() >= 0.1 || r.shift_css[1].abs() >= 0.1 {
            mag.push_str(&format!(
                "shift({:.1},{:.1}) ",
                r.shift_css[0], r.shift_css[1]
            ));
        }
        if let Some(a) = r.recovered_alpha {
            mag.push_str(&format!("α{a:.2} "));
        }
        if let Some(sel) = &r.selector {
            mag.push_str(&format!("· {} ", html_escape(sel)));
        }
        o.push_str(&format!(
            "<tr><td><span class=\"chip cls\" style=\"background:{c}\">{cls}</span></td>\
<td>{bbox}</td><td class=\"num\">{area:.2}</td><td>{mag}</td><td>{reason}</td></tr>",
            c = diag_class_color(&r.class),
            cls = html_escape(&r.class),
            bbox = bbox,
            area = r.area_pct,
            mag = html_escape(mag.trim()),
            reason = html_escape(&r.headline),
        ));
    }
    o.push_str("</tbody></table></details>");
    o
}

/// The source pane (spec §3.3 item 4, the HEADLINE requirement): a `<details open>`
/// block showing the fixture's HTML (`cases/<cat>/<id>.html`), html_escape'd into
/// `<pre><code>` with line numbers. Read at write time (no new dependency). When
/// the file cannot be read, a small note is shown instead so the card still renders.
fn render_source_pane(cases_dir: &Path, category: &str, id: &str) -> String {
    let path = cases_dir.join(category).join(format!("{id}.html"));
    match std::fs::read_to_string(&path) {
        Ok(src) => {
            let mut body = String::new();
            for (i, line) in src.lines().enumerate() {
                body.push_str(&format!(
                    "<span class=\"ln\">{}</span>{}\n",
                    i + 1,
                    html_escape(line)
                ));
            }
            format!(
                "<details open class=\"src\"><summary>source · cases/{}/{}.html</summary>\
<pre><code>{}</code></pre></details>",
                html_escape(category),
                html_escape(id),
                body
            )
        }
        Err(e) => format!(
            "<details class=\"src\"><summary>source · cases/{}/{}.html</summary>\
<p class=\"meta\">could not read fixture source: {}</p></details>",
            html_escape(category),
            html_escape(id),
            html_escape(&e.to_string())
        ),
    }
}

/// Stable anchor slug for a feature name (used for in-page `#feat-…` links).
pub(crate) fn feat_slug(feature: &str) -> String {
    feature
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Write `reports/index.html` and one `reports/<category>.html` per category.
/// Image paths are RELATIVE to the reports/ dir so the gallery renders both from
/// the repo checkout and as a CI artifact:
///   ref      -> ../refs/<cat>/<id>.png
///   ironpress-> ../out/<cat>/<id>.png
///   diff     -> <cat>/<id>.diff.png  (the CLASSED overlay)
///
/// Each fixture card is a diagnostic quad (spec §3.3): a header bar (status badge +
/// primary-class chip + magnitude chips), the Chrome ref | ironpress | classed-diff
/// images, an always-visible colour legend, the fixture's HTML SOURCE pane (read
/// from `cases_dir/<cat>/<id>.html`), and a per-region table. `cases_dir` is the
/// committed fixtures root (`tests/parity/cases`); the source is read at write time
/// so no fixture text needs threading through the report model.
pub(crate) fn write_html_reports(
    reports_dir: &Path,
    cases_dir: &Path,
    report: &Report,
) -> Result<(), String> {
    std::fs::create_dir_all(reports_dir)
        .map_err(|e| format!("cannot create reports dir {}: {e}", reports_dir.display()))?;

    // Per-category pages.
    for c in &report.categories {
        // Features worst-first (lowest score, then most fails) so problems surface.
        let mut feats: Vec<&FeatureReport> = c.features.iter().collect();
        feats.sort_by(|a, b| {
            a.score_pct
                .partial_cmp(&b.score_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.counts.fail.cmp(&a.counts.fail))
                .then(a.feature.cmp(&b.feature))
        });

        let mut o = String::new();
        o.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
        o.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
        o.push_str(&format!(
            "<title>parity · {}</title>",
            html_escape(&c.category)
        ));
        o.push_str(report_css());
        o.push_str("</head><body>");
        o.push_str(&format!(
            "<h1>{} — {:.2}%</h1>",
            html_escape(&c.category),
            c.score_pct
        ));
        o.push_str(&format!(
            "<p class=\"meta\"><a href=\"index.html\">&larr; all categories</a> · \
             PASS {} · PARTIAL {} · FAIL {} · UNKNOWN {} · {} fixtures · \
             V2 classed-diff · {} DPI</p>",
            c.counts.pass,
            c.counts.partial,
            c.counts.fail,
            c.counts.unknown,
            c.counts.pass + c.counts.partial + c.counts.fail + c.counts.unknown,
            report.env.dpi
        ));
        // Feature jump-nav (worst-first) for quick navigation within the category.
        o.push_str("<nav class=\"jump\"><strong>Features:</strong> ");
        for f in &feats {
            o.push_str(&format!(
                "<a href=\"#feat-{slug}\">{name} <span class=\"sc\">{sc:.0}%</span></a> ",
                slug = feat_slug(&f.feature),
                name = html_escape(&f.feature),
                sc = f.score_pct,
            ));
        }
        o.push_str("</nav>");

        // Filter/sort bar (spec §3.3 item 6): turns the page into a fix-by-class
        // worklist. Filters cards by status, by diagnosis primary_class, and by a
        // minimum diff%; sorts within each feature by diff% asc/desc. Pure inline
        // JS (`filterCards`/`sortCards` in `report_css`) — no framework, no dep.
        let mut classes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for f in &c.features {
            for fx in &f.fixtures {
                classes.insert(diag_class(fx));
            }
        }
        o.push_str(
            "<div class=\"filterbar\">\
<label>status <select id=\"f-status\" onchange=\"filterCards()\">\
<option value=\"\">all</option><option value=\"FAIL\">FAIL</option>\
<option value=\"PARTIAL\">PARTIAL</option><option value=\"PASS\">PASS</option>\
<option value=\"UNKNOWN\">UNKNOWN</option></select></label>",
        );
        o.push_str("<label>class <select id=\"f-class\" onchange=\"filterCards()\"><option value=\"\">all</option>");
        for cl in &classes {
            o.push_str(&format!(
                "<option value=\"{v}\">{v}</option>",
                v = html_escape(cl)
            ));
        }
        o.push_str("</select></label>");
        o.push_str(
            "<label>min diff% <input id=\"f-diff\" type=\"number\" min=\"0\" step=\"1\" \
value=\"0\" style=\"width:6ch\" oninput=\"filterCards()\"></label>\
<label>sort <select id=\"f-sort\" onchange=\"sortCards()\">\
<option value=\"fail\">FAIL-first</option><option value=\"diffd\">diff% high→low</option>\
<option value=\"diffa\">diff% low→high</option></select></label>\
<span id=\"f-count\" class=\"meta\"></span></div>",
        );

        // One anchored section per feature; fixtures FAIL-first inside.
        for f in &feats {
            o.push_str(&format!(
                "<section class=\"feat\"><h2 id=\"feat-{slug}\">{name} — {sc:.2}% \
<span class=\"meta\">PASS {p} · PARTIAL {pa} · FAIL {fl}</span>\
<a class=\"top\" href=\"index.html\">all categories ↑</a></h2>",
                slug = feat_slug(&f.feature),
                name = html_escape(&f.feature),
                sc = f.score_pct,
                p = f.counts.pass,
                pa = f.counts.partial,
                fl = f.counts.fail,
            ));

            let mut fxs: Vec<&FixtureResult> = f.fixtures.iter().collect();
            fxs.sort_by(|a, b| {
                status_rank(a.status)
                    .cmp(&status_rank(b.status))
                    .then(
                        b.diff_pct
                            .partial_cmp(&a.diff_pct)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
                    .then(a.id.cmp(&b.id))
            });

            o.push_str("<div class=\"cards\">");
            for fx in &fxs {
                let sub = if !fx.subfeature.is_empty() {
                    fx.subfeature.clone()
                } else if !fx.interaction_of.is_empty() {
                    format!("interaction: {}", fx.interaction_of.join(" × "))
                } else {
                    String::new()
                };
                let sub_html = if sub.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", html_escape(&sub))
                };
                let attr_html = if fx.status == Status::Pass {
                    String::new()
                } else if fx.attribution.starts_with("CONFOUNDED") {
                    format!(
                        " · <span class=\"confound\">{}</span>",
                        html_escape(&fx.attribution)
                    )
                } else {
                    " · <span class=\"real\">REAL</span>".to_string()
                };
                let desc_html = if fx.description.is_empty() {
                    String::new()
                } else {
                    format!("<div class=\"desc\">{}</div>", html_escape(&fx.description))
                };
                let ref_src = format!("../refs/{}/{}.png", c.category, fx.id);
                let out_src = format!("../out/{}/{}.png", c.category, fx.id);
                let diff_src = format!("{}/{}.diff.png", c.category, fx.id);
                // Reference oracle label: Chrome by default; WeasyPrint for CSS GCPM
                // features (footnotes, running elements) Chrome's print path renders
                // blank. Surfacing it makes a non-Chrome comparison explicit and
                // marks Chrome+Paged.js unsupported for that fixture.
                let ref_label = match fx.oracle.as_str() {
                    "weasyprint" => "WeasyPrint ref",
                    "none" => "ironpress only",
                    _ => "Chrome ref",
                };
                let oracle_chip = if fx.oracle == "chrome" {
                    String::new()
                } else {
                    format!(
                        " · <span class=\"confound\">oracle: {} (Chrome/Paged.js unsupported)</span>",
                        html_escape(&fx.oracle)
                    )
                };

                // Multi-page fixtures: render page 2.. so a real page break is
                // VISIBLE (not just asserted). The extra pages live in the committed
                // out/ tree as `<id>.pN.png` (written by process_entry when the
                // candidate paginates); stat them to decide how many quads to append.
                let out_root = cases_dir.parent().map(|p| p.join("out"));
                let mut pages_extra = String::new();
                if let Some(out_root) = &out_root {
                    let mut n = 2usize;
                    while out_root
                        .join(&c.category)
                        .join(format!("{}.p{}.png", fx.id, n))
                        .is_file()
                    {
                        let r2 =
                            html_escape(&format!("../refs/{}/{}.p{}.png", c.category, fx.id, n));
                        let o2 =
                            html_escape(&format!("../out/{}/{}.p{}.png", c.category, fx.id, n));
                        let d2 = html_escape(&format!("{}/{}.p{}.diff.png", c.category, fx.id, n));
                        pages_extra.push_str(&format!(
                            "<div class=\"pglabel\">page {n}</div><div class=\"quad\">\
<figure><img loading=\"lazy\" src=\"{r2}\" alt=\"{ref_label} p{n}\"><figcaption>{ref_label}</figcaption></figure>\
<figure><img loading=\"lazy\" src=\"{o2}\" alt=\"ironpress p{n}\"><figcaption>ironpress</figcaption></figure>\
<figure><img loading=\"lazy\" src=\"{d2}\" alt=\"classed diff p{n}\"><figcaption>classed diff</figcaption></figure>\
</div>"
                        ));
                        n += 1;
                    }
                }
                // Label page 1 only when there ARE extra pages (keeps single-page
                // cards byte-identical to before).
                let p1label = if pages_extra.is_empty() {
                    String::new()
                } else {
                    "<div class=\"pglabel\">page 1</div>".to_string()
                };

                // Header magnitude chips + region table from the V2 diagnosis (if
                // any). The reason line leads with the diagnosis headline (already
                // attribution-prefixed for confounded fixtures).
                let (chips_html, regions_html) = match &fx.diagnosis {
                    Some(d) => (render_diag_chips(d), render_region_table(d)),
                    None => (String::new(), String::new()),
                };
                let reason_html = match diag_reason(fx) {
                    Some(r) => format!(
                        "<div class=\"desc\"><strong>why:</strong> {}</div>",
                        html_escape(&r)
                    ),
                    None => String::new(),
                };
                // The fixture HTML source pane (the headline C5 requirement).
                let source_html = render_source_pane(cases_dir, &c.category, &fx.id);

                o.push_str(&format!(
                    "<div class=\"card fxrow\" data-status=\"{st}\" data-class=\"{cls}\" data-diff=\"{diffk}\">\
<div class=\"chead\">{badge} <span class=\"num\">{diff:.2}%</span> \
<strong>{id}</strong>{sub_html}{attr}{oc}{chips}</div>\
{p1label}<div class=\"quad\">\
<figure><img loading=\"lazy\" src=\"{r}\" alt=\"{rl}\"><figcaption>{rl}</figcaption></figure>\
<figure><img loading=\"lazy\" src=\"{ot}\" alt=\"ironpress\"><figcaption>ironpress</figcaption></figure>\
<figure><img loading=\"lazy\" src=\"{d}\" alt=\"classed diff\"><figcaption>classed diff</figcaption></figure>\
</div>{pages_extra}{legend}{reason}{desc}{regions}{source}</div>",
                    st = fx.status.as_str(),
                    cls = html_escape(&diag_class(fx)),
                    diffk = fx.diff_pct,
                    badge = status_badge(fx.status),
                    diff = fx.diff_pct,
                    id = html_escape(&fx.id),
                    sub_html = sub_html,
                    attr = attr_html,
                    oc = oracle_chip,
                    rl = ref_label,
                    chips = chips_html,
                    r = html_escape(&ref_src),
                    ot = html_escape(&out_src),
                    d = html_escape(&diff_src),
                    p1label = p1label,
                    pages_extra = pages_extra,
                    legend = render_legend(),
                    reason = reason_html,
                    desc = desc_html,
                    regions = regions_html,
                    source = source_html,
                ));
            }
            o.push_str("</div></section>");
        }
        o.push_str("</body></html>");

        let page = reports_dir.join(format!("{}.html", c.category));
        std::fs::write(&page, o).map_err(|e| format!("cannot write {}: {e}", page.display()))?;
    }

    // index.html
    let mut o = String::new();
    let ov = &report.overall;
    o.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    o.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    o.push_str("<title>ironpress parity report</title>");
    o.push_str(report_css());
    o.push_str("</head><body>");
    o.push_str(&format!(
        "<h1>ironpress Chrome-parity — {:.2}%</h1>",
        ov.score_pct
    ));
    o.push_str(&format!(
        "<p class=\"meta\">PASS {} · PARTIAL {} · FAIL {} · UNKNOWN {} · total {} · \
         scored {:.2}%</p>",
        ov.pass, ov.partial, ov.fail, ov.unknown, ov.total, ov.scored_ratio_pct
    ));
    o.push_str(&format!(
        "<p class=\"meta\"><strong>Env:</strong> {} DPI · V2 classed-diff \
         (multi-gate verdict) · pdftoppm {}</p>",
        report.env.dpi,
        if report.env.pdftoppm_available {
            "yes"
        } else {
            "MISSING"
        }
    ));

    // Calibration banner (spec §1.3 / §3.3): the page-origin correction + drift.
    if let Some(cal) = &report.calibration {
        let (col, state) = if cal.drifted {
            ("#cf222e", "DRIFTED")
        } else {
            ("#1a7f37", "audited OK")
        };
        o.push_str(&format!(
            "<p class=\"meta\"><strong>Calibration:</strong> offset ({},{}) device px \
             ({:.2},{:.2} CSS px) · measured ({},{}) · residual {} px · \
             <span style=\"color:{col};font-weight:600\">{state}</span></p>",
            cal.offset_px[0],
            cal.offset_px[1],
            cal.offset_css[0],
            cal.offset_css[1],
            cal.measured_px[0],
            cal.measured_px[1],
            cal.residual_px,
        ));
    }

    // Freshness banner.
    if !report.refs_lock_present {
        o.push_str(
            "<p class=\"meta\" style=\"color:#9a6700\"><strong>refs.lock missing</strong> — \
             reference freshness UNVERIFIED; run scripts/parity-gen-refs.sh.</p>",
        );
    } else if !report.stale_refs.is_empty() {
        o.push_str(&format!(
            "<p class=\"meta\" style=\"color:#cf222e\"><strong>{} STALE reference(s)</strong> — \
             regenerate with scripts/parity-gen-refs.sh.</p>",
            report.stale_refs.len()
        ));
    }

    // expected_support breakdown.
    o.push_str(&format!(
        "<p class=\"meta\"><strong>expected_support:</strong> implemented {} · partial {} · \
         unsupported {} · {} distinct category/feature pairs</p>",
        report.coverage.implemented,
        report.coverage.partial,
        report.coverage.unsupported,
        report.coverage.features_with_fixture
    ));

    o.push_str("<h2>Categories</h2>");
    o.push_str("<table data-sort=\"\"><thead><tr>");
    for (i, h) in ["category", "score%", "pass", "partial", "fail", "unknown"]
        .iter()
        .enumerate()
    {
        o.push_str(&format!(
            "<th onclick=\"sortTable(this.closest('table'),{i})\">{h}</th>"
        ));
    }
    o.push_str("</tr></thead><tbody>");
    for c in &report.categories {
        o.push_str(&format!(
            "<tr>\
<td><a href=\"{cat}.html\">{cat}</a></td>\
<td class=\"num\" data-k=\"{score}\">{score:.2}</td>\
<td class=\"num\">{p}</td><td class=\"num\">{pa}</td>\
<td class=\"num\">{f}</td><td class=\"num\">{u}</td>\
</tr>",
            cat = html_escape(&c.category),
            score = c.score_pct,
            p = c.counts.pass,
            pa = c.counts.partial,
            f = c.counts.fail,
            u = c.counts.unknown,
        ));
    }
    o.push_str("</tbody></table>");

    // Fix-by-class worklist (spec §3.3 item 6): a flat, sortable/filterable list of
    // every non-PASS fixture — class (diagnosis primary_class), diff%, status,
    // category, id, reason. Filter by class; sort any column (the shared sortTable).
    // Each id deep-links to its card on the category page. Turns the index into a
    // worklist grouped by the class of defect.
    {
        let mut rows: Vec<&FixtureResult> = Vec::new();
        let mut classes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for c in &report.categories {
            for f in &c.features {
                for fx in &f.fixtures {
                    if fx.status != Status::Pass {
                        rows.push(fx);
                        classes.insert(diag_class(fx));
                    }
                }
            }
        }
        rows.sort_by(|a, b| {
            status_rank(a.status)
                .cmp(&status_rank(b.status))
                .then(
                    b.diff_pct
                        .partial_cmp(&a.diff_pct)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.id.cmp(&b.id))
        });
        o.push_str("<h2>Fix-by-class worklist</h2>");
        if rows.is_empty() {
            o.push_str("<p class=\"meta\">No non-PASS fixtures.</p>");
        } else {
            o.push_str(
                "<div class=\"filterbar\"><label>class <select id=\"wl-class\" \
onchange=\"filterWorklist()\"><option value=\"\">all</option>",
            );
            for cl in &classes {
                o.push_str(&format!(
                    "<option value=\"{v}\">{v}</option>",
                    v = html_escape(cl)
                ));
            }
            o.push_str(
                "</select></label><span class=\"meta\">click a header to sort · click an id to jump</span></div>",
            );
            o.push_str("<table id=\"worklist\" data-sort=\"\"><thead><tr>");
            for (i, h) in ["class", "status", "diff%", "category", "id", "reason"]
                .iter()
                .enumerate()
            {
                o.push_str(&format!(
                    "<th onclick=\"sortTable(this.closest('table'),{i})\">{h}</th>"
                ));
            }
            o.push_str("</tr></thead><tbody>");
            for fx in &rows {
                let cls = diag_class(fx);
                let reason = diag_reason(fx).unwrap_or_else(|| fx.note.clone());
                o.push_str(&format!(
                    "<tr class=\"wl\" data-class=\"{clsk}\">\
<td><span class=\"chip cls\" style=\"background:{col}\">{cls}</span></td>\
<td><span class=\"badge\" style=\"background:{sc}\">{st}</span></td>\
<td class=\"num\" data-k=\"{diff}\">{diff:.2}</td>\
<td>{cat}</td>\
<td><a href=\"{cat}.html#feat-{slug}\">{id}</a></td>\
<td class=\"desc\">{reason}</td></tr>",
                    clsk = html_escape(&cls),
                    col = diag_class_color(&cls),
                    cls = html_escape(&cls),
                    sc = status_color(fx.status),
                    st = fx.status.as_str(),
                    diff = fx.diff_pct,
                    cat = html_escape(&fx.category),
                    slug = feat_slug(&fx.feature),
                    id = html_escape(&fx.id),
                    reason = html_escape(&reason),
                ));
            }
            o.push_str("</tbody></table>");
        }
    }

    // Per-category feature index: deep links straight to each feature's anchor.
    o.push_str("<h2>Jump to a feature</h2>");
    for c in &report.categories {
        o.push_str(&format!(
            "<details><summary><a href=\"{cat}.html\">{cat}</a> — {sc:.2}% \
<span class=\"meta\">PASS {p} · PARTIAL {pa} · FAIL {fl}</span></summary>\
<ul class=\"featlist\">",
            cat = html_escape(&c.category),
            sc = c.score_pct,
            p = c.counts.pass,
            pa = c.counts.partial,
            fl = c.counts.fail,
        ));
        let mut feats: Vec<&FeatureReport> = c.features.iter().collect();
        feats.sort_by(|a, b| {
            a.score_pct
                .partial_cmp(&b.score_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.feature.cmp(&b.feature))
        });
        for f in &feats {
            o.push_str(&format!(
                "<li><a href=\"{cat}.html#feat-{slug}\">{name}</a> — {sc:.2}% \
<span class=\"meta\">(FAIL {fl})</span></li>",
                cat = html_escape(&c.category),
                slug = feat_slug(&f.feature),
                name = html_escape(&f.feature),
                sc = f.score_pct,
                fl = f.counts.fail,
            ));
        }
        o.push_str("</ul></details>");
    }

    o.push_str(
        "<p class=\"meta\">Generated by <code>cargo test --test feature_parity</code>. \
                Each category page groups fixtures by feature; each card shows Chrome ref | \
                ironpress | classed diff, a colour legend, the fixture HTML source, and a \
                per-region table.</p>",
    );
    o.push_str("</body></html>");

    let index = reports_dir.join("index.html");
    std::fs::write(&index, o).map_err(|e| format!("cannot write {}: {e}", index.display()))
}

pub(crate) fn interaction_kind(
    fx: &FixtureResult,
    by_id: &BTreeMap<String, FixtureResult>,
) -> String {
    if fx.base_ids.is_empty() {
        return String::new();
    }
    let mut failing_base = None;
    let mut unresolved_base = None;
    let mut all_pass = true;
    for b in &fx.base_ids {
        match by_id.get(b) {
            Some(r) if r.status == Status::Pass => {}
            Some(_) => {
                all_pass = false;
                failing_base = Some(b.clone());
            }
            None => {
                all_pass = false;
                unresolved_base = Some(b.clone());
            }
        }
    }
    // An unresolved base is the strongest signal: name it explicitly (never blank).
    if let Some(b) = unresolved_base {
        return format!("UNRESOLVED base `{b}`");
    }
    if all_pass {
        "GENUINE: both bases PASS, interaction FAILs".to_string()
    } else if let Some(b) = failing_base {
        format!("DERIVATIVE: base `{b}` already FAILs")
    } else {
        "DERIVATIVE: a base is non-PASS".to_string()
    }
}
