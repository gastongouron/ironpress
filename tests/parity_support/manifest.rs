//! Manifest schema (`ManifestEntry`), fragment loading + structural validation,
//! per-fixture threshold accessors, and the id != ref-filename mismatch detector.
//!
//! Extracted verbatim from the former monolithic `mod.rs` (C1 mechanical split).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::report::RefMismatch;

// ---------------------------------------------------------------------------
// Manifest schema
// ---------------------------------------------------------------------------

#[derive(Deserialize, Serialize, Clone, Debug)]
pub(crate) struct ManifestEntry {
    pub(crate) id: String,
    pub(crate) category: String,
    pub(crate) feature: String,
    #[serde(default)]
    pub(crate) subfeature: String,
    #[serde(default)]
    pub(crate) description: String,
    pub(crate) file: String,
    #[serde(default)]
    pub(crate) interaction_of: Vec<String>,
    #[serde(default)]
    pub(crate) base_ids: Vec<String>,
    #[serde(default = "default_weight")]
    pub(crate) weight: f64,
    #[serde(default)]
    pub(crate) pass_threshold_pct: Option<f64>,
    #[serde(default)]
    pub(crate) partial_threshold_pct: Option<f64>,
    /// Cross-rasterizer FLOOR (%) for a fixture whose residual is a VISUALLY-VERIFIED
    /// sub-perceptual rasterization difference (conic / repeating-gradient angular
    /// banding, or a mask-edge band) that Chrome and ironpress paint a hair
    /// differently. RELAXES ONLY the PASS↔PARTIAL line: raises the colour/missing/
    /// extra PASS bounds to this value and accepts interior ΔE up to the (fixed) hard-
    /// colour FAIL bound. It NEVER moves the FAIL bounds, and `floor()` clamps it
    /// below the coverage FAIL bound — so a real large-area diff still FAILs and no
    /// per-fixture tuning can manufacture a missing/extra/recolour false-pass.
    #[serde(default)]
    pub(crate) floor_pct: Option<f64>,
    #[serde(default = "default_sanitize")]
    pub(crate) sanitize: bool,
    /// Fixture kind: "feature" (default), "interaction", or "probe".
    #[serde(default = "default_kind")]
    pub(crate) kind: String,
    /// Substrate probe / base ids this fixture renders THROUGH. A non-PASS here
    /// makes a downstream failure CONFOUNDED rather than REAL.
    #[serde(default)]
    pub(crate) depends_on: Vec<String>,
    /// Surface-map expectation: "implemented" (default), "partial", or
    /// "unsupported". Anything != "implemented" is a tracked known-gap, not a
    /// regression.
    #[serde(default = "default_expected_support")]
    pub(crate) expected_support: String,
    /// Geometry class for the V2 calibration audit (spec §1.3): "free" (default)
    /// or "rigid". The deterministic solid probes are tagged "rigid" so the
    /// page-origin offset can be measured and audited once per run.
    #[serde(default = "default_geometry")]
    pub(crate) geometry: String,
    /// Reference ORACLE: which engine generates `refs/<cat>/<id>.png`. "chrome"
    /// (default) = Chrome `--print-to-pdf` / Paged.js. "weasyprint" = WeasyPrint,
    /// used for CSS GCPM features (footnotes, running elements) that Chrome's
    /// print path renders blank, so Chrome+Paged.js are NOT a valid oracle there.
    /// "none" = no pixel oracle exists; the report shows the ironpress render only
    /// and the fixture stays UNKNOWN (non-gating).
    #[serde(default = "default_oracle")]
    pub(crate) oracle: String,
}

impl ManifestEntry {
    /// Verified cross-rasterizer floor, clamped strictly below the coverage FAIL
    /// bound so it can only move the PASS↔PARTIAL line, never rescue a real
    /// large-area diff (which still trips the fixed FAIL gates).
    pub(crate) fn floor(&self) -> f64 {
        self.floor_pct
            .unwrap_or(0.0)
            .clamp(0.0, super::config::G_EXTRA_PCT.1 - 0.01)
    }
}

pub(crate) fn default_weight() -> f64 {
    1.0
}
pub(crate) fn default_sanitize() -> bool {
    true
}
pub(crate) fn default_kind() -> String {
    "feature".to_string()
}
pub(crate) fn default_expected_support() -> String {
    "implemented".to_string()
}
pub(crate) fn default_geometry() -> String {
    "free".to_string()
}
pub(crate) fn default_oracle() -> String {
    "chrome".to_string()
}

// NOTE (C6): the legacy `pass_threshold()`/`partial_threshold()` accessors were
// removed with the legacy comparator. The V2 verdict reads the raw
// `pass_threshold_pct`/`partial_threshold_pct` Option fields directly (only to
// RELAX `G_COLOR_PCT`; see `compare::verdict`), so no floor-clamped accessor is
// needed.

// ---------------------------------------------------------------------------
// Manifest loading + validation
// ---------------------------------------------------------------------------

pub(crate) fn load_manifests(
    manifest_dir: &Path,
    parity_dir: &Path,
) -> Result<Vec<ManifestEntry>, String> {
    let mut frag_files: Vec<PathBuf> = Vec::new();
    if manifest_dir.is_dir() {
        for ent in std::fs::read_dir(manifest_dir)
            .map_err(|e| format!("cannot read manifest dir {}: {e}", manifest_dir.display()))?
        {
            let p = ent.map_err(|e| e.to_string())?.path();
            if p.extension().and_then(|s| s.to_str()) == Some("json") {
                frag_files.push(p);
            }
        }
    }
    frag_files.sort();

    let mut all: Vec<ManifestEntry> = Vec::new();
    let mut seen_ids = BTreeMap::new();
    for f in &frag_files {
        let stem = f
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let txt = std::fs::read_to_string(f)
            .map_err(|e| format!("cannot read manifest {}: {e}", f.display()))?;
        let frag: Vec<ManifestEntry> = serde_json::from_str(&txt)
            .map_err(|e| format!("invalid manifest JSON in {}: {e}", f.display()))?;
        for e in frag {
            if e.category != stem {
                return Err(format!(
                    "manifest {}: entry '{}' has category '{}' != filename stem '{}'",
                    f.display(),
                    e.id,
                    e.category,
                    stem
                ));
            }
            if e.weight <= 0.0 {
                return Err(format!("entry '{}' has weight <= 0", e.id));
            }
            if !e.interaction_of.is_empty() && e.interaction_of.len() < 2 {
                return Err(format!(
                    "entry '{}' has interaction_of with < 2 elements",
                    e.id
                ));
            }
            if let (Some(p), Some(q)) = (e.pass_threshold_pct, e.partial_threshold_pct) {
                if p > q {
                    return Err(format!(
                        "entry '{}' has pass_threshold ({p}) > partial_threshold ({q})",
                        e.id
                    ));
                }
            }
            let fixture = parity_dir.join(&e.file);
            if !fixture.is_file() {
                return Err(format!(
                    "entry '{}': fixture file not found: {}",
                    e.id,
                    fixture.display()
                ));
            }
            // Per-fixture `@page { size: <content>; margin: 0 }` is now the design:
            // each fixture sizes the page to what it tests (no white-space skew) and
            // BOTH engines honor it (Chrome via --print-to-pdf, ironpress via the
            // @page-rule override), so there is no geometry desync. The former guard
            // that REJECTED @page is therefore obsolete and was removed.
            if let Some(prev) = seen_ids.insert(e.id.clone(), f.clone()) {
                return Err(format!(
                    "duplicate fixture id '{}' (in {} and {})",
                    e.id,
                    prev.display(),
                    f.display()
                ));
            }
            all.push(e);
        }
    }

    // Reference resolution guard (mirrors the duplicate-id guard): every
    // `depends_on` id and every interaction `base_id` MUST resolve to a known
    // fixture id, otherwise the manifest is structurally broken.
    let known: std::collections::BTreeSet<&str> = seen_ids.keys().map(|s| s.as_str()).collect();
    let mut ref_problems: Vec<String> = Vec::new();
    for e in &all {
        for d in &e.depends_on {
            if !known.contains(d.as_str()) {
                ref_problems.push(format!(
                    "entry '{}': depends_on `{}` does not resolve to a known fixture id",
                    e.id, d
                ));
            }
        }
        for b in &e.base_ids {
            if !known.contains(b.as_str()) {
                ref_problems.push(format!(
                    "entry '{}': interaction base_id `{}` does not resolve to a known fixture id",
                    e.id, b
                ));
            }
        }
    }
    if !ref_problems.is_empty() {
        return Err(format!(
            "manifest reference validation FAILED ({} problem(s)):\n  - {}",
            ref_problems.len(),
            ref_problems.join("\n  - ")
        ));
    }

    all.sort_by(|a, b| (a.category.clone(), a.id.clone()).cmp(&(b.category.clone(), b.id.clone())));
    Ok(all)
}

/// Detect id != ref-filename mismatches: a manifest id whose expected
/// `refs/<category>/<id>.png` is absent WHILE the category dir contains one or
/// more ref PNGs claimed by no id. That signature means a ref exists but was
/// committed under the wrong name (e.g. `border-box-shadow-offset` whose ref is
/// `box-shadow-offset.png`) -> a permanent silent UNKNOWN. A ref that was simply
/// never generated leaves no orphan and is left to the normal UNKNOWN path.
pub(crate) fn find_ref_mismatches(entries: &[ManifestEntry], refs_dir: &Path) -> Vec<RefMismatch> {
    // category -> set of expected ref file names (one per id).
    let mut expected: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for e in entries {
        expected
            .entry(e.category.clone())
            .or_default()
            .insert(format!("{}.png", e.id));
    }

    let mut out: Vec<RefMismatch> = Vec::new();
    for e in entries {
        let expected_ref = format!("{}.png", e.id);
        let ref_path = refs_dir.join(&e.category).join(&expected_ref);
        if ref_path.is_file() {
            continue; // ref present under the right name; nothing to flag.
        }
        // Gather orphan ref PNGs in this category dir (present on disk but not
        // an expected name for any id in the category).
        let cat_dir = refs_dir.join(&e.category);
        let mut orphans: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&cat_dir) {
            let claimed = expected.get(&e.category);
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".png") {
                    continue;
                }
                let is_claimed = claimed.map(|c| c.contains(&name)).unwrap_or(false);
                if !is_claimed {
                    orphans.push(name);
                }
            }
        }
        if !orphans.is_empty() {
            orphans.sort();
            out.push(RefMismatch {
                id: e.id.clone(),
                category: e.category.clone(),
                expected_ref,
                orphan_refs: orphans,
            });
        }
    }
    out.sort_by(|a, b| (a.category.clone(), a.id.clone()).cmp(&(b.category.clone(), b.id.clone())));
    out
}
