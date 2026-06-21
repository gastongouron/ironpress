//! PDF -> PNG rasterization via `pdftoppm` (poppler).
//!
//! Extracted verbatim from the former monolithic `mod.rs` (C1 mechanical split).

use std::path::Path;
use std::process::Command;

use super::config::DPI;

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
