//! Perceptual colour for the V2 comparator (spec §1.1, amendment A1).
//!
//! Hand-rolled CIEDE2000 — NO `palette` dependency. The pipeline is the standard
//! sRGB8 -> linear -> XYZ (D65) -> CIELab, then the CIE-recommended ΔE2000 colour
//! difference (Sharma, Wu & Dalal 2005). ΔE2000 is perceptually uniform (JND ~2.3)
//! so the verdict can put principled bounds on colour error (`COLOR_DE_PASS` /
//! `COLOR_DE_FAIL`) where the legacy YIQ delta could not.
//!
//! The YIQ primitives (`color_delta`, `rgb2y/i/q`) stay in `compare/mod.rs`: they
//! remain the per-pixel match/AA budget metric (`t_match`/`t_aa`). ΔE2000 is the
//! region-level colour-severity metric only.

/// CIELab colour (D65 reference white).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lab {
    pub(crate) l: f64,
    pub(crate) a: f64,
    pub(crate) b: f64,
}

/// sRGB 8-bit channel -> linear-light [0,1] (IEC 61966-2-1).
#[inline]
fn srgb_to_linear(c: u8) -> f64 {
    let cs = c as f64 / 255.0;
    if cs <= 0.040_45 {
        cs / 12.92
    } else {
        ((cs + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert an sRGB8 triple to CIELab via linear-light XYZ under the D65 white.
pub(crate) fn srgb_to_lab(px: [u8; 3]) -> Lab {
    let r = srgb_to_linear(px[0]);
    let g = srgb_to_linear(px[1]);
    let b = srgb_to_linear(px[2]);

    // linear sRGB (D65) -> XYZ (sRGB matrix, IEC 61966-2-1).
    let x = r * 0.412_456_4 + g * 0.357_576_1 + b * 0.180_437_5;
    let y = r * 0.212_672_9 + g * 0.715_152_2 + b * 0.072_175_0;
    let z = r * 0.019_333_9 + g * 0.119_192_0 + b * 0.950_304_1;

    // D65 reference white (normalized so Y_n = 1).
    const XN: f64 = 0.950_47;
    const YN: f64 = 1.0;
    const ZN: f64 = 1.088_83;

    let fx = lab_f(x / XN);
    let fy = lab_f(y / YN);
    let fz = lab_f(z / ZN);

    Lab {
        l: 116.0 * fy - 16.0,
        a: 500.0 * (fx - fy),
        b: 200.0 * (fy - fz),
    }
}

#[inline]
fn lab_f(t: f64) -> f64 {
    const DELTA: f64 = 6.0 / 29.0;
    if t > DELTA * DELTA * DELTA {
        t.cbrt()
    } else {
        t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
    }
}

/// CIEDE2000 colour difference between two Lab colours (Sharma, Wu & Dalal 2005,
/// "The CIEDE2000 Color-Difference Formula"). Symmetric; 0 for identical colours.
pub(crate) fn ciede2000(x: Lab, y: Lab) -> f64 {
    let (l1, a1, b1) = (x.l, x.a, x.b);
    let (l2, a2, b2) = (y.l, y.a, y.b);

    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let c_bar = (c1 + c2) / 2.0;

    let c_bar7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c_bar7 / (c_bar7 + 25f64.powi(7))).sqrt());

    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;

    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();

    let h1p = hue_deg(b1, a1p);
    let h2p = hue_deg(b2, a2p);

    let dl_p = l2 - l1;
    let dc_p = c2p - c1p;

    let dh_p = if c1p * c2p == 0.0 {
        0.0
    } else {
        let diff = h2p - h1p;
        if diff.abs() <= 180.0 {
            diff
        } else if diff > 180.0 {
            diff - 360.0
        } else {
            diff + 360.0
        }
    };
    let big_dh_p = 2.0 * (c1p * c2p).sqrt() * (deg2rad(dh_p) / 2.0).sin();

    let l_bar_p = (l1 + l2) / 2.0;
    let c_bar_p = (c1p + c2p) / 2.0;

    let h_bar_p = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };

    let t = 1.0 - 0.17 * deg2rad(h_bar_p - 30.0).cos()
        + 0.24 * deg2rad(2.0 * h_bar_p).cos()
        + 0.32 * deg2rad(3.0 * h_bar_p + 6.0).cos()
        - 0.20 * deg2rad(4.0 * h_bar_p - 63.0).cos();

    let delta_theta = 30.0 * (-(((h_bar_p - 275.0) / 25.0).powi(2))).exp();
    let c_bar_p7 = c_bar_p.powi(7);
    let rc = 2.0 * (c_bar_p7 / (c_bar_p7 + 25f64.powi(7))).sqrt();

    let lbar_m50_sq = (l_bar_p - 50.0).powi(2);
    let sl = 1.0 + (0.015 * lbar_m50_sq) / (20.0 + lbar_m50_sq).sqrt();
    let sc = 1.0 + 0.045 * c_bar_p;
    let sh = 1.0 + 0.015 * c_bar_p * t;
    let rt = -(deg2rad(2.0 * delta_theta)).sin() * rc;

    const KL: f64 = 1.0;
    const KC: f64 = 1.0;
    const KH: f64 = 1.0;

    let term_l = dl_p / (KL * sl);
    let term_c = dc_p / (KC * sc);
    let term_h = big_dh_p / (KH * sh);

    (term_l * term_l + term_c * term_c + term_h * term_h + rt * term_c * term_h).sqrt()
}

#[inline]
fn deg2rad(d: f64) -> f64 {
    d * std::f64::consts::PI / 180.0
}

/// hue angle in degrees [0,360) for (b, a'); 0 when both components are 0.
#[inline]
fn hue_deg(b: f64, ap: f64) -> f64 {
    if b == 0.0 && ap == 0.0 {
        return 0.0;
    }
    let mut h = b.atan2(ap) * 180.0 / std::f64::consts::PI;
    if h < 0.0 {
        h += 360.0;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CIEDE2000 against the published Sharma, Wu & Dalal (2005) test vectors.
    /// The formula is asserted DIRECTLY on Lab inputs (so it does not depend on
    /// the sRGB->Lab chain), to within 1e-3 of the reference ΔE values (A1).
    ///
    /// NOTE: these are the AUTHORITATIVE published values from Sharma's table.
    /// The brief's prose gives two example pairs with mis-transcribed expected
    /// numbers (it pairs `(50,2.5,0)` vs `(50,0,-2.5)` with 4.7461 and vs
    /// `(50,3.1736,0.5854)` with 1.6492); the real Sharma table gives 4.3065 and
    /// 1.0000 respectively. The first named example (2.0425) is correct and kept.
    /// We assert the verified published values — using the brief's typo'd numbers
    /// would test a wrong formula. (Reported back to the orchestrator.)
    #[test]
    fn ciede2000_matches_sharma_vectors() {
        // (Lab1, Lab2, expected ΔE2000) — a representative slice of the 34-row
        // Sharma table covering the chroma-rotation (Rt), arc-wrap, neutral-axis,
        // and high-chroma branches. Every value verified against the reference.
        let cases: [(Lab, Lab, f64); 10] = [
            (
                Lab {
                    l: 50.0,
                    a: 2.6772,
                    b: -79.7751,
                },
                Lab {
                    l: 50.0,
                    a: 0.0,
                    b: -82.7485,
                },
                2.0425,
            ),
            (
                Lab {
                    l: 50.0,
                    a: 3.1571,
                    b: -77.2803,
                },
                Lab {
                    l: 50.0,
                    a: 0.0,
                    b: -82.7485,
                },
                2.8615,
            ),
            (
                Lab {
                    l: 50.0,
                    a: 2.8361,
                    b: -74.0200,
                },
                Lab {
                    l: 50.0,
                    a: 0.0,
                    b: -82.7485,
                },
                3.4412,
            ),
            (
                Lab {
                    l: 50.0,
                    a: -1.3802,
                    b: -84.2814,
                },
                Lab {
                    l: 50.0,
                    a: 0.0,
                    b: -82.7485,
                },
                1.0000,
            ),
            (
                Lab {
                    l: 50.0,
                    a: 0.0,
                    b: 0.0,
                },
                Lab {
                    l: 50.0,
                    a: -1.0,
                    b: 2.0,
                },
                2.3669,
            ),
            (
                Lab {
                    l: 50.0,
                    a: 2.49,
                    b: -0.001,
                },
                Lab {
                    l: 50.0,
                    a: -2.49,
                    b: 0.0009,
                },
                7.1792,
            ),
            (
                Lab {
                    l: 50.0,
                    a: 2.5,
                    b: 0.0,
                },
                Lab {
                    l: 50.0,
                    a: 0.0,
                    b: -2.5,
                },
                4.3065,
            ),
            (
                Lab {
                    l: 50.0,
                    a: 2.5,
                    b: 0.0,
                },
                Lab {
                    l: 50.0,
                    a: 3.1736,
                    b: 0.5854,
                },
                1.0000,
            ),
            (
                Lab {
                    l: 50.0,
                    a: 2.5,
                    b: 0.0,
                },
                Lab {
                    l: 73.0,
                    a: 25.0,
                    b: -18.0,
                },
                27.1492,
            ),
            (
                Lab {
                    l: 60.2574,
                    a: -34.0099,
                    b: 36.2677,
                },
                Lab {
                    l: 60.4626,
                    a: -34.1751,
                    b: 39.4387,
                },
                1.2644,
            ),
        ];
        for (i, (a, b, expected)) in cases.iter().enumerate() {
            let got = ciede2000(*a, *b);
            assert!(
                (got - expected).abs() < 1e-3,
                "case {i}: ciede2000 = {got:.4}, expected {expected:.4} (Δ {:.5})",
                (got - expected).abs()
            );
            // Symmetry.
            let got_rev = ciede2000(*b, *a);
            assert!(
                (got - got_rev).abs() < 1e-9,
                "case {i}: ciede2000 not symmetric ({got:.6} vs {got_rev:.6})"
            );
        }
    }

    /// The sRGB->Lab chain sanity: pure white -> L≈100, pure black -> L≈0, and a
    /// mid-grey lands near L≈53.4. Two near-identical reds have ΔE under the JND.
    #[test]
    fn srgb_to_lab_landmarks() {
        let white = srgb_to_lab([255, 255, 255]);
        assert!((white.l - 100.0).abs() < 0.5, "white L = {}", white.l);
        let black = srgb_to_lab([0, 0, 0]);
        assert!(black.l.abs() < 0.5, "black L = {}", black.l);
        let grey = srgb_to_lab([119, 119, 119]);
        assert!((grey.l - 50.0).abs() < 3.0, "mid grey L = {}", grey.l);

        // #cc0000 vs #dd0000 — a real recolour. Its ΔE2000 is ~3.56: above the
        // JND (COLOR_DE_PASS 2.5) so it IS a perceptible colour error, but BELOW
        // COLOR_DE_FAIL (6.0) — so it fails on ColorErr AREA, not the hard-colour
        // gate. (The brief's prose claims ">6" for this pair; the real value is
        // ~3.56 — reported to the orchestrator.)
        let c = srgb_to_lab([0xcc, 0, 0]);
        let d = srgb_to_lab([0xdd, 0, 0]);
        let de = ciede2000(c, d);
        assert!(
            de > 2.5 && de < 5.0,
            "#cc0000 vs #dd0000 ΔE = {de:.3} (expected ~3.56)"
        );
    }
}
