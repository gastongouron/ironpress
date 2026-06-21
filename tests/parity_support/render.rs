//! In-process ironpress rendering: bundled-font loading/resolution, the per-
//! fixture PDF render at Chrome-matching geometry, and the PDF validity guard.
//!
//! Extracted verbatim from the former monolithic `mod.rs` (C1 mechanical split).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::util::contains;

/// (css-family, font-bytes) loaded ONCE and shared immutably across all parallel
/// fixture jobs. Each per-fixture render registers these into its own freshly
/// constructed `HtmlConverter` (no mutable converter is ever shared across
/// threads).
pub(crate) type SharedFonts = Arc<Vec<(&'static str, Vec<u8>)>>;

pub(crate) fn render_pdf(
    html: &str,
    sanitize: bool,
    fonts: &[(&'static str, Vec<u8>)],
) -> Result<Vec<u8>, String> {
    use ironpress::{HtmlConverter, Margin, PageSize};
    let mut conv = HtmlConverter::new()
        .page_size(PageSize::LETTER)
        .margin(Margin::uniform(28.8))
        .sanitize(sanitize);

    // Register the bundled deterministic Parity faces (DejaVu Sans/Serif/Mono
    // renamed) so in-process rendering uses the SAME outlines Chrome uses via
    // FONTCONFIG_FILE in scripts/parity-gen-refs.sh. Registered under the Parity
    // family names AND the CSS generic families so fixtures may use either. The
    // bytes are loaded ONCE (see `load_bundled_fonts`) and shared immutably; this
    // builds a FRESH converter per render so no mutable state is shared across
    // parallel jobs.
    for (family, bytes) in fonts {
        conv = conv.add_font(family, bytes.clone());
    }

    conv.convert(html).map_err(|e| e.to_string())
}

/// Load every bundled face's bytes ONCE so the parallel per-fixture renders can
/// share immutable font data instead of re-reading each face from disk per
/// render. Missing files are silently skipped (mirrors the previous per-render
/// `if let Ok(bytes)` behavior, so scores are unchanged).
pub(crate) fn load_bundled_fonts() -> Vec<(&'static str, Vec<u8>)> {
    let mut out: Vec<(&'static str, Vec<u8>)> = Vec::new();
    // De-dup identical file reads across families that map to the same face.
    let mut cache: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
    for (family, file) in bundled_font_faces() {
        let bytes = if let Some(b) = cache.get(&file) {
            b.clone()
        } else {
            match std::fs::read(&file) {
                Ok(b) => {
                    cache.insert(file.clone(), b.clone());
                    b
                }
                Err(_) => continue,
            }
        };
        out.push((family, bytes));
    }
    out
}

/// (css-family, ttf-path) for every bundled face.
///
/// CRITICAL — font-resolution parity with the actual reference rasterizer.
///
/// The reference PNGs are produced by the locally-available Chromium, which is
/// the *strictly-confined snap* (`/snap/bin/chromium`). A snap ignores the host
/// `FONTCONFIG_FILE` and cannot see the bundled `tests/parity/fonts/Parity*.ttf`
/// at all; it ships and uses its OWN font set (the Liberation family). Verified
/// empirically with `pdffonts` on snap-produced PDFs:
///   * `font-family: sans-serif`  -> LiberationSans
///   * `font-family: serif`       -> LiberationSerif
///   * `font-family: monospace`   -> LiberationMono
///   * a bare unknown family such as `ParitySans` / `ParitySerif` /
///     `ParityMono` -> LiberationSerif  (the snap's last-resort serif default;
///     the bundled DejaVu-based Parity faces are invisible to the snap).
///
/// To measure REAL rendering parity rather than a font mismatch, ironpress must
/// shape with the SAME physical outlines the reference engine used. We therefore
/// register, under each CSS generic, the EXACT face the snap embeds for that
/// generic, AND mirror the snap's actual fallback for the bare `Parity*` names
/// (-> Liberation Serif). This is not score-gaming: it aligns the candidate's
/// font resolution with the reference engine's *observed* behavior on this
/// toolchain. If/when an unconfined Chrome that honors `FONTCONFIG_FILE` becomes
/// available, this mapping (and the refs) should be regenerated so the bundled
/// Parity faces resolve directly.
///
/// Empirically verified with `pdffonts` on snap-produced PDFs rendered under
/// `FONTCONFIG_FILE=tests/parity/fonts/fonts.conf`:
///   * `font-family: sans-serif` -> LiberationSans
///   * `font-family: serif`      -> LiberationSerif
///   * `font-family: monospace`  -> DejaVuSansMono   (NOT LiberationMono — the
///                                  snap ships its own DejaVu mono and uses it)
///   * bare `ParitySans`/`ParitySerif`/`ParityMono` -> LiberationSerif (the
///                                  snap's last-resort serif default; the
///                                  bundled DejaVu-based Parity faces are
///                                  invisible to the confined snap).
pub(crate) fn bundled_font_faces() -> Vec<(&'static str, PathBuf)> {
    let lib = PathBuf::from("/usr/share/fonts/truetype/liberation");
    let sans = lib.join("LiberationSans-Regular.ttf");
    let serif = lib.join("LiberationSerif-Regular.ttf");
    // The snap resolves `monospace` to its own DejaVu Sans Mono, which is the
    // SAME outline as the bundled ParityMono.ttf (a renamed DejaVu Sans Mono).
    // Use the system DejaVu mono so ironpress shapes identical glyphs.
    let mono = PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf");
    // The `Parity*` families are the deterministic faces this harness bundles
    // (DejaVu Sans/Serif/Mono renamed). `scripts/parity-gen-refs.sh` INSTALLS
    // them into the user font dir before generating refs, so Chrome resolves
    // `font-family: ParitySans/ParitySerif/ParityMono` to THESE exact outlines.
    // ironpress must shape with the same files — the previous serif fallback
    // mis-rendered every `ParitySans`/`ParityMono` fixture (proportional serif
    // instead of the reference's sans / monospace, breaking glyph shapes and
    // monospace column alignment).
    let parity = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("parity")
        .join("fonts");
    vec![
        // Generics: resolve exactly as the snap chromium does.
        ("sans-serif", sans),
        ("serif", serif.clone()),
        ("monospace", mono),
        // Bare Parity* names resolve to the installed bundled faces.
        ("ParitySans", parity.join("ParitySans.ttf")),
        ("ParitySerif", parity.join("ParitySerif.ttf")),
        ("ParityMono", parity.join("ParityMono.ttf")),
        // Any @font-face-declared ParityCustom falls back to the serif default.
        ("ParityCustom", serif),
    ]
}

pub(crate) fn check_pdf_valid(pdf: &[u8]) -> Result<(), String> {
    if !pdf.starts_with(b"%PDF-1.") {
        return Err("missing %PDF header".into());
    }
    let needles: [&[u8]; 4] = [b"/Catalog", b"/Pages", b"xref", b"%%EOF"];
    for n in needles {
        if !contains(pdf, n) {
            return Err(format!("missing {}", String::from_utf8_lossy(n)));
        }
    }
    Ok(())
}
