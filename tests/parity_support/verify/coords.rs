//! Coordinate sidecar (spec §2.2) — the renderer-INDEPENDENT "required result"
//! for the `PdfGeometry` verifier, in PDF points, top-left-origin.
//!
//! A sidecar lives at `tests/parity/coords/<category>/<id>.json` and encodes the
//! correct vector geometry of a fixture once, reviewed by a human, so any renderer
//! (ironpress today, a future one) is judged against the SAME contract instead of
//! against pixels. It is loaded into `VerifyCtx.coords`; when absent, the
//! `PdfGeometry` verifier does not apply and geometry authority stays with raster.
//!
//! PHASE 2a: the loader + types + the spec cross-check helper land here, but NO
//! sidecar files are committed yet, so `load_coords_sidecar` returns `None` for
//! every fixture and `PdfGeomVerifier.applies()` is false everywhere (the verdict
//! path is unchanged — proven a no-op). Sidecar generation is Phase 2b.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::super::config::CSS_PX;
use super::super::manifest::ManifestEntry;

/// The committed expected-geometry sidecar for ONE fixture. All coordinates are
/// PDF points in the frame named by `frame` (Phase 2b authors them in the
/// Chrome-reference frame; the verifier cancels the whole-page frame offset, see
/// `pdf_geom.rs`). `#[serde(default)]` on the collections keeps a minimal sidecar
/// (e.g. boxes only) valid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CoordSidecar {
    /// Schema version (currently 1).
    pub(crate) schema: u32,
    /// Coordinate frame the values live in (e.g. "chrome-ref-pt").
    pub(crate) frame: String,
    /// Page size in pt — `[w, h]` (LETTER = `[612.0, 792.0]`).
    pub(crate) page_pt: [f64; 2],
    /// Expected solid fills.
    #[serde(default)]
    pub(crate) boxes: Vec<CoordBox>,
    /// Expected stroked borders (reconstructed border-box rects + width).
    #[serde(default)]
    pub(crate) borders: Vec<CoordBox>,
    /// Expected text-run baseline origins + sizes.
    #[serde(default)]
    pub(crate) text_runs: Vec<CoordText>,
}

/// An expected rectangle (fill, border, or clip), top-left-origin pt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CoordBox {
    /// Human role for the report (e.g. "fill", "border").
    pub(crate) role: String,
    /// `[x, y_topleft, w, h]` in pt.
    pub(crate) rect_pt: [f64; 4],
    /// Optional CSS selector, purely for the report.
    #[serde(default)]
    pub(crate) selector: Option<String>,
}

/// An expected text-run baseline origin + font size, top-left-origin pt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CoordText {
    /// Human role for the report (e.g. "baseline").
    pub(crate) role: String,
    /// `[x, y_topleft]` of the baseline origin in pt.
    pub(crate) origin_pt: [f64; 2],
    /// Font size in pt.
    pub(crate) size_pt: f64,
    /// Optional CSS selector, purely for the report.
    #[serde(default)]
    pub(crate) selector: Option<String>,
}

/// Load the coordinate sidecar for an entry, if one is committed at
/// `tests/parity/coords/<category>/<id>.json`. Absent / unreadable / malformed =>
/// `None` (degrade to the raster geometry fallback — never a false pass/fail).
///
/// PHASE 2a: no sidecar files exist, so this returns `None` for every fixture.
pub(crate) fn load_coords_sidecar(
    parity_dir: &Path,
    entry: &ManifestEntry,
) -> Option<CoordSidecar> {
    let path = parity_dir
        .join("coords")
        .join(&entry.category)
        .join(format!("{}.json", entry.id));
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str::<CoordSidecar>(&raw).ok()
}

/// Spec-compute cross-check (spec §2.4): for a rigid probe whose box is closed-form
/// from the fixture CSS, the expected top-left-origin fill rect in pt is
/// `[ (content_origin_px + edge_inset_px) * pt/px, ..., w_px * pt/px, h_px * pt/px ]`.
/// `CSS_PX` is device-px-per-CSS-px @300dpi (3.125), and pt = CSS_px * 96/72; since
/// 1 CSS px = 0.75 pt, we convert px -> pt by `* 0.75`.
///
/// PHASE 2a: used by a unit test on SYNTHETIC input only (no probe sidecars exist
/// yet); Phase 2b calls this to assert `sidecar == spec` when a sidecar is present.
pub(crate) fn spec_fill_rect_pt(
    content_origin_css: [f64; 2],
    edge_inset_css: [f64; 2],
    size_css: [f64; 2],
) -> [f64; 4] {
    // 1 CSS px = 0.75 pt. (Equivalently 96/72.) Derive from CSS_PX so a DPI change
    // does not silently desync the cross-check: CSS_PX = device_px/CSS_px @300dpi,
    // and pt = device_px * 72/300, so pt/CSS_px = CSS_PX * 72/300 = 0.75.
    let pt_per_css = CSS_PX * 72.0 / 300.0;
    [
        (content_origin_css[0] + edge_inset_css[0]) * pt_per_css,
        (content_origin_css[1] + edge_inset_css[1]) * pt_per_css,
        size_css[0] * pt_per_css,
        size_css[1] * pt_per_css,
    ]
}
