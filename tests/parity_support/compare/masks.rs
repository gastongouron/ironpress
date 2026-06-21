//! Structural-edge masks (spec §1.6) — the FLIP-style "feature pipeline".
//!
//! Anti-aliasing forgiveness is the single largest false-pass risk: a global 1px
//! tolerance laundered wrong fonts, recolours, and mitered-vs-square corners as
//! "noise". The V2 path forgives AA only where it is genuinely legal — on a
//! structural edge present in BOTH images (the `shared_edge_band`). Same glyph in
//! both -> its AA ramp lies in the shared band (forgiven). A wrong glyph/weight,
//! a mitered-vs-butt corner, or a recolour has edges that do NOT coincide -> the
//! differing pixels fall outside the shared band -> they are scored.

use image::RgbaImage;

use super::super::config::EDGE_GRAD;

/// Four packed bitsets over the union-cropped frame (1 bit/px, row-major):
/// - `edge`: per-image structural edge (kept for diagnosis; unused by the gate).
/// - `edge_band`: union of both images' 1px-dilated edge bands — the structural
///   boundary locus (fill-vs-border / box-vs-background). The interior-recolour
///   gate excludes it (interior = `ColorErr && !edge_band`).
/// - `shared`: `edge_band(cand) AND edge_band(ref)` — the only locus of AA mercy.
pub(crate) struct StructuralMasks {
    w: u32,
    h: u32,
    /// Union of both images' raw edges. Informational (drives the C5 edge/heat
    /// overlay and diagnosis); the AA gate uses `shared`, not this.
    #[allow(dead_code)]
    edge: Vec<u64>,
    /// Union of both images' DILATED edge bands. A ColorErr pixel inside this band
    /// is a structural-boundary recolour (geometry-edge jitter / curved-AA ring),
    /// NOT a solid interior recolour — the hard-colour gate excludes it.
    edge_band: Vec<u64>,
    shared: Vec<u64>,
}

impl StructuralMasks {
    #[inline]
    fn test(bits: &[u64], i: usize) -> bool {
        (bits[i >> 6] >> (i & 63)) & 1 == 1
    }
    /// Whether `(x,y)` lies on a structural edge in EITHER image (informational;
    /// consumed by the C5 overlay/diagnosis, not by the AA gate).
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn is_edge(&self, x: u32, y: u32) -> bool {
        if x >= self.w || y >= self.h {
            return false;
        }
        Self::test(&self.edge, (y as usize) * (self.w as usize) + x as usize)
    }
    /// Whether `(x,y)` is within 1px of an edge in BOTH images — AA-forgivable.
    #[inline]
    pub(crate) fn in_shared_band(&self, x: u32, y: u32) -> bool {
        if x >= self.w || y >= self.h {
            return false;
        }
        Self::test(&self.shared, (y as usize) * (self.w as usize) + x as usize)
    }
    /// Whether `(x,y)` is within 1px of a structural edge in EITHER image — a
    /// fill/border/background boundary. A ColorErr pixel here is boundary jitter
    /// (a moved/resized fill abutting a different colour, or a curved-AA ring), NOT
    /// a solid interior recolour; the hard-colour gate excludes it. Out-of-bounds
    /// reads as `false` (a degenerate pixel is treated as interior, never masked).
    #[inline]
    pub(crate) fn in_edge_band(&self, x: u32, y: u32) -> bool {
        if x >= self.w || y >= self.h {
            return false;
        }
        Self::test(&self.edge_band, (y as usize) * (self.w as usize) + x as usize)
    }
}

#[inline]
fn set(bits: &mut [u64], i: usize) {
    bits[i >> 6] |= 1u64 << (i & 63);
}
#[inline]
fn get(bits: &[u64], i: usize) -> bool {
    (bits[i >> 6] >> (i & 63)) & 1 == 1
}

/// A pixel is an edge iff the max over its 4-neighbours of the max-per-channel
/// |Δ| exceeds `EDGE_GRAD` (0..255). Returns the raw edge bitset for `img`.
fn detect_edges(img: &RgbaImage) -> Vec<u64> {
    let (w, h) = img.dimensions();
    let words = ((w as usize * h as usize) + 63) / 64;
    let mut edge = vec![0u64; words.max(1)];
    for y in 0..h {
        for x in 0..w {
            let c = img.get_pixel(x, y).0;
            let mut grad = 0i32;
            // 4-neighbourhood (clamped at the frame border).
            let neighbours = [
                (x.wrapping_sub(1), y, x > 0),
                (x + 1, y, x + 1 < w),
                (x, y.wrapping_sub(1), y > 0),
                (x, y + 1, y + 1 < h),
            ];
            for (nx, ny, ok) in neighbours {
                if !ok {
                    continue;
                }
                let n = img.get_pixel(nx, ny).0;
                let d = (c[0] as i32 - n[0] as i32)
                    .abs()
                    .max((c[1] as i32 - n[1] as i32).abs())
                    .max((c[2] as i32 - n[2] as i32).abs());
                grad = grad.max(d);
            }
            if grad > EDGE_GRAD {
                set(&mut edge, (y as usize) * (w as usize) + x as usize);
            }
        }
    }
    edge
}

/// 3x3 morphological dilation of an edge bitset by 1px.
fn dilate(edge: &[u64], w: u32, h: u32) -> Vec<u64> {
    let words = ((w as usize * h as usize) + 63) / 64;
    let mut out = vec![0u64; words.max(1)];
    for y in 0..h {
        for x in 0..w {
            let i = (y as usize) * (w as usize) + x as usize;
            if !get(edge, i) {
                continue;
            }
            let x0 = x.saturating_sub(1);
            let y0 = y.saturating_sub(1);
            let x2 = (x + 1).min(w - 1);
            let y2 = (y + 1).min(h - 1);
            for yy in y0..=y2 {
                for xx in x0..=x2 {
                    set(&mut out, (yy as usize) * (w as usize) + xx as usize);
                }
            }
        }
    }
    out
}

/// Build `StructuralMasks` for an already-aligned cand/ref pair (same dims).
pub(crate) fn structural_masks(cand: &RgbaImage, reference: &RgbaImage) -> StructuralMasks {
    let (w, h) = cand.dimensions();
    let edge_c = detect_edges(cand);
    let edge_r = detect_edges(reference);

    // Per-image edge-band (1px dilation), then AND for the shared band.
    let band_c = dilate(&edge_c, w, h);
    let band_r = dilate(&edge_r, w, h);

    let words = band_c.len();
    let mut shared = vec![0u64; words];
    let mut edge_band = vec![0u64; words];
    for i in 0..words {
        shared[i] = band_c[i] & band_r[i];
        // Union of the dilated bands: the structural-boundary locus (either image).
        edge_band[i] = band_c[i] | band_r[i];
    }

    // `edge` (informational): union of both images' raw edges.
    let mut edge = vec![0u64; edge_c.len()];
    for i in 0..edge.len() {
        edge[i] = edge_c[i] | edge_r[i];
    }

    StructuralMasks { w, h, edge, edge_band, shared }
}
