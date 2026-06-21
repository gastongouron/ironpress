//! Aggregation (spec §1.9): roll the per-pixel class map and the diff regions up
//! into per-class severities the verdict gates read.
//!
//! DENOMINATORS ARE CONTENT PIXELS, NOT TOTAL — a 13px-too-tall box yields
//! `edge_max_css ~= 4.16` regardless of body size, and a recolour's `color_pct`
//! is a fraction of painted area, not of the whole page. The size signal
//! (`edge_*`) is read from the bbox extents, never from the diluted pixel count.

use image::RgbaImage;

use super::super::config::CSS_PX;
use super::super::geom::Mask;
use super::classify::{ClassMap, PixelClass};
use super::color::{ciede2000, srgb_to_lab};
use super::segment::DiffRegion;

/// Per-class severities for one fixture. All magnitudes are in CSS px / ΔE / % of
/// content area — never raw device px — so they read directly against the CSS.
/// `modal_drgb`/`total_px` are populated now and consumed by the C4 diagnosis /
/// C5 report; kept on the struct so the contract is complete.
#[allow(dead_code)]
pub(crate) struct ClassTally {
    /// ColorErr px / union content px.
    pub(crate) color_pct: f64,
    /// Missing px / ref content px.
    pub(crate) missing_pct: f64,
    /// Extra px / cand content px.
    pub(crate) extra_pct: f64,
    /// max(|per-side bbox delta|) / CSS_PX — the box-size signal.
    pub(crate) edge_max_css: f64,
    /// [left, right, top, bottom] per-side extent delta, CSS px.
    pub(crate) edge_delta_css: [f64; 4],
    /// max region translation magnitude (and whole-frame residual), CSS px.
    pub(crate) shift_max_css: f64,
    /// AaEdge px / union px (informational; never gates).
    pub(crate) aa_pct: f64,
    /// Area-weighted mean ΔE2000 over ColorErr regions.
    pub(crate) color_de: f64,
    /// INTERIOR (non-edge-band) ColorErr px / union content px — the SOLID-recolour
    /// area fraction. A geometry-boundary / curved-AA-ring ColorErr lives in the
    /// edge band and is excluded, so this is ~0 for a moved/resized correct-colour
    /// fill but high for a genuine fill/border recolour. Gates `hard_color`.
    pub(crate) interior_color_pct: f64,
    /// Area-weighted (by interior px) mean of the per-region MEDIAN interior ΔE —
    /// the robust solid-recolour ΔE the hard-colour FAIL gate reads.
    pub(crate) interior_color_de: f64,
    /// Modal (median) per-channel ΔRGB over all ColorErr px.
    pub(crate) modal_drgb: [i16; 3],
    pub(crate) total_px: u64,
}

impl ClassTally {
    /// Derived back-compat scalar: % of real-diff pixels (ColorErr + Missing +
    /// Extra + GeomShift) over the whole frame. Match AND AaEdge are excluded —
    /// genuine cross-rasterizer AA is never a defect.
    pub(crate) fn diff_pct(&self, cm: &ClassMap) -> f64 {
        if cm.px.is_empty() {
            return 0.0;
        }
        let real: u64 = cm
            .px
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    PixelClass::ColorErr
                        | PixelClass::Missing
                        | PixelClass::Extra
                        | PixelClass::GeomShift
                )
            })
            .count() as u64;
        100.0 * real as f64 / cm.px.len() as f64
    }
}

/// Aggregate the class map + regions + bbox delta into `ClassTally`.
pub(crate) fn aggregate(
    cm: &ClassMap,
    regions: &[DiffRegion],
    bbox_delta: [i32; 4],
    mask_c: &Mask,
    mask_r: &Mask,
    cand: &RgbaImage,
    reference: &RgbaImage,
) -> ClassTally {
    let total_px = cm.px.len() as u64;

    // Class counts.
    let mut color = 0u64;
    let mut missing = 0u64;
    let mut extra = 0u64;
    let mut aa = 0u64;
    for c in &cm.px {
        match c {
            PixelClass::ColorErr => color += 1,
            PixelClass::Missing => missing += 1,
            PixelClass::Extra => extra += 1,
            PixelClass::AaEdge => aa += 1,
            _ => {}
        }
    }

    // Content-pixel denominators (spec §1.9).
    let (w, h) = cand.dimensions();
    let mut cand_content = 0u64;
    let mut ref_content = 0u64;
    let mut union_content = 0u64;
    for y in 0..h {
        for x in 0..w {
            let ic = mask_c.get(x, y);
            let ir = mask_r.get(x, y);
            if ic {
                cand_content += 1;
            }
            if ir {
                ref_content += 1;
            }
            if ic || ir {
                union_content += 1;
            }
        }
    }
    let pct = |num: u64, den: u64| {
        if den == 0 {
            0.0
        } else {
            100.0 * num as f64 / den as f64
        }
    };

    let color_pct = pct(color, union_content);
    let missing_pct = pct(missing, ref_content);
    let extra_pct = pct(extra, cand_content);
    let aa_pct = pct(aa, total_px);

    // Size signal: per-side extent delta (device px) -> CSS px.
    let edge_delta_css = [
        bbox_delta[0] as f64 / CSS_PX,
        bbox_delta[1] as f64 / CSS_PX,
        bbox_delta[2] as f64 / CSS_PX,
        bbox_delta[3] as f64 / CSS_PX,
    ];
    let edge_max_css = edge_delta_css.iter().map(|v| v.abs()).fold(0.0, f64::max);

    // Residual translation: the max region shift magnitude.
    let shift_max_css = regions
        .iter()
        .filter(|r| r.is_translation)
        .map(|r| (r.shift_css.0 * r.shift_css.0 + r.shift_css.1 * r.shift_css.1).sqrt())
        .fold(0.0, f64::max);

    // Area-weighted ΔE over ColorErr regions; modal ΔRGB over ALL ColorErr px.
    let mut de_weight = 0.0;
    let mut de_area = 0u32;
    for r in regions {
        if r.dominant == PixelClass::ColorErr && r.delta_e > 0.0 {
            de_weight += r.delta_e * r.area_px as f64;
            de_area += r.area_px;
        }
    }
    let color_de = if de_area > 0 {
        de_weight / de_area as f64
    } else {
        0.0
    };

    // INTERIOR-ColorErr aggregate (the hard-colour gate signal). Sum the per-region
    // interior ColorErr px (edge-band ColorErr excluded by `segment`) and the
    // interior-px-weighted region median ΔE. This fires hard_color on a SOLID
    // recolour (interior area + ΔE) while a geometry-edge strip / curved-AA ring
    // (all ColorErr in the edge band -> interior_color_px ~0) does NOT.
    let mut interior_px = 0u64;
    let mut interior_de_weight = 0.0;
    for r in regions {
        if r.interior_color_px > 0 {
            interior_px += r.interior_color_px as u64;
            interior_de_weight += r.delta_e * r.interior_color_px as f64;
        }
    }
    let interior_color_pct = pct(interior_px, union_content);
    let interior_color_de = if interior_px > 0 {
        interior_de_weight / interior_px as f64
    } else {
        0.0
    };

    let modal_drgb = modal_colorerr_drgb(cm, cand, reference);
    // If ColorErr pixels exist but no ColorErr-dominant region cleared the speck
    // filter, still surface a representative ΔE so the hard-colour gate can act (a
    // thin recolour can be below REGION_MIN_AREA_PX yet a real defect).
    let color_de = if color_de == 0.0 && color > 0 {
        sampled_colorerr_de(cm, cand, reference)
    } else {
        color_de
    };

    ClassTally {
        color_pct,
        missing_pct,
        extra_pct,
        edge_max_css,
        edge_delta_css,
        shift_max_css,
        aa_pct,
        color_de,
        interior_color_pct,
        interior_color_de,
        modal_drgb,
        total_px,
    }
}

/// Median signed per-channel (cand − ref) over all ColorErr pixels.
fn modal_colorerr_drgb(cm: &ClassMap, cand: &RgbaImage, reference: &RgbaImage) -> [i16; 3] {
    let mut dr = Vec::new();
    let mut dg = Vec::new();
    let mut db = Vec::new();
    let w = cm.w;
    for (i, cls) in cm.px.iter().enumerate() {
        if *cls != PixelClass::ColorErr {
            continue;
        }
        let x = (i as u32) % w;
        let y = (i as u32) / w;
        let c = cand.get_pixel(x, y).0;
        let r = reference.get_pixel(x, y).0;
        dr.push(c[0] as i16 - r[0] as i16);
        dg.push(c[1] as i16 - r[1] as i16);
        db.push(c[2] as i16 - r[2] as i16);
    }
    [median(&mut dr), median(&mut dg), median(&mut db)]
}

/// Mean ΔE2000 over all ColorErr pixels (fallback when no region cleared the
/// speck filter but ColorErr pixels exist).
fn sampled_colorerr_de(cm: &ClassMap, cand: &RgbaImage, reference: &RgbaImage) -> f64 {
    let w = cm.w;
    let mut sum = 0.0;
    let mut n = 0u32;
    for (i, cls) in cm.px.iter().enumerate() {
        if *cls != PixelClass::ColorErr {
            continue;
        }
        let x = (i as u32) % w;
        let y = (i as u32) / w;
        let c = cand.get_pixel(x, y).0;
        let r = reference.get_pixel(x, y).0;
        sum += ciede2000(
            srgb_to_lab([r[0], r[1], r[2]]),
            srgb_to_lab([c[0], c[1], c[2]]),
        );
        n += 1;
    }
    if n > 0 { sum / n as f64 } else { 0.0 }
}

fn median(v: &mut [i16]) -> i16 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}
