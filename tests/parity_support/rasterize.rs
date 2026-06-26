//! PDF -> PNG rasterization via `pdftoppm` (poppler).
//!
//! Extracted verbatim from the former monolithic `mod.rs` (C1 mechanical split).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::config::DPI;

/// Rasterize EVERY page of `pdf` to `<tmp>/<id>-<n>.png` (n = 1..N) and return the
/// per-page PNG paths in ascending page order. Unlike [`rasterize`] (page 1 only),
/// this is the multi-page path used to validate pagination: the caller compares
/// each page against its reference and asserts the page COUNT matches Chrome.
///
/// `pdftoppm` (without `-singlefile`) names outputs `<prefix>-<n>.png`, zero-padded
/// to the page count's width, so we collect by numeric suffix and sort. Stale
/// per-page files from a PRIOR render of the same id (the tmp dir is reused across
/// runs and a fixture's page count can change) are removed first, so the returned
/// list reflects ONLY this render.
pub(crate) fn rasterize_all_pages(
    pdf: &Path,
    tmp_dir: &Path,
    id: &str,
) -> Result<Vec<PathBuf>, String> {
    let stale_prefix = format!("{id}-");
    if let Ok(rd) = std::fs::read_dir(tmp_dir) {
        for e in rd.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if name.starts_with(&stale_prefix)
                    && name.ends_with(".png")
                    && name[stale_prefix.len()..name.len() - 4]
                        .chars()
                        .all(|c| c.is_ascii_digit())
                {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }

    let prefix = tmp_dir.join(id);
    let status = Command::new("pdftoppm")
        .args(["-r", &DPI.to_string(), "-png"])
        .arg(pdf)
        .arg(&prefix)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("pdftoppm exit {status}"));
    }

    let mut pages: Vec<(u32, PathBuf)> = Vec::new();
    for e in std::fs::read_dir(tmp_dir)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let p = e.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if let Some(rest) = name.strip_prefix(&stale_prefix) {
                if let Some(num) = rest.strip_suffix(".png") {
                    if let Ok(n) = num.parse::<u32>() {
                        pages.push((n, p.clone()));
                    }
                }
            }
        }
    }
    pages.sort_by_key(|(n, _)| *n);
    if pages.is_empty() {
        return Err("pdftoppm produced no pages".into());
    }
    Ok(pages.into_iter().map(|(_, p)| p).collect())
}

pub(crate) fn rasterize(
    pdf: &Path,
    out_png: &Path,
    tmp_dir: &Path,
    id: &str,
) -> Result<(), String> {
    // pdftoppm -singlefile writes <prefix>.png
    let prefix = tmp_dir.join(id);
    let status = Command::new("pdftoppm")
        .args([
            "-r",
            &DPI.to_string(),
            "-png",
            "-f",
            "1",
            "-l",
            "1",
            "-singlefile",
        ])
        .arg(pdf)
        .arg(&prefix)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("pdftoppm exit {status}"));
    }
    // pdftoppm appends .png to the prefix.
    let produced = tmp_dir.join(format!("{id}.png"));
    if produced != *out_png && produced.is_file() {
        std::fs::rename(&produced, out_png).map_err(|e| e.to_string())?;
    }
    if !out_png.is_file() {
        return Err("pdftoppm produced no png".into());
    }
    Ok(())
}
