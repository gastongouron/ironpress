//! The PDF GEOMETRY verifier (spec §2): a dependency-free content-stream
//! tokenizer + an EXACT-in-pt geometry assertion against a committed coordinate
//! sidecar. Decoupled from the renderer — its only inputs are PDF bytes + a
//! sidecar, so any engine that emits a PDF is judged by the SAME contract.
//!
//! Why a tokenizer and not a PDF library: ironpress writes the page content stream
//! UNCOMPRESSED (the content obj has `/Length` and NO `/Filter`; only fonts/images
//! are FlateDecode'd — see `src/render/pdf.rs`), so a small byte-scan over the
//! operator soup recovers the geometry exactly, with no new crate. If a content
//! stream IS filtered/unfindable, `extract_geometry` returns `None` and the
//! verifier degrades to the raster geometry fallback (§2.6) — never a false
//! pass/fail.
//!
//! THE ROBUSTNESS REFINEMENT (spec §2.3, the brief's key requirement): the
//! verifier finds the SINGLE whole-page `(dx,dy)` translation that best aligns the
//! candidate onto the sidecar, applies it, then requires EVERY element within
//! `GEOM_TOL_PT`. A whole-page offset cancels the Chrome-frame vs ironpress-frame
//! margin-rounding difference EXACTLY at vector level — but because it is ONE
//! translation for the whole page, a PER-ELEMENT bug cannot be aligned away (the
//! other elements then mismatch). SIZES (w,h,size) are frame-independent and are
//! compared WITHOUT the offset (must match exactly).
//!
//! PHASE 2a: this is DORMANT. No sidecar files exist, so `applies()` (which needs
//! `ctx.coords.is_some()`) is false for every fixture and the combined verdict is
//! unchanged. The tokenizer + verifier are exercised entirely by `goldens.rs`.

use super::super::config::{GEOM_TOL_PT, PAGE_H_PT};
use super::super::report::Status;
use super::coords::{CoordBox, CoordSidecar, CoordText};
use super::{Concern, SubVerdict, Verifier, VerifierKind, VerifyCtx};

// ---------------------------------------------------------------------------
// Extracted geometry primitives (candidate side), top-left-origin pt.
// ---------------------------------------------------------------------------

/// A solid fill rect (`x y w h re` + `f`/`f*`/`B`), top-left-origin pt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FillRect {
    /// `[x, y_topleft, w, h]` in pt.
    pub(crate) rect_pt: [f64; 4],
    /// Current non-stroke colour `[r,g,b]` (0..1) at fill time.
    pub(crate) fill: [f64; 3],
}

/// A border-box rect reconstructed from a run of `m..l..S` segments, plus the
/// active stroke width, top-left-origin pt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BorderRect {
    /// `[x, y_topleft, w, h]` of the bounding rect of the segment run, in pt.
    pub(crate) rect_pt: [f64; 4],
    /// Active `{w} w` stroke width in pt.
    pub(crate) width_pt: f64,
    /// How `rect_pt` was reconstructed, which fixes the centerline convention:
    ///   * `true`  — from a run of `m..l..S` per-side strokes. The segment
    ///     endpoints overshoot half the stroke width at each corner, so the bbox is
    ///     the OUTER border-box edge; the centerline is the bbox inset by half-width.
    ///   * `false` — from a self-contained `x y w h re S`. ironpress emits this in
    ///     TWO conventions (the block-uniform path strokes the OUTER box; the
    ///     image/grid-cell path strokes the already-inset CENTERLINE box), so the
    ///     convention here is disambiguated against the element's fill rect in
    ///     `border_prim` rather than assumed.
    pub(crate) from_segments: bool,
}

/// A clip rect (`x y w h re W n`), top-left-origin pt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClipRect {
    /// `[x, y_topleft, w, h]` in pt.
    pub(crate) rect_pt: [f64; 4],
}

/// A text-run baseline origin (`Tm`/`Td`/`TD`/`T*` composed) + font size (`Tf`),
/// top-left-origin pt. Only the baseline origin + size are asserted (NOT glyph
/// advances — that is the raster Presence/Appearance axis, §2.1).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextRun {
    /// `[x, y_topleft]` of the baseline origin in pt.
    pub(crate) origin_pt: [f64; 2],
    /// Font size in pt (from `Tf`).
    pub(crate) size_pt: f64,
}

/// The candidate's extracted vector geometry, all in top-left-origin pt.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PdfGeometry {
    pub(crate) fills: Vec<FillRect>,
    pub(crate) borders: Vec<BorderRect>,
    pub(crate) clips: Vec<ClipRect>,
    pub(crate) text_runs: Vec<TextRun>,
}

// ---------------------------------------------------------------------------
// 2x3 affine matrix [a b c d e f] (PDF `cm`), point map (a*x+c*y+e, b*x+d*y+f).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Mat {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Mat {
    const IDENTITY: Mat = Mat {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Apply this matrix to a point.
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// `self` premultiplied by `m` (i.e. the matrix that first applies `m`, then
    /// `self`): used for `cm`, which prepends to the current CTM (CTM' = cm * CTM,
    /// in PDF row-vector convention `[x y 1] * cm * CTM`).
    fn prepend(&self, m: Mat) -> Mat {
        Mat {
            a: m.a * self.a + m.b * self.c,
            b: m.a * self.b + m.b * self.d,
            c: m.c * self.a + m.d * self.c,
            d: m.c * self.b + m.d * self.d,
            e: m.e * self.a + m.f * self.c + self.e,
            f: m.e * self.b + m.f * self.d + self.f,
        }
    }
}

// ---------------------------------------------------------------------------
// Content-stream location.
// ---------------------------------------------------------------------------

/// Find the page content stream body: the FIRST object whose dict has `/Length`
/// and NO `/Filter`, whose body (between `stream\n` and `\nendstream`) contains
/// ` re` and one of ` rg` / ` cm` / ` m`. Returns the body bytes, or `None` if no
/// such stream is found (filtered / unfindable). Mirrors the simple byte-scan
/// style in `render.rs::check_pdf_valid`.
fn find_content_stream(pdf: &[u8]) -> Option<&[u8]> {
    let mut search_from = 0usize;
    while let Some(rel) = find(&pdf[search_from..], b"stream") {
        let kw = search_from + rel;
        // Look BACK to the start of this object's dict (`<<`) to read `/Length`/
        // `/Filter`. The dict is between the nearest preceding `<<` and `stream`.
        let dict_start = rfind(&pdf[..kw], b"<<").unwrap_or(0);
        let dict = &pdf[dict_start..kw];
        // Body starts after `stream` + the single EOL (`\n` or `\r\n`).
        let mut body_start = kw + b"stream".len();
        if pdf.get(body_start) == Some(&b'\r') {
            body_start += 1;
        }
        if pdf.get(body_start) == Some(&b'\n') {
            body_start += 1;
        }
        // Body ends at the next `endstream` (its preceding EOL is stripped by the
        // op tokenizer's whitespace handling, so an inexact trim is fine).
        let body_end = match find(&pdf[body_start..], b"endstream") {
            Some(r) => body_start + r,
            None => return None,
        };
        let body = &pdf[body_start..body_end];

        let filtered = contains(dict, b"/Filter");
        let has_length = contains(dict, b"/Length");
        let looks_like_content = contains(body, b" re")
            && (contains(body, b" rg") || contains(body, b" cm") || contains(body, b" m"));
        if has_length && !filtered && looks_like_content {
            return Some(body);
        }
        search_from = body_end + b"endstream".len();
    }
    None
}

// --- tiny byte-slice helpers (dependency-free, like util::contains) ---

fn contains(h: &[u8], n: &[u8]) -> bool {
    find(h, n).is_some()
}
fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w == n)
}
fn rfind(h: &[u8], n: &[u8]) -> Option<usize> {
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    h.windows(n.len()).rposition(|w| w == n)
}

// ---------------------------------------------------------------------------
// Tokenizer + state machine.
// ---------------------------------------------------------------------------

/// A content-stream token: a real number, or a bare operator word.
enum Tok<'a> {
    Num(f64),
    Op(&'a [u8]),
}

/// Split a content-stream body into number/operator tokens. Hex strings `<...>`,
/// literal strings `(...)`, and array brackets `[ ]` are handled coarsely: the
/// geometry we assert never depends on string CONTENT (only on the preceding
/// `Tm`/`Td`/`Tf`), so strings are emitted as opaque `Op` words and arrays are
/// transparent (their numeric contents are TJ kerns, ignored by the text logic
/// because no geometry op consumes them).
fn tokenize(body: &[u8]) -> Vec<Tok<'_>> {
    let mut toks = Vec::new();
    let mut i = 0usize;
    let n = body.len();
    while i < n {
        let c = body[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0' => {
                i += 1;
            }
            b'[' | b']' | b'{' | b'}' => {
                // Array/proc delimiters: skip (TJ kern arrays etc. carry no rect).
                i += 1;
            }
            b'(' => {
                // Literal string: skip balanced parens with `\` escapes.
                i += 1;
                let mut depth = 1;
                while i < n && depth > 0 {
                    match body[i] {
                        b'\\' => i += 2,
                        b'(' => {
                            depth += 1;
                            i += 1;
                        }
                        b')' => {
                            depth -= 1;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'<' => {
                // Hex string `<...>` (or `<<` dict, which never appears in a
                // content stream body) — skip to the matching `>`.
                i += 1;
                while i < n && body[i] != b'>' {
                    i += 1;
                }
                i += 1;
            }
            b'>' => {
                i += 1;
            }
            b'/' => {
                // PDF name (`/F1`, `/DeviceRGB`): not a geometry operand. Skip the
                // `/` and the name body so it is never tokenized as an empty word
                // (which would not advance `i` -> infinite loop).
                i += 1;
                while i < n && !is_delim(body[i]) {
                    i += 1;
                }
            }
            _ => {
                let start = i;
                while i < n && !is_delim(body[i]) {
                    i += 1;
                }
                if i == start {
                    // Defensive: an unexpected lone delimiter — never stall.
                    i += 1;
                    continue;
                }
                let word = &body[start..i];
                if let Some(num) = parse_num(word) {
                    toks.push(Tok::Num(num));
                } else {
                    toks.push(Tok::Op(word));
                }
            }
        }
    }
    toks
}

fn is_delim(c: u8) -> bool {
    matches!(
        c,
        b' ' | b'\t'
            | b'\r'
            | b'\n'
            | b'\x0c'
            | b'\0'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'('
            | b')'
            | b'<'
            | b'>'
            | b'/'
    )
}

/// Parse a PDF real/integer token (e.g. `-0.30`, `45.3`, `1`). A leading `/`
/// name or any non-numeric word returns `None`.
fn parse_num(w: &[u8]) -> Option<f64> {
    if w.is_empty() {
        return None;
    }
    let s = std::str::from_utf8(w).ok()?;
    // PDF numbers: optional sign, digits, optional `.`, digits. `str::parse`
    // accepts exactly the forms ironpress emits via `format_pdf_number`.
    s.parse::<f64>().ok()
}

/// Reset accumulated path coords / pending segment run after a path-paint op.
#[derive(Default)]
struct PathState {
    /// Most recent `x y w h re` rect (PDF-space corners), pending its paint op.
    last_re: Option<[f64; 4]>,
    /// Accumulated `m`/`l` points for the current border-segment run (PDF-space).
    seg_pts: Vec<(f64, f64)>,
}

/// Extract candidate geometry from PDF bytes. `None` if the content stream is
/// filtered/unfindable (degrade to raster fallback, never guess).
pub(crate) fn extract_geometry(pdf: &[u8]) -> Option<PdfGeometry> {
    let body = find_content_stream(pdf)?;
    Some(extract_from_body(body))
}

/// Tokenize + interpret an already-located content-stream body. Split out so the
/// golden tests can feed a hand-written stream directly.
pub(crate) fn extract_from_body(body: &[u8]) -> PdfGeometry {
    let toks = tokenize(body);
    let mut geo = PdfGeometry::default();

    // Graphics state.
    let mut ctm = Mat::IDENTITY;
    let mut ctm_stack: Vec<Mat> = Vec::new();
    let mut nonstroke = [0.0f64, 0.0, 0.0];
    let mut line_width = 1.0f64;
    let mut path = PathState::default();

    // Text state (inside BT..ET). `Tm` sets the text matrix absolutely; `Td`/`TD`
    // translate it; `T*` translates by `-leading` (we have no TL, so 0); `Tf`
    // sets the size. The baseline origin is the text matrix * CTM applied to (0,0).
    let mut in_text = false;
    let mut tm = Mat::IDENTITY;
    let mut tlm = Mat::IDENTITY; // text line matrix (start of line)
    let mut font_size = 0.0f64;
    let mut text_emitted_for_run = false;

    // Operand stack (numbers preceding an operator).
    let mut nums: Vec<f64> = Vec::new();

    for tok in &toks {
        match tok {
            Tok::Num(v) => nums.push(*v),
            Tok::Op(op) => {
                match *op {
                    // --- graphics state ---
                    b"q" => {
                        // A graphics-scope change ends the current border group.
                        flush_border(&mut geo, &mut path, line_width, &ctm);
                        ctm_stack.push(ctm);
                    }
                    b"Q" => {
                        // Flush against the CURRENT ctm (the segments were emitted in
                        // it) BEFORE popping the saved state.
                        flush_border(&mut geo, &mut path, line_width, &ctm);
                        if let Some(m) = ctm_stack.pop() {
                            ctm = m;
                        }
                    }
                    b"cm" if nums.len() >= 6 => {
                        let n = nums.len();
                        let m = Mat {
                            a: nums[n - 6],
                            b: nums[n - 5],
                            c: nums[n - 4],
                            d: nums[n - 3],
                            e: nums[n - 2],
                            f: nums[n - 1],
                        };
                        ctm = ctm.prepend(m);
                    }
                    b"w" if !nums.is_empty() => {
                        line_width = *nums.last().unwrap();
                    }
                    b"rg" if nums.len() >= 3 => {
                        let n = nums.len();
                        nonstroke = [nums[n - 3], nums[n - 2], nums[n - 1]];
                    }
                    b"g" if !nums.is_empty() => {
                        let v = *nums.last().unwrap();
                        nonstroke = [v, v, v];
                    }

                    // --- path construction ---
                    b"re" if nums.len() >= 4 => {
                        let n = nums.len();
                        path.last_re = Some([nums[n - 4], nums[n - 3], nums[n - 2], nums[n - 1]]);
                    }
                    b"m" if nums.len() >= 2 => {
                        let n = nums.len();
                        path.seg_pts.push((nums[n - 2], nums[n - 1]));
                    }
                    b"l" if nums.len() >= 2 => {
                        let n = nums.len();
                        path.seg_pts.push((nums[n - 2], nums[n - 1]));
                    }

                    // --- path painting ---
                    b"f" | b"f*" | b"F" => {
                        // A fill ends any pending border group, then paints the rect.
                        flush_border(&mut geo, &mut path, line_width, &ctm);
                        if let Some(re) = path.last_re.take() {
                            geo.fills.push(FillRect {
                                rect_pt: rect_topleft(re, &ctm),
                                fill: nonstroke,
                            });
                        }
                        path.seg_pts.clear();
                    }
                    b"B" | b"B*" | b"b" | b"b*" => {
                        // Fill+stroke: count the rect as a fill (its colour is the
                        // nonstroke fill); also flush any segment run as a border.
                        if let Some(re) = path.last_re.take() {
                            geo.fills.push(FillRect {
                                rect_pt: rect_topleft(re, &ctm),
                                fill: nonstroke,
                            });
                        }
                        flush_border(&mut geo, &mut path, line_width, &ctm);
                    }
                    b"S" | b"s" => {
                        // Stroke. A `re S` is a self-contained stroked-rectangle
                        // border -> emit immediately. A run of `m..l` segments is
                        // the per-side border ironpress emits as SEPARATE strokes
                        // (`x y m x2 y2 l S` per side); those are NOT flushed here —
                        // the points ACCUMULATE across consecutive segment strokes
                        // and are reduced to ONE bounding BorderRect at the next
                        // boundary (q/Q/fill/clip/text/EOF), so the 4 sides group.
                        if let Some(re) = path.last_re.take() {
                            geo.borders.push(BorderRect {
                                rect_pt: rect_topleft(re, &ctm),
                                width_pt: stroke_width_pt(line_width, &ctm),
                                from_segments: false,
                            });
                        }
                        // (segments left in `path.seg_pts` to group with neighbours)
                    }
                    b"W" => { /* clip: paired with the following `n`; rect kept */ }
                    b"W*" => {}
                    b"n" => {
                        // No-op paint: ends any border group; if a `re` preceded a
                        // `W`, it is a clip rect.
                        flush_border(&mut geo, &mut path, line_width, &ctm);
                        if let Some(re) = path.last_re.take() {
                            geo.clips.push(ClipRect {
                                rect_pt: rect_topleft(re, &ctm),
                            });
                        }
                        path.seg_pts.clear();
                    }

                    // --- text ---
                    b"BT" => {
                        // Text start ends any pending border group.
                        flush_border(&mut geo, &mut path, line_width, &ctm);
                        in_text = true;
                        tm = Mat::IDENTITY;
                        tlm = Mat::IDENTITY;
                        text_emitted_for_run = false;
                    }
                    b"ET" => {
                        in_text = false;
                    }
                    b"Tf" if !nums.is_empty() => {
                        font_size = *nums.last().unwrap();
                    }
                    b"Tm" if in_text && nums.len() >= 6 => {
                        let n = nums.len();
                        tm = Mat {
                            a: nums[n - 6],
                            b: nums[n - 5],
                            c: nums[n - 4],
                            d: nums[n - 3],
                            e: nums[n - 2],
                            f: nums[n - 1],
                        };
                        tlm = tm;
                        text_emitted_for_run = false;
                    }
                    b"Td" | b"TD" if in_text && nums.len() >= 2 => {
                        let n = nums.len();
                        let (tx, ty) = (nums[n - 2], nums[n - 1]);
                        tlm = tlm.prepend(Mat {
                            a: 1.0,
                            b: 0.0,
                            c: 0.0,
                            d: 1.0,
                            e: tx,
                            f: ty,
                        });
                        tm = tlm;
                        text_emitted_for_run = false;
                    }
                    b"T*" if in_text => {
                        // No leading (TL) tracked -> translate by 0 in y.
                        tlm = tlm.prepend(Mat {
                            a: 1.0,
                            b: 0.0,
                            c: 0.0,
                            d: 1.0,
                            e: 0.0,
                            f: 0.0,
                        });
                        tm = tlm;
                        text_emitted_for_run = false;
                    }
                    b"Tj" | b"TJ" | b"'" | b"\"" if in_text => {
                        // Emit ONE run per text-positioning setup (Tm/Td). The
                        // baseline origin is (tm * ctm) applied to (0,0).
                        if !text_emitted_for_run {
                            let full = ctm.prepend(tm);
                            let (ox, oy) = full.apply(0.0, 0.0);
                            geo.text_runs.push(TextRun {
                                origin_pt: [ox, PAGE_H_PT - oy],
                                size_pt: font_size,
                            });
                            text_emitted_for_run = true;
                        }
                    }

                    _ => {}
                }
                nums.clear();
            }
        }
    }

    // End of stream: flush any trailing border group (a border drawn as the last
    // thing on the page, with no following boundary op).
    flush_border(&mut geo, &mut path, line_width, &ctm);

    geo
}

/// Convert a PDF-space `re` rect `[x,y,w,h]` (bottom-left origin) through the CTM
/// to a top-left-origin pt rect `[x, y_topleft, w, h]` (axis-aligned bbox of the
/// 4 transformed corners; size is the bbox extent, transform-aware).
fn rect_topleft(re: [f64; 4], ctm: &Mat) -> [f64; 4] {
    let [x, y, w, h] = re;
    let corners = [
        ctm.apply(x, y),
        ctm.apply(x + w, y),
        ctm.apply(x, y + h),
        ctm.apply(x + w, y + h),
    ];
    let (mut minx, mut maxx) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut miny, mut maxy) = (f64::INFINITY, f64::NEG_INFINITY);
    for (px, py) in corners {
        minx = minx.min(px);
        maxx = maxx.max(px);
        miny = miny.min(py);
        maxy = maxy.max(py);
    }
    let ww = maxx - minx;
    let hh = maxy - miny;
    // Top-left origin: y_tl of the rect TOP = PAGE_H - (the PDF-space TOP edge).
    [minx, PAGE_H_PT - maxy, ww, hh]
}

/// Effective stroke width in pt after the CTM scale (uses the x-axis scale of the
/// CTM; ironpress's CTM is identity for borders, so this is the literal width).
fn stroke_width_pt(line_width: f64, ctm: &Mat) -> f64 {
    let scale = (ctm.a * ctm.a + ctm.b * ctm.b).sqrt();
    line_width
        * if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        }
}

/// Reconstruct a `BorderRect` from the accumulated `m..l` segment points (the 4
/// centered per-side segments ironpress emits): the bounding rect of all points +
/// the active stroke width. Clears the segment buffer.
fn flush_border(geo: &mut PdfGeometry, path: &mut PathState, line_width: f64, ctm: &Mat) {
    if path.seg_pts.is_empty() {
        return;
    }
    let (mut minx, mut maxx) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut miny, mut maxy) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(x, y) in &path.seg_pts {
        let (px, py) = ctm.apply(x, y);
        minx = minx.min(px);
        maxx = maxx.max(px);
        miny = miny.min(py);
        maxy = maxy.max(py);
    }
    path.seg_pts.clear();
    geo.borders.push(BorderRect {
        rect_pt: [minx, PAGE_H_PT - maxy, maxx - minx, maxy - miny],
        width_pt: stroke_width_pt(line_width, ctm),
        from_segments: true,
    });
}

// ---------------------------------------------------------------------------
// The verifier.
// ---------------------------------------------------------------------------

/// The PDF geometry verifier. Owns `Concern::Geometry` when a sidecar applies.
pub(crate) struct PdfGeomVerifier;

/// Max sane whole-page alignment offset (pt). ~3pt ≈ 4 CSS px — covers the
/// Chrome-frame vs ironpress-frame margin-rounding offset (≈0.96pt) with margin,
/// but a GROSS global shift still exceeds it and FAILs (spec §2.3 step b).
const MAX_ALIGN_PT: f64 = 3.0;

impl Verifier for PdfGeomVerifier {
    fn kind(&self) -> VerifierKind {
        VerifierKind::PdfGeometry
    }

    /// Applies only when a sidecar is committed AND the candidate PDF is
    /// tokenizable (§2.6). PHASE 2a: no sidecar files exist, so this is false for
    /// every fixture — the verdict path is unchanged (proven a no-op).
    fn applies(&self, ctx: &VerifyCtx) -> bool {
        ctx.coords.is_some() && extract_geometry(ctx.pdf).is_some()
    }

    fn verify(&self, ctx: &VerifyCtx) -> Vec<SubVerdict> {
        let sidecar = match ctx.coords {
            Some(s) => s,
            // `applies()` is the gate; defensively return Unknown if called anyway.
            None => return vec![unknown("no sidecar")],
        };
        let cand = match extract_geometry(ctx.pdf) {
            Some(g) => g,
            None => return vec![unknown("content stream filtered/unfindable")],
        };
        vec![verify_geometry(&cand, sidecar)]
    }
}

fn unknown(why: &str) -> SubVerdict {
    SubVerdict {
        verifier: VerifierKind::PdfGeometry,
        status: Status::Unknown,
        concern: Concern::Geometry,
        headline: format!("pdf-geom unknown: {why}"),
        magnitude: 0.0,
    }
}

/// A candidate primitive's geometry, reduced to the comparable scalars: a rect
/// (`[x, y_tl, w, h]`) for fills/borders/clips, or a text origin+size encoded as
/// `[x, y_tl, size, NaN]` (the NaN marks "no h"). Kept small so the matcher is one
/// code path over `(position[2], size[2])`.
#[derive(Clone, Copy)]
struct Prim {
    pos: [f64; 2],  // x, y_tl  (frame-dependent, offset-cancelled)
    size: [f64; 2], // w,h  OR  size,NaN  (frame-INDEPENDENT, exact)
}

fn box_prim(b: &CoordBox) -> Prim {
    Prim {
        pos: [b.rect_pt[0], b.rect_pt[1]],
        size: [b.rect_pt[2], b.rect_pt[3]],
    }
}
fn fill_prim(r: &[f64; 4]) -> Prim {
    Prim {
        pos: [r[0], r[1]],
        size: [r[2], r[3]],
    }
}
/// Reduce a candidate `BorderRect` to its CENTERLINE rect — the convention the
/// sidecar records (Chrome's `--print-to-pdf` strokes one `x y w h re S` at the
/// border centerline, inset half the border width from the outer edge).
///
/// ironpress emits borders in THREE shapes (see `src/render/pdf.rs`):
///   1. a run of per-side `m..l..S` strokes whose endpoints overshoot half-width
///      at each corner -> the reconstructed bbox is the OUTER border-box edge
///      (`from_segments == true`);
///   2. a self-contained `re S` on the OUTER box (block-uniform path);
///   3. a self-contained `re S` on the already-inset CENTERLINE box (image /
///      grid-cell path).
/// (2) and (3) are indistinguishable from the `re` rect alone, so we DISAMBIGUATE
/// against the element's fill rect: a CSS background fills the OUTER border-box, so
/// the element's outer edge is whichever fill rect is co-located with the border.
/// The centerline is then ALWAYS `outer − width`:
///   * segments / shape-(2): outer = bbox/`re`; centerline = inset by half-width.
///   * shape-(3): the `re` is already the centerline (it equals `fill − width`),
///     so the co-located fill is the outer box and `fill − width == re` -> no
///     double inset (the prior bug inset a second time, shrinking cells by the
///     full border width, e.g. a 73.5pt cell -> 72.0pt, Δ1.5pt).
///
/// `fills` is the candidate's solid-fill rects (already background-filtered). When
/// no fill is co-located (transparent element, e.g. probe-border-box), we fall back
/// to insetting the reconstructed rect, matching the segment/outer convention.
fn border_prim(b: &BorderRect, fills: &[[f64; 4]]) -> Prim {
    let half = b.width_pt / 2.0;
    let [x, y, w, h] = b.rect_pt;
    // The element's OUTER border-box. Prefer the co-located background fill — a CSS
    // background paints the full outer border-box, so it is the authoritative outer
    // edge regardless of which `re S`/segment convention the border used. The right
    // fill is the one whose CENTER coincides with the border AND whose size is the
    // border's own reconstructed box grown by AT MOST one stroke width (a `re S`
    // centerline border sits exactly one width inside its bg; segments / `re S`-outer
    // equal it). A fill more than a width bigger is an ANCESTOR's background
    // (concentric nesting shares a center) and is rejected.
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let (outer_pos, outer_size) = match own_background_fill(fills, cx, cy, [w, h], b.width_pt) {
        Some(f) => ([f[0], f[1]], [f[2], f[3]]),
        // No co-located fill (transparent element, e.g. probe-border-box). A
        // `m..l..S` segment run's bbox (`from_segments`) is the OUTER edge, and a lone
        // `re S` with no fill is the block-uniform OUTER box — both inset to the
        // centerline below, so the reconstructed rect IS the outer box.
        None => ([x, y], [w, h]),
    };
    Prim {
        // Centerline = outer box inset by half the stroke width on every side.
        pos: [outer_pos[0] + half, outer_pos[1] + half],
        size: [
            (outer_size[0] - b.width_pt).max(0.0),
            (outer_size[1] - b.width_pt).max(0.0),
        ],
    }
}
fn text_prim(t: &CoordText) -> Prim {
    Prim {
        pos: t.origin_pt,
        size: [t.size_pt, f64::NAN],
    }
}
fn run_prim(t: &TextRun) -> Prim {
    Prim {
        pos: t.origin_pt,
        size: [t.size_pt, f64::NAN],
    }
}

/// The candidate fill that is the border's OWN element background: its center
/// coincides with the border center (concentric), and its size is the border's own
/// reconstructed box (`own_size`) grown by AT MOST one stroke `width` on each
/// dimension (a `re S` centerline border sits exactly one width inside its bg; a
/// segment/`re S`-outer border equals its bg). Among qualifying fills, the one
/// nearest `own_size` wins, so a strictly larger ANCESTOR background (concentric
/// nesting) is rejected. `None` if no fill qualifies (transparent element).
fn own_background_fill(
    fills: &[[f64; 4]],
    cx: f64,
    cy: f64,
    own_size: [f64; 2],
    width: f64,
) -> Option<[f64; 4]> {
    let center_slack = width.max(GEOM_TOL_PT) + GEOM_TOL_PT;
    let size_grow_max = width + GEOM_TOL_PT;
    fills
        .iter()
        .filter_map(|r| {
            let fcx = r[0] + r[2] / 2.0;
            let fcy = r[1] + r[3] / 2.0;
            let center_d = (fcx - cx).abs().max((fcy - cy).abs());
            if center_d > center_slack {
                return None;
            }
            // Fill must be >= own box (the bg encloses the border) and at most one
            // width bigger. Allow a small negative slack for sub-pt rounding.
            let grow_w = r[2] - own_size[0];
            let grow_h = r[3] - own_size[1];
            if grow_w < -GEOM_TOL_PT
                || grow_h < -GEOM_TOL_PT
                || grow_w > size_grow_max
                || grow_h > size_grow_max
            {
                return None;
            }
            // Rank by closeness to the own box (prefer the tightest enclosing bg).
            Some((grow_w.abs().max(grow_h.abs()), *r))
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, r)| r)
}

/// L-inf distance between two primitive POSITIONS (after an offset is applied to
/// the candidate). Used both for alignment and matching.
fn pos_linf(expected: Prim, cand: Prim, off: (f64, f64)) -> f64 {
    let dx = (cand.pos[0] + off.0 - expected.pos[0]).abs();
    let dy = (cand.pos[1] + off.1 - expected.pos[1]).abs();
    dx.max(dy)
}

/// L-inf distance between two primitive SIZES (frame-independent; NaN axes skipped).
fn size_linf(expected: Prim, cand: Prim) -> f64 {
    let mut d = 0.0f64;
    for axis in 0..2 {
        if cand.size[axis].is_nan() || expected.size[axis].is_nan() {
            continue;
        }
        d = d.max((cand.size[axis] - expected.size[axis]).abs());
    }
    d
}

/// Combined match cost: post-offset POSITION distance plus SIZE distance, equally
/// weighted. Position assigns among equal-size siblings (two same-size borders at
/// different spots go to the right one — pure-position behaviour). Size is the
/// guard that stops a small expected box from being matched to a candidate with the
/// SAME corner but a wildly different size — the full-page background fill (the
/// `fill#0 h Δ629pt` bug): its size distance alone (~629pt) dwarfs the real box's
/// near-zero cost, so the real box wins.
fn match_cost(expected: Prim, cand: Prim, off: (f64, f64)) -> f64 {
    pos_linf(expected, cand, off) + size_linf(expected, cand)
}

/// True if `r` is the full-page background fill ironpress paints (the opaque page
/// rect covering most of the printable area). It is identified by AREA: a fill
/// covering > 55% of the page is the background, never a content box. The sidecar
/// never records it, so dropping it cannot hide an expected box. Tied to the
/// sidecar's `page_pt` so a different page size scales the threshold.
fn is_page_background(r: &[f64; 4], sidecar: &CoordSidecar) -> bool {
    let page_area = sidecar.page_pt[0] * sidecar.page_pt[1];
    if page_area <= 0.0 {
        return false;
    }
    (r[2] * r[3]) / page_area > 0.55
}

/// Test-only access to the core geometry assertion (the goldens feed synthetic
/// `PdfGeometry` + `CoordSidecar` directly, without a PDF or a `VerifyCtx`).
#[cfg(test)]
pub(crate) fn verify_geometry_for_test(cand: &PdfGeometry, sidecar: &CoordSidecar) -> SubVerdict {
    verify_geometry(cand, sidecar)
}

/// The full vector-geometry assertion (spec §2.3) with the whole-page offset
/// refinement. Returns ONE Geometry SubVerdict.
/// A sidecar "fill" whose minor axis is below this (pt) is a BORDER HAIRLINE, not
/// a content box. Chrome's `border-collapse` paints each collapsed cell border as
/// a thin filled rect (`x y w h re f`, ~1.5-4pt minor axis), so it lands in the
/// sidecar's `boxes`. ironpress draws the same borders as STROKED line paths
/// (`m..l..S`), which are not `re` fills, so they can never match — producing a
/// false `fill#N unmatched` FAIL even when the render is pixel-identical (e.g.
/// tables-layout-fixed: 0.18% raster diff, all RasterDiff concerns PASS, yet
/// PdfGeometry FAILs on the 17 border-segment fills). PdfGeometry's contract is
/// CONTENT-BOX geometry (offset-cancelled size verification of real boxes); border
/// REPRESENTATION (fill vs stroke) is engine-specific and is already judged by
/// RasterDiff's Presence/Appearance at the border pixels. So thin fills are
/// excluded from the content-fill match. Content cells/boxes are far larger
/// (≥~20pt), so this never drops a real box; any thin-element defect is still
/// caught by RasterDiff.
const THIN_FILL_PT: f64 = 4.0;

fn verify_geometry(cand: &PdfGeometry, sidecar: &CoordSidecar) -> SubVerdict {
    // Expected + candidate primitives, grouped by kind (fills, borders, text).
    // Border-hairline fills (Chrome's collapsed-border-as-fill segments) are
    // excluded — see THIN_FILL_PT.
    let exp_fills: Vec<Prim> = sidecar
        .boxes
        .iter()
        .filter(|b| b.rect_pt[2].min(b.rect_pt[3]) >= THIN_FILL_PT)
        .map(box_prim)
        .collect();
    let exp_borders: Vec<Prim> = sidecar.borders.iter().map(box_prim).collect();
    let exp_text: Vec<Prim> = sidecar.text_runs.iter().map(text_prim).collect();

    // Candidate fills, with the full-page background rect(s) REMOVED. ironpress
    // paints an opaque page background (`<margin> <margin> <printable_w>
    // <printable_h> re f`) covering the whole printable area; the sidecar never
    // records it (the extractor drops it too). Left in, it shadows real boxes in
    // the matcher because it shares the page's top-left corner with the first real
    // box (the `fill#0 h Δ629pt` bug). Filter it by area fraction of the page.
    let cand_fill_rects: Vec<[f64; 4]> = cand
        .fills
        .iter()
        .map(|f| f.rect_pt)
        .filter(|r| !is_page_background(r, sidecar))
        .collect();
    let cand_fills: Vec<Prim> = cand_fill_rects.iter().map(fill_prim).collect();
    // Borders reconstruct their centerline against the (filtered) fills, so a `re S`
    // border at the centerline is NOT inset twice (the cell Δ1.5pt bug).
    let cand_borders: Vec<Prim> = cand
        .borders
        .iter()
        .map(|b| border_prim(b, &cand_fill_rects))
        .collect();
    let cand_text: Vec<Prim> = cand.text_runs.iter().map(run_prim).collect();

    let groups: [(&str, &[Prim], &[Prim]); 3] = [
        ("fill", &exp_fills, &cand_fills),
        ("border", &exp_borders, &cand_borders),
        ("text", &exp_text, &cand_text),
    ];

    // --- (b) whole-page offset alignment ---
    // The single (dx,dy) that best aligns candidate -> sidecar. Estimate it from
    // the per-axis MEDIAN of (expected.pos - nearest-candidate.pos) over the
    // largest-area matches: a per-element bug perturbs only a minority, so the
    // median is robust to it (a uniform frame offset moves ALL by the same amount).
    let offset = estimate_offset(&groups);
    let off_mag = offset.0.abs().max(offset.1.abs());

    // A gross global shift beyond MAX_ALIGN_PT is itself a failure (§2.3 d): the
    // page is not merely frame-shifted, it is misplaced.
    let gross_offset = off_mag > MAX_ALIGN_PT;

    // --- (c) per-primitive match + worst delta ---
    let mut worst_delta = 0.0f64;
    let mut worst_label = String::new();
    let mut missing = false;

    for (kind, expected, candidates) in groups {
        for (i, &exp) in expected.iter().enumerate() {
            // Best candidate of the same kind by SIZE-then-position cost: size is
            // exact and frame-independent, so the right box is the one whose size
            // matches; position (post-offset) only breaks ties. This avoids matching
            // a small box to the page background by shared corner.
            let best = candidates
                .iter()
                .map(|&c| (match_cost(exp, c, offset), c))
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(_, c)| (pos_linf(exp, c, offset), c));

            let (pos_d, cand) = match best {
                Some((d, c)) => (d, c),
                None => {
                    // No candidate of this kind at all -> missing.
                    missing = true;
                    if pos_d_track(&mut worst_delta, 4.0 * GEOM_TOL_PT + 1.0) {
                        worst_label = format!("{kind}#{i} missing (no candidate)");
                    }
                    continue;
                }
            };

            // Missing / grossly misplaced: no candidate within 4*TOL.
            if pos_d > 4.0 * GEOM_TOL_PT {
                missing = true;
                if pos_d_track(&mut worst_delta, pos_d) {
                    worst_label = format!("{kind}#{i} unmatched (nearest {pos_d:.3}pt)");
                }
                continue;
            }

            // Position delta (post-offset) — the worst of x,y.
            if pos_d_track(&mut worst_delta, pos_d) {
                worst_label = format!("{kind}#{i} pos Δ{pos_d:.3}pt");
            }

            // SIZE delta — compared WITHOUT the offset (frame-independent, exact).
            // For text, size[1] is NaN ("no h") and is skipped.
            for axis in 0..2 {
                if cand.size[axis].is_nan() || exp.size[axis].is_nan() {
                    continue;
                }
                let sd = (cand.size[axis] - exp.size[axis]).abs();
                if pos_d_track(&mut worst_delta, sd) {
                    let an = if axis == 0 { "w/size" } else { "h" };
                    worst_label = format!("{kind}#{i} {an} Δ{sd:.3}pt");
                }
            }
        }
    }

    // --- (d) verdict ---
    let status = if missing || gross_offset || worst_delta > 2.0 * GEOM_TOL_PT {
        Status::Fail
    } else if worst_delta > GEOM_TOL_PT {
        Status::Partial
    } else {
        Status::Pass
    };

    let headline = if gross_offset {
        // A gross median offset means the content is uniformly displaced (e.g. a
        // dropped container margin/padding). The worst per-element delta is also
        // reported so the report distinguishes a pure shift from a mixed failure
        // (shift + mis-sized boxes), rather than implying ONLY the page moved.
        let extra = if worst_label.is_empty() {
            String::new()
        } else {
            format!("; worst {worst_label}")
        };
        format!(
            "pdf-geom: gross page offset {off_mag:.3}pt (> {MAX_ALIGN_PT}pt) — content displaced{extra}"
        )
    } else if status == Status::Fail && missing {
        format!("pdf-geom FAIL: {worst_label}")
    } else {
        format!("pdf-geom: worst {worst_label} (offset {off_mag:.3}pt cancelled)")
    };

    SubVerdict {
        verifier: VerifierKind::PdfGeometry,
        status,
        concern: Concern::Geometry,
        headline,
        magnitude: worst_delta.min(999.0),
    }
}

/// Track the running worst delta; returns true if `d` is the new worst (so the
/// caller can update the label).
fn pos_d_track(worst: &mut f64, d: f64) -> bool {
    if d > *worst {
        *worst = d;
        true
    } else {
        false
    }
}

/// Estimate the single whole-page (dx,dy): the per-axis MEDIAN of
/// `(expected.pos - nearest-candidate.pos)` over all matched primitives (matched
/// at ZERO offset). A uniform frame offset shifts every primitive by the same
/// (dx,dy), so the median equals it; a per-element bug perturbs only a minority of
/// the deltas, so it does NOT move the median — hence it cannot be aligned away.
fn estimate_offset(groups: &[(&str, &[Prim], &[Prim]); 3]) -> (f64, f64) {
    let mut dxs: Vec<f64> = Vec::new();
    let mut dys: Vec<f64> = Vec::new();
    for (_, expected, candidates) in groups {
        for &exp in *expected {
            // Best candidate at zero offset by SIZE-then-position cost (same matcher
            // the verdict uses), so the offset is estimated from correctly-paired
            // boxes and not skewed by a size-mismatched accidental neighbour.
            let best = candidates
                .iter()
                .map(|&c| (match_cost(exp, c, (0.0, 0.0)), c))
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((_, c)) = best {
                dxs.push(exp.pos[0] - c.pos[0]);
                dys.push(exp.pos[1] - c.pos[1]);
            }
        }
    }
    (median(&mut dxs), median(&mut dys))
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}
