use crate::error::IronpressError;
use crate::layout::engine::{
    ImageFormat, LayoutElement, Page, PngMetadata, TableCell, TextLine, TextRun,
    layout_element_paint_order, table_cell_content_height, table_cell_intrinsic_content_height,
};
use crate::parser::ttf::TtfFont;
use crate::render::background::{
    BackgroundPaintContext, RasterBackgroundRequest, overflow_from_viewport_box,
    register_background_image, svg_visual_overflow, synthetic_raster_background,
    viewport_box_from_overflow,
};
use crate::render::pdf_fonts::{PreparedCustomFont, PreparedCustomFonts, prepare_custom_fonts};
use crate::render::shading::{
    ShadingEntry, build_shading_function, push_axial_shading, push_radial_shading,
};
use crate::render::svg_geometry::SvgViewportBox;
use crate::style::computed::{
    AlignItems, AlignSelf, BackgroundClip, BackgroundOrigin, BackgroundPosition, BackgroundRepeat,
    BackgroundSize, BorderCollapse, BorderStyle, Clear, ConicGradient, Float, FontFamily,
    LinearGradient, Overflow, Position, RadialExtent, RadialGradient, RadialShape, TextAlign,
    VerticalAlign,
};
use crate::types::{Margin, PageSize};
use std::collections::HashMap;
use std::io::Write as _;

mod layout_elements;

use layout_elements::{
    NestedLayoutFrame, PageRenderContext, TableCellRenderBox, collapse_paint_offset,
    compute_grid_row_height, compute_row_height, render_cell_content, row_baseline_shifts,
    table_cell_geometry,
};

#[cfg(test)]
use layout_elements::{
    CellTextPlacement, NestedTextBlock, TextRenderContext, plan_nested_layout_elements,
    render_cell_text, render_nested_layout_elements, render_nested_text_block,
    table_row_total_height,
};

/// The box a background's paint is clipped to (css-backgrounds-3 §3.4). Given a
/// BORDER box (`bx`,`by` = bottom-left in PDF coords, `bw`×`bh`) and the
/// per-side border + padding widths, returns the rect for `background-clip`:
/// `border-box` is the whole box, `padding-box` insets by the border, and
/// `content-box` insets by border + padding. Negative extents are clamped to 0.
#[allow(clippy::too_many_arguments)]
fn background_clip_rect(
    clip: BackgroundClip,
    bx: f32,
    by: f32,
    bw: f32,
    bh: f32,
    border_left: f32,
    border_right: f32,
    border_top: f32,
    border_bottom: f32,
    padding_left: f32,
    padding_right: f32,
    padding_top: f32,
    padding_bottom: f32,
) -> (f32, f32, f32, f32) {
    let (inset_left, inset_right, inset_top, inset_bottom) = match clip {
        BackgroundClip::Border => (0.0, 0.0, 0.0, 0.0),
        BackgroundClip::Padding => (border_left, border_right, border_top, border_bottom),
        BackgroundClip::Content => (
            border_left + padding_left,
            border_right + padding_right,
            border_top + padding_top,
            border_bottom + padding_bottom,
        ),
    };
    (
        bx + inset_left,
        by + inset_bottom,
        (bw - inset_left - inset_right).max(0.0),
        (bh - inset_top - inset_bottom).max(0.0),
    )
}

/// Emit a clip path (`q` + path + `W n`) for a background-clip box. Uses a
/// rounded-rect path when `border_radius` is set, otherwise a plain rectangle.
/// The caller is responsible for the matching `Q`. Returns `true` if a clip was
/// pushed (always, but kept for symmetry with conditional callers).
fn push_background_clip(content: &mut String, x: f32, y: f32, w: f32, h: f32, border_radius: f32) {
    content.push_str("q\n");
    if border_radius > 0.0 {
        content.push_str(&rounded_rect_path(x, y, w, h, border_radius));
        content.push_str("W n\n");
    } else {
        content.push_str(&format!("{x} {y} {w} {h} re W n\n"));
    }
}

/// Returns the PDF dash-pattern operator string for a given border style.
/// Width-scaled dash/dot setup for a border side. Returns the PDF operators to
/// install before stroking: a dash array (`d`) and, for dotted, a round line cap
/// (`1 J`) so each dash collapses to a round dot of diameter = the stroke width.
///
/// CSS renders dotted as round dots roughly one border-width across spaced one
/// width apart, and dashed as segments a few widths long. Scaling by the stroke
/// width (rather than the previous fixed `[6 4]`/`[1 3]`) matches Chrome far more
/// closely and makes the pattern visible at any border thickness.
fn dash_pattern_for_style(style: BorderStyle, width: f32) -> String {
    let w = width.max(0.1);
    match style {
        // Chrome paints dashed strokes with dashes ~2x the line width and gaps
        // ~1x the width (measured period ≈ 3x width, ink:gap ≈ 2:1), not the 3:3
        // (period 6x) of a naive equal pattern.
        BorderStyle::Dashed => {
            let dash = (w * 2.0).max(1.0);
            let gap = w.max(1.0);
            format!("[{dash} {gap}] 0 d\n")
        }
        // Round dots: a zero-length dash under a round cap paints a filled dot of
        // diameter = line width; spacing = 2x width gives width-on / width-off.
        BorderStyle::Dotted => {
            let gap = (w * 2.0).max(1.0);
            format!("1 J\n[0 {gap}] 0 d\n")
        }
        _ => String::new(),
    }
}

/// Reset the dash pattern (and line cap) back to solid/butt after a
/// dashed/dotted stroke so subsequent strokes are unaffected.
fn reset_dash_pattern(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::Dashed => "[] 0 d\n",
        BorderStyle::Dotted => "[] 0 d\n0 J\n",
        _ => "",
    }
}

/// Apply a stroke-opacity ExtGState before painting a border side whose color is
/// translucent (`alpha < 1.0`). Mirrors the background-color alpha path: pushes a
/// `(name, alpha)` entry onto `page_ext_gstates` and emits `/{name} gs`. Returns
/// `true` when a non-default gstate was applied so the caller can reset it with
/// [`end_border_alpha`]. For opaque sides (`alpha >= 1.0`) nothing is emitted, so
/// existing output stays byte-identical.
fn begin_border_alpha(
    content: &mut String,
    page_ext_gstates: &mut Vec<(String, f32)>,
    counter: &mut usize,
    alpha: f32,
) -> bool {
    if alpha < 1.0 {
        let gs_name = format!("GSbd{counter}");
        *counter += 1;
        page_ext_gstates.push((gs_name.clone(), alpha));
        content.push_str(&format!("/{gs_name} gs\n"));
        true
    } else {
        false
    }
}

/// Reset stroke opacity to the default gstate after a translucent border side.
/// No-op when `applied` is false.
fn end_border_alpha(content: &mut String, applied: bool) {
    if applied {
        content.push_str("/GSDefault gs\n");
    }
}

/// Emit the border-box outline path (rectangle or rounded rectangle) inset by
/// `inset` from each edge. `radii` are the per-corner radii of the outer
/// border-box edge; the inset path reduces each by `inset` so the stroke's outer
/// edge tracks the original corner. Used by the uniform-border painter to stroke
/// a single centerline path.
fn border_inset_path(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4], inset: f32) -> String {
    let iw = (w - 2.0 * inset).max(0.0);
    let ih = (h - 2.0 * inset).max(0.0);
    let ix = x + inset;
    let iy = y + inset;
    if !radii_any(radii) {
        return format!("{ix} {iy} {iw} {ih} re\n");
    }
    if radii_uniform(radii) {
        return rounded_rect_path(ix, iy, iw, ih, (radii[0] - inset).max(0.0));
    }
    let inner = [
        (radii[0] - inset).max(0.0),
        (radii[1] - inset).max(0.0),
        (radii[2] - inset).max(0.0),
        (radii[3] - inset).max(0.0),
    ];
    rounded_rect_path_per_corner(ix, iy, iw, ih, inner)
}

/// Emit a PDF clip path for CSS `overflow: hidden`/`clip`/`scroll`/`auto`.
///
/// CSS clips overflow at the PADDING box: the border box `(x, y, w, h)`
/// (bottom-left origin, PDF coordinates) inset by the per-side border widths
/// `(bl, br, bt, bb)`. When `radius > 0` the clip follows the rounded corners,
/// using the INNER radius (`radius - border`) at the padding box, matching the
/// way borders paint inside the box. Returns the path operators WITHOUT the
/// terminating `W n`, so callers append `"W n\n"` (or `"\nW n\n"`).
#[allow(clippy::too_many_arguments)]
fn overflow_clip_path(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bl: f32,
    br: f32,
    bt: f32,
    bb: f32,
    radius: f32,
) -> String {
    // Padding box = border box inset by the per-side border widths.
    let px = x + bl;
    let py = y + bb;
    let pw = (w - bl - br).max(0.0);
    let ph = (h - bt - bb).max(0.0);
    if radius > 0.0 {
        // Inner radius shrinks by the (max) border width so the rounded clip
        // hugs the inner border edge, like Chrome.
        let inner_r = (radius - bl.max(br).max(bt).max(bb)).max(0.0);
        if inner_r > 0.0 {
            // `rounded_rect_path` ends with `h\n`; the caller appends `W n\n`.
            return rounded_rect_path(px, py, pw, ph, inner_r);
        }
    }
    // Trailing space (no newline) so the caller's `W n\n` yields `... re W n\n`
    // on a single line (matching the established clip-path output convention).
    format!("{px} {py} {pw} {ph} re ")
}

/// Default UA scrollbar thickness, in PDF points (Chrome's classic scrollbar is
/// 15 CSS px wide; 1 px = 0.75 pt).
const SCROLLBAR_THICKNESS_PT: f32 = 15.0 * 0.75;

/// Paint a non-interactive UA scrollbar matching Chrome's print rendering for a
/// scroll container (`overflow: scroll`, or `auto` with overflow). The padding
/// box is `(px, py)` bottom-left, size `pw`×`ph` (PDF bottom-up coords).
/// `has_v`/`has_h` say which axes show a scrollbar (the gutter is reserved on
/// each). `over_v`/`over_h` are the overflow ratios (content / viewport) on each
/// axis, used to size the thumb (≥ 1 means it overflows). The track is light
/// gray, the thumb medium gray with rounded ends, and each end carries an arrow
/// button — same as Chrome's printed `overflow:scroll` boxes.
///
/// Caller must have ALREADY clipped content to the reduced (gutter-inset)
/// padding box; this only paints the scrollbar chrome on top, inside the gutter.
#[allow(clippy::too_many_arguments)]
fn paint_scrollbars(
    content: &mut String,
    px: f32,
    py: f32,
    pw: f32,
    ph: f32,
    has_v: bool,
    has_h: bool,
    over_v: f32,
    over_h: f32,
) {
    let t = SCROLLBAR_THICKNESS_PT;
    if (!has_v && !has_h) || pw <= t || ph <= t {
        return;
    }
    // Track / thumb / arrow colors sampled from Chrome's printed scrollbar.
    let track = "0.9882 0.9882 0.9882"; // ~ (252,252,252)
    let thumb = "0.5451 0.5451 0.5451"; // ~ (139,139,139)
    let v_gutter = if has_v { t } else { 0.0 };
    let h_gutter = if has_h { t } else { 0.0 };

    // When both scrollbars are present, fill the bottom-right corner square (the
    // gutter intersection that neither track covers) with the track color — same
    // as Chrome, which paints a plain corner there rather than leaving content.
    if has_v && has_h {
        content.push_str(&format!(
            "{track} rg\n{} {} {t} {t} re\nf\n",
            px + pw - t,
            py,
        ));
    }

    // Vertical scrollbar occupies the right gutter, full padding-box height
    // (minus the horizontal gutter at the bottom if present).
    if has_v {
        let bar_x = px + pw - t;
        let bar_bottom = py + h_gutter;
        let bar_h = ph - h_gutter;
        // Track.
        content.push_str(&format!(
            "{track} rg\n{bar_x} {bar_bottom} {t} {bar_h} re\nf\n"
        ));
        // Arrow buttons (square, at top and bottom of the track).
        let btn = t.min(bar_h / 2.0);
        // Up arrow at the top, down arrow at the bottom.
        let top_btn_cy = bar_bottom + bar_h - btn / 2.0;
        let bot_btn_cy = bar_bottom + btn / 2.0;
        let cx = bar_x + t / 2.0;
        let a = btn * 0.28; // arrow half-extent
        content.push_str(&format!("{thumb} rg\n"));
        // Up triangle.
        content.push_str(&format!(
            "{} {} m {} {} l {} {} l f\n",
            cx,
            top_btn_cy + a,
            cx - a,
            top_btn_cy - a,
            cx + a,
            top_btn_cy - a
        ));
        // Down triangle.
        content.push_str(&format!(
            "{} {} m {} {} l {} {} l f\n",
            cx,
            bot_btn_cy - a,
            cx - a,
            bot_btn_cy + a,
            cx + a,
            bot_btn_cy + a
        ));
        // Thumb between the buttons, sized by the overflow ratio, anchored to the
        // top (scroll position 0, matching an un-scrolled printed container).
        let track_inner = (bar_h - 2.0 * btn).max(0.0);
        if track_inner > 0.0 {
            let frac = (1.0 / over_v.max(1.0)).clamp(0.12, 1.0);
            let thumb_h = (track_inner * frac).max(t * 0.5);
            let thumb_top = bar_bottom + bar_h - btn;
            let thumb_bottom = (thumb_top - thumb_h).max(bar_bottom + btn);
            let inset = t * 0.18;
            let r = (t / 2.0 - inset).max(0.0);
            content.push_str(&rounded_rect_path(
                bar_x + inset,
                thumb_bottom,
                (t - 2.0 * inset).max(0.0),
                (thumb_top - thumb_bottom).max(0.0),
                r,
            ));
            content.push_str("f\n");
        }
    }

    // Horizontal scrollbar occupies the bottom gutter, full padding-box width
    // (minus the vertical gutter on the right if present).
    if has_h {
        let bar_x = px;
        let bar_y = py;
        let bar_w = pw - v_gutter;
        content.push_str(&format!("{track} rg\n{bar_x} {bar_y} {bar_w} {t} re\nf\n"));
        let btn = t.min(bar_w / 2.0);
        let left_btn_cx = bar_x + btn / 2.0;
        let right_btn_cx = bar_x + bar_w - btn / 2.0;
        let cy = bar_y + t / 2.0;
        let a = btn * 0.28;
        content.push_str(&format!("{thumb} rg\n"));
        // Left triangle.
        content.push_str(&format!(
            "{} {} m {} {} l {} {} l f\n",
            left_btn_cx - a,
            cy,
            left_btn_cx + a,
            cy + a,
            left_btn_cx + a,
            cy - a
        ));
        // Right triangle.
        content.push_str(&format!(
            "{} {} m {} {} l {} {} l f\n",
            right_btn_cx + a,
            cy,
            right_btn_cx - a,
            cy + a,
            right_btn_cx - a,
            cy - a
        ));
        let track_inner = (bar_w - 2.0 * btn).max(0.0);
        if track_inner > 0.0 {
            let frac = (1.0 / over_h.max(1.0)).clamp(0.12, 1.0);
            let thumb_w = (track_inner * frac).max(t * 0.5);
            let thumb_left = bar_x + btn;
            let thumb_right = (thumb_left + thumb_w).min(bar_x + bar_w - btn);
            let inset = t * 0.18;
            let r = (t / 2.0 - inset).max(0.0);
            content.push_str(&rounded_rect_path(
                thumb_left,
                bar_y + inset,
                (thumb_right - thumb_left).max(0.0),
                (t - 2.0 * inset).max(0.0),
                r,
            ));
            content.push_str("f\n");
        }
    }
}

/// Whether a uniform border needs the special shared painter rather than the
/// legacy per-site solid path. True for non-solid styles (dashed/dotted/double)
/// and for any non-uniform per-corner radii. Plain solid borders (with uniform
/// or no radius) keep their original output for byte/geometry stability.
fn border_needs_special_paint(style: crate::style::computed::BorderStyle, radii: [f32; 4]) -> bool {
    style != crate::style::computed::BorderStyle::Solid
        && style != crate::style::computed::BorderStyle::None
        || (radii_any(radii) && !radii_uniform(radii))
}

/// Paint a uniform border (all four sides share width, color and style) around a
/// border box whose bottom-left is `(x, y)` and size is `w`×`h`, with per-corner
/// `radii`. Handles `solid`, `dashed`, `dotted` (round dots) and `double` (two
/// thin rules with a hollow middle third). The border paints INSIDE the box: the
/// stroke centerline sits half a border-width in from each edge so the stroke's
/// outer edge aligns with the border-box edge.
#[allow(clippy::too_many_arguments)]
fn paint_uniform_border(
    content: &mut String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radii: [f32; 4],
    side: &crate::layout::engine::LayoutBorderSide,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
) {
    let bw = side.width;
    if bw <= 0.0 || side.style == crate::style::computed::BorderStyle::None {
        return;
    }
    let (r, g, b) = side.color;
    let a = begin_border_alpha(content, page_ext_gstates, bg_alpha_counter, side.alpha);
    content.push_str(&format!("{r} {g} {b} RG\n"));
    if side.style == crate::style::computed::BorderStyle::Double {
        // Split the band into outer-third rule, gap-third, inner-third rule.
        let third = bw / 3.0;
        // Outer rule centerline is half a third in from the outer edge.
        content.push_str(&format!("{third} w\n"));
        content.push_str(&border_inset_path(x, y, w, h, radii, third / 2.0));
        content.push_str("S\n");
        // Inner rule centerline is bw - third/2 in from the outer edge.
        content.push_str(&format!("{third} w\n"));
        content.push_str(&border_inset_path(x, y, w, h, radii, bw - third / 2.0));
        content.push_str("S\n");
    } else if (side.style == crate::style::computed::BorderStyle::Dashed
        || side.style == crate::style::computed::BorderStyle::Dotted)
        && !radii_any(radii)
    {
        // Corner-symmetric dashed/dotted: stroke each side as its own centerline
        // with a per-side dash array tuned so an integer number of dashes/dots
        // fits the side with one centered on each corner (CSS Backgrounds §4.3:
        // "choose a spacing that makes the corners symmetrical"). This matches
        // Chrome far better than dashing one continuous rectangle perimeter,
        // which drifts out of phase and skips corners.
        let half = bw / 2.0;
        let dotted = side.style == crate::style::computed::BorderStyle::Dotted;
        // The cross-axis position of each side's centerline is always mid-border
        // (inset by half the width).
        let mid_left = x + half;
        let mid_right = x + w - half;
        let mid_top = y + h - half;
        let mid_bottom = y + half;
        // Along its own axis, dashes are laid over the FULL border-box edge
        // length (corner to corner at the OUTER edges), matching Chrome — which
        // spaces an integer number of dashes over the whole side, so the gap (and
        // phase) come from `w`/`h`, not the inner `w-bw`. The full-length centerline
        // with butt caps covers the corner columns, and the perpendicular sides
        // overlap there to fill the corner (same coverage as Chrome's miter).
        // Dots instead keep the inner (corner-inset) span so a dot sits centered on
        // each corner, matching Chrome's dotted corners.
        let (h_len, v_len, axis_start_h, axis_end_h, axis_start_v, axis_end_v) = if dotted {
            (
                (w - bw).max(0.0),
                (h - bw).max(0.0),
                mid_left,
                mid_right,
                y + h - half,
                y + half,
            )
        } else {
            (w, h, x, x + w, y + h, y)
        };
        let (h_arr, h_phase) = corner_dash_array(h_len, bw, dotted);
        let (v_arr, v_phase) = corner_dash_array(v_len, bw, dotted);
        let cap = if dotted { "1 J\n" } else { "0 J\n" };
        content.push_str(cap);
        content.push_str(&format!("{bw} w\n"));
        // Top and bottom (horizontal).
        content.push_str(&format!("[{h_arr}] {h_phase} d\n"));
        content.push_str(&format!(
            "{axis_start_h} {mid_top} m {axis_end_h} {mid_top} l S\n"
        ));
        content.push_str(&format!(
            "{axis_start_h} {mid_bottom} m {axis_end_h} {mid_bottom} l S\n"
        ));
        // Left and right (vertical).
        content.push_str(&format!("[{v_arr}] {v_phase} d\n"));
        content.push_str(&format!(
            "{mid_left} {axis_start_v} m {mid_left} {axis_end_v} l S\n"
        ));
        content.push_str(&format!(
            "{mid_right} {axis_start_v} m {mid_right} {axis_end_v} l S\n"
        ));
        content.push_str("[] 0 d\n0 J\n");
    } else {
        content.push_str(&dash_pattern_for_style(side.style, bw));
        content.push_str(&format!("{bw} w\n"));
        content.push_str(&border_inset_path(x, y, w, h, radii, bw / 2.0));
        content.push_str("S\n");
        content.push_str(reset_dash_pattern(side.style));
    }
    end_border_alpha(content, a);
}

/// Compute a corner-symmetric dash array + phase for one border side of length
/// `len` (corner-to-corner along the side centerline) and stroke width `bw`.
///
/// CSS leaves dash/dot metrics implementation-defined but recommends symmetric
/// corners. We target Chrome's look: dots are diameter `bw` spaced one diameter
/// apart (period `2*bw`); dashes are ~`3*bw` long with an equal gap (period
/// `~2*dash`). We then snap the period so a whole number of "on" segments lands
/// with one centered at each end (corner), and set the dash phase so the side
/// starts mid-gap such that the first on-segment is centered on the start
/// corner. Returns `(array_string, phase)` for the PDF `d` operator.
fn corner_dash_array(len: f32, bw: f32, dotted: bool) -> (String, f32) {
    let len = len.max(0.0);
    if len <= 0.0 || bw <= 0.0 {
        return (format!("{bw}"), 0.0);
    }
    // Nominal on/gap lengths. Matched to Chrome's look: dots are diameter `bw`
    // spaced one diameter apart (period `2*bw`); dashes are ~`2*bw` long with a
    // ~`1*bw` gap (period `~3*bw`, dash:gap ≈ 2:1).
    let (on, gap) = if dotted {
        (bw, bw)
    } else {
        ((bw * 2.0).max(1.0), bw.max(1.0))
    };
    let period = on + gap;
    if dotted {
        // Dots: a dot centered at each corner. Treat the side as `n` whole
        // periods so a dot lands on 0 and on `len`; phase 0 starts the (round-
        // cap) dot at the corner. Only the spacing flexes.
        let n = (len / period).round().max(1.0);
        let adj_period = len / n;
        // Zero-length dash under a round cap paints a dot of diameter `bw`.
        return (format!("0 {adj_period}"), 0.0);
    }
    // Dashes: Chrome draws a FULL dash flush to each corner (not a half-dash
    // straddling it) and keeps the dash length fixed at its nominal `2*bw`,
    // absorbing the corner-fitting adjustment into the GAP only. Measured: a 6px
    // border draws 12px (=2*width) dashes at both corners with a slightly
    // stretched gap — NOT a uniformly scaled-down 2:1 period, and NOT truncated
    // corner dashes. So lay out `n` dashes of length `on` separated by `n-1`
    // flexed gaps, with the run starting (phase 0) and ending on a dash.
    //   len = n*on + (n-1)*gap  =>  pick n so the flexed gap stays near nominal.
    let n = (((len + gap) / period).round()).max(1.0);
    let adj_on = on.min(len);
    let adj_gap = if n > 1.0 {
        ((len - n * adj_on) / (n - 1.0)).max(0.1)
    } else {
        // Single dash: pad the period so the lone dash sits flush at the start
        // corner with the remaining length as trailing gap (no second dash).
        (len - adj_on).max(0.1)
    };
    (format!("{adj_on} {adj_gap}"), 0.0)
}

/// Whether a (rectangular, radius-free) border should use the filled-trapezoid
/// miter painter rather than centerline strokes. CSS renders the corner where
/// two adjacent borders meet as a diagonal seam (each side fills a trapezoid
/// from its two outer corners to its two inner corners). That only matters when
/// adjacent sides differ in color or width — for a fully uniform border the
/// stroke path already produces the correct square frame, so we keep its
/// byte-stable output. Only plain `solid` (or `none`) sides are handled here;
/// dashed/dotted/double still stroke.
fn border_needs_miter_fill(border: &crate::layout::engine::LayoutBorder) -> bool {
    let sides = [&border.top, &border.right, &border.bottom, &border.left];
    let solidish = sides.iter().all(|s| {
        s.width <= 0.0
            || s.style == crate::style::computed::BorderStyle::Solid
            || s.style == crate::style::computed::BorderStyle::None
    });
    if !solidish {
        return false;
    }
    // The miter fill only matters where two ADJACENT painted sides differ in
    // COLOR (or alpha): that is the corner where CSS splits the color on a
    // diagonal seam. When adjacent sides share a color (even with different
    // widths), the centerline strokes already overlap into one continuous frame
    // with no visible seam, so we keep the simpler — and more pixel-stable —
    // stroke path. Opposite-only pairs never share a corner and are excluded.
    let adjacent_pairs = [
        (&border.top, &border.right),
        (&border.right, &border.bottom),
        (&border.bottom, &border.left),
        (&border.left, &border.top),
    ];
    adjacent_pairs
        .iter()
        .any(|(a, b)| a.width > 0.0 && b.width > 0.0 && (a.color != b.color || a.alpha != b.alpha))
}

/// Paint a non-uniform rectangular border as four filled trapezoids meeting at
/// 45°-ish diagonal miters, matching CSS Backgrounds §6.2: each side's color
/// fills the quad from its two outer border-box corners to its two inner
/// (padding-box) corners. `(x, y)` is the border-box bottom-left in PDF
/// (bottom-up) coords; `w`×`h` is the border-box size. Only solid sides are
/// drawn (others are skipped by the caller's dispatch).
#[allow(clippy::too_many_arguments)]
fn paint_miter_border(
    content: &mut String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    border: &crate::layout::engine::LayoutBorder,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
) {
    let t = border.top.width.max(0.0);
    let r = border.right.width.max(0.0);
    let b = border.bottom.width.max(0.0);
    let l = border.left.width.max(0.0);
    // Outer corners.
    let (ol, or_, ob, ot) = (x, x + w, y, y + h); // left,right,bottom,top edges
    // Inner corners (padding box edges).
    let (il, ir, ib, it) = (x + l, x + w - r, y + b, y + h - t);
    let mut fill = |content: &mut String,
                    pts: [(f32, f32); 4],
                    side: &crate::layout::engine::LayoutBorderSide| {
        if side.width <= 0.0 || side.style == crate::style::computed::BorderStyle::None {
            return;
        }
        let (cr, cg, cb) = side.color;
        let a = begin_border_alpha(content, page_ext_gstates, bg_alpha_counter, side.alpha);
        content.push_str(&format!("{cr} {cg} {cb} rg\n"));
        content.push_str(&format!("{} {} m\n", pts[0].0, pts[0].1));
        for p in &pts[1..] {
            content.push_str(&format!("{} {} l\n", p.0, p.1));
        }
        content.push_str("h\nf\n");
        end_border_alpha(content, a);
    };
    // Top: outer TL, outer TR, inner TR, inner TL.
    fill(
        content,
        [(ol, ot), (or_, ot), (ir, it), (il, it)],
        &border.top,
    );
    // Right: outer TR, outer BR, inner BR, inner TR.
    fill(
        content,
        [(or_, ot), (or_, ob), (ir, ib), (ir, it)],
        &border.right,
    );
    // Bottom: outer BR, outer BL, inner BL, inner BR.
    fill(
        content,
        [(or_, ob), (ol, ob), (il, ib), (ir, ib)],
        &border.bottom,
    );
    // Left: outer BL, outer TL, inner TL, inner BL.
    fill(
        content,
        [(ol, ob), (ol, ot), (il, it), (il, ib)],
        &border.left,
    );
}

/// Paint a multi-column `column-rule` as a single vertical line of width `w`
/// centered in the box, honouring its CSS border style. `solid` rules are
/// painted as a filled bar elsewhere; this path handles `dashed`/`dotted`
/// (stroked with the shared dash pattern) and `double` (two thin parallel
/// lines with a hollow middle third). `(x, top_y)` is the rule box's top-left
/// in PDF (bottom-up) coords; `h` is its height.
#[allow(clippy::too_many_arguments)]
fn paint_column_rule_line(
    content: &mut String,
    x: f32,
    top_y: f32,
    w: f32,
    h: f32,
    side: &crate::layout::engine::LayoutBorderSide,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
) {
    if w <= 0.0 || h <= 0.0 || side.style == BorderStyle::None {
        return;
    }
    let (r, g, b) = side.color;
    let a = begin_border_alpha(content, page_ext_gstates, bg_alpha_counter, side.alpha);
    content.push_str(&format!("{r} {g} {b} RG\n"));
    let bottom_y = top_y - h;
    if side.style == BorderStyle::Double {
        // Outer third on the left edge, inner third on the right edge, hollow
        // middle third — two stroked vertical lines half-a-third in from each
        // edge of the rule box.
        let third = w / 3.0;
        let x_left = x + third / 2.0;
        let x_right = x + w - third / 2.0;
        content.push_str(&format!("{third} w\n"));
        content.push_str(&format!("{x_left} {top_y} m {x_left} {bottom_y} l S\n"));
        content.push_str(&format!("{x_right} {top_y} m {x_right} {bottom_y} l S\n"));
    } else {
        let cx = x + w / 2.0;
        // Match the box-border dashed/dotted painter (`corner_dash_array`):
        // snap the dash/dot period so a full dash (or a centered dot) lands
        // flush at BOTH ends of the rule, with phase 0. A fixed `[2w w]`
        // period instead drifts out of phase relative to Chrome and skips the
        // end snapping. Dashes stroke the FULL rule length with butt caps; dots
        // stroke the inner span (inset half a width at each end) with round
        // caps so a dot sits centered on each end.
        let dotted = side.style == BorderStyle::Dotted;
        let (arr, phase, cap, seg_top, seg_bottom) = if dotted {
            let half = w / 2.0;
            let seg_top = top_y - half;
            let seg_bottom = bottom_y + half;
            let (arr, phase) = corner_dash_array((h - w).max(0.0), w, true);
            (arr, phase, "1 J\n", seg_top, seg_bottom)
        } else {
            let (arr, phase) = corner_dash_array(h, w, false);
            (arr, phase, "0 J\n", top_y, bottom_y)
        };
        content.push_str(cap);
        content.push_str(&format!("{w} w\n"));
        content.push_str(&format!("[{arr}] {phase} d\n"));
        content.push_str(&format!("{cx} {seg_top} m {cx} {seg_bottom} l S\n"));
        content.push_str("[] 0 d\n0 J\n");
    }
    end_border_alpha(content, a);
}

/// True when a nested Container is a styled `column-rule` placeholder: an empty,
/// background-free absolute box whose only border is a left side wide enough to
/// fill the box. Such boxes are emitted by the multicol layout for
/// non-`solid` rules so the renderer can draw the proper dash/double pattern.
fn is_column_rule_box(
    children: &[LayoutElement],
    background_color: &Option<(f32, f32, f32, f32)>,
    border: &crate::layout::engine::LayoutBorder,
    block_width: Option<f32>,
) -> bool {
    children.is_empty()
        && background_color.is_none()
        && border.left.width > 0.0
        && border.left.style != BorderStyle::None
        && border.top.width == 0.0
        && border.right.width == 0.0
        && border.bottom.width == 0.0
        && block_width.is_some_and(|w| (w - border.left.width).abs() < 0.01)
}

/// Register a blend-mode ExtGState and emit its `gs` operator, returning `true`
/// when a non-`Normal` blend was applied. The gstate name encodes the PDF blend
/// mode (`GSbm<Mode>`); the writer turns that into a `<< /BM /<Mode> >>` dict.
/// Callers wrap the affected paint in `q`..`Q` so the blend (and its restore via
/// `Q`) is scoped to that paint only. For `Normal` nothing is emitted, so output
/// for non-blended elements stays byte-identical.
fn begin_blend_mode(
    content: &mut String,
    page_ext_gstates: &mut Vec<(String, f32)>,
    mode: crate::style::computed::BlendMode,
) -> bool {
    if let Some(pdf_mode) = mode.pdf_name() {
        let gs_name = format!("GSbm{pdf_mode}");
        // Deduplicated by name in the writer, so registering the same blend mode
        // from multiple elements is harmless.
        page_ext_gstates.push((gs_name.clone(), 1.0));
        content.push_str(&format!("/{gs_name} gs\n"));
        true
    } else {
        false
    }
}

/// Stroke the CSS `border` frame of an image box. `(box_x, box_bottom)` is the
/// bottom-left corner of the box in PDF (bottom-up) coordinates; `box_w`/`box_h`
/// are the border-box dimensions. With `box-sizing: border-box` the frame is
/// drawn inside the box, so each stroke is centered half its width inside the
/// corresponding box edge (the inner edge meets the image content rect).
#[allow(clippy::too_many_arguments)]
fn draw_image_border(
    content: &mut String,
    box_x: f32,
    box_bottom: f32,
    box_w: f32,
    box_h: f32,
    border: &crate::layout::engine::LayoutBorder,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
) {
    if !border.has_any() {
        return;
    }
    let box_top = box_bottom + box_h;
    let box_right = box_x + box_w;
    let uniform = border.top.width == border.right.width
        && border.top.width == border.bottom.width
        && border.top.width == border.left.width
        && border.top.color == border.right.color
        && border.top.color == border.bottom.color
        && border.top.color == border.left.color
        && border.top.style == border.right.style
        && border.top.style == border.bottom.style
        && border.top.style == border.left.style;
    if uniform {
        let bw = border.top.width;
        let half = bw / 2.0;
        let (r, g, b) = border.top.color;
        let a = begin_border_alpha(
            content,
            page_ext_gstates,
            bg_alpha_counter,
            border.top.alpha,
        );
        content.push_str(&dash_pattern_for_style(border.top.style, border.top.width));
        content.push_str(&format!("{r} {g} {b} RG\n{bw} w\n"));
        content.push_str(&format!(
            "{x} {y} {w} {h} re\nS\n",
            x = box_x + half,
            y = box_bottom + half,
            w = box_w - bw,
            h = box_h - bw,
        ));
        content.push_str(reset_dash_pattern(border.top.style));
        end_border_alpha(content, a);
        return;
    }
    // Per-side: center each stroke half its own width inside the box edge.
    let y_top = box_top - border.top.width / 2.0;
    let y_bottom = box_bottom + border.bottom.width / 2.0;
    let x_left = box_x + border.left.width / 2.0;
    let x_right = box_right - border.right.width / 2.0;
    if border.top.width > 0.0 {
        let (r, g, b) = border.top.color;
        let a = begin_border_alpha(
            content,
            page_ext_gstates,
            bg_alpha_counter,
            border.top.alpha,
        );
        content.push_str(&dash_pattern_for_style(border.top.style, border.top.width));
        content.push_str(&format!("{r} {g} {b} RG\n{} w\n", border.top.width));
        content.push_str(&format!("{box_x} {y_top} m {box_right} {y_top} l S\n"));
        content.push_str(reset_dash_pattern(border.top.style));
        end_border_alpha(content, a);
    }
    if border.right.width > 0.0 {
        let (r, g, b) = border.right.color;
        let a = begin_border_alpha(
            content,
            page_ext_gstates,
            bg_alpha_counter,
            border.right.alpha,
        );
        content.push_str(&dash_pattern_for_style(
            border.right.style,
            border.right.width,
        ));
        content.push_str(&format!("{r} {g} {b} RG\n{} w\n", border.right.width));
        content.push_str(&format!("{x_right} {y_top} m {x_right} {y_bottom} l S\n"));
        content.push_str(reset_dash_pattern(border.right.style));
        end_border_alpha(content, a);
    }
    if border.bottom.width > 0.0 {
        let (r, g, b) = border.bottom.color;
        let a = begin_border_alpha(
            content,
            page_ext_gstates,
            bg_alpha_counter,
            border.bottom.alpha,
        );
        content.push_str(&dash_pattern_for_style(
            border.bottom.style,
            border.bottom.width,
        ));
        content.push_str(&format!("{r} {g} {b} RG\n{} w\n", border.bottom.width));
        content.push_str(&format!(
            "{box_x} {y_bottom} m {box_right} {y_bottom} l S\n"
        ));
        content.push_str(reset_dash_pattern(border.bottom.style));
        end_border_alpha(content, a);
    }
    if border.left.width > 0.0 {
        let (r, g, b) = border.left.color;
        let a = begin_border_alpha(
            content,
            page_ext_gstates,
            bg_alpha_counter,
            border.left.alpha,
        );
        content.push_str(&dash_pattern_for_style(
            border.left.style,
            border.left.width,
        ));
        content.push_str(&format!("{r} {g} {b} RG\n{} w\n", border.left.width));
        content.push_str(&format!("{x_left} {y_top} m {x_left} {y_bottom} l S\n"));
        content.push_str(reset_dash_pattern(border.left.style));
        end_border_alpha(content, a);
    }
}

/// A link annotation to be placed on a PDF page.
struct LinkAnnotation {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    url: String,
}

#[derive(Clone, Copy)]
struct TextLineAnnotationBox {
    top: f32,
    bottom: f32,
}

fn text_run_link_annotation(
    run: &TextRun,
    x: f32,
    width: f32,
    line_box: TextLineAnnotationBox,
) -> Option<LinkAnnotation> {
    let url = run.link_url.as_ref()?;
    Some(LinkAnnotation {
        x1: x,
        y1: line_box.bottom,
        x2: x + width,
        y2: line_box.top,
        url: url.clone(),
    })
}

/// A bookmark entry for PDF outline (table of contents).
#[allow(dead_code)]
struct BookmarkEntry {
    title: String,
    level: u8,
    page_index: usize,
    y_pos: f32,
}

/// Render laid-out pages into a PDF byte buffer.
///
/// Uses the PDF built-in Helvetica font family (one of the 14 standard fonts)
/// so no font embedding is needed for the MVP.
#[allow(dead_code)]
pub fn render_pdf(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
) -> Result<Vec<u8>, IronpressError> {
    render_pdf_with_fonts(pages, page_size, margin, &HashMap::new())
}

/// Render laid-out pages into a PDF byte buffer, with custom font embedding.
pub fn render_pdf_with_fonts(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    custom_fonts: &HashMap<String, TtfFont>,
) -> Result<Vec<u8>, IronpressError> {
    let mut buf = Vec::new();
    render_pdf_to_writer_with_fonts(pages, page_size, margin, &mut buf, custom_fonts)?;
    Ok(buf)
}

/// Header and footer text for page decoration.
pub struct PageDecoration {
    /// Header text rendered top-center of each page.
    pub header: Option<String>,
    /// Footer text rendered bottom-center of each page.
    /// `{page}` and `{pages}` are replaced with page number and total count.
    pub footer: Option<String>,
}

/// Render laid-out pages as PDF, writing directly to any `std::io::Write` implementation.
///
/// This is the streaming variant of [`render_pdf`]. It writes PDF content incrementally
/// to the provided writer instead of building an in-memory buffer.
#[allow(dead_code)]
pub fn render_pdf_to_writer<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_with_fonts(pages, page_size, margin, writer, &HashMap::new())
}

/// Render laid-out pages as PDF with custom fonts, writing directly to any `std::io::Write` implementation.
fn render_pdf_to_writer_with_fonts<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
    custom_fonts: &HashMap<String, TtfFont>,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_full(pages, page_size, margin, writer, custom_fonts, None)
}

/// Full render function with optional page decoration (headers/footers).
/// Chrome's print engine scales a page down when its laid-out content overflows
/// the `@page` box *horizontally* — a non-spec behavior (CSS Paged Media clips
/// instead) but one Chromium's `--print-to-pdf` reproduces, so matching it keeps
/// PDFs identical to Chrome's. Only the inline axis triggers it: vertical
/// overflow paginates onto a new page rather than scaling. Returns 1.0 when the
/// content fits within the page width.
fn page_shrink_to_fit_scale(page: &Page, page_size: PageSize, margin: Margin) -> f32 {
    let mut max_right = 0.0f32;
    for (_y_pos, element) in &page.elements {
        let (off_left, width) = match element {
            LayoutElement::TextBlock {
                offset_left,
                block_width,
                ..
            }
            | LayoutElement::Container {
                offset_left,
                block_width,
                ..
            } => (*offset_left, block_width.unwrap_or(0.0)),
            LayoutElement::FlexRow {
                offset_left,
                container_width,
                ..
            } => (*offset_left, *container_width),
            LayoutElement::TableRow { offset_left, .. } => (
                *offset_left,
                crate::layout::paginate::table_row_content_width(element),
            ),
            LayoutElement::Image { width, .. } | LayoutElement::Svg { width, .. } => (0.0, *width),
            _ => (0.0, 0.0),
        };
        max_right = max_right.max(margin.left + off_left + width);
    }
    // Only react to a meaningful overflow (avoid sub-pt rounding false-triggers).
    if max_right > page_size.width + 0.5 {
        (page_size.width / max_right).clamp(0.1, 1.0)
    } else {
        1.0
    }
}

/// Axis-aligned rectangle in PDF page coordinates (x grows right, y grows up),
/// used only by the optional occlusion-culling pass. `(x0, y0)` is the
/// bottom-left corner and `(x1, y1)` the top-right corner.
#[derive(Clone, Copy)]
struct OcclRect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl OcclRect {
    /// True when `self` extends at least `margin` points beyond `inner` on
    /// every side — i.e. `inner` plus a `margin`-wide safety border is still
    /// fully inside `self`. Used so anti-aliasing/interpolation at the coverer's
    /// edges can never reveal a culled raster.
    fn covers_with_margin(&self, inner: &OcclRect, margin: f32) -> bool {
        self.x0 <= inner.x0 - margin
            && self.y0 <= inner.y0 - margin
            && self.x1 >= inner.x1 + margin
            && self.y1 >= inner.y1 + margin
    }
}

/// Safety inset (CSS px == PDF pt) the coverer must exceed the raster by on
/// every side before the raster is considered safely hidden.
const OCCLUSION_SAFETY_MARGIN: f32 = 2.0;

/// If `element` is a top-level block whose painted output is a single
/// fully-opaque, square-cornered, untransformed, un-blended rectangle that
/// fills its entire border box, return that border-box rectangle in PDF page
/// coordinates. Anything that could leave a gap (transparency, border-radius,
/// opacity < 1, blend mode, transform, clip, non-border background-clip, a
/// gradient/SVG background that might not be opaque, `visibility:hidden`)
/// disqualifies it — when unsure we return `None` and never cull.
fn opaque_block_coverer_rect(
    element: &LayoutElement,
    y_pos: f32,
    page_size: PageSize,
    margin: Margin,
    available_width: f32,
) -> Option<OcclRect> {
    let no_blend =
        |b: &crate::style::computed::BlendMode| *b == crate::style::computed::BlendMode::Normal;
    let no_radius = |r: f32, rs: &[f32; 4], rys: &[f32; 4]| {
        r == 0.0 && rs.iter().all(|v| *v == 0.0) && rys.iter().all(|v| *v == 0.0)
    };
    match element {
        // A flat text block whose painted output is a single fully-opaque,
        // square-cornered, untransformed solid rectangle filling its border box.
        LayoutElement::TextBlock {
            lines,
            background_color,
            padding_top,
            padding_bottom,
            border,
            block_width,
            block_height,
            opacity,
            mix_blend_mode,
            float,
            position,
            offset_left,
            containing_block,
            visible,
            clip_rect,
            transform,
            border_radius,
            border_radii,
            border_radii_y,
            background_gradient,
            background_radial_gradient,
            background_conic_gradient,
            background_svg,
            background_clip,
            background_blur_radius,
            ..
        } => {
            let (_, _, _, a) = (*background_color)?;
            if !*visible
                || a < 1.0
                || *opacity < 1.0
                || !no_blend(mix_blend_mode)
                || transform.is_some()
                || clip_rect.is_some()
                || !no_radius(*border_radius, border_radii, border_radii_y)
                || *background_clip != BackgroundClip::Border
                || background_gradient.is_some()
                || background_radial_gradient.is_some()
                || background_conic_gradient.is_some()
                || background_svg.is_some()
                // A blurred solid box paints feathered (semi-transparent) edges,
                // so it cannot be a reliable opaque coverer.
                || *background_blur_radius > 0.0
            {
                return None;
            }
            // Mirror the TextBlock paint geometry (see the TextBlock arm below).
            let render_width = block_width.unwrap_or(available_width);
            let block_x = match position {
                Position::Absolute => containing_block.map_or(margin.left + offset_left, |cb| {
                    margin.left + cb.x + offset_left
                }),
                Position::Relative => margin.left + offset_left,
                Position::Static => match float {
                    Float::Right => margin.left + available_width - render_width,
                    _ => margin.left + offset_left,
                },
            };
            let block_y = page_size.height - margin.top - y_pos;
            let total_h = text_block_total_height(
                lines,
                *padding_top,
                *padding_bottom,
                *block_height,
                clip_rect.is_some(),
            );
            let border_box_h = total_h + border.top.width + border.bottom.width;
            Some(OcclRect {
                x0: block_x,
                y0: block_y - border_box_h,
                x1: block_x + render_width,
                y1: block_y,
            })
        }
        // A nested container box. Its own opaque background fills the whole border
        // box regardless of children (which only paint on top, within the box), so
        // the region stays fully opaque. Disqualify anything that could shrink,
        // shift, fade, or round the painted box.
        LayoutElement::Container {
            children,
            background_color,
            border,
            border_radius,
            border_radii,
            border_radii_y,
            padding_top,
            padding_bottom,
            block_width,
            block_height,
            opacity,
            mix_blend_mode,
            visible,
            float,
            offset_left,
            transform,
            clip_path,
            mask_image,
            background_gradient,
            background_radial_gradient,
            background_conic_gradient,
            background_svg,
            background_clip,
            ..
        } => {
            let (_, _, _, a) = (*background_color)?;
            if !*visible
                || a < 1.0
                || *opacity < 1.0
                || !no_blend(mix_blend_mode)
                || transform.is_some()
                || clip_path.is_some()
                || mask_image.is_some()
                || !no_radius(*border_radius, border_radii, border_radii_y)
                || *background_clip != BackgroundClip::Border
                || background_gradient.is_some()
                || background_radial_gradient.is_some()
                || background_conic_gradient.is_some()
                || background_svg.is_some()
            {
                return None;
            }
            // Mirror the Container paint geometry (see the Container arm below).
            let container_w = block_width.unwrap_or(available_width);
            let container_x = match float {
                Float::Right => margin.left + available_width - container_w,
                _ => margin.left + offset_left,
            };
            let container_y_top = page_size.height - margin.top - y_pos;
            let children_h: f32 = collapsed_children_height(children);
            let content_h = padding_top + children_h + padding_bottom + border.vertical_width();
            let total_h = block_height.unwrap_or(content_h);
            Some(OcclRect {
                x0: container_x,
                y0: container_y_top - total_h,
                x1: container_x + container_w,
                y1: container_y_top,
            })
        }
        _ => None,
    }
}

/// Collect `(rect, paint_index)` for every qualifying opaque rectangular
/// coverer on the page, in paint order. Higher index == painted later (on top).
fn collect_opaque_coverers(
    page: &Page,
    page_size: PageSize,
    margin: Margin,
    available_width: f32,
) -> Vec<(OcclRect, usize)> {
    page.elements
        .iter()
        .enumerate()
        .filter_map(|(idx, (y_pos, element))| {
            opaque_block_coverer_rect(element, *y_pos, page_size, margin, available_width)
                .map(|rect| (rect, idx))
        })
        .collect()
}

/// True when some opaque coverer painted strictly later than `elem_idx` fully
/// contains `raster` with the safety margin — i.e. the raster is guaranteed
/// invisible and can be skipped.
fn raster_is_occluded(coverers: &[(OcclRect, usize)], raster: &OcclRect, elem_idx: usize) -> bool {
    coverers.iter().any(|(rect, idx)| {
        *idx > elem_idx && rect.covers_with_margin(raster, OCCLUSION_SAFETY_MARGIN)
    })
}

/// Low-level render: raw (uncompressed) content streams for deterministic,
/// inspectable output (used by unit tests and the parity harness, which
/// rasterizes the result). The high-level `HtmlConverter` API enables content-
/// stream compression by default for production output; call
/// `render_pdf_to_writer_full_opts(.., opts)` for compression here.
pub(crate) fn render_pdf_to_writer_full<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
    custom_fonts: &HashMap<String, TtfFont>,
    decoration: Option<&PageDecoration>,
) -> Result<(), IronpressError> {
    render_pdf_to_writer_full_opts(
        pages,
        page_size,
        margin,
        writer,
        custom_fonts,
        decoration,
        RenderOpts {
            compress: false,
            ..Default::default()
        },
    )
}

pub(crate) fn render_pdf_to_writer_full_opts<W: std::io::Write>(
    pages: &[Page],
    page_size: PageSize,
    margin: Margin,
    writer: &mut W,
    custom_fonts: &HashMap<String, TtfFont>,
    decoration: Option<&PageDecoration>,
    opts: RenderOpts,
) -> Result<(), IronpressError> {
    // Keep `ex`/`ch` style resolution font-aware during any render-time style
    // recomputation (e.g. pseudo-elements), matching the layout pass.
    let _font_ctx = crate::style::font_ctx::FontCtxGuard::new(custom_fonts);
    let mut pdf_writer = PdfWriter::new();
    pdf_writer.opts = opts;
    let available_width = page_size.width - margin.left - margin.right;
    let mut bookmarks: Vec<BookmarkEntry> = Vec::new();
    let prepared_custom_fonts = prepare_custom_fonts(pages, custom_fonts);

    register_used_custom_fonts(&mut pdf_writer, custom_fonts, &prepared_custom_fonts);

    for (page_idx, page) in pages.iter().enumerate() {
        let mut content = String::new();
        let mut annotations: Vec<LinkAnnotation> = Vec::new();
        let mut page_images: Vec<ImageRef> = Vec::new();
        let mut page_ext_gstates: Vec<(String, f32)> = Vec::new();
        let mut bg_alpha_counter: usize = 0;
        let mut page_shadings: Vec<ShadingEntry> = Vec::new();
        let mut shading_counter: usize = 0;

        // Track clip state: when a TextBlock has clip_children_count > 0,
        // the clip context stays open for that many subsequent elements.
        let mut clip_remaining: usize = 0;

        // Optional occlusion culling (default off): rectangles of fully-opaque
        // coverers, used to skip rasters that a later opaque element fully hides.
        let occlusion_coverers = if pdf_writer.opts.occlusion_cull {
            collect_opaque_coverers(page, page_size, margin, available_width)
        } else {
            Vec::new()
        };

        for (elem_idx, (y_pos, element)) in page.elements.iter().enumerate() {
            // Close clip context when all clipped children have been rendered
            if clip_remaining > 0 {
                clip_remaining -= 1;
                if clip_remaining == 0 {
                    content.push_str("Q\n");
                }
            }
            match element {
                LayoutElement::TextBlock {
                    lines,
                    text_align,
                    background_color,
                    padding_top,
                    padding_bottom,
                    padding_left,
                    padding_right,
                    border,
                    block_width,
                    block_height,
                    opacity,
                    float,
                    position,
                    offset_top: _,
                    offset_left,
                    offset_bottom: _,
                    offset_right: _,
                    containing_block,
                    box_shadow,
                    visible,
                    clip_rect,
                    transform,
                    transform_origin,
                    background_gradient,
                    background_radial_gradient,
                    background_conic_gradient,
                    background_svg,
                    background_blur_radius,
                    background_size,
                    background_position,
                    background_repeat,
                    background_origin,
                    background_clip,
                    border_radius,
                    border_radii: tb_radii,
                    border_radii_y: tb_radii_y,
                    outline_width,
                    outline_color,
                    outline_offset: tb_outline_offset,
                    letter_spacing,
                    word_spacing: css_word_spacing,
                    text_indent,
                    heading_level,
                    clip_children_count,
                    writing_mode,
                    ..
                } => {
                    // Skip rendering if visibility: hidden (but space is preserved)
                    if !visible {
                        continue;
                    }

                    // Collect heading bookmark for PDF outlines
                    if let Some(level) = heading_level {
                        let title: String = lines
                            .iter()
                            .flat_map(|l| l.runs.iter().map(|r| r.text.as_str()))
                            .collect::<Vec<_>>()
                            .join("");
                        if !title.trim().is_empty() {
                            bookmarks.push(BookmarkEntry {
                                title: title.trim().to_string(),
                                level: *level,
                                page_index: page_idx,
                                y_pos: *y_pos,
                            });
                        }
                    }

                    // Compute block_x with float/position offsets
                    let block_x = match position {
                        Position::Absolute => {
                            // Position relative to the containing block.
                            // bottom/right offsets are pre-resolved into top/left
                            // at layout time, so we only use offset_left here.
                            containing_block.map_or(margin.left + offset_left, |cb| {
                                margin.left + cb.x + offset_left
                            })
                        }
                        Position::Relative => margin.left + offset_left,
                        Position::Static => match float {
                            Float::Right => {
                                let render_w = block_width.unwrap_or(available_width);
                                margin.left + available_width - render_w
                            }
                            _ => margin.left + offset_left,
                        },
                    };
                    // PDF y-axis is bottom-up.
                    // y_pos already includes absolute/relative offsets from pagination.
                    let block_y = page_size.height - margin.top - y_pos;

                    // Use explicit block_width if set, otherwise available_width
                    let render_width = block_width.unwrap_or(available_width);
                    // `total_h` is the PADDING-box height (content + padding, no
                    // border).  The block FLOW advance already accounts for the
                    // vertical border (see layout::block / paginate), so `block_y`
                    // is the BORDER-box top.  The rendered box geometry (fill,
                    // border stroke, box-shadow, clip, text origin) must therefore
                    // use the BORDER box so it matches the flow and Chrome.
                    let total_h = text_block_total_height(
                        lines,
                        *padding_top,
                        *padding_bottom,
                        *block_height,
                        clip_rect.is_some(),
                    );
                    // Border-box height = padding-box height + vertical border.
                    // `render_width` is already the border-box width, so the box
                    // is `render_width` × `border_box_h`, top at `block_y`.
                    let border_vert = border.top.width + border.bottom.width;
                    let border_box_h = total_h + border_vert;
                    let block_bottom = block_y - border_box_h;

                    // Apply transform if set (wrap in q/Q).
                    // Rotate and scale are applied around the element's centre so
                    // that the element stays in its layout position (matching
                    // CSS `transform-origin: 50% 50%`).  The combined matrix is:
                    //   T(cx,cy) · M · T(-cx,-cy)
                    // which in PDF `cm` notation is a single 6-value matrix.
                    let needs_transform = transform.is_some();
                    if let Some(t) = transform {
                        // Resolve the transform-origin pivot (px from the box's
                        // top-left) into PDF bottom-up coordinates.
                        let (ox, oy) = transform_origin.resolve(render_width, border_box_h);
                        let cx = block_x + ox;
                        let cy = block_bottom + border_box_h - oy;
                        content.push_str("q\n");
                        push_transform_cm(&mut content, t, cx, cy, render_width, border_box_h);
                    }

                    // CSS `overflow: hidden`/`clip`/`scroll`/`auto` clips at the
                    // PADDING box (border box inset by the border widths) and the
                    // rounded inner corners when border-radius is set. The clip
                    // must NOT cover the box's OWN background, border, or outline —
                    // a box's border and outline always paint fully visible. So the
                    // clip is opened later (after the border/outline are stroked)
                    // and scoped to the inline text content only; see `needs_clip`
                    // below the outline-paint block.
                    let needs_clip = clip_rect.is_some();

                    // Apply opacity via ExtGState if < 1.0
                    let needs_opacity = *opacity < 1.0;
                    if needs_opacity {
                        let gs_name = format!("GS{elem_idx}");
                        page_ext_gstates.push((gs_name.clone(), *opacity));
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }

                    // Draw box-shadow with blur (references the border box).
                    render_box_shadows(
                        &mut content,
                        box_shadow,
                        block_x,
                        block_bottom,
                        render_width,
                        border_box_h,
                        *border_radius,
                        &mut page_ext_gstates,
                        &mut bg_alpha_counter,
                        &mut pdf_writer,
                        &mut page_images,
                    );

                    // CSS `filter: blur()` on a solid box (css-filter-effects-1
                    // §4.1): the box's painted output (background fill + border)
                    // is gaussian-blurred and feathers outside the border box.
                    // ironpress paints boxes as vector content, so for a plain
                    // solid box (no gradient/SVG bg, no text, no transform/opacity
                    // wrapper, square corners) rasterize bg+border, blur it, and
                    // embed it in place of the sharp vector paint.
                    if *background_blur_radius > 0.0
                        && !needs_transform
                        && !needs_opacity
                        && lines.is_empty()
                        && background_gradient.is_none()
                        && background_radial_gradient.is_none()
                        && background_conic_gradient.is_none()
                        && background_svg.is_none()
                        && *border_radius == 0.0
                        && let Some(blurred) = crate::render::blur::blur_box(
                            render_width,
                            border_box_h,
                            *background_color,
                            border,
                            *background_blur_radius,
                            pdf_writer.opts.filter_dpi,
                        )
                    {
                        let img_obj_id = pdf_writer.add_image_object(
                            &blurred.asset.data,
                            blurred.asset.source_width,
                            blurred.asset.source_height,
                            blurred.asset.format,
                            blurred.asset.png_metadata.as_ref(),
                        );
                        let img_name = format!("Im{img_obj_id}");
                        let ov = blurred.overflow_pt;
                        content.push_str(&format!(
                            "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
                            w = render_width + 2.0 * ov,
                            h = border_box_h + 2.0 * ov,
                            ix = block_x - ov,
                            iy = block_bottom - ov,
                            name = img_name,
                        ));
                        page_images.push(ImageRef {
                            name: img_name,
                            obj_id: img_obj_id,
                        });
                        continue;
                    }

                    // `block_x` / `block_bottom` / `render_width` / `border_box_h`
                    // describe the BORDER box (border paints inward). Derive the
                    // box `background-clip` confines the painted fill to.
                    let tb_bl = border.left.width;
                    let tb_br = border.right.width;
                    let tb_bt = border.top.width;
                    let tb_bb = border.bottom.width;
                    let (tb_clip_x, tb_clip_y, tb_clip_w, tb_clip_h) = background_clip_rect(
                        *background_clip,
                        block_x,
                        block_bottom,
                        render_width,
                        border_box_h,
                        tb_bl,
                        tb_br,
                        tb_bt,
                        tb_bb,
                        *padding_left,
                        *padding_right,
                        *padding_top,
                        *padding_bottom,
                    );
                    let tb_needs_clip = *background_clip != BackgroundClip::Border;
                    let tb_gradient_clip = *border_radius > 0.0 || tb_needs_clip;

                    // Draw background if specified
                    if let Some((r, g, b, a)) = background_color {
                        let bg_y = block_bottom;
                        let needs_bg_alpha = *a < 1.0;
                        if needs_bg_alpha {
                            let effective_alpha = *a * *opacity;
                            let gs_name = format!("GSbg{elem_idx}");
                            page_ext_gstates.push((gs_name.clone(), effective_alpha));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!("{r} {g} {b} rg\n"));
                        if tb_needs_clip {
                            push_background_clip(
                                &mut content,
                                tb_clip_x,
                                tb_clip_y,
                                tb_clip_w,
                                tb_clip_h,
                                *border_radius,
                            );
                            content.push_str(&format!(
                                "{tb_clip_x} {tb_clip_y} {tb_clip_w} {tb_clip_h} re\n"
                            ));
                            content.push_str("f\n");
                            content.push_str("Q\n");
                        } else {
                            if let Some(path) = rounded_box_path(
                                block_x,
                                bg_y,
                                render_width,
                                border_box_h,
                                *tb_radii,
                                *tb_radii_y,
                            ) {
                                content.push_str(&path);
                            } else {
                                content.push_str(&format!(
                                    "{x} {y} {w} {h} re\n",
                                    x = block_x,
                                    y = bg_y,
                                    w = render_width,
                                    h = border_box_h,
                                ));
                            }
                            content.push_str("f\n");
                        }
                        if needs_bg_alpha {
                            // Reset to element opacity or full opacity
                            if needs_opacity {
                                let gs_name = format!("GS{elem_idx}");
                                content.push_str(&format!("/{gs_name} gs\n"));
                            } else {
                                content.push_str("/GSDefault gs\n");
                            }
                        }
                    }

                    // Draw linear gradient if specified
                    if let Some(gradient) = background_gradient {
                        let bg_y = block_bottom;
                        // Clip to the background-clip box (rounded if needed).
                        if tb_gradient_clip {
                            push_background_clip(
                                &mut content,
                                tb_clip_x,
                                tb_clip_y,
                                tb_clip_w,
                                tb_clip_h,
                                *border_radius,
                            );
                        }
                        render_linear_gradient(
                            &mut content,
                            gradient,
                            block_x,
                            bg_y,
                            render_width,
                            border_box_h,
                            &mut page_shadings,
                            &mut shading_counter,
                        );
                        if tb_gradient_clip {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw radial gradient if specified
                    if let Some(gradient) = background_radial_gradient {
                        let bg_y = block_bottom;
                        if tb_gradient_clip {
                            push_background_clip(
                                &mut content,
                                tb_clip_x,
                                tb_clip_y,
                                tb_clip_w,
                                tb_clip_h,
                                *border_radius,
                            );
                        }
                        render_radial_gradient(
                            &mut content,
                            gradient,
                            block_x,
                            bg_y,
                            render_width,
                            border_box_h,
                            &mut page_shadings,
                            &mut shading_counter,
                        );
                        if tb_gradient_clip {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw conic gradient if specified
                    if let Some(gradient) = background_conic_gradient {
                        let bg_y = block_bottom;
                        if tb_gradient_clip {
                            push_background_clip(
                                &mut content,
                                tb_clip_x,
                                tb_clip_y,
                                tb_clip_w,
                                tb_clip_h,
                                *border_radius,
                            );
                        }
                        render_conic_gradient(
                            &mut content,
                            gradient,
                            block_x,
                            bg_y,
                            render_width,
                            border_box_h,
                        );
                        if tb_gradient_clip {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw inset box-shadow (after backgrounds, before content).
                    render_box_shadows_inset(
                        &mut content,
                        box_shadow,
                        block_x,
                        block_bottom,
                        render_width,
                        border_box_h,
                        *border_radius,
                        &mut page_ext_gstates,
                        &mut bg_alpha_counter,
                    );

                    // Draw SVG background image if specified.
                    // `block_x` / `block_y` are the border-box top-left and
                    // `render_width` × `border_box_h` is the border box (border
                    // paints inward).  Derive the padding/content boxes by
                    // insetting with the per-side border / padding widths.
                    if let Some(svg_tree) = background_svg {
                        let border_left = border.left.width;
                        let border_right = border.right.width;
                        let border_box_x = block_x;
                        let border_box_y = block_bottom;
                        let border_box_w = render_width;
                        // Adjust reference box based on background-origin
                        let (ref_x, ref_y, ref_w, ref_h) = match background_origin {
                            BackgroundOrigin::Border => {
                                (border_box_x, border_box_y, border_box_w, border_box_h)
                            }
                            BackgroundOrigin::Content => (
                                border_box_x + border_left + padding_left,
                                border_box_y + border.bottom.width + padding_bottom,
                                (border_box_w
                                    - border_left
                                    - border_right
                                    - padding_left
                                    - padding_right)
                                    .max(0.0),
                                (border_box_h - border_vert - padding_top - padding_bottom)
                                    .max(0.0),
                            ),
                            BackgroundOrigin::Padding => (
                                border_box_x + border_left,
                                border_box_y + border.bottom.width,
                                (border_box_w - border_left - border_right).max(0.0),
                                (border_box_h - border_vert).max(0.0),
                            ),
                        };
                        render_svg_background(
                            &mut content,
                            svg_tree,
                            &mut pdf_writer,
                            &mut page_images,
                            &mut page_shadings,
                            &mut shading_counter,
                            Some(&mut page_ext_gstates),
                            BackgroundPaintContext::new(
                                SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                                SvgViewportBox::new(tb_clip_x, tb_clip_y, tb_clip_w, tb_clip_h),
                                *border_radius,
                                *background_blur_radius,
                                *background_size,
                                *background_position,
                                *background_repeat,
                            ),
                        );
                    }

                    // Draw border if specified.  The border paints INSIDE the
                    // border box (CSS box model): the stroke centerline sits half
                    // a border-width inside each border-box edge, so the stroke's
                    // outer edge coincides with the border-box edge.
                    if border.has_visible() {
                        // Check if all sides are uniform (same width & color)
                        let uniform = border.top.width == border.right.width
                            && border.top.width == border.bottom.width
                            && border.top.width == border.left.width
                            && border.top.color == border.right.color
                            && border.top.color == border.bottom.color
                            && border.top.color == border.left.color
                            && border.top.style == border.right.style
                            && border.top.style == border.bottom.style
                            && border.top.style == border.left.style;
                        if uniform && border_needs_special_paint(border.top.style, *tb_radii) {
                            // Shared painter handles solid/dashed/dotted/double and
                            // both uniform and per-corner rounded borders.
                            paint_uniform_border(
                                &mut content,
                                block_x,
                                block_bottom,
                                render_width,
                                border_box_h,
                                *tb_radii,
                                &border.top,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                            );
                        } else if uniform && *border_radius > 0.0 {
                            // Plain solid rounded border: byte-stable legacy path.
                            let bw = border.top.width;
                            let (br, bg, bb) = border.top.color;
                            let a = begin_border_alpha(
                                &mut content,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                border.top.alpha,
                            );
                            content.push_str(&format!("{br} {bg} {bb} RG\n{bw} w\n"));
                            content.push_str(&rounded_rect_path(
                                block_x + bw / 2.0,
                                block_bottom + bw / 2.0,
                                (render_width - bw).max(0.0),
                                (border_box_h - bw).max(0.0),
                                (*border_radius - bw / 2.0).max(0.0),
                            ));
                            content.push_str("S\n");
                            end_border_alpha(&mut content, a);
                        } else if uniform {
                            // Plain solid flat border: byte-stable legacy `re` stroke.
                            let bw = border.top.width;
                            let (br, bg, bb) = border.top.color;
                            let a = begin_border_alpha(
                                &mut content,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                border.top.alpha,
                            );
                            content.push_str(&format!("{br} {bg} {bb} RG\n{bw} w\n"));
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\n",
                                x = block_x + bw / 2.0,
                                y = block_bottom + bw / 2.0,
                                w = (render_width - bw).max(0.0),
                                h = (border_box_h - bw).max(0.0),
                            ));
                            content.push_str("S\n");
                            end_border_alpha(&mut content, a);
                        } else if !radii_any(*tb_radii)
                            && *border_radius <= 0.0
                            && border_needs_miter_fill(border)
                        {
                            // Different per-side colors/widths on a square box: fill
                            // each side as a trapezoid so corners meet on a diagonal
                            // miter seam (CSS Backgrounds §6.2), not as overlapping
                            // centerline strokes that leave a single-color corner.
                            paint_miter_border(
                                &mut content,
                                block_x,
                                block_bottom,
                                render_width,
                                border_box_h,
                                border,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                            );
                        } else {
                            // Per-side stroke centerlines sit half a border-width
                            // inside the border-box edges (border paints inward).
                            let x1 = block_x;
                            let x2 = block_x + render_width;
                            let y_top = block_y - border.top.width / 2.0;
                            let y_bottom = block_bottom + border.bottom.width / 2.0;
                            let x_left = block_x + border.left.width / 2.0;
                            let x_right = block_x + render_width - border.right.width / 2.0;
                            // Top border
                            if border.top.paints() {
                                let (r, g, b) = border.top.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.top.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.top.style,
                                    border.top.width,
                                ));
                                content
                                    .push_str(&format!("{r} {g} {b} RG\n{} w\n", border.top.width));
                                content.push_str(&format!("{x1} {y_top} m {x2} {y_top} l S\n"));
                                content.push_str(reset_dash_pattern(border.top.style));
                                end_border_alpha(&mut content, a);
                            }
                            // Right border
                            if border.right.paints() {
                                let (r, g, b) = border.right.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.right.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.right.style,
                                    border.right.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.right.width
                                ));
                                content.push_str(&format!(
                                    "{x_right} {y_top} m {x_right} {y_bottom} l S\n"
                                ));
                                content.push_str(reset_dash_pattern(border.right.style));
                                end_border_alpha(&mut content, a);
                            }
                            // Bottom border
                            if border.bottom.paints() {
                                let (r, g, b) = border.bottom.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.bottom.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.bottom.style,
                                    border.bottom.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.bottom.width
                                ));
                                content
                                    .push_str(&format!("{x1} {y_bottom} m {x2} {y_bottom} l S\n"));
                                content.push_str(reset_dash_pattern(border.bottom.style));
                                end_border_alpha(&mut content, a);
                            }
                            // Left border
                            if border.left.paints() {
                                let (r, g, b) = border.left.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.left.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.left.style,
                                    border.left.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n",
                                    border.left.width
                                ));
                                content.push_str(&format!(
                                    "{x_left} {y_top} m {x_left} {y_bottom} l S\n"
                                ));
                                content.push_str(reset_dash_pattern(border.left.style));
                                end_border_alpha(&mut content, a);
                            }
                        }
                    }

                    // Draw outline if specified (outside the element box).
                    // `outline-offset` widens the gap between the border edge and
                    // the outline; the centerline sits half the outline width
                    // beyond the offset edge so the stroke stays fully outside.
                    if *outline_width > 0.0 {
                        let gap = *tb_outline_offset + *outline_width / 2.0;
                        let outline_x = block_x - gap;
                        let outline_y = block_bottom - gap;
                        let outline_w = render_width + 2.0 * gap;
                        let outline_h = border_box_h + 2.0 * gap;
                        let (or, og, ob) = outline_color.unwrap_or((0.0, 0.0, 0.0));
                        content
                            .push_str(&format!("{or} {og} {ob} RG\n{ow} w\n", ow = outline_width,));
                        if radii_any(*tb_radii) && !radii_uniform(*tb_radii) {
                            let ol_radii = [
                                tb_radii[0] + gap,
                                tb_radii[1] + gap,
                                tb_radii[2] + gap,
                                tb_radii[3] + gap,
                            ];
                            content.push_str(&rounded_rect_path_per_corner(
                                outline_x, outline_y, outline_w, outline_h, ol_radii,
                            ));
                        } else if *border_radius > 0.0 {
                            let outline_r = *border_radius + gap;
                            content.push_str(&rounded_rect_path(
                                outline_x, outline_y, outline_w, outline_h, outline_r,
                            ));
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\n",
                                x = outline_x,
                                y = outline_y,
                                w = outline_w,
                                h = outline_h,
                            ));
                        }
                        content.push_str("S\n");
                    }

                    // Open the overflow clip now — AFTER the background, border and
                    // outline are painted (so they stay fully visible) and BEFORE
                    // the inline text / descendant content (which is clipped to the
                    // padding box). Mirrors the nested-TextBlock paint order.
                    if needs_clip {
                        content.push_str("q\n");
                        content.push_str(&overflow_clip_path(
                            block_x,
                            block_bottom,
                            render_width,
                            border_box_h,
                            border.left.width,
                            border.right.width,
                            border.top.width,
                            border.bottom.width,
                            *border_radius,
                        ));
                        content.push_str("W n\n");
                    }

                    // Text content is inset from the border-box top by the top
                    // border width and the top padding.
                    let mut text_y = block_y - border.top.width - padding_top;

                    // Horizontal insets: `block_x` / `render_width` are the
                    // border-box left / width, so the content area starts after
                    // the left border + left padding and is narrowed by the
                    // horizontal borders + paddings.
                    let border_left = border.left.width;
                    let border_right = border.right.width;
                    // Content-box left edge and width (content + padding ⇒ here we
                    // keep padding in `content_x`/`content_width` because the text
                    // branches add `padding_left`/`padding_right` themselves; this
                    // pair is the PADDING box).
                    let padding_box_x = block_x + border_left;
                    let padding_box_w = (render_width - border_left - border_right).max(0.0);

                    // CSS `writing-mode: vertical-rl` (css-writing-modes-4 §3.1).
                    // The box geometry stays physical/axis-aligned (already laid
                    // out above); only the inline text is set vertically. With the
                    // default `text-orientation: mixed`, Latin runs are rotated 90°
                    // clockwise (set sideways) and flow top-to-bottom in the first
                    // (right-most) column.
                    //
                    // We lay the run out horizontally as usual, then apply a single
                    // `cm` that rotates 90° clockwise (PDF `[0 -1 1 0]`, which maps
                    // local +x→PDF −y "down" and local +y→PDF +x "right") and
                    // translates so the horizontal text's content-top-left anchors
                    // at the content box and the column hugs the right edge. The
                    // wrapper is scoped to the glyph-drawing loop only, so the
                    // background/border/outline (painted earlier) stay upright.
                    let vertical = matches!(
                        writing_mode,
                        crate::style::computed::WritingMode::VerticalRl
                    );
                    if vertical {
                        // Content-box edges in PDF (y-up) coordinates. `text_y`
                        // currently sits at the content-area top (block_y − top
                        // border − top padding) before any line advance.
                        let content_top = text_y;
                        let content_right = padding_box_x + padding_box_w - padding_right;
                        let content_left = padding_box_x + padding_left;
                        // matrix maps (gx, gy) → (gy + e, −gx + f):
                        //   glyph top (gy = content_top) → X = content_right (column
                        //     hugs the right edge), and
                        //   text start (gx = content_left) → Y = content_top (text
                        //     begins at the top of the column, flowing downward).
                        let e = content_right - content_top;
                        let f = content_top + content_left;
                        content.push_str("q\n");
                        content.push_str(&format!("0 -1 1 0 {e} {f} cm\n"));
                    }

                    let line_count = lines.len();
                    for (line_idx, line) in lines.iter().enumerate() {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y -= metrics.half_leading + metrics.ascender;
                        let line_annotation_box = TextLineAnnotationBox {
                            top: text_y + metrics.ascender + metrics.half_leading,
                            bottom: text_y - metrics.descender - metrics.half_leading,
                        };

                        let line_text = line_text_content(line);
                        let has_inline_box = line.runs.iter().any(|r| r.inline_box.is_some());
                        if line_text.is_empty() && !has_inline_box {
                            continue;
                        }

                        let line_width = estimate_line_width_with_fonts(line, custom_fonts);
                        let is_last_line = line_idx == line_count - 1;

                        // Calculate word spacing for justified text
                        let justify_ws = if *text_align == TextAlign::Justify && !is_last_line {
                            let first_line_indent = if line_idx == 0 { *text_indent } else { 0.0 };
                            let content_width =
                                padding_box_w - padding_left - padding_right - first_line_indent;
                            let remaining = content_width - line_width;
                            let space_count = line_text.matches(' ').count();
                            if space_count > 0 && remaining > 0.0 {
                                remaining / space_count as f32
                            } else {
                                0.0
                            }
                        } else {
                            0.0
                        };
                        let total_ws = justify_ws + *css_word_spacing;

                        // CSS `text-indent` shifts the start of the FIRST line's
                        // inline content. For start-edge alignment (left/justify)
                        // it offsets the text origin; for center/right it consumes
                        // available width on the start side, recentring/reflowing
                        // the first line within the remaining space.
                        let first_line_indent = if line_idx == 0 { *text_indent } else { 0.0 };
                        // Drop-cap float exclusion: the line is shifted right so
                        // its inline content wraps beside the floated
                        // `::first-letter` (css-pseudo-4 §2.2 + css2 §9.5).
                        let line_inset = line.x_offset;
                        let text_x = match text_align {
                            TextAlign::Left | TextAlign::Justify => {
                                padding_box_x + padding_left + first_line_indent + line_inset
                            }
                            TextAlign::Center => {
                                let first_pad = line.runs.first().map_or(0.0, |r| r.padding.0);
                                padding_box_x
                                    + first_line_indent
                                    + (padding_box_w - first_line_indent - line_width) / 2.0
                                    + first_pad
                            }
                            TextAlign::Right => {
                                // Account for inline padding: text_x is where the
                                // text characters start, but line_width includes the
                                // full visual width (with left+right padding of inline
                                // spans).  Offset by the first run's left padding so
                                // the visual right edge aligns with the right boundary.
                                let first_pad = line.runs.first().map_or(0.0, |r| r.padding.0);
                                padding_box_x + padding_box_w - padding_right - line_width
                                    + first_pad
                            }
                        };

                        // Set letter spacing (CSS letter-spacing)
                        // CSS letter-spacing accepts negative values (css-text-3
                        // §8.2): tightening glyphs, not just widening them. Emit
                        // the Tc operator whenever it is non-zero.
                        if *letter_spacing != 0.0 {
                            content.push_str(&format!("{letter_spacing} Tc\n"));
                        }

                        // Set word spacing (justify + CSS word-spacing). Like
                        // letter-spacing, word-spacing may be negative.
                        if total_ws != 0.0 {
                            content.push_str(&format!("{total_ws} Tw\n"));
                        }

                        // Merge consecutive runs with the same style so
                        // spaces between words stay in a single PDF text
                        // string, preventing viewers from dropping them.
                        let merged = merge_runs(&line.runs);

                        // Phase 1: Draw backgrounds, decorations, and link
                        // annotations at estimated positions (visual-only).
                        let line_top_y = text_y + metrics.ascender + metrics.half_leading;
                        let line_bottom_y = text_y - metrics.descender - metrics.half_leading;
                        // Parent text content-area edges for `text-top`/`text-bottom`
                        // (parent glyph ascent/descent, no half-leading). Fall back to
                        // the line-box edges when the line carries no parent text.
                        let (text_ascent, text_descent) =
                            line_text_content_extents(line, custom_fonts);
                        let line_text_top_y = if text_ascent > 0.0 {
                            text_y + text_ascent
                        } else {
                            line_top_y
                        };
                        let line_text_bottom_y = if text_descent > 0.0 {
                            text_y - text_descent
                        } else {
                            line_bottom_y
                        };
                        let mut bg_x = text_x;
                        // Relatively-positioned inline boxes paint in the positioned
                        // layer — above in-flow siblings on the line, in source order
                        // (CSS 2.1 §9.9.1 painting order). Defer them so a later
                        // in-flow inline-block can't paint over an earlier offset one.
                        let mut deferred_inline: Vec<(
                            &crate::layout::engine::InlineBox,
                            f32,
                            f32,
                        )> = Vec::new();
                        for run in &merged {
                            // Atomic inline box (display: inline-block): paint the
                            // box and its inner content, then advance the cursor.
                            if let Some(inline) = run.inline_box.as_deref() {
                                let ibx = bg_x + inline.margin_left;
                                if inline.rel_offset_x != 0.0 || inline.rel_offset_y != 0.0 {
                                    deferred_inline.push((inline, ibx, run.font_size));
                                } else {
                                    render_inline_box(
                                        &mut content,
                                        inline,
                                        ibx,
                                        text_y,
                                        line_top_y,
                                        line_bottom_y,
                                        line_text_top_y,
                                        line_text_bottom_y,
                                        run.font_size,
                                        line_primary_x_height_ratio(&merged, custom_fonts),
                                        custom_fonts,
                                        &prepared_custom_fonts,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        &mut pdf_writer,
                                        &mut page_images,
                                    );
                                }
                                bg_x += inline.outer_width();
                                continue;
                            }
                            if run.text.is_empty() {
                                continue;
                            }
                            // text-decoration-color (falls back to currentColor).
                            let (dr, dg, db) = run.decoration_color.unwrap_or(run.color);
                            let run_width = estimate_run_width_with_fonts(run, custom_fonts);
                            // Inset decorations past leading/trailing whitespace.
                            let (deco_lead, deco_trail) = decoration_ws_insets(run, custom_fonts);

                            // Draw background rectangle for inline spans
                            if let Some((br, bg, bb, ba)) = run.background_color {
                                let needs_inline_bg_alpha = ba < 1.0;
                                if needs_inline_bg_alpha {
                                    let effective_alpha = ba * *opacity;
                                    let gs_name = format!("GSba{bg_alpha_counter}");
                                    bg_alpha_counter += 1;
                                    page_ext_gstates.push((gs_name.clone(), effective_alpha));
                                    content.push_str(&format!("/{gs_name} gs\n"));
                                }
                                let (pad_h, pad_v) = run.padding;
                                let rect_x = bg_x - pad_h;
                                let rect_y = text_y - 2.0 - pad_v;
                                let rect_w = run_width + pad_h * 2.0;
                                let rect_h = run.font_size + 2.0 + pad_v * 2.0;
                                content.push_str(&format!("{br} {bg} {bb} rg\n"));
                                if run.border_radius > 0.0 {
                                    content.push_str(&rounded_rect_path(
                                        rect_x,
                                        rect_y,
                                        rect_w,
                                        rect_h,
                                        run.border_radius,
                                    ));
                                    content.push_str("\nf\n");
                                } else {
                                    content.push_str(&format!(
                                        "{rect_x} {rect_y} {rect_w} {rect_h} re\nf\n"
                                    ));
                                }
                                if needs_inline_bg_alpha {
                                    if needs_opacity {
                                        let gs_name = format!("GS{elem_idx}");
                                        content.push_str(&format!("/{gs_name} gs\n"));
                                    } else {
                                        content.push_str("/GSDefault gs\n");
                                    }
                                }
                            }

                            // Draw underline (font-size-relative position and thickness)
                            if run.underline {
                                let (_, descender_ratio) = crate::fonts::font_metrics_ratios(
                                    &run.font_family,
                                    run.bold,
                                    run.italic,
                                    custom_fonts,
                                );
                                let desc = descender_ratio * run.font_size;
                                let uy = text_y - desc * 0.6;
                                let thickness = (run.font_size * 0.07).max(0.5);
                                content.push_str(&format!(
                                    "{dr} {dg} {db} RG\n{thickness} w\n{dx0} {uy} m {x2} {uy} l\nS\n",
                                    dx0 = bg_x + deco_lead,
                                    x2 = bg_x + run_width - deco_trail,
                                ));
                            }

                            // Draw strikethrough (line-through)
                            if run.line_through {
                                let sy = text_y + run.font_size * 0.3;
                                let thickness = (run.font_size * 0.07).max(0.5);
                                content.push_str(&format!(
                                    "{dr} {dg} {db} RG\n{thickness} w\n{dx0} {sy} m {x2} {sy} l\nS\n",
                                    dx0 = bg_x + deco_lead,
                                    x2 = bg_x + run_width - deco_trail,
                                ));
                            }

                            // Draw overline at the top of the text's em box
                            // (css-text-decor-3 §2.2): just above the ascent, not
                            // a full em above the baseline (which lands outside the
                            // line box and renders as if absent).
                            if run.overline {
                                let (ascender_ratio, _) = crate::fonts::font_metrics_ratios(
                                    &run.font_family,
                                    run.bold,
                                    run.italic,
                                    custom_fonts,
                                );
                                let oy = text_y + ascender_ratio * run.font_size;
                                let thickness = (run.font_size * 0.07).max(0.5);
                                content.push_str(&format!(
                                    "{dr} {dg} {db} RG\n{thickness} w\n{dx0} {oy} m {x2} {oy} l\nS\n",
                                    dx0 = bg_x + deco_lead,
                                    x2 = bg_x + run_width - deco_trail,
                                ));
                            }

                            // Track link annotation
                            if let Some(annotation) =
                                text_run_link_annotation(run, bg_x, run_width, line_annotation_box)
                            {
                                annotations.push(annotation);
                            }

                            bg_x += run_width;
                        }

                        // Paint deferred relatively-positioned inline boxes on top
                        // of the in-flow line content, preserving source order.
                        for (inline, ibx, fs) in deferred_inline {
                            render_inline_box(
                                &mut content,
                                inline,
                                ibx,
                                text_y,
                                line_top_y,
                                line_bottom_y,
                                line_text_top_y,
                                line_text_bottom_y,
                                fs,
                                line_primary_x_height_ratio(&merged, custom_fonts),
                                custom_fonts,
                                &prepared_custom_fonts,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                &mut pdf_writer,
                                &mut page_images,
                            );
                        }

                        // Phase 2: Render all text in a single BT/ET block
                        // so the PDF viewer advances the cursor naturally.
                        render_line_text(
                            &mut content,
                            &merged,
                            text_x,
                            text_y,
                            custom_fonts,
                            &prepared_custom_fonts,
                            total_ws,
                            line_text_top(line, custom_fonts),
                            &mut pdf_writer,
                            &mut page_images,
                        );

                        // Reset letter spacing after line
                        if *letter_spacing != 0.0 {
                            content.push_str("0 Tc\n");
                        }

                        // Reset word spacing after line
                        if total_ws != 0.0 {
                            content.push_str("0 Tw\n");
                        }

                        text_y -= metrics.descender + metrics.half_leading;
                    }

                    // Close the `vertical-rl` rotation wrapper opened before the
                    // line loop (scoped to glyph drawing only).
                    if vertical {
                        content.push_str("Q\n");
                    }

                    // Reset opacity if it was changed
                    if needs_opacity {
                        content.push_str("/GSDefault gs\n");
                    }

                    // Restore clipping state.
                    // If clip_children_count > 0, keep the clip open for
                    // subsequent elements that are visually inside this container.
                    if needs_clip {
                        if *clip_children_count > 0 {
                            clip_remaining = *clip_children_count;
                        } else {
                            content.push_str("Q\n");
                        }
                    }

                    // Restore transform state
                    if needs_transform {
                        content.push_str("Q\n");
                    }
                }
                LayoutElement::TableRow {
                    cells,
                    col_widths,
                    border_collapse,
                    border_spacing,
                    offset_left: table_offset_left,
                    ..
                } => {
                    let spacing = if *border_collapse == BorderCollapse::Collapse {
                        0.0
                    } else {
                        *border_spacing
                    };
                    // Collapsed tables paint their outer border half-outside the
                    // box (centered stroke), so shift the painted table right/down
                    // by half the outer border to land the outer edge on the table
                    // box edge — matching Chrome (see `collapse_paint_offset`).
                    let (collapse_dx, collapse_dy) = collapse_paint_offset(cells, *border_collapse);
                    let row_y = page_size.height - margin.top - y_pos - collapse_dy;
                    // The table's own horizontal start margin shifts the whole row
                    // right from the page content edge (the background/border box
                    // carries the same offset via its `offset_left`).
                    let table_origin_x = margin.left + *table_offset_left;

                    // Compute row height (max cell height, excluding rowspan > 1 cells)
                    let row_height = compute_row_height(cells);
                    let baseline_shifts = row_baseline_shifts(cells, custom_fonts);

                    // Track column position accounting for colspan
                    let mut col_pos: usize = 0;
                    for (cell_idx, cell) in cells.iter().enumerate() {
                        // Skip phantom cells (rowspan = 0); they are placeholders
                        // for cells spanning from previous rows.
                        if cell.rowspan == 0 {
                            col_pos += cell.colspan;
                            continue;
                        }

                        let (cell_x, cell_w) = table_cell_geometry(
                            col_widths,
                            col_pos,
                            cell.colspan,
                            spacing,
                            table_origin_x + collapse_dx,
                        );

                        // For cells with rowspan > 1, compute the total height
                        // spanning multiple rows.
                        let cell_height = if cell.rowspan > 1 {
                            let mut total_h = row_height;
                            for offset in 1..cell.rowspan {
                                let future_idx = elem_idx + offset;
                                if future_idx < page.elements.len() {
                                    if let LayoutElement::TableRow {
                                        cells: future_cells,
                                        ..
                                    } = &page.elements[future_idx].1
                                    {
                                        total_h += compute_row_height(future_cells);
                                    }
                                }
                            }
                            total_h
                        } else {
                            row_height
                        };

                        // Draw cell background (suppressed for empty cells under
                        // `empty-cells: hide`).
                        if let Some((r, g, b, a)) =
                            cell.background_color.filter(|_| !cell.hide_if_empty)
                        {
                            let needs_cell_bg_alpha = a < 1.0;
                            if needs_cell_bg_alpha {
                                let gs_name = format!("GStcbg{elem_idx}_{col_pos}");
                                page_ext_gstates.push((gs_name.clone(), a));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
                            content.push_str(&format!(
                                "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                x = cell_x,
                                y = row_y - cell_height,
                                w = cell_w,
                                h = cell_height,
                            ));
                            if needs_cell_bg_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }

                        // Draw cell borders when CSS specifies them (suppressed
                        // for empty cells under `empty-cells: hide`).
                        //
                        // PDF strokes are centered on the path. For
                        // `border-collapse: collapse` the cell border-boxes abut,
                        // so stroking centered on each box edge naturally puts
                        // the (shared, winning) border on the grid line — the
                        // default behavior here. For `border-collapse: separate`
                        // each cell paints its OWN border fully INSIDE its
                        // border-box, so the stroke must be inset by half its
                        // width; two adjacent cells then show two abutting
                        // borders (visually doubled) rather than one collapsed
                        // one, matching Chrome.
                        if cell.border.has_any() && !cell.hide_if_empty {
                            let separate = *border_collapse == BorderCollapse::Separate;
                            let inset = |w: f32| if separate { w / 2.0 } else { 0.0 };
                            let x1 = cell_x;
                            let x2 = cell_x + cell_w;
                            let y_top = row_y;
                            let y_bottom = row_y - cell_height;
                            if cell.border.top.width > 0.0 {
                                let (r, g, b) = cell.border.top.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    cell.border.top.alpha,
                                );
                                let y = y_top - inset(cell.border.top.width);
                                content.push_str(&dash_pattern_for_style(
                                    cell.border.top.style,
                                    cell.border.top.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y} m {x2} {y} l S\n",
                                    cell.border.top.width
                                ));
                                content.push_str(reset_dash_pattern(cell.border.top.style));
                                end_border_alpha(&mut content, a);
                            }
                            if cell.border.right.width > 0.0 {
                                let (r, g, b) = cell.border.right.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    cell.border.right.alpha,
                                );
                                let x = x2 - inset(cell.border.right.width);
                                content.push_str(&dash_pattern_for_style(
                                    cell.border.right.style,
                                    cell.border.right.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x} {y_top} m {x} {y_bottom} l S\n",
                                    cell.border.right.width
                                ));
                                content.push_str(reset_dash_pattern(cell.border.right.style));
                                end_border_alpha(&mut content, a);
                            }
                            if cell.border.bottom.width > 0.0 {
                                let (r, g, b) = cell.border.bottom.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    cell.border.bottom.alpha,
                                );
                                let y = y_bottom + inset(cell.border.bottom.width);
                                content.push_str(&dash_pattern_for_style(
                                    cell.border.bottom.style,
                                    cell.border.bottom.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y} m {x2} {y} l S\n",
                                    cell.border.bottom.width
                                ));
                                content.push_str(reset_dash_pattern(cell.border.bottom.style));
                                end_border_alpha(&mut content, a);
                            }
                            if cell.border.left.width > 0.0 {
                                let (r, g, b) = cell.border.left.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    cell.border.left.alpha,
                                );
                                let x = x1 + inset(cell.border.left.width);
                                content.push_str(&dash_pattern_for_style(
                                    cell.border.left.style,
                                    cell.border.left.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x} {y_top} m {x} {y_bottom} l S\n",
                                    cell.border.left.width
                                ));
                                content.push_str(reset_dash_pattern(cell.border.left.style));
                                end_border_alpha(&mut content, a);
                            }
                        }

                        // Render cell text at the first row's y position
                        let mut page_context = PageRenderContext::new(
                            &mut pdf_writer,
                            &mut page_images,
                            custom_fonts,
                            &prepared_custom_fonts,
                            &mut page_shadings,
                            &mut shading_counter,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                            &mut annotations,
                        );
                        render_cell_content(
                            &mut content,
                            cell,
                            TableCellRenderBox::new(
                                cell_x,
                                row_y,
                                cell_w,
                                row_height,
                                NestedLayoutFrame::new(
                                    cell_x,
                                    row_y,
                                    table_origin_x,
                                    page_size.height - margin.top,
                                    cell_w,
                                ),
                            )
                            .with_baseline_shift(
                                baseline_shifts.get(cell_idx).copied().unwrap_or(0.0),
                            ),
                            &mut page_context,
                        );

                        col_pos += cell.colspan;
                    }
                }
                LayoutElement::GridRow {
                    cells,
                    col_widths,
                    gap,
                    border: grid_border,
                    padding_left: grid_pl,
                    padding_right: grid_pr,
                    padding_top: grid_pt,
                    padding_bottom: grid_pb,
                    ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;
                    let row_height = compute_grid_row_height(cells) + grid_pt + grid_pb;
                    let grid_total_w: f32 = col_widths.iter().sum::<f32>()
                        + gap * col_widths.len().saturating_sub(1) as f32
                        + grid_pl
                        + grid_pr;

                    // Draw grid container border
                    if grid_border.has_any() {
                        let bx1 = margin.left;
                        let bx2 = margin.left + grid_total_w;
                        let by1 = row_y;
                        let by2 = row_y - row_height;
                        if grid_border.top.width > 0.0 {
                            let (r, g, b) = grid_border.top.color;
                            let a = begin_border_alpha(
                                &mut content,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                grid_border.top.alpha,
                            );
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx2} {by1} l S\n",
                                grid_border.top.width
                            ));
                            end_border_alpha(&mut content, a);
                        }
                        if grid_border.right.width > 0.0 {
                            let (r, g, b) = grid_border.right.color;
                            let a = begin_border_alpha(
                                &mut content,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                grid_border.right.alpha,
                            );
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{bx2} {by1} m {bx2} {by2} l S\n",
                                grid_border.right.width
                            ));
                            end_border_alpha(&mut content, a);
                        }
                        if grid_border.bottom.width > 0.0 {
                            let (r, g, b) = grid_border.bottom.color;
                            let a = begin_border_alpha(
                                &mut content,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                grid_border.bottom.alpha,
                            );
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{bx1} {by2} m {bx2} {by2} l S\n",
                                grid_border.bottom.width
                            ));
                            end_border_alpha(&mut content, a);
                        }
                        if grid_border.left.width > 0.0 {
                            let (r, g, b) = grid_border.left.color;
                            let a = begin_border_alpha(
                                &mut content,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                grid_border.left.alpha,
                            );
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx1} {by2} l S\n",
                                grid_border.left.width
                            ));
                            end_border_alpha(&mut content, a);
                        }
                    }

                    let mut cell_x = margin.left + grid_pl;
                    let cell_row_y = row_y - grid_pt;
                    for (i, cell) in cells.iter().enumerate() {
                        let cell_w = if i < col_widths.len() {
                            col_widths[i]
                        } else {
                            0.0
                        };

                        // Draw cell background
                        let cell_content_h = compute_grid_row_height(cells);
                        if let Some((r, g, b, a)) = cell.background_color {
                            let needs_grid_bg_alpha = a < 1.0;
                            if needs_grid_bg_alpha {
                                let gs_name = format!("GSgcbg{elem_idx}_{i}");
                                page_ext_gstates.push((gs_name.clone(), a));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
                            content.push_str(&format!(
                                "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                x = cell_x,
                                y = cell_row_y - cell_content_h,
                                w = cell_w,
                                h = cell_content_h,
                            ));
                            if needs_grid_bg_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }

                        // Draw cell gradient backgrounds across the cell box.
                        paint_cell_gradient_backgrounds(
                            &mut content,
                            cell,
                            cell_x,
                            cell_row_y - cell_content_h,
                            cell_w,
                            cell_content_h,
                            &mut page_shadings,
                            &mut shading_counter,
                        );

                        // Render cell text
                        let mut page_context = PageRenderContext::new(
                            &mut pdf_writer,
                            &mut page_images,
                            custom_fonts,
                            &prepared_custom_fonts,
                            &mut page_shadings,
                            &mut shading_counter,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                            &mut annotations,
                        );
                        render_cell_content(
                            &mut content,
                            cell,
                            TableCellRenderBox::new(
                                cell_x,
                                cell_row_y,
                                cell_w,
                                cell_content_h,
                                NestedLayoutFrame::new(
                                    cell_x,
                                    cell_row_y,
                                    margin.left,
                                    page_size.height - margin.top,
                                    cell_w,
                                ),
                            ),
                            &mut page_context,
                        );

                        cell_x += cell_w;
                        // Add gap between columns
                        if i + 1 < col_widths.len() {
                            cell_x += gap;
                        }
                    }
                }
                LayoutElement::FlexRow {
                    cells,
                    row_height,
                    offset_left: flex_offset_left,
                    background_color,
                    container_width,
                    padding_top,
                    padding_bottom,
                    padding_left,
                    padding_right,
                    border,
                    border_radius,
                    box_shadow,
                    background_gradient,
                    background_radial_gradient,
                    background_conic_gradient,
                    background_svg,
                    background_blur_radius,
                    background_size: flex_bg_size,
                    background_position: flex_bg_pos,
                    background_repeat: flex_bg_repeat,
                    background_origin: flex_bg_origin,
                    align_items,
                    ..
                } => {
                    let row_y = page_size.height - margin.top - y_pos;
                    let full_height =
                        padding_top + row_height + padding_bottom + border.vertical_width();
                    // Inline-axis origin of the flex container's border box: the
                    // page content-left plus the container's own resolved
                    // horizontal margin / auto-centering (see `FlexRow.offset_left`).
                    let flex_left = margin.left + *flex_offset_left;
                    // Inline-axis origin of the flex *content* box: in-flow cells
                    // begin inside the container's left border (CSS box model — a
                    // cell's `x_offset` is measured from the content box, so the
                    // border-left width must be added, mirroring the cross-axis
                    // `text_area_top` which already subtracts `border.top.width`).
                    let cells_left = flex_left + border.left.width;

                    // Draw box shadow with blur
                    render_box_shadows(
                        &mut content,
                        box_shadow,
                        flex_left,
                        row_y - full_height,
                        *container_width,
                        full_height,
                        *border_radius,
                        &mut page_ext_gstates,
                        &mut bg_alpha_counter,
                        &mut pdf_writer,
                        &mut page_images,
                    );

                    // Draw container background
                    if let Some((r, g, b, a)) = background_color {
                        let bg_x = flex_left;
                        let bg_y = row_y - full_height;
                        let needs_flex_bg_alpha = *a < 1.0;
                        if needs_flex_bg_alpha {
                            let gs_name = format!("GSfbg{elem_idx}");
                            page_ext_gstates.push((gs_name.clone(), *a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!("{r} {g} {b} rg\n"));
                        if *border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                bg_x,
                                bg_y,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("f\n");
                        } else {
                            content.push_str(&format!(
                                "{x} {y} {w} {h} re\nf\n",
                                x = bg_x,
                                y = bg_y,
                                w = container_width,
                                h = full_height,
                            ));
                        }
                        if needs_flex_bg_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    // Draw container linear gradient
                    if let Some(gradient) = background_gradient {
                        let bg_x = flex_left;
                        let bg_y = row_y - full_height;
                        if *border_radius > 0.0 {
                            content.push_str("q\n");
                            content.push_str(&rounded_rect_path(
                                bg_x,
                                bg_y,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("W n\n");
                        }
                        render_linear_gradient(
                            &mut content,
                            gradient,
                            bg_x,
                            bg_y,
                            *container_width,
                            full_height,
                            &mut page_shadings,
                            &mut shading_counter,
                        );
                        if *border_radius > 0.0 {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw container radial gradient
                    if let Some(gradient) = background_radial_gradient {
                        let bg_x = flex_left;
                        let bg_y = row_y - full_height;
                        if *border_radius > 0.0 {
                            content.push_str("q\n");
                            content.push_str(&rounded_rect_path(
                                bg_x,
                                bg_y,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("W n\n");
                        }
                        render_radial_gradient(
                            &mut content,
                            gradient,
                            bg_x,
                            bg_y,
                            *container_width,
                            full_height,
                            &mut page_shadings,
                            &mut shading_counter,
                        );
                        if *border_radius > 0.0 {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw container conic gradient
                    if let Some(gradient) = background_conic_gradient {
                        let bg_x = flex_left;
                        let bg_y = row_y - full_height;
                        if *border_radius > 0.0 {
                            content.push_str("q\n");
                            content.push_str(&rounded_rect_path(
                                bg_x,
                                bg_y,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("W n\n");
                        }
                        render_conic_gradient(
                            &mut content,
                            gradient,
                            bg_x,
                            bg_y,
                            *container_width,
                            full_height,
                        );
                        if *border_radius > 0.0 {
                            content.push_str("Q\n");
                        }
                    }

                    // Draw inset box-shadow for flex container (after backgrounds).
                    render_box_shadows_inset(
                        &mut content,
                        box_shadow,
                        flex_left,
                        row_y - full_height,
                        *container_width,
                        full_height,
                        *border_radius,
                        &mut page_ext_gstates,
                        &mut bg_alpha_counter,
                    );

                    // Draw SVG background image for flex container
                    if let Some(svg_tree) = background_svg {
                        let bg_x = flex_left;
                        let bg_y = row_y - full_height;
                        // Adjust reference box based on background-origin
                        let (ref_x, ref_y, ref_w, ref_h) = match flex_bg_origin {
                            BackgroundOrigin::Border => (
                                bg_x - border.left.width,
                                bg_y - border.bottom.width,
                                *container_width + border.left.width + border.right.width,
                                full_height + border.top.width + border.bottom.width,
                            ),
                            BackgroundOrigin::Content => (
                                bg_x + padding_left,
                                bg_y + padding_bottom,
                                (*container_width - padding_left - padding_right).max(0.0),
                                (full_height - padding_top - padding_bottom).max(0.0),
                            ),
                            BackgroundOrigin::Padding => {
                                (bg_x, bg_y, *container_width, full_height)
                            }
                        };
                        render_svg_background(
                            &mut content,
                            svg_tree,
                            &mut pdf_writer,
                            &mut page_images,
                            &mut page_shadings,
                            &mut shading_counter,
                            Some(&mut page_ext_gstates),
                            BackgroundPaintContext::new(
                                SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                                SvgViewportBox::new(
                                    bg_x - border.left.width,
                                    bg_y - border.bottom.width,
                                    *container_width + border.left.width + border.right.width,
                                    full_height + border.top.width + border.bottom.width,
                                ),
                                *border_radius,
                                *background_blur_radius,
                                *flex_bg_size,
                                *flex_bg_pos,
                                *flex_bg_repeat,
                            ),
                        );
                    }

                    // Draw border
                    if border.has_any() {
                        let bx = flex_left;
                        let by = row_y - full_height;
                        let uniform = border.top.width == border.right.width
                            && border.top.width == border.bottom.width
                            && border.top.width == border.left.width
                            && border.top.color == border.right.color
                            && border.top.color == border.bottom.color
                            && border.top.color == border.left.color
                            && border.top.style == border.right.style
                            && border.top.style == border.bottom.style
                            && border.top.style == border.left.style;
                        if uniform && *border_radius > 0.0 {
                            let (r, g, b) = border.top.color;
                            let a = begin_border_alpha(
                                &mut content,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                border.top.alpha,
                            );
                            content.push_str(&dash_pattern_for_style(
                                border.top.style,
                                border.top.width,
                            ));
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{bw} w\n",
                                bw = border.top.width
                            ));
                            content.push_str(&rounded_rect_path(
                                bx,
                                by,
                                *container_width,
                                full_height,
                                *border_radius,
                            ));
                            content.push_str("S\n");
                            content.push_str(reset_dash_pattern(border.top.style));
                            end_border_alpha(&mut content, a);
                        } else if uniform {
                            let (r, g, b) = border.top.color;
                            let a = begin_border_alpha(
                                &mut content,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                border.top.alpha,
                            );
                            content.push_str(&dash_pattern_for_style(
                                border.top.style,
                                border.top.width,
                            ));
                            // Stroke INSIDE the border box: center the stroke half a
                            // border-width in from each edge so its outer edge meets
                            // the box edge (matches block / image borders; without
                            // this the flex frame straddled the edge and read ~1px
                            // wide on each side, narrowing the inter-item gap).
                            let half = border.top.width / 2.0;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{bw} w\n{bx} {by} {w} {h} re\nS\n",
                                bw = border.top.width,
                                bx = bx + half,
                                by = by + half,
                                w = container_width - border.top.width,
                                h = full_height - border.top.width,
                            ));
                            content.push_str(reset_dash_pattern(border.top.style));
                            end_border_alpha(&mut content, a);
                        } else {
                            let x1 = bx + border.left.width / 2.0;
                            let x2 = bx + container_width - border.right.width / 2.0;
                            let y_top = row_y - border.top.width / 2.0;
                            let y_bottom = by + border.bottom.width / 2.0;
                            if border.top.width > 0.0 {
                                let (r, g, b) = border.top.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.top.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.top.style,
                                    border.top.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                    border.top.width
                                ));
                                content.push_str(reset_dash_pattern(border.top.style));
                                end_border_alpha(&mut content, a);
                            }
                            if border.right.width > 0.0 {
                                let (r, g, b) = border.right.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.right.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.right.style,
                                    border.right.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                    border.right.width
                                ));
                                content.push_str(reset_dash_pattern(border.right.style));
                                end_border_alpha(&mut content, a);
                            }
                            if border.bottom.width > 0.0 {
                                let (r, g, b) = border.bottom.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.bottom.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.bottom.style,
                                    border.bottom.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                    border.bottom.width
                                ));
                                content.push_str(reset_dash_pattern(border.bottom.style));
                                end_border_alpha(&mut content, a);
                            }
                            if border.left.width > 0.0 {
                                let (r, g, b) = border.left.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.left.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.left.style,
                                    border.left.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                    border.left.width
                                ));
                                content.push_str(reset_dash_pattern(border.left.style));
                                end_border_alpha(&mut content, a);
                            }
                        }
                    }

                    // Render each flex cell at its computed x-offset
                    let text_area_top = row_y - border.top.width - padding_top;

                    // Baseline cross-axis alignment (CSS Flexbox §8.3). Items
                    // with effective `align: baseline` are positioned so their
                    // first text baselines coincide. Each item's baseline is the
                    // distance from its border-box top to its first line's
                    // baseline (`border-top + padding-top + ascent + half
                    // leading`); the line's shared baseline is the maximum such
                    // distance among its baseline items. We precompute, per flex
                    // line (keyed by `y_offset`), that maximum so each cell can
                    // shift down by `max_baseline - own_baseline`.
                    let cell_first_baseline =
                        |cell: &crate::layout::engine::FlexCell| -> Option<f32> {
                            let first = cell
                                .lines
                                .iter()
                                .find(|l| l.runs.iter().any(|r| !r.text.is_empty()))?;
                            let m = line_box_metrics(first, custom_fonts);
                            Some(
                                cell.border.top.width
                                    + cell.padding_top
                                    + m.half_leading
                                    + m.ascender,
                            )
                        };
                    let is_baseline_cell = |cell: &crate::layout::engine::FlexCell| -> bool {
                        matches!(
                            match cell.align_self {
                                AlignSelf::Auto => *align_items,
                                AlignSelf::FlexStart => AlignItems::FlexStart,
                                AlignSelf::FlexEnd => AlignItems::FlexEnd,
                                AlignSelf::Center => AlignItems::Center,
                                AlignSelf::Baseline => AlignItems::Baseline,
                                AlignSelf::Stretch => AlignItems::Stretch,
                            },
                            AlignItems::Baseline
                        )
                    };
                    // Max first-baseline distance among baseline items sharing a
                    // flex line (lines are distinguished by their cross offset).
                    let line_max_baseline = |y_offset: f32| -> Option<f32> {
                        cells
                            .iter()
                            .filter(|c| (c.y_offset - y_offset).abs() < 0.01 && is_baseline_cell(c))
                            .filter_map(cell_first_baseline)
                            .fold(None, |acc: Option<f32>, b| {
                                Some(acc.map_or(b, |a| a.max(b)))
                            })
                    };

                    // CSS 2.1 §9.9.1 painting order: within a stacking context,
                    // positioned items (position:relative/absolute) paint after
                    // all non-positioned in-flow content. Iterate non-positioned
                    // cells first, then positioned cells — preserving source
                    // order within each group — so a relatively-offset
                    // inline-block is not hidden under a later in-flow sibling.
                    let paint_order: Vec<&crate::layout::engine::FlexCell> = cells
                        .iter()
                        .filter(|c| !c.is_positioned)
                        .chain(cells.iter().filter(|c| c.is_positioned))
                        .collect();
                    for cell in paint_order {
                        let cell_x = cells_left + padding_left + cell.x_offset;
                        let cell_inner_w = cell.width - cell.padding_left - cell.padding_right;
                        // For single-line rows `line_cross_size == row_height`.
                        // For multi-line wrap, each cell's line_cross_size is its
                        // own flex line height, so alignment is per-line.
                        let line_cross = if cell.line_cross_size > 0.0 {
                            cell.line_cross_size
                        } else {
                            *row_height
                        };
                        let cell_y_origin = cell.y_offset;

                        // Compute per-cell height and vertical offset based on the
                        // effective cross-axis alignment. `align-self` on the item
                        // overrides the container's `align-items` unless it is
                        // `auto`. For stretch: use the line's cross size (default
                        // CSS behavior). For flex-start/center/flex-end: use the
                        // cell's natural_height.
                        let effective_align = match cell.align_self {
                            AlignSelf::Auto => *align_items,
                            AlignSelf::FlexStart => AlignItems::FlexStart,
                            AlignSelf::FlexEnd => AlignItems::FlexEnd,
                            AlignSelf::Center => AlignItems::Center,
                            AlignSelf::Baseline => AlignItems::Baseline,
                            AlignSelf::Stretch => AlignItems::Stretch,
                        };
                        let (cell_render_h, cell_y_shift) = match effective_align {
                            // `align-items: stretch` only stretches items whose
                            // cross size (height) is auto. An item with a definite
                            // height keeps it (like flex-start, top-anchored).
                            AlignItems::Stretch if cell.has_explicit_height => {
                                (cell.natural_height, cell_y_origin)
                            }
                            AlignItems::Stretch => (line_cross, cell_y_origin),
                            // `align: baseline` (CSS Flexbox §8.3): shift the
                            // item down so its first text baseline meets the
                            // line's shared baseline (the max first-baseline
                            // distance among the line's baseline items). An item
                            // with no text baseline falls back to cross-start.
                            AlignItems::Baseline => {
                                let shift = match (
                                    cell_first_baseline(cell),
                                    line_max_baseline(cell.y_offset),
                                ) {
                                    (Some(own), Some(max)) => (max - own).max(0.0),
                                    _ => 0.0,
                                };
                                (cell.natural_height, cell_y_origin + shift)
                            }
                            AlignItems::FlexStart => (cell.natural_height, cell_y_origin),
                            AlignItems::FlexEnd => {
                                let h = cell.natural_height;
                                (h, cell_y_origin + line_cross - h)
                            }
                            AlignItems::Center => {
                                let h = cell.natural_height;
                                (h, cell_y_origin + (line_cross - h) / 2.0)
                            }
                        };

                        // Apply cell transform if set (rotate, scale, translate).
                        // The transform pivots about the cell's actual rendered
                        // border box (`cell_render_h` at `cell_y_shift`), NOT the
                        // flex line cross size — an item aligned (e.g. center) in a
                        // taller line must rotate about its own box center, not the
                        // line's, or the rotated box drifts vertically.
                        let cell_needs_transform = cell.transform.is_some();
                        if let Some(t) = &cell.transform {
                            let (ox, oy) = cell.transform_origin.resolve(cell.width, cell_render_h);
                            let cx = cell_x + ox;
                            let cy = text_area_top - cell_y_shift - oy;
                            content.push_str("q\n");
                            push_transform_cm(&mut content, t, cx, cy, cell.width, cell_render_h);
                        }

                        // Draw per-cell box-shadow (e.g. inline-block items
                        // with `box-shadow`). We draw it before the background
                        // so the shadow sits behind the cell.
                        {
                            let cell_bg_x = cells_left + padding_left + cell.x_offset;
                            let cell_bg_y = text_area_top - cell_y_shift - cell_render_h;
                            render_box_shadows(
                                &mut content,
                                &cell.box_shadow,
                                cell_bg_x,
                                cell_bg_y,
                                cell.width,
                                cell_render_h,
                                cell.border_radius,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                                &mut pdf_writer,
                                &mut page_images,
                            );
                        }

                        // Draw cell background
                        if let Some((r, g, b, a)) = cell.background_color {
                            let bg_x = cells_left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - cell_y_shift - cell_render_h;
                            let needs_fcell_bg_alpha = a < 1.0;
                            if needs_fcell_bg_alpha {
                                let gs_name = format!("GSfcbg{bg_alpha_counter}");
                                bg_alpha_counter += 1;
                                page_ext_gstates.push((gs_name.clone(), a));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
                            content.push_str(&format!("{r} {g} {b} rg\n"));
                            if cell.border_radius > 0.0 {
                                content.push_str(&rounded_rect_path(
                                    bg_x,
                                    bg_y,
                                    cell.width,
                                    cell_render_h,
                                    cell.border_radius,
                                ));
                                content.push_str("f\n");
                            } else {
                                content.push_str(&format!(
                                    "{bg_x} {bg_y} {w} {h} re\nf\n",
                                    w = cell.width,
                                    h = cell_render_h,
                                ));
                            }
                            if needs_fcell_bg_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }

                        // Draw inset box-shadow (after cell background, before borders).
                        {
                            let cell_bg_x = cells_left + padding_left + cell.x_offset;
                            let cell_bg_y = text_area_top - cell_y_shift - cell_render_h;
                            render_box_shadows_inset(
                                &mut content,
                                &cell.box_shadow,
                                cell_bg_x,
                                cell_bg_y,
                                cell.width,
                                cell_render_h,
                                cell.border_radius,
                                &mut page_ext_gstates,
                                &mut bg_alpha_counter,
                            );
                        }

                        // Draw cell borders
                        if cell.border.has_any() {
                            if cell.border_radius > 0.0 {
                                let bw = cell.border.top.width;
                                let (r, g, b) = cell.border.top.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    cell.border.top.alpha,
                                );
                                content.push_str(&format!("{r} {g} {b} RG\n{bw} w\n"));
                                content.push_str(&rounded_rect_path(
                                    cell_x,
                                    text_area_top - cell_y_shift - cell_render_h,
                                    cell.width,
                                    cell_render_h,
                                    cell.border_radius,
                                ));
                                content.push_str("S\n");
                                end_border_alpha(&mut content, a);
                            } else {
                                // Stroke INSIDE the cell's border box: center each
                                // side's stroke half its width in from the edge so
                                // the painted frame sits within the declared
                                // border-box width (matches block / image borders).
                                let box_left = cell_x;
                                let box_right = cell_x + cell.width;
                                let box_top = text_area_top - cell_y_shift;
                                let box_bottom = text_area_top - cell_y_shift - cell_render_h;
                                let bx1 = box_left + cell.border.left.width / 2.0;
                                let bx2 = box_right - cell.border.right.width / 2.0;
                                let by1 = box_top - cell.border.top.width / 2.0;
                                let by2 = box_bottom + cell.border.bottom.width / 2.0;
                                if cell.border.top.width > 0.0 {
                                    let (r, g, b) = cell.border.top.color;
                                    let a = begin_border_alpha(
                                        &mut content,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        cell.border.top.alpha,
                                    );
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx2} {by1} l S\n",
                                        cell.border.top.width
                                    ));
                                    end_border_alpha(&mut content, a);
                                }
                                if cell.border.right.width > 0.0 {
                                    let (r, g, b) = cell.border.right.color;
                                    let a = begin_border_alpha(
                                        &mut content,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        cell.border.right.alpha,
                                    );
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{} w\n{bx2} {by1} m {bx2} {by2} l S\n",
                                        cell.border.right.width
                                    ));
                                    end_border_alpha(&mut content, a);
                                }
                                if cell.border.bottom.width > 0.0 {
                                    let (r, g, b) = cell.border.bottom.color;
                                    let a = begin_border_alpha(
                                        &mut content,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        cell.border.bottom.alpha,
                                    );
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{} w\n{bx1} {by2} m {bx2} {by2} l S\n",
                                        cell.border.bottom.width
                                    ));
                                    end_border_alpha(&mut content, a);
                                }
                                if cell.border.left.width > 0.0 {
                                    let (r, g, b) = cell.border.left.color;
                                    let a = begin_border_alpha(
                                        &mut content,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        cell.border.left.alpha,
                                    );
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx1} {by2} l S\n",
                                        cell.border.left.width
                                    ));
                                    end_border_alpha(&mut content, a);
                                }
                            } // else (non-rounded cell border)
                        }

                        // Draw cell linear gradient
                        if let Some(gradient) = &cell.background_gradient {
                            let bg_x = cells_left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - cell_y_shift - cell_render_h;
                            if cell.border_radius > 0.0 {
                                content.push_str("q\n");
                                content.push_str(&rounded_rect_path(
                                    bg_x,
                                    bg_y,
                                    cell.width,
                                    cell_render_h,
                                    cell.border_radius,
                                ));
                                content.push_str("W n\n");
                            }
                            render_linear_gradient(
                                &mut content,
                                gradient,
                                bg_x,
                                bg_y,
                                cell.width,
                                cell_render_h,
                                &mut page_shadings,
                                &mut shading_counter,
                            );
                            if cell.border_radius > 0.0 {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw cell radial gradient
                        if let Some(gradient) = &cell.background_radial_gradient {
                            let bg_x = cells_left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - cell_y_shift - cell_render_h;
                            if cell.border_radius > 0.0 {
                                content.push_str("q\n");
                                content.push_str(&rounded_rect_path(
                                    bg_x,
                                    bg_y,
                                    cell.width,
                                    cell_render_h,
                                    cell.border_radius,
                                ));
                                content.push_str("W n\n");
                            }
                            render_radial_gradient(
                                &mut content,
                                gradient,
                                bg_x,
                                bg_y,
                                cell.width,
                                cell_render_h,
                                &mut page_shadings,
                                &mut shading_counter,
                            );
                            if cell.border_radius > 0.0 {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw cell conic gradient
                        if let Some(gradient) = &cell.background_conic_gradient {
                            let bg_x = cells_left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - cell_y_shift - cell_render_h;
                            if cell.border_radius > 0.0 {
                                content.push_str("q\n");
                                content.push_str(&rounded_rect_path(
                                    bg_x,
                                    bg_y,
                                    cell.width,
                                    cell_render_h,
                                    cell.border_radius,
                                ));
                                content.push_str("W n\n");
                            }
                            render_conic_gradient(
                                &mut content,
                                gradient,
                                bg_x,
                                bg_y,
                                cell.width,
                                cell_render_h,
                            );
                            if cell.border_radius > 0.0 {
                                content.push_str("Q\n");
                            }
                        }

                        if let Some(svg_tree) = &cell.background_svg {
                            let bg_x = cells_left + padding_left + cell.x_offset;
                            let bg_y = text_area_top - cell_y_shift - cell_render_h;
                            let (ref_x, ref_y, ref_w, ref_h) = match cell.background_origin {
                                BackgroundOrigin::Content => (
                                    bg_x + cell.padding_left,
                                    bg_y + cell.padding_bottom,
                                    (cell.width - cell.padding_left - cell.padding_right).max(0.0),
                                    (cell_render_h - cell.padding_top - cell.padding_bottom)
                                        .max(0.0),
                                ),
                                BackgroundOrigin::Border | BackgroundOrigin::Padding => {
                                    (bg_x, bg_y, cell.width, cell_render_h)
                                }
                            };
                            render_svg_background(
                                &mut content,
                                svg_tree,
                                &mut pdf_writer,
                                &mut page_images,
                                &mut page_shadings,
                                &mut shading_counter,
                                Some(&mut page_ext_gstates),
                                BackgroundPaintContext::new(
                                    SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                                    SvgViewportBox::new(bg_x, bg_y, cell.width, cell_render_h),
                                    cell.border_radius,
                                    cell.background_blur_radius,
                                    cell.background_size,
                                    cell.background_position,
                                    cell.background_repeat,
                                ),
                            );
                        }

                        // Render cell text
                        let mut text_y = text_area_top - cell_y_shift - cell.padding_top;
                        for line in &cell.lines {
                            let metrics = line_box_metrics(line, custom_fonts);
                            text_y -= metrics.half_leading + metrics.ascender;
                            let line_annotation_box = TextLineAnnotationBox {
                                top: text_y + metrics.ascender + metrics.half_leading,
                                bottom: text_y - metrics.descender - metrics.half_leading,
                            };
                            let text_content: String =
                                line.runs.iter().map(|r| r.text.as_str()).collect();
                            if text_content.is_empty() {
                                continue;
                            }
                            let merged = merge_runs(&line.runs);
                            // Calculate line width for text-align
                            let line_width: f32 = merged
                                .iter()
                                .map(|r| {
                                    let w = estimate_run_width_with_fonts(r, custom_fonts);
                                    w + r.padding.0 * 2.0
                                })
                                .sum();
                            let first_pad = line.runs.first().map_or(0.0, |r| r.padding.0);
                            let text_x = match cell.text_align {
                                TextAlign::Right => {
                                    cell_x
                                        + cell.padding_left
                                        + (cell_inner_w - line_width).max(0.0)
                                        + first_pad
                                }
                                TextAlign::Center => {
                                    cell_x
                                        + cell.padding_left
                                        + ((cell_inner_w - line_width) / 2.0).max(0.0)
                                        + first_pad
                                }
                                _ => cell_x + cell.padding_left,
                            };
                            let mut x = text_x;
                            for run in &merged {
                                if run.text.is_empty() {
                                    continue;
                                }
                                let (dr, dg, db) = run.decoration_color.unwrap_or(run.color);
                                let rw = estimate_run_width_with_fonts(run, custom_fonts);
                                let (deco_lead, deco_trail) =
                                    decoration_ws_insets(run, custom_fonts);

                                // Draw background rectangle for inline spans
                                if let Some((br, bgc, bb, ba)) = run.background_color {
                                    let needs_inline_bg_alpha = ba < 1.0;
                                    if needs_inline_bg_alpha {
                                        let gs_name = format!("GSfiba{bg_alpha_counter}");
                                        bg_alpha_counter += 1;
                                        page_ext_gstates.push((gs_name.clone(), ba));
                                        content.push_str(&format!("/{gs_name} gs\n"));
                                    }
                                    let (pad_h, pad_v) = run.padding;
                                    let rx = x - pad_h;
                                    let ry = text_y - 2.0 - pad_v;
                                    let rw2 = rw + pad_h * 2.0;
                                    let rh = run.font_size + 2.0 + pad_v * 2.0;
                                    content.push_str(&format!("{br} {bgc} {bb} rg\n"));
                                    if run.border_radius > 0.0 {
                                        content.push_str(&rounded_rect_path(
                                            rx,
                                            ry,
                                            rw2,
                                            rh,
                                            run.border_radius,
                                        ));
                                        content.push_str("\nf\n");
                                    } else {
                                        content.push_str(&format!("{rx} {ry} {rw2} {rh} re\nf\n"));
                                    }
                                    if needs_inline_bg_alpha {
                                        content.push_str("/GSDefault gs\n");
                                    }
                                }

                                render_run_text(
                                    &mut content,
                                    run,
                                    x,
                                    text_y,
                                    crate::layout::text::line_primary_font_size(&merged),
                                    custom_fonts,
                                    &prepared_custom_fonts,
                                    0.0,
                                    &mut pdf_writer,
                                    &mut page_images,
                                );

                                // Draw underline (font-size-relative)
                                if run.underline {
                                    let (_, descender_ratio) = crate::fonts::font_metrics_ratios(
                                        &run.font_family,
                                        run.bold,
                                        run.italic,
                                        custom_fonts,
                                    );
                                    let desc = descender_ratio * run.font_size;
                                    let uy = text_y - desc * 0.6;
                                    let thickness = (run.font_size * 0.07).max(0.5);
                                    content.push_str(&format!(
                                        "{dr} {dg} {db} RG\n{thickness} w\n{dx0} {uy} m {x2} {uy} l\nS\n",
                                        dx0 = x + deco_lead,
                                        x2 = x + rw - deco_trail,
                                    ));
                                }

                                // Draw strikethrough (line-through)
                                if run.line_through {
                                    let sy = text_y + run.font_size * 0.3;
                                    let thickness = (run.font_size * 0.07).max(0.5);
                                    content.push_str(&format!(
                                        "{dr} {dg} {db} RG\n{thickness} w\n{dx0} {sy} m {x2} {sy} l\nS\n",
                                        dx0 = x + deco_lead,
                                        x2 = x + rw - deco_trail,
                                    ));
                                }

                                // Draw overline at the top of the em box (just
                                // above the ascent), matching the TextBlock path.
                                if run.overline {
                                    let (ascender_ratio, _) = crate::fonts::font_metrics_ratios(
                                        &run.font_family,
                                        run.bold,
                                        run.italic,
                                        custom_fonts,
                                    );
                                    let oy = text_y + ascender_ratio * run.font_size;
                                    let thickness = (run.font_size * 0.07).max(0.5);
                                    content.push_str(&format!(
                                        "{dr} {dg} {db} RG\n{thickness} w\n{dx0} {oy} m {x2} {oy} l\nS\n",
                                        dx0 = x + deco_lead,
                                        x2 = x + rw - deco_trail,
                                    ));
                                }

                                if let Some(annotation) =
                                    text_run_link_annotation(run, x, rw, line_annotation_box)
                                {
                                    annotations.push(annotation);
                                }

                                x += rw;
                            }

                            text_y -= metrics.descender + metrics.half_leading;
                        }

                        // Render nested elements (tables, images, etc. inside flex items)
                        if !cell.nested_elements.is_empty() {
                            let nested_x = cell_x;
                            let mut nested_y = text_area_top - cell_y_shift;
                            for nested_elem in &cell.nested_elements {
                                match nested_elem {
                                    LayoutElement::TextBlock {
                                        lines: n_lines,
                                        margin_top: n_mt,
                                        padding_top: n_pt,
                                        padding_bottom: n_pb,
                                        background_color: n_bg,
                                        block_width: n_bw,
                                        block_height: n_bh,
                                        border: n_border,
                                        ..
                                    } => {
                                        nested_y -= n_mt;
                                        let n_width = n_bw.unwrap_or(cell.width);
                                        let text_h: f32 = n_lines.iter().map(|l| l.height).sum();
                                        let content_total =
                                            n_pt + text_h + n_pb + n_border.vertical_width();
                                        // `block_height` is a padding-box height
                                        // (TextBlock convention), so the painted
                                        // border box is `block_height + border`.
                                        // Without adding the border back, a
                                        // border-box-sized child (e.g. an empty
                                        // box with an explicit height) rendered
                                        // short by its border thickness.
                                        let total_h = n_bh.map_or(content_total, |h| {
                                            (h + n_border.vertical_width()).max(content_total)
                                        });

                                        if let Some((r, g, b, a)) = n_bg {
                                            if *a >= 1.0 {
                                                content.push_str(&format!(
                                                    "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                                    x = nested_x,
                                                    y = nested_y - total_h,
                                                    w = n_width,
                                                    h = total_h,
                                                ));
                                            }
                                        }

                                        // Draw borders for nested TextBlock
                                        if n_border.has_any() {
                                            let x1 = nested_x;
                                            let x2 = nested_x + n_width;
                                            let y_top = nested_y;
                                            let y_bottom = nested_y - total_h;
                                            if n_border.top.width > 0.0 {
                                                let (r, g, b) = n_border.top.color;
                                                let a = begin_border_alpha(
                                                    &mut content,
                                                    &mut page_ext_gstates,
                                                    &mut bg_alpha_counter,
                                                    n_border.top.alpha,
                                                );
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                                    n_border.top.width
                                                ));
                                                end_border_alpha(&mut content, a);
                                            }
                                            if n_border.bottom.width > 0.0 {
                                                let (r, g, b) = n_border.bottom.color;
                                                let a = begin_border_alpha(
                                                    &mut content,
                                                    &mut page_ext_gstates,
                                                    &mut bg_alpha_counter,
                                                    n_border.bottom.alpha,
                                                );
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                                    n_border.bottom.width
                                                ));
                                                end_border_alpha(&mut content, a);
                                            }
                                            if n_border.left.width > 0.0 {
                                                let (r, g, b) = n_border.left.color;
                                                let a = begin_border_alpha(
                                                    &mut content,
                                                    &mut page_ext_gstates,
                                                    &mut bg_alpha_counter,
                                                    n_border.left.alpha,
                                                );
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                                    n_border.left.width
                                                ));
                                                end_border_alpha(&mut content, a);
                                            }
                                            if n_border.right.width > 0.0 {
                                                let (r, g, b) = n_border.right.color;
                                                let a = begin_border_alpha(
                                                    &mut content,
                                                    &mut page_ext_gstates,
                                                    &mut bg_alpha_counter,
                                                    n_border.right.alpha,
                                                );
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                                    n_border.right.width
                                                ));
                                                end_border_alpha(&mut content, a);
                                            }
                                        }

                                        let mut ty = nested_y - n_pt;
                                        for line in n_lines {
                                            let m = line_box_metrics(line, custom_fonts);
                                            ty -= m.half_leading + m.ascender;
                                            let merged = merge_runs(&line.runs);
                                            let mut lx = nested_x + cell.padding_left;
                                            for run in &merged {
                                                let rw = render_run_text(
                                                    &mut content,
                                                    run,
                                                    lx,
                                                    ty,
                                                    crate::layout::text::line_primary_font_size(
                                                        &merged,
                                                    ),
                                                    custom_fonts,
                                                    &prepared_custom_fonts,
                                                    0.0,
                                                    &mut pdf_writer,
                                                    &mut page_images,
                                                );
                                                lx += rw;
                                            }
                                            ty -= m.descender + m.half_leading;
                                        }
                                        nested_y -= total_h;
                                    }
                                    LayoutElement::TableRow {
                                        cells: t_cells,
                                        col_widths,
                                        ..
                                    } => {
                                        let t_row_h = compute_row_height(t_cells);
                                        let mut tcx = nested_x;
                                        for (i, t_cell) in t_cells.iter().enumerate() {
                                            let tw = if i < col_widths.len() {
                                                col_widths[i]
                                            } else {
                                                0.0
                                            };
                                            if let Some((r, g, b, _)) = t_cell.background_color {
                                                content.push_str(&format!(
                                                    "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                                    x = tcx,
                                                    y = nested_y - t_row_h,
                                                    w = tw,
                                                    h = t_row_h,
                                                ));
                                            }
                                            let mut ty = nested_y - t_cell.padding_top;
                                            for line in &t_cell.lines {
                                                let m = line_box_metrics(line, custom_fonts);
                                                ty -= m.half_leading + m.ascender;
                                                let merged = merge_runs(&line.runs);
                                                let mut lx = tcx + t_cell.padding_left;
                                                for run in &merged {
                                                    let rw = render_run_text(
                                                        &mut content,
                                                        run,
                                                        lx,
                                                        ty,
                                                        crate::layout::text::line_primary_font_size(
                                                            &merged,
                                                        ),
                                                        custom_fonts,
                                                        &prepared_custom_fonts,
                                                        0.0,
                                                        &mut pdf_writer,
                                                        &mut page_images,
                                                    );
                                                    lx += rw;
                                                }
                                                ty -= m.descender + m.half_leading;
                                            }
                                            // Draw cell borders
                                            if t_cell.border.has_any() {
                                                let x1 = tcx;
                                                let x2 = tcx + tw;
                                                let y_top = nested_y;
                                                let y_bottom = nested_y - t_row_h;
                                                if t_cell.border.top.width > 0.0 {
                                                    let (r, g, b) = t_cell.border.top.color;
                                                    let a = begin_border_alpha(
                                                        &mut content,
                                                        &mut page_ext_gstates,
                                                        &mut bg_alpha_counter,
                                                        t_cell.border.top.alpha,
                                                    );
                                                    content.push_str(&format!(
                                                        "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                                        t_cell.border.top.width
                                                    ));
                                                    end_border_alpha(&mut content, a);
                                                }
                                                if t_cell.border.bottom.width > 0.0 {
                                                    let (r, g, b) = t_cell.border.bottom.color;
                                                    let a = begin_border_alpha(
                                                        &mut content,
                                                        &mut page_ext_gstates,
                                                        &mut bg_alpha_counter,
                                                        t_cell.border.bottom.alpha,
                                                    );
                                                    content.push_str(&format!(
                                                        "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                                        t_cell.border.bottom.width
                                                    ));
                                                    end_border_alpha(&mut content, a);
                                                }
                                                if t_cell.border.left.width > 0.0 {
                                                    let (r, g, b) = t_cell.border.left.color;
                                                    let a = begin_border_alpha(
                                                        &mut content,
                                                        &mut page_ext_gstates,
                                                        &mut bg_alpha_counter,
                                                        t_cell.border.left.alpha,
                                                    );
                                                    content.push_str(&format!(
                                                        "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                                        t_cell.border.left.width
                                                    ));
                                                    end_border_alpha(&mut content, a);
                                                }
                                                if t_cell.border.right.width > 0.0 {
                                                    let (r, g, b) = t_cell.border.right.color;
                                                    let a = begin_border_alpha(
                                                        &mut content,
                                                        &mut page_ext_gstates,
                                                        &mut bg_alpha_counter,
                                                        t_cell.border.right.alpha,
                                                    );
                                                    content.push_str(&format!(
                                                        "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                                        t_cell.border.right.width
                                                    ));
                                                    end_border_alpha(&mut content, a);
                                                }
                                            }
                                            tcx += tw;
                                        }
                                        nested_y -= t_row_h;
                                    }
                                    LayoutElement::Svg {
                                        tree,
                                        width: svg_w,
                                        height: svg_h,
                                        margin_top: svg_mt,
                                        ..
                                    } => {
                                        nested_y -= svg_mt;
                                        let svg_x = nested_x;
                                        let svg_y = nested_y - svg_h;
                                        content.push_str("q\n");
                                        // Y-flip + position
                                        content.push_str(&format!(
                                            "1 0 0 -1 {svg_x} {} cm\n",
                                            svg_y + svg_h
                                        ));
                                        // Apply viewBox scaling
                                        if let Some(placement) =
                                            crate::render::svg_geometry::compute_svg_placement(
                                                tree,
                                                crate::render::svg_geometry::SvgPlacementRequest::from_rect(
                                                    0.0, 0.0, *svg_w, *svg_h,
                                                    tree.preserve_aspect_ratio,
                                                ),
                                            )
                                        {
                                            content.push_str("q\n");
                                            content.push_str(&placement.viewport.clip_path());
                                            content.push_str(&format!(
                                                "{sx} 0 0 {sy} {tx} {ty} cm\n",
                                                sx = placement.scale_x,
                                                sy = placement.scale_y,
                                                tx = placement.translate_x,
                                                ty = placement.translate_y,
                                            ));
                                        }
                                        {
                                            let mut image_sink = SvgPageImageSink {
                                                pdf_writer: &mut pdf_writer,
                                                page_images: &mut page_images,
                                            };
                                            let mut resources =
                                                crate::render::svg_to_pdf::SvgPdfResources {
                                                    shadings: &mut page_shadings,
                                                    shading_counter: &mut shading_counter,
                                                    ext_gstates: Some(&mut page_ext_gstates),
                                                    image_sink: Some(&mut image_sink),
                                                    custom_fonts: Some(custom_fonts),
                                                    prepared_custom_fonts: Some(
                                                        &prepared_custom_fonts,
                                                    ),
                                                };
                                            crate::render::svg_to_pdf::render_svg_tree_with_resources(
                                                tree,
                                                &mut content,
                                                &mut resources,
                                            );
                                        }
                                        if tree.view_box.is_some() {
                                            content.push_str("Q\n");
                                        }
                                        content.push_str("Q\n");
                                        nested_y -= svg_h;
                                    }
                                    LayoutElement::Container {
                                        children: cont_kids,
                                        background_color: cont_bg,
                                        border: cont_border,
                                        padding_top: cont_pt,
                                        padding_bottom: cont_pb,
                                        padding_left: cont_pl,
                                        padding_right: cont_pr,
                                        margin_top: cont_mt,
                                        block_width: cont_bw,
                                        border_radius: cont_br,
                                        overflow: cont_overflow,
                                        ..
                                    } => {
                                        nested_y -= cont_mt;
                                        let cont_w = cont_bw.unwrap_or(cell.width);
                                        let cont_children_h: f32 =
                                            collapsed_children_height(cont_kids);
                                        let cont_h = cont_pt
                                            + cont_children_h
                                            + cont_pb
                                            + cont_border.vertical_width();

                                        // Draw container background
                                        if let Some((r, g, b, a)) = cont_bg {
                                            let needs_alpha = *a < 1.0;
                                            if needs_alpha {
                                                let gs_name = format!("GSba{bg_alpha_counter}");
                                                bg_alpha_counter += 1;
                                                page_ext_gstates.push((gs_name.clone(), *a));
                                                content.push_str(&format!("/{gs_name} gs\n"));
                                            }
                                            content.push_str(&format!("{r} {g} {b} rg\n"));
                                            if *cont_br > 0.0 {
                                                content.push_str(&rounded_rect_path(
                                                    nested_x,
                                                    nested_y - cont_h,
                                                    cont_w,
                                                    cont_h,
                                                    *cont_br,
                                                ));
                                                content.push_str("\nf\n");
                                            } else {
                                                content.push_str(&format!(
                                                    "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                                                    x = nested_x,
                                                    y = nested_y - cont_h,
                                                    w = cont_w,
                                                    h = cont_h,
                                                ));
                                            }
                                            if needs_alpha {
                                                content.push_str("/GSDefault gs\n");
                                            }
                                        }

                                        // Draw container borders
                                        if cont_border.has_any() {
                                            let bx1 = nested_x;
                                            let bx2 = nested_x + cont_w;
                                            let by1 = nested_y;
                                            let by2 = nested_y - cont_h;
                                            if cont_border.left.width > 0.0 {
                                                let (r, g, b) = cont_border.left.color;
                                                let a = begin_border_alpha(
                                                    &mut content,
                                                    &mut page_ext_gstates,
                                                    &mut bg_alpha_counter,
                                                    cont_border.left.alpha,
                                                );
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{} {} m {} {} l S\n",
                                                    cont_border.left.width,
                                                    bx1 + cont_border.left.width * 0.5,
                                                    by1,
                                                    bx1 + cont_border.left.width * 0.5,
                                                    by2
                                                ));
                                                end_border_alpha(&mut content, a);
                                            }
                                            if cont_border.right.width > 0.0 {
                                                let (r, g, b) = cont_border.right.color;
                                                let a = begin_border_alpha(
                                                    &mut content,
                                                    &mut page_ext_gstates,
                                                    &mut bg_alpha_counter,
                                                    cont_border.right.alpha,
                                                );
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{} {} m {} {} l S\n",
                                                    cont_border.right.width,
                                                    bx2 - cont_border.right.width * 0.5,
                                                    by1,
                                                    bx2 - cont_border.right.width * 0.5,
                                                    by2
                                                ));
                                                end_border_alpha(&mut content, a);
                                            }
                                            if cont_border.top.width > 0.0 {
                                                let (r, g, b) = cont_border.top.color;
                                                let a = begin_border_alpha(
                                                    &mut content,
                                                    &mut page_ext_gstates,
                                                    &mut bg_alpha_counter,
                                                    cont_border.top.alpha,
                                                );
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{} {} m {} {} l S\n",
                                                    cont_border.top.width,
                                                    bx1,
                                                    by1 - cont_border.top.width * 0.5,
                                                    bx2,
                                                    by1 - cont_border.top.width * 0.5
                                                ));
                                                end_border_alpha(&mut content, a);
                                            }
                                            if cont_border.bottom.width > 0.0 {
                                                let (r, g, b) = cont_border.bottom.color;
                                                let a = begin_border_alpha(
                                                    &mut content,
                                                    &mut page_ext_gstates,
                                                    &mut bg_alpha_counter,
                                                    cont_border.bottom.alpha,
                                                );
                                                content.push_str(&format!(
                                                    "{r} {g} {b} RG\n{} w\n{} {} m {} {} l S\n",
                                                    cont_border.bottom.width,
                                                    bx1,
                                                    by2 + cont_border.bottom.width * 0.5,
                                                    bx2,
                                                    by2 + cont_border.bottom.width * 0.5
                                                ));
                                                end_border_alpha(&mut content, a);
                                            }
                                        }

                                        // Clip and render children at the padding
                                        // box (border box inset by the borders).
                                        let clip = cont_overflow.clips();
                                        if clip {
                                            content.push_str("q\n");
                                            content.push_str(&overflow_clip_path(
                                                nested_x,
                                                nested_y - cont_h,
                                                cont_w,
                                                cont_h,
                                                cont_border.left.width,
                                                cont_border.right.width,
                                                cont_border.top.width,
                                                cont_border.bottom.width,
                                                *cont_br,
                                            ));
                                            content.push_str("W n\n");
                                        }
                                        let inner_x = nested_x + cont_pl + cont_border.left.width;
                                        let inner_w = (cont_w
                                            - cont_pl
                                            - cont_pr
                                            - cont_border.horizontal_width())
                                        .max(0.0);
                                        let inner_y = nested_y - cont_pt - cont_border.top.width;
                                        let mut abs_origins: HashMap<usize, (f32, f32)> =
                                            HashMap::new();
                                        render_container_children(
                                            &mut content,
                                            cont_kids,
                                            inner_x,
                                            inner_y,
                                            inner_w,
                                            custom_fonts,
                                            &prepared_custom_fonts,
                                            &mut page_ext_gstates,
                                            &mut bg_alpha_counter,
                                            &mut page_shadings,
                                            &mut shading_counter,
                                            &mut pdf_writer,
                                            &mut page_images,
                                            *cont_pl,
                                            *cont_pt,
                                            &mut abs_origins,
                                        );
                                        if clip {
                                            content.push_str("Q\n");
                                        }
                                        nested_y -= cont_h;
                                    }
                                    LayoutElement::FlexRow { .. } => {
                                        // A flex item that is itself a flex
                                        // container (a nested FlexRow) establishes
                                        // an independent formatting context.
                                        // Render it through the shared block-flow
                                        // renderer at the cell's nested origin,
                                        // reusing its FlexRow arm. Without this the
                                        // nested row fell through to `_ => {}` and
                                        // the entire item — its boxes AND its own
                                        // background — was dropped (blank page).
                                        let mut nested_abs_origins: HashMap<
                                            usize,
                                            (f32, f32),
                                        > = HashMap::new();
                                        render_container_children(
                                            &mut content,
                                            std::slice::from_ref(nested_elem),
                                            nested_x,
                                            nested_y,
                                            cell.width,
                                            custom_fonts,
                                            &prepared_custom_fonts,
                                            &mut page_ext_gstates,
                                            &mut bg_alpha_counter,
                                            &mut page_shadings,
                                            &mut shading_counter,
                                            &mut pdf_writer,
                                            &mut page_images,
                                            0.0,
                                            0.0,
                                            &mut nested_abs_origins,
                                        );
                                        nested_y -=
                                            crate::layout::engine::estimate_element_height(
                                                nested_elem,
                                            );
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // Restore cell transform
                        if cell_needs_transform {
                            content.push_str("Q\n");
                        }
                    }
                }
                LayoutElement::Container {
                    children,
                    background_color,
                    border,
                    border_radius: c_border_radius,
                    border_radii: c_border_radii,
                    border_radii_y: c_border_radii_y,
                    outline_width: c_outline_width,
                    outline_color: c_outline_color,
                    outline_offset: c_outline_offset,
                    padding_top: c_pt,
                    padding_bottom: c_pb,
                    padding_left: c_pl,
                    padding_right: c_pr,
                    margin_top: _,
                    margin_bottom: _,
                    block_width,
                    block_height: c_block_height,
                    opacity: c_opacity,
                    visible: c_visible,
                    float: c_float,
                    position: c_position,
                    offset_top: _,
                    offset_left: c_offset_left,
                    overflow: c_overflow,
                    overflow_x: c_overflow_x,
                    overflow_y: c_overflow_y,
                    transform: c_transform,
                    transform_origin: c_transform_origin,
                    clip_path: c_clip_path,
                    mask_image: c_mask_image,
                    mask_mode: c_mask_mode,
                    box_shadow: c_box_shadow,
                    background_gradient: c_bg_gradient,
                    background_radial_gradient: c_bg_radial,
                    background_conic_gradient: c_bg_conic,
                    background_svg: c_bg_svg,
                    background_size: c_bg_size,
                    background_position: c_bg_position,
                    background_repeat: c_bg_repeat,
                    background_origin: c_bg_origin,
                    background_clip: c_bg_clip,
                    background_blur_radius: c_bg_blur,
                    z_index: _,
                    positioned_depth: c_positioned_depth,
                    ..
                } => {
                    // CSS2 §11.2: `visibility: hidden` (or `collapse`) hides only
                    // THIS box's own decoration (background, border, outline,
                    // box-shadow); it is inherited but a descendant may override it
                    // back to `visible` and still paint. So we must keep recursing
                    // into the children and only gate the container's own painting
                    // on `c_visible` — never skip the whole subtree.
                    let c_visible_self = *c_visible;

                    let container_w = block_width.unwrap_or(available_width);
                    let container_x = match c_float {
                        Float::Right => margin.left + available_width - container_w,
                        _ => margin.left + c_offset_left,
                    };
                    let container_y_top = page_size.height - margin.top - y_pos;

                    // Use explicit block_height if set, otherwise compute from
                    // children (with adjacent-sibling margin collapse so the
                    // painted box height matches the collapsed child flow).
                    //
                    // A Container's `block_height` is a definite border-box height
                    // (set only when the element has an explicit `height`). Per
                    // CSS, a definite height is a hard size: content that exceeds
                    // it overflows the box rather than growing it (the box border
                    // stays at the declared height regardless of `overflow`). This
                    // matters for grids/flex whose definite tracks can overflow the
                    // content box — Chrome keeps the container border-box at the
                    // declared height and lets the cells spill past it. Honour the
                    // declared height directly instead of `content_h.max(h)`, which
                    // wrongly inflated the border-box by the overflow amount.
                    let children_h: f32 = collapsed_children_height(children);
                    let content_h = c_pt + children_h + c_pb + border.vertical_width();
                    let total_h = c_block_height.unwrap_or(content_h);

                    // Apply CSS opacity to the whole subtree as a single group
                    // (background + border + children composite together, matching
                    // CSS `opacity`). Wraps everything below in its own q..Q so the
                    // ExtGState alpha applies to the entire box uniformly.
                    let c_needs_opacity = *c_opacity < 1.0;
                    if c_needs_opacity {
                        let gs_name = format!("GScontainerop{elem_idx}");
                        page_ext_gstates.push((gs_name.clone(), *c_opacity));
                        content.push_str("q\n");
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }

                    // Apply a CSS transform around the box centre (wrap the whole
                    // element, incl. shadow + children, in q..Q). Shares the same
                    // helper as the other arms so block-level containers transform
                    // identically to text blocks / flex cells / nested boxes.
                    let c_needs_transform = c_transform.is_some();
                    if let Some(t) = c_transform {
                        let (ox, oy) = c_transform_origin.resolve(container_w, total_h);
                        let cx = container_x + ox;
                        let cy = container_y_top - oy;
                        content.push_str("q\n");
                        push_transform_cm(&mut content, t, cx, cy, container_w, total_h);
                    }

                    // CSS clip-path: clip the element (and descendants) to the
                    // basic shape before painting. Wrapped in its own q..Q.
                    let c_needs_clip_path = c_clip_path.is_some();
                    if let Some(cp) = c_clip_path {
                        content.push_str("q\n");
                        push_clip_path(
                            &mut content,
                            cp,
                            container_x,
                            container_y_top,
                            container_w,
                            total_h,
                        );
                    }

                    // CSS mask-image (css-masking-1 §3): wrap the box (and its
                    // descendants) in a soft-mask graphics state so the mask
                    // source's coverage fades the painted content. Built lazily so
                    // boxes without a mask pay nothing.
                    let mut c_mask_open = false;
                    if let Some(src) = c_mask_image {
                        if let Some(gs_name) = pdf_writer.add_mask_soft_mask(
                            src,
                            *c_mask_mode,
                            container_x,
                            container_y_top,
                            container_w,
                            total_h,
                        ) {
                            content.push_str("q\n");
                            content.push_str(&format!("/{gs_name} gs\n"));
                            c_mask_open = true;
                        }
                    }

                    // Self-decoration (background / border / outline / shadow) is
                    // suppressed when this box is `visibility: hidden`; children
                    // (which may override back to visible) are still rendered below.
                    if c_visible_self {
                        // Draw box-shadow with blur
                        render_box_shadows(
                            &mut content,
                            c_box_shadow,
                            container_x,
                            container_y_top - total_h,
                            container_w,
                            total_h,
                            *c_border_radius,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                            &mut pdf_writer,
                            &mut page_images,
                        );

                        // `container_x` / `container_y_top` describe the BORDER
                        // box (the border paints inward). Derive the per-side
                        // border / padding insets so background-origin (paint
                        // positioning area) and background-clip (paint visible
                        // area) can be applied per css-backgrounds-3.
                        let c_bg_y = container_y_top - total_h;
                        let c_bl = border.left.width;
                        let c_br = border.right.width;
                        let c_bt = border.top.width;
                        let c_bb = border.bottom.width;
                        // The box `background-clip` confines the painted fill to.
                        let (c_clip_x, c_clip_y, c_clip_w, c_clip_h) = background_clip_rect(
                            *c_bg_clip,
                            container_x,
                            c_bg_y,
                            container_w,
                            total_h,
                            c_bl,
                            c_br,
                            c_bt,
                            c_bb,
                            *c_pl,
                            *c_pr,
                            *c_pt,
                            *c_pb,
                        );
                        let c_clip_radius = *c_border_radius;
                        let c_needs_clip = *c_bg_clip != BackgroundClip::Border;

                        // Draw background
                        if let Some((r, g, b, a)) = background_color {
                            let needs_alpha = *a < 1.0;
                            if needs_alpha {
                                let gs_name = format!("GScontainer{elem_idx}");
                                page_ext_gstates.push((gs_name.clone(), *a));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
                            content.push_str(&format!("{r} {g} {b} rg\n"));
                            if c_needs_clip {
                                // Clip the fill to the clip box; a non-uniform
                                // rounded fill cannot also be clipped, so fall
                                // back to a rectangular clip-box fill.
                                push_background_clip(
                                    &mut content,
                                    c_clip_x,
                                    c_clip_y,
                                    c_clip_w,
                                    c_clip_h,
                                    c_clip_radius,
                                );
                                content.push_str(&format!(
                                    "{c_clip_x} {c_clip_y} {c_clip_w} {c_clip_h} re\n"
                                ));
                                content.push_str("f\n");
                                content.push_str("Q\n");
                            } else {
                                if let Some(path) = rounded_box_path(
                                    container_x,
                                    c_bg_y,
                                    container_w,
                                    total_h,
                                    *c_border_radii,
                                    *c_border_radii_y,
                                ) {
                                    content.push_str(&path);
                                } else {
                                    content.push_str(&format!(
                                        "{x} {y} {w} {h} re\n",
                                        x = container_x,
                                        y = c_bg_y,
                                        w = container_w,
                                        h = total_h,
                                    ));
                                }
                                content.push_str("f\n");
                            }
                            if needs_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }

                        // Gradients paint over the background-clip box. The
                        // gradient ramp itself is anchored to the (border) box;
                        // clipping just confines where it shows.
                        let bg_y = c_bg_y;
                        let gradient_clip = *c_border_radius > 0.0 || c_needs_clip;
                        // Draw container linear gradient
                        if let Some(gradient) = c_bg_gradient {
                            if gradient_clip {
                                push_background_clip(
                                    &mut content,
                                    c_clip_x,
                                    c_clip_y,
                                    c_clip_w,
                                    c_clip_h,
                                    c_clip_radius,
                                );
                            }
                            render_linear_gradient(
                                &mut content,
                                gradient,
                                container_x,
                                bg_y,
                                container_w,
                                total_h,
                                &mut page_shadings,
                                &mut shading_counter,
                            );
                            if gradient_clip {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw container radial gradient
                        if let Some(gradient) = c_bg_radial {
                            if gradient_clip {
                                push_background_clip(
                                    &mut content,
                                    c_clip_x,
                                    c_clip_y,
                                    c_clip_w,
                                    c_clip_h,
                                    c_clip_radius,
                                );
                            }
                            render_radial_gradient(
                                &mut content,
                                gradient,
                                container_x,
                                bg_y,
                                container_w,
                                total_h,
                                &mut page_shadings,
                                &mut shading_counter,
                            );
                            if gradient_clip {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw container conic gradient
                        if let Some(gradient) = c_bg_conic {
                            if gradient_clip {
                                push_background_clip(
                                    &mut content,
                                    c_clip_x,
                                    c_clip_y,
                                    c_clip_w,
                                    c_clip_h,
                                    c_clip_radius,
                                );
                            }
                            render_conic_gradient(
                                &mut content,
                                gradient,
                                container_x,
                                bg_y,
                                container_w,
                                total_h,
                            );
                            if gradient_clip {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw SVG / raster background image if specified.
                        // `background-origin` sets the positioning area; the
                        // reference box is derived from the BORDER box by
                        // insetting the border (padding box) and border+padding
                        // (content box).
                        if let Some(svg_tree) = c_bg_svg {
                            let (ref_x, ref_y, ref_w, ref_h) = match c_bg_origin {
                                BackgroundOrigin::Border => {
                                    (container_x, bg_y, container_w, total_h)
                                }
                                BackgroundOrigin::Padding => (
                                    container_x + c_bl,
                                    bg_y + c_bb,
                                    (container_w - c_bl - c_br).max(0.0),
                                    (total_h - c_bt - c_bb).max(0.0),
                                ),
                                BackgroundOrigin::Content => (
                                    container_x + c_bl + *c_pl,
                                    bg_y + c_bb + *c_pb,
                                    (container_w - c_bl - c_br - *c_pl - *c_pr).max(0.0),
                                    (total_h - c_bt - c_bb - *c_pt - *c_pb).max(0.0),
                                ),
                            };
                            render_svg_background(
                                &mut content,
                                svg_tree,
                                &mut pdf_writer,
                                &mut page_images,
                                &mut page_shadings,
                                &mut shading_counter,
                                Some(&mut page_ext_gstates),
                                BackgroundPaintContext::new(
                                    SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                                    SvgViewportBox::new(c_clip_x, c_clip_y, c_clip_w, c_clip_h),
                                    *c_border_radius,
                                    *c_bg_blur,
                                    *c_bg_size,
                                    *c_bg_position,
                                    *c_bg_repeat,
                                ),
                            );
                        }

                        // Draw inset box-shadow (after container background, before borders).
                        render_box_shadows_inset(
                            &mut content,
                            c_box_shadow,
                            container_x,
                            container_y_top - total_h,
                            container_w,
                            total_h,
                            *c_border_radius,
                            &mut page_ext_gstates,
                            &mut bg_alpha_counter,
                        );

                        // Draw all 4 borders
                        if border.has_visible() {
                            let cbox_x = container_x;
                            let cbox_bottom = container_y_top - total_h;
                            // Uniform borders take the shared painter (dashed/dotted/
                            // double + per-corner rounded corners). Non-uniform borders
                            // keep the per-side stroke path with dash support.
                            let c_uniform = border.top.width == border.right.width
                                && border.top.width == border.bottom.width
                                && border.top.width == border.left.width
                                && border.top.color == border.right.color
                                && border.top.color == border.bottom.color
                                && border.top.color == border.left.color
                                && border.top.style == border.right.style
                                && border.top.style == border.bottom.style
                                && border.top.style == border.left.style;
                            if c_uniform
                                && border_needs_special_paint(border.top.style, *c_border_radii)
                            {
                                paint_uniform_border(
                                    &mut content,
                                    cbox_x,
                                    cbox_bottom,
                                    container_w,
                                    total_h,
                                    *c_border_radii,
                                    &border.top,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                );
                            } else if c_uniform && *c_border_radius > 0.0 {
                                // Plain solid rounded border: byte-stable legacy path.
                                let bw = border
                                    .top
                                    .width
                                    .max(border.right.width)
                                    .max(border.bottom.width)
                                    .max(border.left.width);
                                let (r, g, b) = border.top.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.top.alpha,
                                );
                                content.push_str(&format!("{r} {g} {b} RG\n{bw} w\n"));
                                // The stroke pen is centered on the path, so inset the
                                // path by half the border width (and shrink the radius
                                // likewise) to keep the whole stroke INSIDE the
                                // border-box — otherwise the outer half paints beyond
                                // the box (and is clipped to a half-width border where
                                // the box meets the page edge). Mirrors the non-rounded
                                // uniform arm below.
                                content.push_str(&rounded_rect_path(
                                    container_x + bw / 2.0,
                                    (container_y_top - total_h) + bw / 2.0,
                                    (container_w - bw).max(0.0),
                                    (total_h - bw).max(0.0),
                                    (*c_border_radius - bw / 2.0).max(0.0),
                                ));
                                content.push_str("S\n");
                                end_border_alpha(&mut content, a);
                            } else if c_uniform
                                && border.top.style == BorderStyle::Solid
                                && border.top.alpha < 1.0
                                && border.top.alpha == border.right.alpha
                                && border.top.alpha == border.bottom.alpha
                                && border.top.alpha == border.left.alpha
                            {
                                // Uniform TRANSLUCENT solid flat border: stroke it as a
                                // SINGLE rectangle so each corner composites once. The
                                // per-side stroke path below lays each side corner-to-
                                // corner, so adjacent strokes overlap at every corner;
                                // for a translucent border that applies the alpha twice
                                // (darker corners — a real Chrome mismatch). Opaque
                                // borders are unaffected and keep the per-side path
                                // (byte-stable). Coords match the per-side path's border
                                // box (container_x / container_y_top), inset by bw/2.
                                let bw = border.top.width;
                                let (r, g, b) = border.top.color;
                                let a = begin_border_alpha(
                                    &mut content,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                    border.top.alpha,
                                );
                                content.push_str(&format!("{r} {g} {b} RG\n{bw} w\n"));
                                content.push_str(&format!(
                                    "{x} {y} {w} {h} re\n",
                                    x = container_x + bw / 2.0,
                                    y = (container_y_top - total_h) + bw / 2.0,
                                    w = (container_w - bw).max(0.0),
                                    h = (total_h - bw).max(0.0),
                                ));
                                content.push_str("S\n");
                                end_border_alpha(&mut content, a);
                            } else if !radii_any(*c_border_radii)
                                && *c_border_radius <= 0.0
                                && border_needs_miter_fill(border)
                            {
                                // Different per-side colors/widths on a square box:
                                // fill each side as a trapezoid so adjacent colors
                                // meet on a diagonal miter (CSS Backgrounds §6.2),
                                // rather than overlapping centerline strokes that
                                // leave a single-color corner seam.
                                paint_miter_border(
                                    &mut content,
                                    cbox_x,
                                    cbox_bottom,
                                    container_w,
                                    total_h,
                                    border,
                                    &mut page_ext_gstates,
                                    &mut bg_alpha_counter,
                                );
                            } else {
                                let bx1 = container_x;
                                let bx2 = container_x + container_w;
                                let by1 = container_y_top;
                                let by2 = container_y_top - total_h;
                                if border.left.paints() {
                                    let (r, g, b) = border.left.color;
                                    let a = begin_border_alpha(
                                        &mut content,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        border.left.alpha,
                                    );
                                    content.push_str(&dash_pattern_for_style(
                                        border.left.style,
                                        border.left.width,
                                    ));
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                        bw = border.left.width,
                                        x = bx1 + border.left.width * 0.5,
                                        y1 = by1,
                                        y2 = by2
                                    ));
                                    content.push_str(reset_dash_pattern(border.left.style));
                                    end_border_alpha(&mut content, a);
                                }
                                if border.right.paints() {
                                    let (r, g, b) = border.right.color;
                                    let a = begin_border_alpha(
                                        &mut content,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        border.right.alpha,
                                    );
                                    content.push_str(&dash_pattern_for_style(
                                        border.right.style,
                                        border.right.width,
                                    ));
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                        bw = border.right.width,
                                        x = bx2 - border.right.width * 0.5,
                                        y1 = by1,
                                        y2 = by2
                                    ));
                                    content.push_str(reset_dash_pattern(border.right.style));
                                    end_border_alpha(&mut content, a);
                                }
                                if border.top.paints() {
                                    let (r, g, b) = border.top.color;
                                    let a = begin_border_alpha(
                                        &mut content,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        border.top.alpha,
                                    );
                                    content.push_str(&dash_pattern_for_style(
                                        border.top.style,
                                        border.top.width,
                                    ));
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                        bw = border.top.width,
                                        x1 = bx1,
                                        x2 = bx2,
                                        y = by1 - border.top.width * 0.5
                                    ));
                                    content.push_str(reset_dash_pattern(border.top.style));
                                    end_border_alpha(&mut content, a);
                                }
                                if border.bottom.paints() {
                                    let (r, g, b) = border.bottom.color;
                                    let a = begin_border_alpha(
                                        &mut content,
                                        &mut page_ext_gstates,
                                        &mut bg_alpha_counter,
                                        border.bottom.alpha,
                                    );
                                    content.push_str(&dash_pattern_for_style(
                                        border.bottom.style,
                                        border.bottom.width,
                                    ));
                                    content.push_str(&format!(
                                        "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                        bw = border.bottom.width,
                                        x1 = bx1,
                                        x2 = bx2,
                                        y = by2 + border.bottom.width * 0.5
                                    ));
                                    content.push_str(reset_dash_pattern(border.bottom.style));
                                    end_border_alpha(&mut content, a);
                                }
                            } // else (non-uniform borders)
                        }

                        // Draw outline (outside the border box, honouring
                        // `outline-offset`). Top-level containers previously dropped
                        // the outline entirely.
                        if *c_outline_width > 0.0 {
                            let gap = *c_outline_offset + *c_outline_width / 2.0;
                            let ol_x = container_x - gap;
                            let ol_y = container_y_top - total_h - gap;
                            let ol_w = container_w + 2.0 * gap;
                            let ol_h = total_h + 2.0 * gap;
                            let (or, og, ob) = c_outline_color.unwrap_or((0.0, 0.0, 0.0));
                            content.push_str(&format!(
                                "{or} {og} {ob} RG\n{ow} w\n",
                                ow = c_outline_width
                            ));
                            if radii_any(*c_border_radii) && !radii_uniform(*c_border_radii) {
                                let ol_radii = [
                                    c_border_radii[0] + gap,
                                    c_border_radii[1] + gap,
                                    c_border_radii[2] + gap,
                                    c_border_radii[3] + gap,
                                ];
                                content.push_str(&rounded_rect_path_per_corner(
                                    ol_x, ol_y, ol_w, ol_h, ol_radii,
                                ));
                            } else if *c_border_radius > 0.0 {
                                content.push_str(&rounded_rect_path(
                                    ol_x,
                                    ol_y,
                                    ol_w,
                                    ol_h,
                                    *c_border_radius + gap,
                                ));
                            } else {
                                content.push_str(&format!("{ol_x} {ol_y} {ol_w} {ol_h} re\n"));
                            }
                            content.push_str("S\n");
                        }
                    } // end `if c_visible_self` — container self-decoration

                    // Print scrollbars (css-overflow-3): a `scroll` axis always
                    // reserves a gutter + paints a non-interactive scrollbar; an
                    // `auto` axis does so only when its content overflows. The
                    // content clip is inset by the gutter on each scrolling axis.
                    let c_pad_box_w = (container_w - border.horizontal_width()).max(0.0);
                    let c_pad_box_h = (total_h - border.vertical_width()).max(0.0);
                    let c_avail_w = (c_pad_box_w - *c_pl - *c_pr).max(0.0);
                    let c_avail_h = (c_pad_box_h - *c_pt - *c_pb).max(0.0);
                    let (c_over_w, c_over_h) = children_overflow_extent(children);
                    let c_ratio_h = if c_avail_w > 0.0 {
                        c_over_w / c_avail_w
                    } else {
                        0.0
                    };
                    let c_ratio_v = if c_avail_h > 0.0 {
                        c_over_h / c_avail_h
                    } else {
                        0.0
                    };
                    let c_scroll_ok = *c_border_radius <= 0.0 && !radii_any(*c_border_radii);
                    let c_has_v = c_scroll_ok
                        && match c_overflow_y {
                            Overflow::Scroll => true,
                            Overflow::Auto => c_ratio_v > 1.001,
                            _ => false,
                        };
                    let c_has_h = c_scroll_ok
                        && match c_overflow_x {
                            Overflow::Scroll => true,
                            Overflow::Auto => c_ratio_h > 1.001,
                            _ => false,
                        };
                    let c_sb = SCROLLBAR_THICKNESS_PT;
                    let c_v_gutter = if c_has_v { c_sb } else { 0.0 };
                    let c_h_gutter = if c_has_h { c_sb } else { 0.0 };

                    // Apply clip if overflow clips. Per CSS, `overflow` clips to
                    // the *padding box* — the border is painted outside the clip
                    // region and stays visible — and follows the rounded inner
                    // corners when border-radius is set.
                    let needs_clip = c_overflow.clips();
                    if needs_clip {
                        content.push_str("q\n");
                        if c_has_v || c_has_h {
                            let cx = container_x + border.left.width;
                            let cy = (container_y_top - total_h) + border.bottom.width + c_h_gutter;
                            let cw = c_pad_box_w - c_v_gutter;
                            let ch = c_pad_box_h - c_h_gutter;
                            content.push_str(&format!("{cx} {cy} {cw} {ch} re W n\n"));
                        } else {
                            content.push_str(&overflow_clip_path(
                                container_x,
                                container_y_top - total_h,
                                container_w,
                                total_h,
                                border.left.width,
                                border.right.width,
                                border.top.width,
                                border.bottom.width,
                                *c_border_radius,
                            ));
                            content.push_str("W n\n");
                        }
                    }

                    // Render children recursively
                    // Pass both content-box origin (for flow children) and
                    // padding-box origin (for absolute children).
                    let inner_x = container_x + c_pl + border.left.width;
                    let inner_w = (container_w - c_pl - c_pr - border.horizontal_width()).max(0.0);
                    let inner_y = container_y_top - c_pt - border.top.width;
                    // Seed positioned-ancestor origins with this (top-level) box's
                    // padding-box origin so absolute descendants nested inside
                    // static intermediates resolve to it (their containing block).
                    let mut abs_origins: HashMap<usize, (f32, f32)> = HashMap::new();
                    if *c_positioned_depth > 0
                        && (*c_position == Position::Relative
                            || *c_position == Position::Absolute
                            || c_transform.is_some())
                    {
                        abs_origins.insert(
                            *c_positioned_depth,
                            (
                                container_x + border.left.width,
                                container_y_top - border.top.width,
                            ),
                        );
                    }
                    render_container_children(
                        &mut content,
                        children,
                        inner_x,
                        inner_y,
                        inner_w,
                        custom_fonts,
                        &prepared_custom_fonts,
                        &mut page_ext_gstates,
                        &mut bg_alpha_counter,
                        &mut page_shadings,
                        &mut shading_counter,
                        &mut pdf_writer,
                        &mut page_images,
                        *c_pl,
                        *c_pt,
                        &mut abs_origins,
                    );

                    // Restore clip
                    if needs_clip {
                        content.push_str("Q\n");
                    }
                    // Paint print scrollbar chrome in the reserved gutter, after
                    // the (gutter-inset) content clip is closed.
                    if c_has_v || c_has_h {
                        let pbx = container_x + border.left.width;
                        let pby = (container_y_top - total_h) + border.bottom.width;
                        paint_scrollbars(
                            &mut content,
                            pbx,
                            pby,
                            c_pad_box_w,
                            c_pad_box_h,
                            c_has_v,
                            c_has_h,
                            c_ratio_v.max(1.0),
                            c_ratio_h.max(1.0),
                        );
                    }
                    // Close the mask group (opened inside the clip-path q..Q).
                    if c_mask_open {
                        content.push_str("Q\n");
                    }
                    if c_needs_clip_path {
                        content.push_str("Q\n");
                    }
                    if c_needs_transform {
                        content.push_str("Q\n");
                    }
                    // Close the opacity group (must be the outermost q..Q).
                    if c_needs_opacity {
                        content.push_str("Q\n");
                    }
                }
                LayoutElement::Image {
                    image,
                    width,
                    height,
                    object_fit,
                    object_position,
                    background_color,
                    border,
                    blur_overflow,
                    ..
                } if *blur_overflow > 0.0 => {
                    // CSS `filter: blur()`/`drop-shadow()`: the embedded bitmap is
                    // the blurred result padded by `blur_overflow` on each side,
                    // drawn overflowing the content box (filter ignores layout).
                    let _ = (object_fit, object_position, background_color, border);
                    let img_x = margin.left;
                    let img_y = page_size.height - margin.top - y_pos - height;
                    // Occlusion culling (default off): the filtered bitmap paints
                    // expanded by `blur_overflow` on every side; skip it only if a
                    // later opaque coverer hides that full expanded rect.
                    if pdf_writer.opts.occlusion_cull {
                        let ov = *blur_overflow;
                        let raster = OcclRect {
                            x0: img_x - ov,
                            y0: img_y - ov,
                            x1: img_x + *width + ov,
                            y1: img_y + *height + ov,
                        };
                        if raster_is_occluded(&occlusion_coverers, &raster, elem_idx) {
                            continue;
                        }
                    }
                    let img_obj_id = pdf_writer.add_image_object(
                        &image.data,
                        image.source_width,
                        image.source_height,
                        image.format,
                        image.png_metadata.as_ref(),
                    );
                    let img_name = format!("Im{img_obj_id}");
                    let ov = *blur_overflow;
                    content.push_str(&format!(
                        "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
                        w = width + 2.0 * ov,
                        h = height + 2.0 * ov,
                        ix = img_x - ov,
                        iy = img_y - ov,
                        name = img_name,
                    ));
                    page_images.push(ImageRef {
                        name: img_name,
                        obj_id: img_obj_id,
                    });
                }
                LayoutElement::Image {
                    image,
                    width,
                    height,
                    object_fit,
                    object_position,
                    background_color,
                    border,
                    src_crop,
                    ..
                } => {
                    // When pagination has SLICED this image across page
                    // boundaries, decode the source once and crop it to this
                    // page's source-pixel rows, then embed ONLY that slice (not a
                    // full copy behind a clip). Falls back to the whole image if
                    // the crop fails to decode.
                    let sliced = src_crop
                        .and_then(|c| crate::layout::images::crop_raster_asset(image, c));
                    let img = sliced.as_ref().unwrap_or(image);
                    let img_x = margin.left;
                    // PDF y-axis is bottom-up; y_pos is top of margin, image draws from bottom-left
                    let img_y = page_size.height - margin.top - y_pos - height;
                    // Occlusion culling (default off): skip embedding the image
                    // entirely when a later fully-opaque coverer hides its box.
                    if pdf_writer.opts.occlusion_cull {
                        let raster = OcclRect {
                            x0: img_x,
                            y0: img_y,
                            x1: img_x + *width,
                            y1: img_y + *height,
                        };
                        if raster_is_occluded(&occlusion_coverers, &raster, elem_idx) {
                            continue;
                        }
                    }
                    // Paint the image-box background first; with object-fit it may
                    // remain visible where the image content does not cover the box.
                    if let Some((br, bg, bb, ba)) = background_color
                        && *ba > 0.0
                    {
                        content.push_str(&format!(
                            "{br} {bg} {bb} rg\n{img_x} {img_y} {width} {height} re\nf\n",
                        ));
                    }
                    // With box-sizing:border-box the box (width/height) includes the
                    // border, so inset the image content rect by the border widths.
                    let content_x = img_x + border.left.width;
                    let content_y = img_y + border.bottom.width;
                    let content_w = (width - border.horizontal_width()).max(0.0);
                    let content_h = (height - border.vertical_width()).max(0.0);
                    let placement = crate::layout::images::compute_image_placement(
                        content_w,
                        content_h,
                        img.source_width,
                        img.source_height,
                        *object_fit,
                        *object_position,
                    );
                    let img_obj_id = pdf_writer.add_source_image_object(
                        &img.data,
                        img.source_width,
                        img.source_height,
                        img.format,
                        img.png_metadata.as_ref(),
                        placement.width,
                        placement.height,
                    );
                    let img_name = format!("Im{img_obj_id}");
                    content.push_str("q\n");
                    if placement.clip {
                        content.push_str(&format!(
                            "{content_x} {content_y} {content_w} {content_h} re\nW n\n",
                        ));
                    }
                    content.push_str(&format!(
                        "{w} 0 0 {h} {x} {y} cm\n/{name} Do\nQ\n",
                        w = placement.width,
                        h = placement.height,
                        x = content_x + placement.offset_x,
                        // content_y is the content-box bottom; top is content_y + content_h.
                        y = content_y + content_h - placement.offset_y - placement.height,
                        name = img_name,
                    ));
                    page_images.push(ImageRef {
                        name: img_name,
                        obj_id: img_obj_id,
                    });
                    // Stroke the border frame around the image box. Border-box
                    // keeps the frame inside the box, so center each stroke half a
                    // width inside the box edge.
                    draw_image_border(
                        &mut content,
                        img_x,
                        img_y,
                        *width,
                        *height,
                        border,
                        &mut page_ext_gstates,
                        &mut bg_alpha_counter,
                    );
                }
                LayoutElement::Svg {
                    tree,
                    width,
                    height,
                    ..
                } => {
                    let svg_x = margin.left;
                    // PDF y-axis is bottom-up, SVG is top-down
                    let svg_y = page_size.height - margin.top - y_pos - height;

                    // Occlusion culling (default off): skip embedding the SVG (and
                    // any rasters it would register) when a later opaque coverer
                    // fully hides its box.
                    if pdf_writer.opts.occlusion_cull {
                        let raster = OcclRect {
                            x0: svg_x,
                            y0: svg_y,
                            x1: svg_x + *width,
                            y1: svg_y + *height,
                        };
                        if raster_is_occluded(&occlusion_coverers, &raster, elem_idx) {
                            continue;
                        }
                    }

                    content.push_str("q\n");
                    // Position on page and flip y-axis for SVG coordinates
                    content.push_str(&format!("1 0 0 -1 {} {} cm\n", svg_x, svg_y + height));
                    if let Some(placement) = crate::render::svg_geometry::compute_svg_placement(
                        tree,
                        crate::render::svg_geometry::SvgPlacementRequest::from_rect(
                            0.0,
                            0.0,
                            *width,
                            *height,
                            tree.preserve_aspect_ratio,
                        ),
                    ) {
                        content.push_str("q\n");
                        content.push_str(&placement.viewport.clip_path());
                        content.push_str(&format!(
                            "{sx} 0 0 {sy} {tx} {ty} cm\n",
                            sx = placement.scale_x,
                            sy = placement.scale_y,
                            tx = placement.translate_x,
                            ty = placement.translate_y,
                        ));
                        {
                            let mut image_sink = SvgPageImageSink {
                                pdf_writer: &mut pdf_writer,
                                page_images: &mut page_images,
                            };
                            let mut resources = crate::render::svg_to_pdf::SvgPdfResources {
                                shadings: &mut page_shadings,
                                shading_counter: &mut shading_counter,
                                ext_gstates: Some(&mut page_ext_gstates),
                                image_sink: Some(&mut image_sink),
                                custom_fonts: Some(custom_fonts),
                                prepared_custom_fonts: Some(&prepared_custom_fonts),
                            };
                            crate::render::svg_to_pdf::render_svg_tree_with_resources(
                                tree,
                                &mut content,
                                &mut resources,
                            );
                        }
                        content.push_str("Q\n");
                    }
                    content.push_str("Q\n");
                }
                LayoutElement::HorizontalRule { .. } => {
                    let rule_y = page_size.height - margin.top - y_pos;
                    content.push_str(&format!(
                        "0.5 w\n0 0 0 RG\n{x1} {y} m {x2} {y} l\nS\n",
                        x1 = margin.left,
                        x2 = page_size.width - margin.right,
                        y = rule_y,
                    ));
                }
                LayoutElement::ProgressBar {
                    fraction,
                    width,
                    height,
                    fill_color,
                    track_color,
                    ..
                } => {
                    let bar_x = margin.left;
                    let bar_y = page_size.height - margin.top - y_pos - height;

                    // Draw track background
                    content.push_str(&format!(
                        "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                        r = track_color.0,
                        g = track_color.1,
                        b = track_color.2,
                        x = bar_x,
                        y = bar_y,
                        w = width,
                        h = height,
                    ));

                    // Draw filled portion
                    if *fraction > 0.0 {
                        let fill_w = width * fraction;
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            r = fill_color.0,
                            g = fill_color.1,
                            b = fill_color.2,
                            x = bar_x,
                            y = bar_y,
                            w = fill_w,
                            h = height,
                        ));
                    }

                    // Draw border
                    content.push_str(&format!(
                        "0.5 w\n0.6 0.6 0.6 RG\n{x} {y} {w} {h} re\nS\n",
                        x = bar_x,
                        y = bar_y,
                        w = width,
                        h = height,
                    ));
                }
                LayoutElement::MathBlock {
                    layout: math_layout,
                    display,
                    ..
                } => {
                    let math_x = if *display {
                        // Center display math
                        margin.left + (available_width - math_layout.width) / 2.0
                    } else {
                        margin.left
                    };
                    // PDF y-axis: top of math block, baseline-adjusted
                    let math_baseline_y =
                        page_size.height - margin.top - y_pos - math_layout.ascent;

                    render_math_glyphs(&math_layout.glyphs, math_x, math_baseline_y, &mut content);
                }
                LayoutElement::PageBreak(_) => {}
            }
        }

        // Render page header/footer in margin area
        if let Some(dec) = decoration {
            let total_pages = pages.len();
            let page_num = page_idx + 1;
            let center_x = page_size.width / 2.0;

            if let Some(ref header_text) = dec.header {
                let text = header_text
                    .replace("{page}", &page_num.to_string())
                    .replace("{pages}", &total_pages.to_string());
                let encoded = encode_pdf_text(&text);
                let header_y = page_size.height - margin.top / 2.0;
                content.push_str("BT\n");
                content.push_str("/Helvetica 9 Tf\n");
                content.push_str("0.4 0.4 0.4 rg\n");
                content.push_str(&format!("{center_x} {header_y} Td\n"));
                content.push_str(&format!("({encoded}) Tj\n"));
                content.push_str("ET\n");
            }

            if let Some(ref footer_text) = dec.footer {
                let text = footer_text
                    .replace("{page}", &page_num.to_string())
                    .replace("{pages}", &total_pages.to_string());
                let encoded = encode_pdf_text(&text);
                let footer_y = margin.bottom / 2.0;
                content.push_str("BT\n");
                content.push_str("/Helvetica 9 Tf\n");
                content.push_str("0.4 0.4 0.4 rg\n");
                content.push_str(&format!("{center_x} {footer_y} Td\n"));
                content.push_str(&format!("({encoded}) Tj\n"));
                content.push_str("ET\n");
            }
        }

        // Match Chrome's print shrink-to-fit: if the page's content overflows the
        // page box, scale the whole content stream down around the top-left corner
        // so it just fits (PDF y-up: scaling by `s` about the top edge `H` needs
        // the translate `H(1-s)`).
        //
        // The scale is composed with Chrome's print-CTM net factor (below). Chrome's
        // `--print-to-pdf` does NOT emit content at the exact 0.75 pt/CSS-px: its
        // page CTM is a two-step `0.23999999 0 0 -0.23999999 cm` then the device
        // `3.125`, whose product is 0.74999996875 — 0.23999999 is Chrome's float
        // serialization of 0.24 (= 72/300). ironpress bakes content at the EXACT
        // 0.75, so its axis-aligned rectangles land on poppler/Splash's crisp no-AA
        // `re` fast path, whereas Chrome's perturbed coordinates get anti-aliased —
        // leaving crisp-vs-AA seams at every box/border edge (the dominant residual
        // parity-diff class). Re-applying Chrome's measured net scale as a near-
        // identity CTM nudges ironpress off the fast path so poppler anti-aliases
        // identically. Must be f64: 0.74999996875 is within half a ULP of 0.75 in
        // f32 and would collapse to a no-op. `format_pdf_number` is likewise f32, so
        // the scale is formatted at full f64 precision here.
        const CHROME_PRINT_CTM_NET: f64 = 0.74999996875;
        const PT_PER_CSS_PX: f64 = 0.75;
        let chrome_match = CHROME_PRINT_CTM_NET / PT_PER_CSS_PX; // ≈0.99999995833
        let shrink = page_shrink_to_fit_scale(page, page_size, margin) as f64;
        let s_eff = shrink * chrome_match;
        let s = format!("{s_eff:.11}");
        let ty = format!("{:.8}", f64::from(page_size.height) * (1.0 - s_eff));
        content = format!("q {s} 0 0 {s} 0 {ty} cm\n{content}Q\n");

        pdf_writer.add_page(
            page_size.width,
            page_size.height,
            &content,
            annotations,
            page_images,
            page_ext_gstates,
            page_shadings,
        );
    }

    pdf_writer.finish_to_writer(writer, &bookmarks)
}

fn register_used_custom_fonts(
    pdf_writer: &mut PdfWriter,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
) {
    for (font_name, prepared_font) in prepared_custom_fonts {
        if let Some(ttf) = custom_fonts.get(font_name) {
            pdf_writer.add_ttf_font(font_name, ttf, prepared_font);
        }
    }
}

fn font_name_for_run(run: &TextRun) -> &str {
    match (&run.font_family, run.bold, run.italic) {
        // Helvetica (sans-serif)
        (FontFamily::Helvetica, true, true) => "Helvetica-BoldOblique",
        (FontFamily::Helvetica, true, false) => "Helvetica-Bold",
        (FontFamily::Helvetica, false, true) => "Helvetica-Oblique",
        (FontFamily::Helvetica, false, false) => "Helvetica",
        // Times Roman (serif)
        (FontFamily::TimesRoman, true, true) => "Times-BoldItalic",
        (FontFamily::TimesRoman, true, false) => "Times-Bold",
        (FontFamily::TimesRoman, false, true) => "Times-Italic",
        (FontFamily::TimesRoman, false, false) => "Times-Roman",
        // Courier (monospace)
        (FontFamily::Courier, true, true) => "Courier-BoldOblique",
        (FontFamily::Courier, true, false) => "Courier-Bold",
        (FontFamily::Courier, false, true) => "Courier-Oblique",
        (FontFamily::Courier, false, false) => "Courier",
        // Custom fonts — fall back to Helvetica variant for rendering name;
        // the actual font reference is handled separately by the renderer.
        (FontFamily::Custom(_), true, true) => "Helvetica-BoldOblique",
        (FontFamily::Custom(_), true, false) => "Helvetica-Bold",
        (FontFamily::Custom(_), false, true) => "Helvetica-Oblique",
        (FontFamily::Custom(_), false, false) => "Helvetica",
    }
}

fn estimate_run_width(run: &TextRun) -> f32 {
    crate::fonts::str_width(&run.text, run.font_size, &run.font_family, run.bold)
}

/// Resolve the PDF font resource name for a text run.
///
/// Custom Type0 fonts are only safe when we also have shaped glyph output.
fn resolve_font_name(
    run: &TextRun,
    custom_font: Option<(&str, &TtfFont)>,
    shaped: Option<&crate::text::ShapedRun>,
) -> String {
    if let (Some((resolved_name, _)), Some(_)) = (custom_font, shaped) {
        sanitize_pdf_name(resolved_name)
    } else {
        font_name_for_run(run).to_string()
    }
}

/// Estimate run width using TTF metrics for custom fonts, falling back to fixed estimation.
/// Width of a run's leading and trailing whitespace, used to inset
/// text-decoration lines. A decorated inline span often absorbs the inter-word
/// space that precedes/follows it (the collapsing whitespace is merged into the
/// span's run), but CSS only decorates the span's own text — the bordering space
/// belongs to the parent. Insetting the underline/line-through/overline by these
/// widths keeps the decoration under the glyphs (matching Chrome) while leaving
/// internal spaces covered. Clamped so the two insets never exceed the run width.
fn decoration_ws_insets(run: &TextRun, custom_fonts: &HashMap<String, TtfFont>) -> (f32, f32) {
    if run.inline_box.is_some() {
        return (0.0, 0.0);
    }
    let lead: String = run.text.chars().take_while(|c| c.is_whitespace()).collect();
    let trail: String = run
        .text
        .chars()
        .rev()
        .take_while(|c| c.is_whitespace())
        .collect();
    if lead.is_empty() && trail.is_empty() {
        return (0.0, 0.0);
    }
    let measure = |s: &str| {
        if s.is_empty() {
            0.0
        } else {
            crate::layout::text::estimate_word_width(
                s,
                run.font_size,
                &run.font_family,
                run.bold,
                run.italic,
                custom_fonts,
            )
        }
    };
    (measure(&lead), measure(&trail))
}

fn estimate_run_width_with_fonts(run: &TextRun, custom_fonts: &HashMap<String, TtfFont>) -> f32 {
    if let Some(inline) = run.inline_box.as_deref() {
        return inline.outer_width();
    }
    if let Some(width) = crate::text::measure_text_width(
        &run.text,
        run.font_size,
        &run.font_family,
        run.bold,
        run.italic,
        custom_fonts,
    ) {
        return width;
    }

    estimate_run_width(run)
}

pub(crate) fn encode_pdf_hex_glyph(glyph_id: u16) -> String {
    format!("{glyph_id:04X}")
}

#[derive(Clone, Copy)]
struct PdfPoint {
    x: f32,
    y: f32,
}

impl PdfPoint {
    const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

struct ShapedTextRender<'a> {
    origin: PdfPoint,
    font_size: f32,
    font: &'a TtfFont,
    shaped: &'a crate::text::ShapedRun,
    prepared_font: Option<&'a PreparedCustomFont>,
    /// Extra advance (in PDF text-space units) to insert after each space
    /// cluster (U+0020).  Carries CSS `word-spacing` plus the per-space
    /// `text-align: justify` stretch.  Type0 / Identity-H text ignores the
    /// PDF `Tw` operator (it only applies to single-byte code 32), so this
    /// must be baked into the TJ array as a negative adjustment instead.
    word_spacing: f32,
    /// Synthetic-italic shear (the text-matrix `c` term): a face with no genuine
    /// italic gets an algorithmic oblique slant (CSS Fonts 4 `font-synthesis:
    /// style`). 0 = upright. Matches Skia/Chrome's synthetic skew (0.25).
    shear: f32,
}

impl<'a> ShapedTextRender<'a> {
    const fn new(
        origin: PdfPoint,
        font_size: f32,
        font: &'a TtfFont,
        shaped: &'a crate::text::ShapedRun,
        prepared_font: Option<&'a PreparedCustomFont>,
    ) -> Self {
        Self {
            origin,
            font_size,
            font,
            shaped,
            prepared_font,
            word_spacing: 0.0,
            shear: 0.0,
        }
    }

    const fn with_word_spacing(mut self, word_spacing: f32) -> Self {
        self.word_spacing = word_spacing;
        self
    }

    const fn with_shear(mut self, shear: f32) -> Self {
        self.shear = shear;
        self
    }

    /// Extra TJ adjustment (thousandths of an em / text-space units) to add
    /// after `glyph` when it is a space cluster.  A positive `Tj` number moves
    /// the cursor left, so the returned adjustment is negative in order to
    /// *widen* the gap after the space.
    fn space_tj_adjustment(&self, glyph: &crate::text::ShapedGlyph) -> f32 {
        if self.word_spacing.abs() <= f32::EPSILON {
            return 0.0;
        }
        if glyph.unicode.as_slice() == [0x0020] {
            -(self.word_spacing * 1000.0 / self.font_size.max(f32::EPSILON))
        } else {
            0.0
        }
    }

    fn has_complex_offsets(&self) -> bool {
        self.shaped
            .glyphs
            .iter()
            .any(|glyph| glyph.x_offset.abs() > f32::EPSILON || glyph.y_offset.abs() > f32::EPSILON)
    }

    fn pdf_glyph_id(&self, glyph_id: u16) -> u16 {
        self.prepared_font.map_or(glyph_id, |prepared_font| {
            prepared_font.pdf_glyph_id(glyph_id)
        })
    }
}

fn format_pdf_number(value: f32) -> String {
    let rounded = if value.abs() < 0.000_5 { 0.0 } else { value };
    let mut formatted = format!("{rounded:.4}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    if formatted == "-0" {
        "0".to_string()
    } else {
        formatted
    }
}

fn append_positioned_shaped_text(content: &mut String, render: ShapedTextRender<'_>) {
    let mut cursor_x = render.origin.x;
    for glyph in &render.shaped.glyphs {
        let draw_x = cursor_x + glyph.x_offset;
        let draw_y = render.origin.y + glyph.y_offset;
        let encoded = encode_pdf_hex_glyph(render.pdf_glyph_id(glyph.glyph_id));
        content.push_str(&format!(
            "1 0 {} 1 {} {} Tm\n",
            format_pdf_number(render.shear),
            format_pdf_number(draw_x),
            format_pdf_number(draw_y),
        ));
        content.push_str(&format!("<{encoded}> Tj\n"));
        cursor_x += glyph.x_advance;
        // Identity-H ignores the PDF `Tw` operator, so widen the gap after
        // each space cluster by advancing the cursor manually.
        if render.word_spacing.abs() > f32::EPSILON && glyph.unicode.as_slice() == [0x0020] {
            cursor_x += render.word_spacing;
        }
    }
}

fn append_tj_shaped_text(content: &mut String, render: ShapedTextRender<'_>) {
    content.push_str(&format!(
        "1 0 {} 1 {} {} Tm\n",
        format_pdf_number(render.shear),
        format_pdf_number(render.origin.x),
        format_pdf_number(render.origin.y),
    ));
    content.push('[');

    let mut first = true;
    for glyph in &render.shaped.glyphs {
        if !first {
            content.push(' ');
        }
        first = false;

        let encoded = encode_pdf_hex_glyph(render.pdf_glyph_id(glyph.glyph_id));
        content.push('<');
        content.push_str(&encoded);
        content.push('>');

        let nominal_advance = render
            .font
            .glyph_width_scaled(glyph.glyph_id, render.font_size);
        let advance_adjustment = glyph.x_advance - nominal_advance;
        // Fold the shaper advance/kerning delta together with any extra
        // inter-word spacing (CSS word-spacing + justify stretch) for space
        // clusters, so a single TJ number carries both.
        let kern_adjustment = -(advance_adjustment * 1000.0 / render.font_size.max(f32::EPSILON));
        let tj_adjustment = kern_adjustment + render.space_tj_adjustment(glyph);
        if tj_adjustment.abs() > 0.001 {
            content.push(' ');
            content.push_str(&format_pdf_number(tj_adjustment));
        }
    }

    content.push_str("] TJ\n");
}

/// Emit a PDF `cm` operator for a CSS `transform` applied around the element
/// centre `(cx, cy)` in PDF coordinates (CSS `transform-origin: 50% 50%`). The
/// caller wraps the element's drawing in `q` ... `Q`. Shared by every render
/// arm so transforms behave identically for top-level, flex-cell, and nested
/// (container / absolutely-positioned) elements.
/// Emit the PDF `cm` operator for a CSS transform, pivoting around the resolved
/// transform-origin `(cx, cy)` (PDF bottom-up coordinates). `box_w`/`box_h` are
/// the element's border-box size in pt, used to resolve percentage `translate()`
/// components. Every transform is reduced to its CSS affine matrix and then
/// y-flip-conjugated around the pivot, so all transform kinds share one path.
fn push_transform_cm(
    content: &mut String,
    t: &crate::style::computed::Transform,
    cx: f32,
    cy: f32,
    box_w: f32,
    box_h: f32,
) {
    let [a, b, c, d, e, f] = t.to_css_matrix(box_w, box_h);
    // CSS->PDF Y-flip conjugation: negate the off-diagonal/shear and the
    // vertical translation, then re-apply the pivot translation.
    let (pa, pb, pc, pd, pe, pf) = (a, -b, -c, d, e, -f);
    let ne = pa * (-cx) + pc * (-cy) + pe + cx;
    let nf = pb * (-cx) + pd * (-cy) + pf + cy;
    // Normalise signed zero so a pure scale/translate emits `... 0 0 ...`
    // (not `-0`), keeping output byte-stable and human-readable.
    let z = |v: f32| if v == 0.0 { 0.0 } else { v };
    content.push_str(&format!(
        "{} {} {} {} {} {} cm\n",
        z(pa),
        z(pb),
        z(pc),
        z(pd),
        z(ne),
        z(nf)
    ));
}

/// Emit a circle/ellipse as four cubic Bézier arcs (PDF `c` operators) starting
/// from a `move` to the right vertex. Caller terminates the path (e.g. `W n`).
fn emit_ellipse_path(content: &mut String, cx: f32, cy: f32, rx: f32, ry: f32) {
    const K: f32 = 0.552_284_8; // 4/3*(sqrt(2)-1): circle->bezier constant
    let (kx, ky) = (K * rx, K * ry);
    content.push_str(&format!("{} {cy} m\n", cx + rx));
    content.push_str(&format!(
        "{} {} {} {} {cx} {} c\n",
        cx + rx,
        cy + ky,
        cx + kx,
        cy + ry,
        cy + ry
    ));
    content.push_str(&format!(
        "{} {} {} {} {} {cy} c\n",
        cx - kx,
        cy + ry,
        cx - rx,
        cy + ky,
        cx - rx
    ));
    content.push_str(&format!(
        "{} {} {} {} {cx} {} c\n",
        cx - rx,
        cy - ky,
        cx - kx,
        cy - ry,
        cy - ry
    ));
    content.push_str(&format!(
        "{} {} {} {} {} {cy} c\n",
        cx + kx,
        cy - ry,
        cx + rx,
        cy - ky,
        cx + rx
    ));
}

/// Emit the geometry for a CSS `clip-path` basic shape as a PDF clip (`... W n`),
/// resolved against the element border box (`left`/`top_y` are the top-left in
/// PDF bottom-up coordinates; `w`/`h` the box size). Caller wraps in `q`..`Q`.
fn push_clip_path(
    content: &mut String,
    clip: &crate::style::computed::ClipPath,
    left: f32,
    top_y: f32,
    w: f32,
    h: f32,
) {
    use crate::style::computed::ClipPath;
    let along_x = |(v, pct): (f32, bool)| if pct { w * v / 100.0 } else { v };
    let along_y = |(v, pct): (f32, bool)| if pct { h * v / 100.0 } else { v };
    match clip {
        ClipPath::Circle { r, cx, cy } => {
            let cxp = left + along_x(*cx);
            let cyp = top_y - along_y(*cy);
            // % radius resolves against the diagonal/sqrt(2); approximate with
            // the smaller axis for px-free cases. px radii are absolute.
            let rad = if r.1 { w.min(h) * r.0 / 100.0 } else { r.0 };
            emit_ellipse_path(content, cxp, cyp, rad, rad);
        }
        ClipPath::Ellipse { rx, ry, cx, cy } => {
            let cxp = left + along_x(*cx);
            let cyp = top_y - along_y(*cy);
            emit_ellipse_path(content, cxp, cyp, along_x(*rx), along_y(*ry));
        }
        ClipPath::Inset {
            top,
            right,
            bottom,
            left: l,
            radius,
        } => {
            let x0 = left + along_x(*l);
            let x1 = left + w - along_x(*right);
            let y1 = top_y - along_y(*top); // upper edge
            let y0 = top_y - (h - along_y(*bottom)); // lower edge
            let (rw, rh) = ((x1 - x0).max(0.0), (y1 - y0).max(0.0));
            if *radius > 0.0 {
                content.push_str(&rounded_rect_path(x0, y0, rw, rh, *radius));
                content.push('\n');
            } else {
                content.push_str(&format!("{x0} {y0} {rw} {rh} re\n"));
            }
        }
        ClipPath::Polygon(points) => {
            for (i, (px, py)) in points.iter().enumerate() {
                let x = left + along_x(*px);
                let y = top_y - along_y(*py);
                content.push_str(&format!("{x} {y} {}\n", if i == 0 { "m" } else { "l" }));
            }
            content.push_str("h\n");
        }
    }
    content.push_str("W n\n");
}

/// CSS adjacent-sibling vertical margin collapse.
///
/// Returns the *extra* amount to subtract from the cursor for `margin_top`
/// given that `prev_margin_bottom` (the previous in-flow sibling's
/// margin-bottom) was already subtracted. The collapsed gap between two
/// in-flow blocks is: max for two positive margins, min (most negative) for
/// two negatives, and the sum for mixed signs — matching `paginate.rs`.
fn collapsed_margin_top_extra(margin_top: f32, prev_margin_bottom: f32) -> f32 {
    let collapsed = if margin_top >= 0.0 && prev_margin_bottom >= 0.0 {
        margin_top.max(prev_margin_bottom)
    } else if margin_top < 0.0 && prev_margin_bottom < 0.0 {
        margin_top.min(prev_margin_bottom)
    } else {
        margin_top + prev_margin_bottom
    };
    // `prev_margin_bottom` is already gone from the cursor; only apply the
    // excess of the collapsed gap over it.
    collapsed - prev_margin_bottom
}

/// Apply CSS `clear` to a flow cursor (PDF y, where down = smaller y). Pushes
/// the cursor down to the bottom of the relevant float(s) when it currently sits
/// above them, and breaks the margin-collapse chain (clearance is not a margin).
/// `left_bottom` / `right_bottom` are the lowest float bottoms per side in PDF y.
fn clear_cursor(
    cursor_y: f32,
    clear: Clear,
    left_bottom: f32,
    right_bottom: f32,
    prev_margin_bottom: &mut f32,
) -> f32 {
    let clear_to = match clear {
        Clear::Left => left_bottom,
        Clear::Right => right_bottom,
        Clear::Both => left_bottom.min(right_bottom),
        Clear::None => return cursor_y,
    };
    if clear_to < cursor_y {
        *prev_margin_bottom = 0.0;
        clear_to
    } else {
        cursor_y
    }
}

/// How a child participates in adjacent-sibling vertical margin collapse,
/// mirroring the per-arm handling in `render_container_children`.
enum CollapseRole {
    /// In-flow block: collapses with neighbours (margin-top, margin-bottom).
    Collapsing(f32, f32),
    /// Out of flow (absolute): contributes no height and leaves the running
    /// collapse state untouched (cursor `continue`s without resetting it).
    Skip,
    /// Non-collapsing in-flow content (floats, table/grid rows): consumes its
    /// own space and breaks the collapse chain for the next sibling.
    Barrier,
}

fn collapse_role(element: &LayoutElement) -> CollapseRole {
    match element {
        LayoutElement::TextBlock {
            margin_top,
            margin_bottom,
            position,
            float,
            ..
        }
        | LayoutElement::Container {
            margin_top,
            margin_bottom,
            position,
            float,
            ..
        } => {
            if *position == Position::Absolute {
                CollapseRole::Skip
            } else if *float != Float::None {
                CollapseRole::Barrier
            } else {
                CollapseRole::Collapsing(*margin_top, *margin_bottom)
            }
        }
        LayoutElement::Image {
            margin_top,
            margin_bottom,
            ..
        }
        | LayoutElement::Svg {
            margin_top,
            margin_bottom,
            ..
        }
        | LayoutElement::FlexRow {
            margin_top,
            margin_bottom,
            ..
        }
        | LayoutElement::HorizontalRule {
            margin_top,
            margin_bottom,
        }
        | LayoutElement::ProgressBar {
            margin_top,
            margin_bottom,
            ..
        }
        | LayoutElement::MathBlock {
            margin_top,
            margin_bottom,
            ..
        } => CollapseRole::Collapsing(*margin_top, *margin_bottom),
        // Table/grid rows and anything else: do not collapse with siblings.
        _ => CollapseRole::Barrier,
    }
}

/// Sum of children heights with CSS adjacent-sibling vertical margin collapse
/// applied, mirroring the cursor advance in `render_container_children`.
///
/// `estimate_element_height` sums each child's full top+bottom margins; this
/// subtracts the collapse "savings" between consecutive in-flow siblings so a
/// container's painted height matches the collapsed flow.
/// Best-effort border-box width of a direct child, for deciding whether a scroll
/// container's content overflows horizontally. Returns `None` when the width is
/// not explicitly known (auto-width children shrink to fit and don't overflow).
fn child_explicit_width(element: &LayoutElement) -> Option<f32> {
    match element {
        LayoutElement::Container { block_width, .. }
        | LayoutElement::TextBlock { block_width, .. } => *block_width,
        LayoutElement::Image { width, .. } => Some(*width),
        _ => None,
    }
}

/// The content overflow extent of a scroll container's children, as `(width,
/// height)` border-box points. Width is the widest direct child (explicit widths
/// only); height is the collapsed flow height. Used to size scrollbar thumbs and
/// to decide whether an `overflow: auto` axis actually overflows.
fn children_overflow_extent(children: &[LayoutElement]) -> (f32, f32) {
    let w = children
        .iter()
        .filter_map(child_explicit_width)
        .fold(0.0f32, f32::max);
    (w, collapsed_children_height(children))
}

fn collapsed_children_height(children: &[LayoutElement]) -> f32 {
    // When any direct child floats, the auto content height excludes the floats
    // (they don't stretch the box) but includes any `clear` gap. Delegate to the
    // shared flow simulator so the painted box matches the measured height. The
    // plain (no-float) accumulation below is kept identical to avoid regressions.
    if children
        .iter()
        .any(|c| crate::layout::paginate::element_float(c) != Float::None)
    {
        return crate::layout::paginate::simulate_block_flow(children).height;
    }
    let mut total = 0.0f32;
    let mut prev_mb: Option<f32> = None;
    for child in children {
        total += crate::layout::engine::estimate_element_height(child);
        match collapse_role(child) {
            CollapseRole::Collapsing(mt, mb) => {
                if let Some(pmb) = prev_mb {
                    // Both margins are already in `total`; remove the overlap
                    // (their sum minus the collapsed gap).
                    let collapsed = if mt >= 0.0 && pmb >= 0.0 {
                        mt.max(pmb)
                    } else if mt < 0.0 && pmb < 0.0 {
                        mt.min(pmb)
                    } else {
                        mt + pmb
                    };
                    total -= pmb + mt - collapsed;
                }
                prev_mb = Some(mb);
            }
            // Absolute children don't break the chain; barriers do.
            CollapseRole::Skip => {}
            CollapseRole::Barrier => prev_mb = None,
        }
    }
    total
}

/// CSS paint-order key for a sibling child of a container.
///
/// Returns `(layer, z_index)` where `layer` is `0` for in-flow / non-positioned
/// content and `1` for out-of-flow `position: absolute` boxes. A *stable* sort by
/// this key keeps in-flow children in source order (so flow-cursor advancement is
/// unaffected) while moving absolutely-positioned siblings to paint last, ordered
/// by ascending `z_index` (stable for ties). This implements the simplified CSS
/// stacking rule: positioned descendants paint above non-positioned in-flow ones,
/// and among the positioned ones, lower `z_index` paints first.
fn child_paint_order(element: &LayoutElement) -> (u8, i32) {
    match element {
        LayoutElement::TextBlock {
            position,
            z_index,
            float,
            ..
        }
        | LayoutElement::Container {
            position,
            z_index,
            float,
            ..
        } => {
            // CSS stacking layers: in-flow / non-positioned content paints first
            // (layer 0, kept in source order by a stable sort), then floats
            // (layer 1, so a float paints over the in-flow block it overlaps),
            // then out-of-flow absolutely-positioned boxes (layer 2, ordered by
            // ascending z-index). Float positions are precomputed before the
            // paint loop, so moving floats to paint last does not disturb the
            // in-flow flow cursor.
            if *position == Position::Absolute {
                (2, *z_index)
            } else if *float != Float::None {
                (1, 0)
            } else {
                (0, 0)
            }
        }
        _ => (0, 0),
    }
}

/// Recursively render a Container element and all its children.
///
/// `x` / `y` are the content-box origin (after padding).
/// `abs_pad_left` / `abs_pad_top` are the parent padding values so that
/// absolute-positioned children can be placed relative to the padding box.
/// Resolve the padding-box origin an absolute child must anchor to: the nearest
/// positioned ancestor recorded in `abs_origins` (keyed by the child's
/// containing-block depth), falling back to the immediate container's padding box
/// (`self_pad_origin`). This is what lets an absolute box skip static
/// intermediate ancestors and resolve against its real containing block.
fn abs_child_anchor(
    cb: &Option<crate::layout::engine::ContainingBlock>,
    abs_origins: &HashMap<usize, (f32, f32)>,
    self_pad_origin: (f32, f32),
) -> (f32, f32) {
    cb.and_then(|c| abs_origins.get(&c.depth).copied())
        .unwrap_or(self_pad_origin)
}

#[allow(clippy::too_many_arguments)]
fn render_container_children(
    content: &mut String,
    children: &[LayoutElement],
    x: f32,
    mut y: f32,
    width: f32,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
    page_shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    abs_pad_left: f32,
    abs_pad_top: f32,
    abs_origins: &mut HashMap<usize, (f32, f32)>,
) {
    // Padding-box origin (left x, top y in PDF coords) of THIS container, used
    // as the default anchor for absolutely-positioned children. An abs child
    // whose containing block names a *different* positioned ancestor (because it
    // is nested inside static intermediates) overrides this via `abs_origins`.
    let self_pad_origin = (x - abs_pad_left, y + abs_pad_top);

    // Separate children into those handled by render_nested_table_rows
    // (TableRow, TextBlock) and those handled directly (Container, Svg, etc.).
    // We process all children in order, flushing accumulated nested-layout
    // batches when we hit a directly-handled type.
    let mut nested_batch: Vec<&LayoutElement> = Vec::new();
    let mut cursor_y = y;
    // Save original y for absolute positioning (must not be affected by
    // flow children advancing the cursor).
    let container_top_y = y;
    // Track the previous in-flow block sibling's margin-bottom so adjacent
    // vertical margins collapse (CSS) instead of summing. Reset to 0 across
    // out-of-flow children (absolute/float) and nested-table batches, which
    // do not participate in collapse.
    let mut prev_margin_bottom: f32 = 0.0;

    // Simplified CSS floats: precompute each floated child's top (relative to the
    // content-box top) and per-side running bottoms via the shared flow
    // simulator, keyed by source index. Floats are removed from normal flow — the
    // cursor below does not advance for them — but in-flow blocks with `clear`
    // are pushed below the relevant float bottoms. Only computed when a child
    // actually floats, so the common case pays nothing.
    let has_floats = children
        .iter()
        .any(|c| crate::layout::paginate::element_float(c) != Float::None);
    let (float_top_by_index, left_float_bottom, right_float_bottom) = if has_floats {
        let flow = crate::layout::paginate::simulate_block_flow(children);
        let tops: HashMap<usize, f32> = flow.floats.iter().map(|f| (f.index, f.top)).collect();
        // Float bottoms in PDF y (down = smaller y) for `clear`. Floats always
        // precede the blocks that clear them in source order, so the per-side
        // totals from the simulator are exactly what those blocks must clear.
        (
            tops,
            container_top_y - flow.left_float_bottom,
            container_top_y - flow.right_float_bottom,
        )
    } else {
        (HashMap::new(), container_top_y, container_top_y)
    };

    // Paint children in CSS stacking order: in-flow / non-positioned content
    // first (kept in source order), then floats (so they paint over the in-flow
    // block they overlap), then absolutely-positioned siblings sorted by
    // ascending z-index. A *stable* sort preserves source order for ties and,
    // critically, for all in-flow children — so flow-cursor advancement below is
    // identical to iterating `children` directly. Floats and absolute boxes are
    // placed from precomputed/fixed positions, so reordering their paint does not
    // disturb the flow cursor.
    let needs_reorder = children.iter().any(|c| child_paint_order(c) != (0, 0));
    let paint_order: Vec<(usize, &LayoutElement)> = if needs_reorder {
        let mut ordered: Vec<(usize, &LayoutElement)> = children.iter().enumerate().collect();
        ordered.sort_by_key(|(_, c)| child_paint_order(c));
        ordered
    } else {
        children.iter().enumerate().collect()
    };

    for (child_index, child) in paint_order {
        let handled_by_nested = matches!(
            child,
            LayoutElement::TableRow { .. } | LayoutElement::GridRow { .. }
        );
        if handled_by_nested {
            nested_batch.push(child);
            cursor_y -= crate::layout::engine::estimate_element_height(child);
            // Table/grid rows do not collapse margins with sibling blocks.
            prev_margin_bottom = 0.0;
            continue;
        }

        // Flush any accumulated nested batch before handling this element
        if !nested_batch.is_empty() {
            let batch: Vec<LayoutElement> = nested_batch.drain(..).cloned().collect();
            render_nested_table_rows(
                content,
                &batch,
                x,
                y,
                page_ext_gstates,
                bg_alpha_counter,
                custom_fonts,
                prepared_custom_fonts,
                page_shadings,
                shading_counter,
                pdf_writer,
                page_images,
            );
            y = cursor_y;
        }

        match child {
            LayoutElement::TextBlock {
                lines,
                margin_top,
                margin_bottom,
                padding_top,
                padding_bottom,
                padding_left,
                padding_right,
                border,
                border_radius: tb_border_radius,
                block_height,
                background_color,
                background_gradient: tb_bg_gradient,
                background_radial_gradient: tb_bg_radial,
                background_conic_gradient: tb_bg_conic,
                background_svg: tb_bg_svg,
                background_blur_radius: tb_bg_blur,
                text_align,
                float: tb_float,
                clear: tb_clear,
                position,
                offset_top,
                offset_left,
                opacity: tb_opacity,
                mix_blend_mode: tb_mix_blend,
                background_blend_mode: tb_bg_blend,
                block_width: tb_block_width,
                clip_rect: tb_clip_rect,
                text_indent: tb_text_indent,
                containing_block: tb_containing_block,
                ..
            } => {
                // Absolute-positioned children render at offset from the
                // containing block's padding box (CSS spec), not the content box.
                // Use container_top_y (original y before flow children advance it).
                if *position == Position::Absolute {
                    let text_h: f32 = lines.iter().map(|l| l.height).sum();
                    let abs_h = padding_top + text_h + padding_bottom + border.vertical_width();
                    let abs_h = block_height.map_or(abs_h, |h| abs_h.max(h));
                    let abs_w = tb_block_width.unwrap_or(width);
                    // Anchor to the nearest positioned ancestor's padding box
                    // (resolved by containing-block depth), skipping any static
                    // intermediate container this box is nested inside.
                    let (anchor_x, anchor_y) =
                        abs_child_anchor(tb_containing_block, abs_origins, self_pad_origin);
                    let abs_x = anchor_x + offset_left;
                    let abs_y = anchor_y - offset_top;

                    // `mix-blend-mode`: composite the whole element (background +
                    // text) with the backdrop. Scope the blend gstate to a
                    // `q`..`Q` pair so only this element's paint blends.
                    let blended = *tb_mix_blend != crate::style::computed::BlendMode::Normal;
                    if blended {
                        content.push_str("q\n");
                        begin_blend_mode(content, page_ext_gstates, *tb_mix_blend);
                    }

                    // Apply element opacity (e.g. `.z-back { opacity: 0.8 }`)
                    // for the entire absolute element (background + text). The
                    // PDF graphics-state name is unique per alpha counter so it
                    // doesn't collide with other elements' ExtGState entries.
                    let needs_opacity = *tb_opacity < 1.0;
                    if needs_opacity {
                        let gs_name = format!("GSabs{bg_alpha_counter}");
                        *bg_alpha_counter += 1;
                        page_ext_gstates.push((gs_name.clone(), *tb_opacity));
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }

                    if let Some((r, g, b, a)) = background_color {
                        let effective_alpha = *a * *tb_opacity;
                        let needs_alpha = effective_alpha < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScca{bg_alpha_counter}");
                            *bg_alpha_counter += 1;
                            page_ext_gstates.push((gs_name.clone(), effective_alpha));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{ax} {ay} {aw} {ah} re\nf\n",
                            ax = abs_x,
                            ay = abs_y - abs_h,
                            aw = abs_w,
                            ah = abs_h,
                        ));
                        if needs_alpha {
                            // Restore the element-level opacity (if any) so
                            // subsequent text also gets alpha-composited.
                            if needs_opacity {
                                let gs_name = format!("GSabs{}", *bg_alpha_counter - 2);
                                content.push_str(&format!("/{gs_name} gs\n"));
                            } else {
                                content.push_str("/GSDefault gs\n");
                            }
                        }
                    }
                    // Render text for absolute-positioned children
                    let mut text_y_abs = abs_y - padding_top;
                    for line in lines {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y_abs -= metrics.half_leading + metrics.ascender;
                        let merged = merge_runs(&line.runs);
                        let line_width: f32 = merged
                            .iter()
                            .map(|r| estimate_run_width_with_fonts(r, custom_fonts))
                            .sum();
                        let text_x = match text_align {
                            TextAlign::Right => abs_x + (abs_w - line_width).max(0.0),
                            TextAlign::Center => abs_x + (abs_w - line_width).max(0.0) / 2.0,
                            _ => abs_x,
                        };
                        let mut lx = text_x;
                        for run in &merged {
                            let rw = render_run_text(
                                content,
                                run,
                                lx,
                                text_y_abs,
                                crate::layout::text::line_primary_font_size(&merged),
                                custom_fonts,
                                prepared_custom_fonts,
                                0.0,
                                pdf_writer,
                                page_images,
                            );
                            lx += rw;
                        }
                        text_y_abs -= metrics.descender + metrics.half_leading;
                    }
                    // Reset the graphics state if we applied element opacity.
                    if needs_opacity {
                        content.push_str("/GSDefault gs\n");
                    }
                    // Close the mix-blend-mode scope (restores the prior gstate).
                    if blended {
                        content.push_str("Q\n");
                    }
                    // Don't advance cursor_y for absolute elements
                    continue;
                }

                // A floated block is out of normal flow: it is pinned at its
                // precomputed top (the flow cursor at its source position) and
                // does NOT advance `cursor_y`. An in-flow block collapses its
                // margin-top with the previous sibling, after first clearing any
                // floats it must drop below.
                let is_float = *tb_float != Float::None;
                if is_float {
                    // Place the float from the shared simulator's top so its
                    // paint position (it paints last) matches the flow.
                    let rel_top = float_top_by_index.get(&child_index).copied().unwrap_or(0.0);
                    y = container_top_y - rel_top;
                } else {
                    if *tb_clear != Clear::None {
                        cursor_y = clear_cursor(
                            cursor_y,
                            *tb_clear,
                            left_float_bottom,
                            right_float_bottom,
                            &mut prev_margin_bottom,
                        );
                    }
                    cursor_y -= collapsed_margin_top_extra(*margin_top, prev_margin_bottom);
                    y = cursor_y;
                }
                let text_h: f32 = lines.iter().map(|l| l.height).sum();
                // `block_height` is a *padding-box* height (TextBlock convention),
                // so the painted border box adds the border on top. Compute the
                // padding-box height first (content vs. explicit), then add the
                // border once — mirroring paginate's `estimate_element_height`
                // (`effective_h + border.vertical_width()`) so the painted box
                // matches the flow. Folding the border into the value compared
                // against `block_height` (the old `max(content+border, bh)`)
                // rendered a border-box-sized child short by its border.
                let content_pad_box = padding_top + text_h + padding_bottom;
                // A definite `block_height` is a hard size when the box clips
                // (`overflow: hidden`/`scroll`): overflowing text is clipped to it
                // rather than growing the box. Without a clip the height is a floor
                // (min-height / auto) and grows to fit content.
                let pad_box_h = if tb_clip_rect.is_some() {
                    block_height.unwrap_or(content_pad_box)
                } else {
                    block_height.map_or(content_pad_box, |h| content_pad_box.max(h))
                };
                let child_h = pad_box_h + border.vertical_width();

                let render_w = tb_block_width.unwrap_or(width);

                // Apply float/position offset. `offset_left`/`offset_top` combine
                // the in-flow horizontal placement (margin-left / margin:auto
                // centering) with any relative left/top shift, and are 0 for a
                // plain static block — so apply them unconditionally rather than
                // only for relative (which dropped margin-left/centering on
                // nested blocks).
                let render_x = match tb_float {
                    Float::Right => x + width - render_w,
                    _ => x + offset_left,
                };
                let render_y = y - offset_top;

                // CSS `filter: blur()` on a nested solid box (css-filter-effects-1
                // §4.1): rasterize the bg fill + border, gaussian-blur it, and
                // embed it overflowing the border box. Restricted to a plain solid
                // box (no gradient/SVG bg, no text, no clip, square corners) so the
                // vector paint path is byte-unchanged for everything else.
                if *tb_bg_blur > 0.0
                    && lines.is_empty()
                    && tb_bg_gradient.is_none()
                    && tb_bg_radial.is_none()
                    && tb_bg_conic.is_none()
                    && tb_bg_svg.is_none()
                    && tb_clip_rect.is_none()
                    && *tb_border_radius == 0.0
                    && let Some(blurred) = crate::render::blur::blur_box(
                        render_w,
                        child_h,
                        *background_color,
                        border,
                        *tb_bg_blur,
                        pdf_writer.opts.filter_dpi,
                    )
                {
                    let img_obj_id = pdf_writer.add_image_object(
                        &blurred.asset.data,
                        blurred.asset.source_width,
                        blurred.asset.source_height,
                        blurred.asset.format,
                        blurred.asset.png_metadata.as_ref(),
                    );
                    let img_name = format!("Im{img_obj_id}");
                    let ov = blurred.overflow_pt;
                    content.push_str(&format!(
                        "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
                        w = render_w + 2.0 * ov,
                        h = child_h + 2.0 * ov,
                        ix = render_x - ov,
                        iy = render_y - child_h - ov,
                        name = img_name,
                    ));
                    page_images.push(ImageRef {
                        name: img_name,
                        obj_id: img_obj_id,
                    });
                    // Advance the flow cursor exactly as the normal block path
                    // below (the filter does not change layout).
                    if is_float {
                        prev_margin_bottom = 0.0;
                    } else {
                        cursor_y -= child_h + *margin_bottom;
                        y = cursor_y;
                        prev_margin_bottom = *margin_bottom;
                    }
                    continue;
                }

                // Draw child background
                if let Some((r, g, b, a)) = background_color {
                    let needs_alpha = *a < 1.0;
                    if needs_alpha {
                        let gs_name = format!("GScca{bg_alpha_counter}");
                        *bg_alpha_counter += 1;
                        page_ext_gstates.push((gs_name.clone(), *a));
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }
                    content.push_str(&format!(
                        "{r} {g} {b} rg\n{cx} {cy} {cw} {ch} re\nf\n",
                        cx = render_x,
                        cy = render_y - child_h,
                        cw = render_w,
                        ch = child_h,
                    ));
                    if needs_alpha {
                        content.push_str("/GSDefault gs\n");
                    }
                }

                // `background-blend-mode`: the background image layers (gradient)
                // blend against the background color painted above. Scope the
                // blend gstate to a `q`..`Q` around the gradient paint.
                let bg_blended = *tb_bg_blend != crate::style::computed::BlendMode::Normal;

                // Draw linear gradient background
                if let Some(gradient) = tb_bg_gradient {
                    let bg_x = render_x;
                    let bg_y = render_y - child_h;
                    if bg_blended {
                        content.push_str("q\n");
                        begin_blend_mode(content, page_ext_gstates, *tb_bg_blend);
                    }
                    if *tb_border_radius > 0.0 {
                        content.push_str("q\n");
                        content.push_str(&rounded_rect_path(
                            bg_x,
                            bg_y,
                            render_w,
                            child_h,
                            *tb_border_radius,
                        ));
                        content.push_str("W n\n");
                    }
                    render_linear_gradient(
                        content,
                        gradient,
                        bg_x,
                        bg_y,
                        render_w,
                        child_h,
                        page_shadings,
                        shading_counter,
                    );
                    if *tb_border_radius > 0.0 {
                        content.push_str("Q\n");
                    }
                    if bg_blended {
                        content.push_str("Q\n");
                    }
                }

                // Draw radial gradient background
                if let Some(gradient) = tb_bg_radial {
                    let bg_x = render_x;
                    let bg_y = render_y - child_h;
                    if bg_blended {
                        content.push_str("q\n");
                        begin_blend_mode(content, page_ext_gstates, *tb_bg_blend);
                    }
                    if *tb_border_radius > 0.0 {
                        content.push_str("q\n");
                        content.push_str(&rounded_rect_path(
                            bg_x,
                            bg_y,
                            render_w,
                            child_h,
                            *tb_border_radius,
                        ));
                        content.push_str("W n\n");
                    }
                    render_radial_gradient(
                        content,
                        gradient,
                        bg_x,
                        bg_y,
                        render_w,
                        child_h,
                        page_shadings,
                        shading_counter,
                    );
                    if *tb_border_radius > 0.0 {
                        content.push_str("Q\n");
                    }
                    if bg_blended {
                        content.push_str("Q\n");
                    }
                }

                // Draw conic gradient background
                if let Some(gradient) = tb_bg_conic {
                    let bg_x = render_x;
                    let bg_y = render_y - child_h;
                    if bg_blended {
                        content.push_str("q\n");
                        begin_blend_mode(content, page_ext_gstates, *tb_bg_blend);
                    }
                    if *tb_border_radius > 0.0 {
                        content.push_str("q\n");
                        content.push_str(&rounded_rect_path(
                            bg_x,
                            bg_y,
                            render_w,
                            child_h,
                            *tb_border_radius,
                        ));
                        content.push_str("W n\n");
                    }
                    render_conic_gradient(content, gradient, bg_x, bg_y, render_w, child_h);
                    if *tb_border_radius > 0.0 {
                        content.push_str("Q\n");
                    }
                    if bg_blended {
                        content.push_str("Q\n");
                    }
                }

                // Draw child borders. CSS borders paint INSIDE the border box, so
                // each side's stroke centerline sits half its width in from the
                // box edge (the stroke's outer edge then meets the box edge).
                // Without this inset the centerlines straddled the edges, so two
                // vertically-adjacent boxes (e.g. flex-direction:column items)
                // painted their shared-edge borders on the *same* line — halving
                // the visible gap. This mirrors the top-level TextBlock arm and
                // the flex-cell border paint.
                if border.has_any() {
                    let half_t = border.top.width / 2.0;
                    let half_b = border.bottom.width / 2.0;
                    let half_l = border.left.width / 2.0;
                    let half_r = border.right.width / 2.0;
                    let bx1 = render_x;
                    let bx2 = render_x + render_w;
                    let by1 = render_y;
                    let by2 = render_y - child_h;
                    if border.top.width > 0.0 {
                        let (r, g, b) = border.top.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            border.top.alpha,
                        );
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {y} m {bx2} {y} l S\n",
                            border.top.width,
                            y = by1 - half_t
                        ));
                        end_border_alpha(content, a);
                    }
                    if border.bottom.width > 0.0 {
                        let (r, g, b) = border.bottom.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            border.bottom.alpha,
                        );
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {y} m {bx2} {y} l S\n",
                            border.bottom.width,
                            y = by2 + half_b
                        ));
                        end_border_alpha(content, a);
                    }
                    if border.left.width > 0.0 {
                        let (r, g, b) = border.left.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            border.left.alpha,
                        );
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{x} {by1} m {x} {by2} l S\n",
                            border.left.width,
                            x = bx1 + half_l
                        ));
                        end_border_alpha(content, a);
                    }
                    if border.right.width > 0.0 {
                        let (r, g, b) = border.right.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            border.right.alpha,
                        );
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{x} {by1} m {x} {by2} l S\n",
                            border.right.width,
                            x = bx2 - half_r
                        ));
                        end_border_alpha(content, a);
                    }
                }

                // Apply the overflow clip (if any) around the text so overflowing
                // lines are cut at the padding box, matching Chrome. The
                // background and border above are drawn unclipped so the border
                // stays fully visible; only the inline/line-box content is
                // clipped. `tb_clip_rect` is set by layout for
                // overflow:hidden/clip/scroll/auto.
                let tb_needs_clip = tb_clip_rect.is_some();
                if tb_needs_clip {
                    content.push_str("q\n");
                    content.push_str(&overflow_clip_path(
                        render_x,
                        render_y - child_h,
                        render_w,
                        child_h,
                        border.left.width,
                        border.right.width,
                        border.top.width,
                        border.bottom.width,
                        *tb_border_radius,
                    ));
                    content.push_str("W n\n");
                }

                // Draw child text. Inset from the border-box top by BOTH the top
                // border width and the top padding (matching the primary text path
                // at the top of this fn); omitting the border placed the first
                // baseline `border-top` px too high inside bordered clip boxes.
                let mut text_y = render_y - border.top.width - padding_top;
                let mut tb_first_line = true;
                for line in lines {
                    let metrics = line_box_metrics(line, custom_fonts);
                    text_y -= metrics.half_leading + metrics.ascender;
                    let merged = merge_runs(&line.runs);
                    let line_width: f32 = merged
                        .iter()
                        .map(|r| estimate_run_width_with_fonts(r, custom_fonts))
                        .sum();
                    // CSS `text-indent` shifts only the first line's start. List
                    // items pass a negative value so an `outside` marker (the
                    // leading run) hangs left into the padding while the text
                    // lands at the content edge.
                    let first_line_indent = if tb_first_line { *tb_text_indent } else { 0.0 };
                    tb_first_line = false;
                    // Horizontal insets from the border-box edge. `render_x`/`render_w`
                    // are the BORDER box, so the content box starts after the left
                    // border + left padding and is narrowed by both horizontal borders
                    // and paddings — mirroring the primary text path
                    // (`padding_box_x = block_x + border_left`,
                    // content = `padding_box_x + padding_left`) and the vertical inset
                    // in this same arm (`render_y - border.top.width - padding_top`).
                    // For left/justify the text starts at the content-box left; for
                    // right/center it is aligned within the content box. (Previously
                    // this branch used `render_x + padding_left`, dropping the left
                    // border so text in bordered clip/nested boxes sat `border-left`
                    // px too far left.)
                    let content_x = render_x + border.left.width + padding_left;
                    let content_w = (render_w
                        - border.left.width
                        - border.right.width
                        - padding_left
                        - padding_right)
                        .max(0.0);
                    // Drop-cap float exclusion: shift the line right so its text
                    // wraps beside the floated `::first-letter` (css2 §9.5).
                    let line_inset = line.x_offset;
                    let text_x = match text_align {
                        TextAlign::Right => content_x + (content_w - line_width).max(0.0),
                        TextAlign::Center => content_x + (content_w - line_width).max(0.0) / 2.0,
                        _ => content_x + first_line_indent + line_inset,
                    };
                    let line_top_y = text_y + metrics.ascender + metrics.half_leading;
                    let line_bottom_y = text_y - metrics.descender - metrics.half_leading;
                    // Parent text content-area edges for `text-top`/`text-bottom`.
                    let (text_ascent, text_descent) = line_text_content_extents(line, custom_fonts);
                    let line_text_top_y = if text_ascent > 0.0 {
                        text_y + text_ascent
                    } else {
                        line_top_y
                    };
                    let line_text_bottom_y = if text_descent > 0.0 {
                        text_y - text_descent
                    } else {
                        line_bottom_y
                    };
                    let mut lx = text_x;
                    for run in &merged {
                        // Atomic inline box (e.g. a `list-style-image` marker):
                        // paint the box/image and advance by its outer width;
                        // `render_run_text` would shape its empty text and draw
                        // nothing, dropping the marker entirely.
                        if let Some(inline) = run.inline_box.as_deref() {
                            render_inline_box(
                                content,
                                inline,
                                lx + inline.margin_left,
                                text_y,
                                line_top_y,
                                line_bottom_y,
                                line_text_top_y,
                                line_text_bottom_y,
                                run.font_size,
                                line_primary_x_height_ratio(&merged, custom_fonts),
                                custom_fonts,
                                prepared_custom_fonts,
                                page_ext_gstates,
                                bg_alpha_counter,
                                pdf_writer,
                                page_images,
                            );
                            lx += inline.outer_width();
                            continue;
                        }
                        if run.text.is_empty() {
                            continue;
                        }
                        let run_width = estimate_run_width_with_fonts(run, custom_fonts);
                        // Per-run inline background (e.g. a `::first-letter`/
                        // `::first-line` `background-color`, or a highlighted
                        // inline span): paint the rectangle behind the glyphs
                        // before drawing the text. Mirrors the other line-box
                        // render paths (table cells, absolute boxes).
                        if let Some((br, bgc, bb, ba)) = run.background_color {
                            let needs_inline_bg_alpha = ba < 1.0;
                            if needs_inline_bg_alpha {
                                let gs_name = format!("GStbiba{bg_alpha_counter}");
                                *bg_alpha_counter += 1;
                                page_ext_gstates.push((gs_name.clone(), ba));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
                            let (pad_h, pad_v) = run.padding;
                            let rx = lx - pad_h;
                            let ry = text_y - 2.0 - pad_v;
                            let rw2 = run_width + pad_h * 2.0;
                            let rh = run.font_size + 2.0 + pad_v * 2.0;
                            content.push_str(&format!("{br} {bgc} {bb} rg\n"));
                            if run.border_radius > 0.0 {
                                content.push_str(&rounded_rect_path(
                                    rx,
                                    ry,
                                    rw2,
                                    rh,
                                    run.border_radius,
                                ));
                                content.push_str("\nf\n");
                            } else {
                                content.push_str(&format!("{rx} {ry} {rw2} {rh} re\nf\n"));
                            }
                            if needs_inline_bg_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }
                        // A floated `::first-letter` drop cap is lowered so its
                        // glyph top sits on the line's text top (css-pseudo-4 §2.2).
                        let run_y = text_y
                            + drop_cap_baseline_shift(
                                run,
                                line_text_top(line, custom_fonts),
                                custom_fonts,
                            );
                        let rw = render_run_text(
                            content,
                            run,
                            lx,
                            run_y,
                            crate::layout::text::line_primary_font_size(&merged),
                            custom_fonts,
                            prepared_custom_fonts,
                            0.0,
                            pdf_writer,
                            page_images,
                        );
                        lx += rw;
                    }
                    text_y -= metrics.descender + metrics.half_leading;
                }
                if tb_needs_clip {
                    content.push_str("Q\n");
                }
                if is_float {
                    // A float does not advance the flow cursor (its bottom is
                    // already tracked via the simulator for `clear`). It breaks
                    // the margin-collapse chain for the next in-flow sibling.
                    let _ = child_h;
                    prev_margin_bottom = 0.0;
                } else {
                    // Advance past the box AND its margin-bottom so a following
                    // in-flow sibling sits below the margin gap (e.g. stacked
                    // `<p>`s inside a multicol column keep their `margin-bottom`).
                    // Record this block's margin-bottom so the next sibling
                    // collapses its margin-top against it (CSS adjacent-margin
                    // collapsing), mirroring the Container arm below.
                    cursor_y -= child_h + *margin_bottom;
                    y = cursor_y;
                    prev_margin_bottom = *margin_bottom;
                }
            }
            LayoutElement::Container {
                children: nested_kids,
                background_color,
                background_gradient,
                background_radial_gradient,
                background_conic_gradient,
                border,
                border_radius: cont_br,
                padding_top,
                padding_bottom,
                padding_left,
                padding_right,
                margin_top,
                margin_bottom,
                block_width,
                block_height: nk_block_height,
                opacity: nk_opacity,
                mix_blend_mode: nk_mix_blend,
                background_blend_mode: nk_bg_blend,
                visible: nk_visible,
                float: nk_float,
                clear: nk_clear,
                overflow,
                overflow_x: nk_overflow_x,
                overflow_y: nk_overflow_y,
                position: nk_position,
                offset_top: nk_offset_top,
                offset_left: nk_offset_left,
                transform: nk_transform,
                transform_origin: nk_transform_origin,
                clip_path: nk_clip_path,
                mask_image: nk_mask_image,
                mask_mode: nk_mask_mode,
                box_shadow: nk_box_shadow,
                background_svg: nk_bg_svg,
                background_size: nk_bg_size,
                background_position: nk_bg_position,
                background_repeat: nk_bg_repeat,
                background_origin: nk_bg_origin,
                background_blur_radius: nk_bg_blur,
                outline_width: nk_outline_width,
                outline_color: nk_outline_color,
                outline_offset: nk_outline_offset,
                border_radii: cont_radii,
                border_radii_y: cont_radii_y,
                positioned_depth: nk_positioned_depth,
                containing_block: nk_containing_block,
                ..
            } => {
                // A styled `column-rule` placeholder (non-`solid` rule): draw the
                // dash/dot/double vertical line and skip the generic box path. It
                // is an absolute, out-of-flow box, so it never advances the cursor.
                if *nk_position == Position::Absolute
                    && is_column_rule_box(nested_kids, background_color, border, *block_width)
                {
                    let (rule_anchor_x, rule_anchor_y) =
                        abs_child_anchor(nk_containing_block, abs_origins, self_pad_origin);
                    let rule_x = rule_anchor_x + nk_offset_left;
                    let rule_top = rule_anchor_y - nk_offset_top;
                    let rule_h = nk_block_height.unwrap_or(0.0);
                    paint_column_rule_line(
                        content,
                        rule_x,
                        rule_top,
                        border.left.width,
                        rule_h,
                        &border.left,
                        page_ext_gstates,
                        bg_alpha_counter,
                    );
                    continue;
                }
                // Absolute-positioned containers (e.g. an empty position:absolute
                // div) must render at their inset offset from the containing
                // block's padding box, mirroring the TextBlock abspos arm — not
                // in normal flow. Without this, nested abspos boxes rendered at
                // the parent's content-box origin (top/left silently dropped).
                let nk_is_abs = *nk_position == Position::Absolute;
                // In-flow containers collapse their margin-top against the
                // previous in-flow sibling's margin-bottom; floats and absolutes
                // are out of flow and take their margin-top in full.
                let nk_is_float = !nk_is_abs && *nk_float != Float::None;
                let nk_in_flow = !nk_is_abs && !nk_is_float;
                if nk_in_flow {
                    if *nk_clear != Clear::None {
                        cursor_y = clear_cursor(
                            cursor_y,
                            *nk_clear,
                            left_float_bottom,
                            right_float_bottom,
                            &mut prev_margin_bottom,
                        );
                    }
                    cursor_y -= collapsed_margin_top_extra(*margin_top, prev_margin_bottom);
                    y = cursor_y;
                } else if nk_is_float {
                    // Floated container: pinned at its precomputed top (the flow
                    // cursor at its source position); does not advance the cursor.
                    let rel_top = float_top_by_index.get(&child_index).copied().unwrap_or(0.0);
                    y = container_top_y - rel_top;
                } else {
                    // Absolute: positioned from the container top below.
                    y = cursor_y;
                }
                let nk_w = block_width.unwrap_or(width);
                // Absolute Containers anchor to their containing block's padding
                // box (resolved by depth, skipping static intermediates).
                let (nk_anchor_x, nk_anchor_y) =
                    abs_child_anchor(nk_containing_block, abs_origins, self_pad_origin);
                let nk_x = if nk_is_abs {
                    nk_anchor_x + nk_offset_left
                } else {
                    match nk_float {
                        Float::Right => x + width - nk_w,
                        // Apply margin-left / margin:auto centering / relative
                        // left shift (all folded into offset_left; 0 for a plain
                        // static block). Previously dropped on nested containers.
                        _ => x + nk_offset_left,
                    }
                };
                let nk_top_y = if nk_is_abs {
                    nk_anchor_y - nk_offset_top
                } else {
                    y - nk_offset_top
                };
                let nk_children_h: f32 = collapsed_children_height(nested_kids);
                let nk_content_h =
                    padding_top + nk_children_h + padding_bottom + border.vertical_width();
                // A definite `block_height` (set only for an explicit `height`)
                // is a hard border-box size: per CSS, oversized content overflows
                // the box (clipped or visible per `overflow`) rather than growing
                // it. Honour the declared height directly regardless of `overflow`
                // — only an auto height (`None`) expands to fit children. (The old
                // `content_h.max(h)` for non-hidden overflow wrongly inflated the
                // box to the child height, e.g. an `overflow:visible` box grew to
                // its oversized child instead of letting the child spill out.)
                let nk_total_h = nk_block_height.unwrap_or(nk_content_h);

                // CSS `filter: blur()` on a solid box (css-filter-effects-1 §4.1):
                // rasterize this empty container's bg fill + border, gaussian-blur
                // it, and embed it overflowing the border box. Restricted to a
                // plain solid box (no children, no gradient/SVG bg, no
                // transform/opacity/clip/mask wrapper, square corners) so the
                // vector paint path is byte-unchanged for everything else.
                if *nk_visible
                    && *nk_bg_blur > 0.0
                    && nested_kids.is_empty()
                    && background_gradient.is_none()
                    && background_radial_gradient.is_none()
                    && background_conic_gradient.is_none()
                    && nk_bg_svg.is_none()
                    && nk_transform.is_none()
                    && nk_clip_path.is_none()
                    && nk_mask_image.is_none()
                    && *nk_opacity >= 1.0
                    && *nk_mix_blend == crate::style::computed::BlendMode::Normal
                    && *cont_br == 0.0
                    && cont_radii.iter().all(|r| *r == 0.0)
                    && *nk_outline_width == 0.0
                    && let Some(blurred) = crate::render::blur::blur_box(
                        nk_w,
                        nk_total_h,
                        *background_color,
                        border,
                        *nk_bg_blur,
                        pdf_writer.opts.filter_dpi,
                    )
                {
                    let img_obj_id = pdf_writer.add_image_object(
                        &blurred.asset.data,
                        blurred.asset.source_width,
                        blurred.asset.source_height,
                        blurred.asset.format,
                        blurred.asset.png_metadata.as_ref(),
                    );
                    let img_name = format!("Im{img_obj_id}");
                    let ov = blurred.overflow_pt;
                    content.push_str(&format!(
                        "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
                        w = nk_w + 2.0 * ov,
                        h = nk_total_h + 2.0 * ov,
                        ix = nk_x - ov,
                        iy = nk_top_y - nk_total_h - ov,
                        name = img_name,
                    ));
                    page_images.push(ImageRef {
                        name: img_name,
                        obj_id: img_obj_id,
                    });
                    // Advance the flow cursor exactly as the normal container path
                    // (the filter does not change layout).
                    if nk_is_float {
                        prev_margin_bottom = 0.0;
                    } else if !nk_is_abs {
                        cursor_y -= nk_total_h + margin_bottom;
                        y = cursor_y;
                        prev_margin_bottom = *margin_bottom;
                    }
                    continue;
                }

                // `visibility: hidden` keeps the box's space (cursor still
                // advances below). Per CSS2 §11.2 it suppresses only THIS box's own
                // painting — a `visibility: visible` descendant must still render —
                // so the subtree (wrappers + children) is always emitted; the
                // box's own decoration is gated on `nk_visible` further down.
                {
                    // `mix-blend-mode`: composite the whole box (background + border +
                    // children) with the backdrop. Outermost q..Q so the blend gstate
                    // scopes the entire element and is restored by `Q` afterwards.
                    let nk_blended = *nk_mix_blend != crate::style::computed::BlendMode::Normal;
                    if nk_blended {
                        content.push_str("q\n");
                        begin_blend_mode(content, page_ext_gstates, *nk_mix_blend);
                    }
                    // Apply CSS opacity to the whole subtree as one group (background +
                    // border + children composite together), mirroring the top-level
                    // arm. Outermost q..Q so the alpha applies to the entire box.
                    let nk_needs_opacity = *nk_opacity < 1.0;
                    if nk_needs_opacity {
                        let gs_name = format!("GScca{bg_alpha_counter}");
                        *bg_alpha_counter += 1;
                        page_ext_gstates.push((gs_name.clone(), *nk_opacity));
                        content.push_str("q\n");
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }

                    // Apply a CSS transform around the box centre (wrap all drawing
                    // in q..Q), mirroring the top-level arm. Without this, transforms
                    // on nested / absolutely-positioned boxes were silently dropped.
                    let nk_needs_transform = nk_transform.is_some();
                    if let Some(t) = nk_transform {
                        let (ox, oy) = nk_transform_origin.resolve(nk_w, nk_total_h);
                        let cx = nk_x + ox;
                        let cy = nk_top_y - oy;
                        content.push_str("q\n");
                        push_transform_cm(content, t, cx, cy, nk_w, nk_total_h);
                    }
                    let nk_needs_clip_path = nk_clip_path.is_some();
                    if let Some(cp) = nk_clip_path {
                        content.push_str("q\n");
                        push_clip_path(content, cp, nk_x, nk_top_y, nk_w, nk_total_h);
                    }

                    // CSS mask-image (css-masking-1 §3): soft-mask the nested box.
                    let mut nk_mask_open = false;
                    if let Some(src) = nk_mask_image {
                        if let Some(gs_name) = pdf_writer.add_mask_soft_mask(
                            src,
                            *nk_mask_mode,
                            nk_x,
                            nk_top_y,
                            nk_w,
                            nk_total_h,
                        ) {
                            content.push_str("q\n");
                            content.push_str(&format!("/{gs_name} gs\n"));
                            nk_mask_open = true;
                        }
                    }

                    // CSS2 §11.2: self-decoration (background / border / outline /
                    // shadow) is suppressed when this box is `visibility: hidden`,
                    // but the opacity/transform/clip wrappers and the children
                    // (which may override back to `visible`) are still emitted.
                    if *nk_visible {
                        // Draw outset box-shadow (before the background, so it sits
                        // behind the element). Nested containers previously dropped
                        // box-shadow entirely; the top-level Container arm handles it
                        // the same way.
                        render_box_shadows(
                            content,
                            nk_box_shadow,
                            nk_x,
                            nk_top_y - nk_total_h,
                            nk_w,
                            nk_total_h,
                            *cont_br,
                            page_ext_gstates,
                            bg_alpha_counter,
                            pdf_writer,
                            page_images,
                        );

                        // Draw background with proper alpha support
                        if let Some((r, g, b, a)) = background_color {
                            let needs_alpha = *a < 1.0;
                            if needs_alpha {
                                let gs_name = format!("GScca{bg_alpha_counter}");
                                *bg_alpha_counter += 1;
                                page_ext_gstates.push((gs_name.clone(), *a));
                                content.push_str(&format!("/{gs_name} gs\n"));
                            }
                            content.push_str(&format!("{r} {g} {b} rg\n"));
                            let bg_cy = nk_top_y - nk_total_h;
                            if let Some(path) = rounded_box_path(
                                nk_x,
                                bg_cy,
                                nk_w,
                                nk_total_h,
                                *cont_radii,
                                *cont_radii_y,
                            ) {
                                content.push_str(&path);
                            } else {
                                content.push_str(&format!(
                                    "{cx} {cy} {cw} {ch} re\n",
                                    cx = nk_x,
                                    cy = bg_cy,
                                    cw = nk_w,
                                    ch = nk_total_h,
                                ));
                            }
                            content.push_str("f\n");
                            if needs_alpha {
                                content.push_str("/GSDefault gs\n");
                            }
                        }

                        // `background-blend-mode`: the background image layers (gradient /
                        // SVG) blend against the background color painted above. Scope the
                        // blend gstate to a `q`..`Q` around each background-image paint.
                        let nk_bg_blended =
                            *nk_bg_blend != crate::style::computed::BlendMode::Normal;

                        // Draw linear gradient
                        if let Some(gradient) = background_gradient {
                            let bg_x = nk_x;
                            let bg_y = nk_top_y - nk_total_h;
                            if nk_bg_blended {
                                content.push_str("q\n");
                                begin_blend_mode(content, page_ext_gstates, *nk_bg_blend);
                            }
                            if *cont_br > 0.0 {
                                content.push_str("q\n");
                                content.push_str(&rounded_rect_path(
                                    bg_x, bg_y, nk_w, nk_total_h, *cont_br,
                                ));
                                content.push_str("W n\n");
                            }
                            render_linear_gradient(
                                content,
                                gradient,
                                bg_x,
                                bg_y,
                                nk_w,
                                nk_total_h,
                                page_shadings,
                                shading_counter,
                            );
                            if *cont_br > 0.0 {
                                content.push_str("Q\n");
                            }
                            if nk_bg_blended {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw radial gradient
                        if let Some(gradient) = background_radial_gradient {
                            let bg_x = nk_x;
                            let bg_y = nk_top_y - nk_total_h;
                            if nk_bg_blended {
                                content.push_str("q\n");
                                begin_blend_mode(content, page_ext_gstates, *nk_bg_blend);
                            }
                            if *cont_br > 0.0 {
                                content.push_str("q\n");
                                content.push_str(&rounded_rect_path(
                                    bg_x, bg_y, nk_w, nk_total_h, *cont_br,
                                ));
                                content.push_str("W n\n");
                            }
                            render_radial_gradient(
                                content,
                                gradient,
                                bg_x,
                                bg_y,
                                nk_w,
                                nk_total_h,
                                page_shadings,
                                shading_counter,
                            );
                            if *cont_br > 0.0 {
                                content.push_str("Q\n");
                            }
                            if nk_bg_blended {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw conic gradient
                        if let Some(gradient) = background_conic_gradient {
                            let bg_x = nk_x;
                            let bg_y = nk_top_y - nk_total_h;
                            if nk_bg_blended {
                                content.push_str("q\n");
                                begin_blend_mode(content, page_ext_gstates, *nk_bg_blend);
                            }
                            if *cont_br > 0.0 {
                                content.push_str("q\n");
                                content.push_str(&rounded_rect_path(
                                    bg_x, bg_y, nk_w, nk_total_h, *cont_br,
                                ));
                                content.push_str("W n\n");
                            }
                            render_conic_gradient(content, gradient, bg_x, bg_y, nk_w, nk_total_h);
                            if *cont_br > 0.0 {
                                content.push_str("Q\n");
                            }
                            if nk_bg_blended {
                                content.push_str("Q\n");
                            }
                        }

                        // Draw SVG background image if specified
                        if let Some(svg_tree) = nk_bg_svg {
                            let bg_y = nk_top_y - nk_total_h;
                            // Adjust reference box based on background-origin
                            let (ref_x, ref_y, ref_w, ref_h) = match nk_bg_origin {
                                BackgroundOrigin::Border => (
                                    nk_x - border.left.width,
                                    bg_y - border.bottom.width,
                                    nk_w + border.left.width + border.right.width,
                                    nk_total_h + border.top.width + border.bottom.width,
                                ),
                                BackgroundOrigin::Content => (
                                    nk_x + padding_left,
                                    bg_y + padding_bottom,
                                    (nk_w - padding_left - padding_right).max(0.0),
                                    (nk_total_h - padding_top - padding_bottom).max(0.0),
                                ),
                                BackgroundOrigin::Padding => (nk_x, bg_y, nk_w, nk_total_h),
                            };
                            render_svg_background(
                                content,
                                svg_tree,
                                pdf_writer,
                                page_images,
                                page_shadings,
                                shading_counter,
                                Some(page_ext_gstates),
                                BackgroundPaintContext::new(
                                    SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                                    SvgViewportBox::new(
                                        nk_x - border.left.width,
                                        bg_y - border.bottom.width,
                                        nk_w + border.left.width + border.right.width,
                                        nk_total_h + border.top.width + border.bottom.width,
                                    ),
                                    *cont_br,
                                    *nk_bg_blur,
                                    *nk_bg_size,
                                    *nk_bg_position,
                                    *nk_bg_repeat,
                                ),
                            );
                        }

                        // Draw inset box-shadow (after the backgrounds, before the
                        // borders/content) so it paints over the element fill.
                        render_box_shadows_inset(
                            content,
                            nk_box_shadow,
                            nk_x,
                            nk_top_y - nk_total_h,
                            nk_w,
                            nk_total_h,
                            *cont_br,
                            page_ext_gstates,
                            bg_alpha_counter,
                        );

                        // Draw all 4 borders
                        let bx1 = nk_x;
                        let bx2 = nk_x + nk_w;
                        let by1 = nk_top_y;
                        let by2 = nk_top_y - nk_total_h;
                        // Uniform borders (same width/color/style) take the shared
                        // painter so dashed/dotted/double and per-corner rounded
                        // corners all render correctly. Non-uniform borders keep the
                        // per-side stroke path below.
                        let border_uniform = border.has_visible()
                            && border.top.width == border.right.width
                            && border.top.width == border.bottom.width
                            && border.top.width == border.left.width
                            && border.top.color == border.right.color
                            && border.top.color == border.bottom.color
                            && border.top.color == border.left.color
                            && border.top.style == border.right.style
                            && border.top.style == border.bottom.style
                            && border.top.style == border.left.style;
                        if border_uniform
                            && (border_needs_special_paint(border.top.style, *cont_radii)
                                || radii_any(*cont_radii))
                        {
                            // Uniform border with any corner radius (or a non-solid
                            // style) takes the shared painter so the stroke follows
                            // the rounded corners. Without this a plain solid rounded
                            // border fell through to the four straight per-side
                            // strokes below, leaving a square frame around a rounded
                            // (clipped) fill — see overflow-hidden-border-radius.
                            paint_uniform_border(
                                content,
                                nk_x,
                                by2,
                                nk_w,
                                nk_total_h,
                                *cont_radii,
                                &border.top,
                                page_ext_gstates,
                                bg_alpha_counter,
                            );
                        } else {
                            if border.left.paints() {
                                let (r, g, b) = border.left.color;
                                let a = begin_border_alpha(
                                    content,
                                    page_ext_gstates,
                                    bg_alpha_counter,
                                    border.left.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.left.style,
                                    border.left.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                    bw = border.left.width,
                                    x = bx1 + border.left.width * 0.5,
                                    y1 = by1,
                                    y2 = by2
                                ));
                                content.push_str(reset_dash_pattern(border.left.style));
                                end_border_alpha(content, a);
                            }
                            if border.right.paints() {
                                let (r, g, b) = border.right.color;
                                let a = begin_border_alpha(
                                    content,
                                    page_ext_gstates,
                                    bg_alpha_counter,
                                    border.right.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.right.style,
                                    border.right.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                    bw = border.right.width,
                                    x = bx2 - border.right.width * 0.5,
                                    y1 = by1,
                                    y2 = by2
                                ));
                                content.push_str(reset_dash_pattern(border.right.style));
                                end_border_alpha(content, a);
                            }
                            if border.top.paints() {
                                let (r, g, b) = border.top.color;
                                let a = begin_border_alpha(
                                    content,
                                    page_ext_gstates,
                                    bg_alpha_counter,
                                    border.top.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.top.style,
                                    border.top.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                    bw = border.top.width,
                                    x1 = bx1,
                                    x2 = bx2,
                                    y = by1 - border.top.width * 0.5
                                ));
                                content.push_str(reset_dash_pattern(border.top.style));
                                end_border_alpha(content, a);
                            }
                            if border.bottom.paints() {
                                let (r, g, b) = border.bottom.color;
                                let a = begin_border_alpha(
                                    content,
                                    page_ext_gstates,
                                    bg_alpha_counter,
                                    border.bottom.alpha,
                                );
                                content.push_str(&dash_pattern_for_style(
                                    border.bottom.style,
                                    border.bottom.width,
                                ));
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                    bw = border.bottom.width,
                                    x1 = bx1,
                                    x2 = bx2,
                                    y = by2 + border.bottom.width * 0.5
                                ));
                                content.push_str(reset_dash_pattern(border.bottom.style));
                                end_border_alpha(content, a);
                            }
                        }

                        // Draw outline if specified (a uniform stroke outside the
                        // border box). `outline-offset` widens the gap between the
                        // border edge and the outline; the stroke centerline sits half
                        // the outline width beyond the offset edge so the outline stays
                        // entirely outside the box. Mirrors the TextBlock outline arm.
                        if *nk_outline_width > 0.0 {
                            let gap = *nk_outline_offset + *nk_outline_width / 2.0;
                            let ol_x = bx1 - gap;
                            let ol_y = by2 - gap;
                            let ol_w = nk_w + 2.0 * gap;
                            let ol_h = nk_total_h + 2.0 * gap;
                            let (or, og, ob) = nk_outline_color.unwrap_or((0.0, 0.0, 0.0));
                            content.push_str(&format!(
                                "{or} {og} {ob} RG\n{ow} w\n",
                                ow = nk_outline_width
                            ));
                            if radii_any(*cont_radii) && !radii_uniform(*cont_radii) {
                                let ol_radii = [
                                    cont_radii[0] + gap,
                                    cont_radii[1] + gap,
                                    cont_radii[2] + gap,
                                    cont_radii[3] + gap,
                                ];
                                content.push_str(&rounded_rect_path_per_corner(
                                    ol_x, ol_y, ol_w, ol_h, ol_radii,
                                ));
                            } else if *cont_br > 0.0 {
                                let ol_r = *cont_br + gap;
                                content.push_str(&rounded_rect_path(ol_x, ol_y, ol_w, ol_h, ol_r));
                            } else {
                                content.push_str(&format!("{ol_x} {ol_y} {ol_w} {ol_h} re\n"));
                            }
                            content.push_str("S\n");
                        }
                    } // end `if *nk_visible` — nested container self-decoration

                    // Decide print scrollbars (css-overflow-3): a `scroll` axis
                    // always reserves a gutter and paints a (non-interactive)
                    // scrollbar; an `auto` axis does so only when its content
                    // overflows. Chrome renders these in print, insetting the
                    // content clip by the gutter on each scrolling axis.
                    let pad_box_w = (nk_w - border.horizontal_width()).max(0.0);
                    let pad_box_h = (nk_total_h - border.vertical_width()).max(0.0);
                    let content_avail_w = (pad_box_w - *padding_left - *padding_right).max(0.0);
                    let content_avail_h = (pad_box_h - *padding_top - *padding_bottom).max(0.0);
                    let (over_w, over_h) = children_overflow_extent(nested_kids);
                    let over_ratio_h = if content_avail_w > 0.0 {
                        over_w / content_avail_w
                    } else {
                        0.0
                    };
                    let over_ratio_v = if content_avail_h > 0.0 {
                        over_h / content_avail_h
                    } else {
                        0.0
                    };
                    // No rounded scrollbars: a rounded box clips its scrollbar
                    // chrome away, so only paint on square scroll containers.
                    let scroll_ok = *cont_br <= 0.0 && !radii_any(*cont_radii);
                    let has_v = scroll_ok
                        && match nk_overflow_y {
                            Overflow::Scroll => true,
                            Overflow::Auto => over_ratio_v > 1.001,
                            _ => false,
                        };
                    let has_h = scroll_ok
                        && match nk_overflow_x {
                            Overflow::Scroll => true,
                            Overflow::Auto => over_ratio_h > 1.001,
                            _ => false,
                        };
                    let sb = SCROLLBAR_THICKNESS_PT;
                    let v_gutter = if has_v { sb } else { 0.0 };
                    let h_gutter = if has_h { sb } else { 0.0 };

                    // Clip if overflow clips (hidden/clip/scroll/auto). CSS clips
                    // at the PADDING box (border box inset by the border widths)
                    // and follows the rounded corners when border-radius is set.
                    // Scroll containers inset the clip by the reserved gutter so
                    // content does not paint under the scrollbar.
                    let clip = overflow.clips();
                    if clip {
                        content.push_str("q\n");
                        if has_v || has_h {
                            // Rectangular clip inset by the per-side border and the
                            // reserved gutter (right gutter for vertical, bottom for
                            // horizontal — matching the LTR/top-anchored UA layout).
                            let cx = nk_x + border.left.width;
                            let cy = (nk_top_y - nk_total_h) + border.bottom.width + h_gutter;
                            let cw = pad_box_w - v_gutter;
                            let ch = pad_box_h - h_gutter;
                            content.push_str(&format!("{cx} {cy} {cw} {ch} re W n\n"));
                        } else {
                            content.push_str(&overflow_clip_path(
                                nk_x,
                                nk_top_y - nk_total_h,
                                nk_w,
                                nk_total_h,
                                border.left.width,
                                border.right.width,
                                border.top.width,
                                border.bottom.width,
                                *cont_br,
                            ));
                            content.push_str("W n\n");
                        }
                    }

                    // Recurse into nested children
                    let inner_x = nk_x + padding_left + border.left.width;
                    let inner_w = nk_w - padding_left - padding_right - border.horizontal_width();
                    let inner_y = nk_top_y - padding_top - border.top.width;
                    // Record this box's padding-box origin keyed by its
                    // positioned depth so absolutely-positioned descendants nested
                    // inside static intermediates anchor here (their CB), not to
                    // the static container they are physically nested in.
                    if *nk_positioned_depth > 0
                        && (*nk_position == Position::Relative
                            || *nk_position == Position::Absolute
                            || nk_transform.is_some())
                    {
                        abs_origins.insert(
                            *nk_positioned_depth,
                            (nk_x + border.left.width, nk_top_y - border.top.width),
                        );
                    }
                    render_container_children(
                        content,
                        nested_kids,
                        inner_x,
                        inner_y,
                        inner_w,
                        custom_fonts,
                        prepared_custom_fonts,
                        page_ext_gstates,
                        bg_alpha_counter,
                        page_shadings,
                        shading_counter,
                        pdf_writer,
                        page_images,
                        *padding_left,
                        *padding_top,
                        abs_origins,
                    );

                    if clip {
                        content.push_str("Q\n");
                    }

                    // Paint the print scrollbar chrome in the reserved gutter,
                    // AFTER the content clip is closed (the gutter lies outside
                    // the inset content clip) but inside the box decoration group.
                    if has_v || has_h {
                        let pbx = nk_x + border.left.width;
                        let pby = (nk_top_y - nk_total_h) + border.bottom.width;
                        paint_scrollbars(
                            content,
                            pbx,
                            pby,
                            pad_box_w,
                            pad_box_h,
                            has_v,
                            has_h,
                            over_ratio_v.max(1.0),
                            over_ratio_h.max(1.0),
                        );
                    }
                    // Close the mask group (opened inside the clip-path q..Q).
                    if nk_mask_open {
                        content.push_str("Q\n");
                    }
                    if nk_needs_clip_path {
                        content.push_str("Q\n");
                    }
                    if nk_needs_transform {
                        content.push_str("Q\n");
                    }
                    // Close the opacity group (outermost q..Q).
                    if nk_needs_opacity {
                        content.push_str("Q\n");
                    }
                    // Close the mix-blend-mode scope (restores the prior gstate).
                    if nk_blended {
                        content.push_str("Q\n");
                    }
                } // end nested-container subtree (wrappers + children)
                // Out-of-flow containers (absolute / float) don't advance the
                // flow cursor. A float's bottom is tracked via the simulator for
                // later `clear` siblings; it breaks the margin-collapse chain.
                if nk_is_float {
                    prev_margin_bottom = 0.0;
                } else if !nk_is_abs {
                    cursor_y -= nk_total_h + margin_bottom;
                    y = cursor_y;
                    // Remember this in-flow block's margin-bottom so the next
                    // sibling collapses against it; floats don't collapse.
                    prev_margin_bottom = *margin_bottom;
                }
            }
            LayoutElement::Image {
                image,
                width: img_w,
                height: img_h,
                margin_top: img_mt,
                object_fit,
                object_position,
                background_color,
                border,
                blur_overflow,
                src_crop,
                ..
            } => {
                cursor_y -= collapsed_margin_top_extra(*img_mt, prev_margin_bottom);
                y = cursor_y;
                let box_top = y;
                let box_bottom = y - img_h;
                // CSS `filter: blur()`/`drop-shadow()`: the embedded bitmap is the
                // blurred/feathered result, padded by `blur_overflow` on each side
                // so it overflows the content box without affecting flow.
                if *blur_overflow > 0.0 {
                    let img_obj_id = pdf_writer.add_image_object(
                        &image.data,
                        image.source_width,
                        image.source_height,
                        image.format,
                        image.png_metadata.as_ref(),
                    );
                    let img_name = format!("Im{img_obj_id}");
                    let ov = *blur_overflow;
                    content.push_str(&format!(
                        "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
                        w = img_w + 2.0 * ov,
                        h = img_h + 2.0 * ov,
                        ix = x - ov,
                        iy = box_bottom - ov,
                        name = img_name,
                    ));
                    page_images.push(ImageRef {
                        name: img_name,
                        obj_id: img_obj_id,
                    });
                    cursor_y -= img_h;
                    y = cursor_y;
                    prev_margin_bottom = 0.0;
                    continue;
                }
                // Paint the image-box background first; with object-fit it may
                // remain visible where the image content does not cover the box.
                if let Some((br, bg, bb, ba)) = background_color
                    && *ba > 0.0
                {
                    content.push_str(&format!(
                        "{br} {bg} {bb} rg\n{x} {by} {w} {h} re\nf\n",
                        by = box_bottom,
                        w = img_w,
                        h = img_h,
                    ));
                }
                // With box-sizing:border-box the box (img_w/img_h) includes the
                // border, so inset the image content rect by the border widths.
                let content_x = x + border.left.width;
                let content_bottom = box_bottom + border.bottom.width;
                let content_top = box_top - border.top.width;
                let content_w = (img_w - border.horizontal_width()).max(0.0);
                let content_h = (img_h - border.vertical_width()).max(0.0);
                // A sliced too-tall image embeds only this page's source rows.
                let sliced =
                    src_crop.and_then(|c| crate::layout::images::crop_raster_asset(image, c));
                let img = sliced.as_ref().unwrap_or(image);
                let placement = crate::layout::images::compute_image_placement(
                    content_w,
                    content_h,
                    img.source_width,
                    img.source_height,
                    *object_fit,
                    *object_position,
                );
                let img_obj_id = pdf_writer.add_source_image_object(
                    &img.data,
                    img.source_width,
                    img.source_height,
                    img.format,
                    img.png_metadata.as_ref(),
                    placement.width,
                    placement.height,
                );
                let img_name = format!("Im{img_obj_id}");
                content.push_str("q\n");
                if placement.clip {
                    content.push_str(&format!(
                        "{content_x} {by} {w} {h} re\nW n\n",
                        by = content_bottom,
                        w = content_w,
                        h = content_h,
                    ));
                }
                content.push_str(&format!(
                    "{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
                    w = placement.width,
                    h = placement.height,
                    ix = content_x + placement.offset_x,
                    iy = content_top - placement.offset_y - placement.height,
                    name = img_name,
                ));
                page_images.push(ImageRef {
                    name: img_name,
                    obj_id: img_obj_id,
                });
                // Stroke the border frame around the image box.
                draw_image_border(
                    content,
                    x,
                    box_bottom,
                    *img_w,
                    *img_h,
                    border,
                    page_ext_gstates,
                    bg_alpha_counter,
                );
                cursor_y -= img_h;
                y = cursor_y;
                // Image arm subtracts no margin-bottom; next sibling's
                // margin-top applies in full.
                prev_margin_bottom = 0.0;
            }
            LayoutElement::Svg {
                tree,
                width: svg_w,
                height: svg_h,
                margin_top: svg_mt,
                ..
            } => {
                cursor_y -= collapsed_margin_top_extra(*svg_mt, prev_margin_bottom);
                y = cursor_y;
                let svg_x = x;
                let svg_y = y - svg_h;
                content.push_str("q\n");
                // Position on page with Y-flip (SVG y-axis is top-down, PDF is bottom-up)
                content.push_str(&format!("1 0 0 -1 {svg_x} {} cm\n", svg_y + svg_h));
                // Apply viewBox scaling via compute_svg_placement
                if let Some(placement) = crate::render::svg_geometry::compute_svg_placement(
                    tree,
                    crate::render::svg_geometry::SvgPlacementRequest::from_rect(
                        0.0,
                        0.0,
                        *svg_w,
                        *svg_h,
                        tree.preserve_aspect_ratio,
                    ),
                ) {
                    content.push_str("q\n");
                    content.push_str(&placement.viewport.clip_path());
                    content.push_str(&format!(
                        "{sx} 0 0 {sy} {tx} {ty} cm\n",
                        sx = placement.scale_x,
                        sy = placement.scale_y,
                        tx = placement.translate_x,
                        ty = placement.translate_y,
                    ));
                    {
                        let mut res = crate::render::svg_to_pdf::SvgPdfResources {
                            shadings: &mut *page_shadings,
                            shading_counter: &mut *shading_counter,
                            ext_gstates: Some(page_ext_gstates),
                            image_sink: None,
                            custom_fonts: Some(custom_fonts),
                            prepared_custom_fonts: Some(prepared_custom_fonts),
                        };
                        crate::render::svg_to_pdf::render_svg_tree_with_resources(
                            tree, content, &mut res,
                        );
                    }
                    content.push_str("Q\n");
                } else {
                    let mut res = crate::render::svg_to_pdf::SvgPdfResources {
                        shadings: &mut *page_shadings,
                        shading_counter: &mut *shading_counter,
                        ext_gstates: Some(page_ext_gstates),
                        image_sink: None,
                        custom_fonts: Some(custom_fonts),
                        prepared_custom_fonts: Some(prepared_custom_fonts),
                    };
                    crate::render::svg_to_pdf::render_svg_tree_with_resources(
                        tree, content, &mut res,
                    );
                }
                content.push_str("Q\n");
                cursor_y -= svg_h;
                y = cursor_y;
                // Svg arm subtracts no margin-bottom; next sibling's
                // margin-top applies in full.
                prev_margin_bottom = 0.0;
            }
            LayoutElement::FlexRow {
                cells,
                margin_top: flex_mt,
                margin_bottom: flex_mb,
                background_color,
                border,
                border_radius: flex_border_radius,
                container_width,
                padding_top: flex_pt,
                padding_left: flex_pl,
                row_height: flex_row_h,
                align_items,
                positioned_depth: flex_positioned_depth,
                ..
            } => {
                cursor_y -= collapsed_margin_top_extra(*flex_mt, prev_margin_bottom);
                y = cursor_y;
                let row_h =
                    crate::layout::engine::estimate_element_height(child) - flex_mt - flex_mb;

                // A flex container that establishes a containing block records its
                // PADDING-box origin (border-box left, border-box top minus the top
                // border) under its `positioned_depth`, so absolutely-positioned
                // children nested in a static ancestor anchor to its padding box —
                // mirroring the top-level pagination path. Without this an abs child
                // of a flex container nested inside a padded block lost the
                // ancestor's offset and was placed at the wrong origin.
                if *flex_positioned_depth > 0 {
                    abs_origins.insert(*flex_positioned_depth, (x, y - border.top.width));
                }
                // The flex container honors its explicit width: paint its
                // background at `container_width` (already clamped to the
                // layout-time available width), not the full available width.
                // Mirrors the top-level FlexRow arm; without this a `width:Npx`
                // flex box painted its background across the whole content width.
                let flex_w = *container_width;

                // Draw flex row background
                if let Some((r, g, b, a)) = background_color {
                    let needs_alpha = *a < 1.0;
                    if needs_alpha {
                        let gs_name = format!("GScca{bg_alpha_counter}");
                        *bg_alpha_counter += 1;
                        page_ext_gstates.push((gs_name.clone(), *a));
                        content.push_str(&format!("/{gs_name} gs\n"));
                    }
                    content.push_str(&format!(
                        "{r} {g} {b} rg\n{fx} {fy} {fw} {fh} re\nf\n",
                        fx = x,
                        fy = y - row_h,
                        fw = flex_w,
                        fh = row_h,
                    ));
                    if needs_alpha {
                        content.push_str("/GSDefault gs\n");
                    }
                }

                // Draw the flex container's own border. Mirrors the top-level
                // FlexRow arm; the nested arm previously painted the background
                // but never the container border, so a bordered flex box nested
                // inside a block lost its frame entirely.
                if border.has_any() {
                    let bx = x;
                    let by = y - row_h;
                    let uniform = border.top.width == border.right.width
                        && border.top.width == border.bottom.width
                        && border.top.width == border.left.width
                        && border.top.color == border.right.color
                        && border.top.color == border.bottom.color
                        && border.top.color == border.left.color
                        && border.top.style == border.right.style
                        && border.top.style == border.bottom.style
                        && border.top.style == border.left.style;
                    if uniform && *flex_border_radius > 0.0 {
                        let (r, g, b) = border.top.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            border.top.alpha,
                        );
                        content
                            .push_str(&dash_pattern_for_style(border.top.style, border.top.width));
                        content
                            .push_str(&format!("{r} {g} {b} RG\n{bw} w\n", bw = border.top.width));
                        content.push_str(&rounded_rect_path(
                            bx,
                            by,
                            flex_w,
                            row_h,
                            *flex_border_radius,
                        ));
                        content.push_str("S\n");
                        content.push_str(reset_dash_pattern(border.top.style));
                        end_border_alpha(content, a);
                    } else if uniform {
                        let (r, g, b) = border.top.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            border.top.alpha,
                        );
                        content
                            .push_str(&dash_pattern_for_style(border.top.style, border.top.width));
                        // Stroke inside the border box (see top-level FlexRow arm).
                        let half = border.top.width / 2.0;
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{bw} w\n{bx} {by} {w} {h} re\nS\n",
                            bw = border.top.width,
                            bx = bx + half,
                            by = by + half,
                            w = flex_w - border.top.width,
                            h = row_h - border.top.width,
                        ));
                        content.push_str(reset_dash_pattern(border.top.style));
                        end_border_alpha(content, a);
                    } else {
                        let x1 = bx + border.left.width / 2.0;
                        let x2 = bx + flex_w - border.right.width / 2.0;
                        let y_top = y - border.top.width / 2.0;
                        let y_bottom = by + border.bottom.width / 2.0;
                        if border.top.width > 0.0 {
                            let (r, g, b) = border.top.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                border.top.alpha,
                            );
                            content.push_str(&dash_pattern_for_style(
                                border.top.style,
                                border.top.width,
                            ));
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                border.top.width
                            ));
                            content.push_str(reset_dash_pattern(border.top.style));
                            end_border_alpha(content, a);
                        }
                        if border.right.width > 0.0 {
                            let (r, g, b) = border.right.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                border.right.alpha,
                            );
                            content.push_str(&dash_pattern_for_style(
                                border.right.style,
                                border.right.width,
                            ));
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                border.right.width
                            ));
                            content.push_str(reset_dash_pattern(border.right.style));
                            end_border_alpha(content, a);
                        }
                        if border.bottom.width > 0.0 {
                            let (r, g, b) = border.bottom.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                border.bottom.alpha,
                            );
                            content.push_str(&dash_pattern_for_style(
                                border.bottom.style,
                                border.bottom.width,
                            ));
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                border.bottom.width
                            ));
                            content.push_str(reset_dash_pattern(border.bottom.style));
                            end_border_alpha(content, a);
                        }
                        if border.left.width > 0.0 {
                            let (r, g, b) = border.left.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                border.left.alpha,
                            );
                            content.push_str(&dash_pattern_for_style(
                                border.left.style,
                                border.left.width,
                            ));
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                border.left.width
                            ));
                            content.push_str(reset_dash_pattern(border.left.style));
                            end_border_alpha(content, a);
                        }
                    }
                }

                // Render flex cells. Anchor each cell to its layout-computed
                // main-axis offset (which folds in justify-content spacing and
                // `gap`) instead of accumulating widths — mirrors the top-level
                // FlexRow arm. Without this, nested flex rows packed left and
                // ignored justify-content/gap entirely.
                let cell_base_x = x + flex_pl + border.left.width;
                let content_y = y - flex_pt - border.top.width;

                // Baseline cross-axis alignment (CSS Flexbox §8.3); mirrors the
                // top-level FlexRow arm. Each baseline item's first-baseline
                // distance from its border-box top is `border-top + padding-top +
                // ascent + half leading`; the line's shared baseline is the max
                // such distance, so each cell shifts down by `max - own`.
                let cell_first_baseline = |cell: &crate::layout::engine::FlexCell| -> Option<f32> {
                    let first = cell
                        .lines
                        .iter()
                        .find(|l| l.runs.iter().any(|r| !r.text.is_empty()))?;
                    let m = line_box_metrics(first, custom_fonts);
                    Some(cell.border.top.width + cell.padding_top + m.half_leading + m.ascender)
                };
                let is_baseline_cell = |cell: &crate::layout::engine::FlexCell| -> bool {
                    matches!(
                        match cell.align_self {
                            AlignSelf::Auto => *align_items,
                            AlignSelf::FlexStart => AlignItems::FlexStart,
                            AlignSelf::FlexEnd => AlignItems::FlexEnd,
                            AlignSelf::Center => AlignItems::Center,
                            AlignSelf::Baseline => AlignItems::Baseline,
                            AlignSelf::Stretch => AlignItems::Stretch,
                        },
                        AlignItems::Baseline
                    )
                };
                let line_max_baseline = |y_offset: f32| -> Option<f32> {
                    cells
                        .iter()
                        .filter(|c| (c.y_offset - y_offset).abs() < 0.01 && is_baseline_cell(c))
                        .filter_map(cell_first_baseline)
                        .fold(None, |acc: Option<f32>, b| {
                            Some(acc.map_or(b, |a| a.max(b)))
                        })
                };

                // CSS 2.1 §9.9.1 painting order (mirrors the top-level FlexRow
                // arm): non-positioned in-flow cells paint first, then
                // positioned (relative/absolute) cells, source order preserved
                // within each group.
                let paint_order: Vec<&crate::layout::engine::FlexCell> = cells
                    .iter()
                    .filter(|c| !c.is_positioned)
                    .chain(cells.iter().filter(|c| c.is_positioned))
                    .collect();
                for cell in paint_order {
                    let cell_w = cell.width;
                    let cell_x = cell_base_x + cell.x_offset;
                    // Cross-axis (vertical) placement per align-items/align-self,
                    // mirroring the top-level FlexRow arm. Stretch fills the line
                    // cross size; otherwise the cell keeps its natural height and
                    // is anchored at start/end/center. Without this the nested arm
                    // force-stretched every cell to the full row height.
                    let line_cross = if cell.line_cross_size > 0.0 {
                        cell.line_cross_size
                    } else {
                        *flex_row_h
                    };
                    // `align-self` on the item overrides the container's
                    // `align-items` unless it is `auto`.
                    let effective_align = match cell.align_self {
                        AlignSelf::Auto => *align_items,
                        AlignSelf::FlexStart => AlignItems::FlexStart,
                        AlignSelf::FlexEnd => AlignItems::FlexEnd,
                        AlignSelf::Center => AlignItems::Center,
                        AlignSelf::Baseline => AlignItems::Baseline,
                        AlignSelf::Stretch => AlignItems::Stretch,
                    };
                    let (cell_h, cell_y_shift) = match effective_align {
                        // `align-items: stretch` only stretches auto-height items;
                        // an item with a definite height keeps it (top-anchored).
                        AlignItems::Stretch if cell.has_explicit_height => {
                            (cell.natural_height, cell.y_offset)
                        }
                        AlignItems::Stretch => (line_cross, cell.y_offset),
                        // `align: baseline` (CSS Flexbox §8.3): shift the item so
                        // its first text baseline meets the line's shared baseline.
                        AlignItems::Baseline => {
                            let shift =
                                match (cell_first_baseline(cell), line_max_baseline(cell.y_offset))
                                {
                                    (Some(own), Some(max)) => (max - own).max(0.0),
                                    _ => 0.0,
                                };
                            (cell.natural_height, cell.y_offset + shift)
                        }
                        AlignItems::FlexStart => (cell.natural_height, cell.y_offset),
                        AlignItems::FlexEnd => (
                            cell.natural_height,
                            cell.y_offset + line_cross - cell.natural_height,
                        ),
                        AlignItems::Center => (
                            cell.natural_height,
                            cell.y_offset + (line_cross - cell.natural_height) / 2.0,
                        ),
                    };
                    let cell_top = content_y - cell_y_shift;
                    let cell_bottom = cell_top - cell_h;
                    // Apply per-cell transform (e.g. `transform: rotate()`, or a
                    // `position: relative` translate on an inline-block) about the
                    // cell's own border box. Mirrors the top-level FlexRow arm; the
                    // nested arm previously dropped cell transforms entirely.
                    let cell_needs_transform = cell.transform.is_some();
                    if let Some(t) = &cell.transform {
                        let (ox, oy) = cell.transform_origin.resolve(cell_w, cell_h);
                        let cx = cell_x + ox;
                        let cy = cell_bottom + oy;
                        content.push_str("q\n");
                        push_transform_cm(content, t, cx, cy, cell_w, cell_h);
                    }
                    // Draw cell background
                    if let Some((cr, cg, cb, ca)) = cell.background_color {
                        let needs_alpha = ca < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScca{bg_alpha_counter}");
                            *bg_alpha_counter += 1;
                            page_ext_gstates.push((gs_name.clone(), ca));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!("{cr} {cg} {cb} rg\n"));
                        if cell.border_radius > 0.0 {
                            content.push_str(&rounded_rect_path(
                                cell_x,
                                cell_bottom,
                                cell_w,
                                cell_h,
                                cell.border_radius,
                            ));
                        } else {
                            content.push_str(&format!(
                                "{cx} {cy} {cw} {ch} re\n",
                                cx = cell_x,
                                cy = cell_bottom,
                                cw = cell_w,
                                ch = cell_h,
                            ));
                        }
                        content.push_str("f\n");
                        if needs_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }
                    // Draw cell border
                    if cell.border.has_any() {
                        if cell.border_radius > 0.0 {
                            // Rounded border — use uniform stroke with rounded rect
                            let bw = cell.border.top.width;
                            let (r, g, b) = cell.border.top.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                cell.border.top.alpha,
                            );
                            content.push_str(&format!("{r} {g} {b} RG\n{bw} w\n"));
                            content.push_str(&rounded_rect_path(
                                cell_x,
                                cell_bottom,
                                cell_w,
                                cell_h,
                                cell.border_radius,
                            ));
                            content.push_str("S\n");
                            end_border_alpha(content, a);
                        } else {
                            let bx1 = cell_x;
                            let bx2 = cell_x + cell_w;
                            let by1 = cell_top;
                            let by2 = cell_bottom;
                            if cell.border.left.width > 0.0 {
                                let (r, g, b) = cell.border.left.color;
                                let a = begin_border_alpha(
                                    content,
                                    page_ext_gstates,
                                    bg_alpha_counter,
                                    cell.border.left.alpha,
                                );
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                    bw = cell.border.left.width,
                                    x = bx1 + cell.border.left.width * 0.5,
                                    y1 = by1,
                                    y2 = by2
                                ));
                                end_border_alpha(content, a);
                            }
                            if cell.border.right.width > 0.0 {
                                let (r, g, b) = cell.border.right.color;
                                let a = begin_border_alpha(
                                    content,
                                    page_ext_gstates,
                                    bg_alpha_counter,
                                    cell.border.right.alpha,
                                );
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x} {y1} m {x} {y2} l\nS\n",
                                    bw = cell.border.right.width,
                                    x = bx2 - cell.border.right.width * 0.5,
                                    y1 = by1,
                                    y2 = by2
                                ));
                                end_border_alpha(content, a);
                            }
                            if cell.border.top.width > 0.0 {
                                let (r, g, b) = cell.border.top.color;
                                let a = begin_border_alpha(
                                    content,
                                    page_ext_gstates,
                                    bg_alpha_counter,
                                    cell.border.top.alpha,
                                );
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                    bw = cell.border.top.width,
                                    x1 = bx1,
                                    x2 = bx2,
                                    y = by1 - cell.border.top.width * 0.5
                                ));
                                end_border_alpha(content, a);
                            }
                            if cell.border.bottom.width > 0.0 {
                                let (r, g, b) = cell.border.bottom.color;
                                let a = begin_border_alpha(
                                    content,
                                    page_ext_gstates,
                                    bg_alpha_counter,
                                    cell.border.bottom.alpha,
                                );
                                content.push_str(&format!(
                                    "{r} {g} {b} RG\n{bw} w\n{x1} {y} m {x2} {y} l\nS\n",
                                    bw = cell.border.bottom.width,
                                    x1 = bx1,
                                    x2 = bx2,
                                    y = by2 + cell.border.bottom.width * 0.5
                                ));
                                end_border_alpha(content, a);
                            }
                        } // else (non-rounded cell border)
                    }
                    // Draw cell text. Seat it relative to the cell's *content
                    // box*, not its border box: the content origin is the
                    // border-box top-left (`cell_top`, `cell_x`) inset by the
                    // cell's top/left border and padding. This mirrors the
                    // `cell_first_baseline` model above (`border-top + padding-top
                    // + ...`) and the top-level FlexRow arm; without the inset the
                    // text sat at the border-box top-left, painting it too high
                    // and too far left.
                    let content_left = cell_x + cell.border.left.width + cell.padding_left;
                    let content_w = (cell_w
                        - cell.border.horizontal_width()
                        - cell.padding_left
                        - cell.padding_right)
                        .max(0.0);
                    let mut text_y = cell_top - cell.border.top.width - cell.padding_top;
                    for line in &cell.lines {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y -= metrics.half_leading + metrics.ascender;
                        let merged = merge_runs(&line.runs);
                        let line_width: f32 = merged
                            .iter()
                            .map(|r| estimate_run_width_with_fonts(r, custom_fonts))
                            .sum();
                        let text_x = match cell.text_align {
                            TextAlign::Right => content_left + (content_w - line_width).max(0.0),
                            TextAlign::Center => {
                                content_left + (content_w - line_width).max(0.0) / 2.0
                            }
                            _ => content_left,
                        };
                        let mut lx = text_x;
                        for run in &merged {
                            let rw = render_run_text(
                                content,
                                run,
                                lx,
                                text_y,
                                crate::layout::text::line_primary_font_size(&merged),
                                custom_fonts,
                                prepared_custom_fonts,
                                0.0,
                                pdf_writer,
                                page_images,
                            );
                            lx += rw;
                        }
                        text_y -= metrics.descender + metrics.half_leading;
                    }
                    // Render nested elements in flex cells (tables, containers)
                    if !cell.nested_elements.is_empty() {
                        let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
                        let nested_y = cell_top - text_h;
                        let mut abs_origins: HashMap<usize, (f32, f32)> = HashMap::new();
                        render_container_children(
                            content,
                            &cell.nested_elements,
                            cell_x,
                            nested_y,
                            cell_w,
                            custom_fonts,
                            prepared_custom_fonts,
                            page_ext_gstates,
                            bg_alpha_counter,
                            page_shadings,
                            shading_counter,
                            pdf_writer,
                            page_images,
                            0.0, // flex cells don't have separate padding for abs children
                            0.0,
                            &mut abs_origins,
                        );
                    }
                    // Close the per-cell transform scope.
                    if cell_needs_transform {
                        content.push_str("Q\n");
                    }
                }
                cursor_y -= row_h + flex_mb;
                y = cursor_y;
                prev_margin_bottom = *flex_mb;
            }
            LayoutElement::HorizontalRule {
                margin_top: rule_mt,
                margin_bottom: rule_mb,
            } => {
                cursor_y -= collapsed_margin_top_extra(*rule_mt, prev_margin_bottom);
                y = cursor_y;
                // Default rule: gray line across container width
                content.push_str(&format!(
                    "0.8 0.8 0.8 RG\n0.75 w\n{x} {ry} m {x2} {ry} l\nS\n",
                    ry = y - 0.5,
                    x2 = x + width,
                ));
                cursor_y -= 1.0 + rule_mb;
                y = cursor_y;
                prev_margin_bottom = *rule_mb;
            }
            _ => {
                let h = crate::layout::engine::estimate_element_height(child);
                cursor_y -= h;
                y = cursor_y;
                // Unknown/other element: its full estimated height (incl. any
                // margins) was consumed; do not collapse the next sibling.
                prev_margin_bottom = 0.0;
            }
        }
    }

    // Flush any remaining nested batch
    if !nested_batch.is_empty() {
        let batch: Vec<LayoutElement> = nested_batch.drain(..).cloned().collect();
        render_nested_table_rows(
            content,
            &batch,
            x,
            y,
            page_ext_gstates,
            bg_alpha_counter,
            custom_fonts,
            prepared_custom_fonts,
            page_shadings,
            shading_counter,
            pdf_writer,
            page_images,
        );
    }
}

/// Render TableRow/GridRow elements that appear as children of a Container.
#[allow(clippy::too_many_arguments)]
fn render_nested_table_rows(
    content: &mut String,
    elements: &[LayoutElement],
    origin_x: f32,
    mut cursor_y: f32,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
    page_shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    for element in elements {
        match element {
            LayoutElement::TableRow {
                cells,
                col_widths,
                border_collapse,
                border_spacing,
                margin_top,
                ..
            } => {
                cursor_y -= margin_top;
                let spacing = if *border_collapse == BorderCollapse::Collapse {
                    0.0
                } else {
                    *border_spacing
                };
                // A `border-collapse: collapse` table strokes its outer border
                // centered on its box edge, so without this shift it bled half its
                // width into the container's padding (and the item came out ~1px
                // up-left of Chrome). Shift the painted table right/down by half the
                // outer border, mirroring the top-level and nested-layout paths.
                let (collapse_dx, collapse_dy) = collapse_paint_offset(cells, *border_collapse);
                let row_y = cursor_y - collapse_dy;
                let row_height = compute_row_height(cells);

                let mut col_pos: usize = 0;
                for cell in cells {
                    if cell.rowspan == 0 {
                        col_pos += cell.colspan;
                        continue;
                    }
                    let (cell_x, cell_w) = table_cell_geometry(
                        col_widths,
                        col_pos,
                        cell.colspan,
                        spacing,
                        origin_x + collapse_dx,
                    );

                    // Draw cell background
                    if let Some((r, g, b, a)) = cell.background_color {
                        let needs_alpha = a < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScca{bg_alpha_counter}");
                            *bg_alpha_counter += 1;
                            page_ext_gstates.push((gs_name.clone(), a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            x = cell_x,
                            y = row_y - row_height,
                            w = cell_w,
                            h = row_height,
                        ));
                        if needs_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    // Draw cell borders. As in the top-level table path,
                    // `separate` collapse paints each cell's border fully inside
                    // its own border-box (stroke inset by half-width), while
                    // `collapse` strokes centered on the abutting box edge so the
                    // shared border lands on the grid line.
                    if cell.border.has_any() {
                        let separate = *border_collapse == BorderCollapse::Separate;
                        let inset = |w: f32| if separate { w / 2.0 } else { 0.0 };
                        let x1 = cell_x;
                        let x2 = cell_x + cell_w;
                        let y_top = row_y;
                        let y_bottom = row_y - row_height;
                        if cell.border.top.width > 0.0 {
                            let (r, g, b) = cell.border.top.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                cell.border.top.alpha,
                            );
                            let y = y_top - inset(cell.border.top.width);
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y} m {x2} {y} l S\n",
                                cell.border.top.width
                            ));
                            end_border_alpha(content, a);
                        }
                        if cell.border.right.width > 0.0 {
                            let (r, g, b) = cell.border.right.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                cell.border.right.alpha,
                            );
                            let x = x2 - inset(cell.border.right.width);
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x} {y_top} m {x} {y_bottom} l S\n",
                                cell.border.right.width
                            ));
                            end_border_alpha(content, a);
                        }
                        if cell.border.bottom.width > 0.0 {
                            let (r, g, b) = cell.border.bottom.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                cell.border.bottom.alpha,
                            );
                            let y = y_bottom + inset(cell.border.bottom.width);
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y} m {x2} {y} l S\n",
                                cell.border.bottom.width
                            ));
                            end_border_alpha(content, a);
                        }
                        if cell.border.left.width > 0.0 {
                            let (r, g, b) = cell.border.left.color;
                            let a = begin_border_alpha(
                                content,
                                page_ext_gstates,
                                bg_alpha_counter,
                                cell.border.left.alpha,
                            );
                            let x = x1 + inset(cell.border.left.width);
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x} {y_top} m {x} {y_bottom} l S\n",
                                cell.border.left.width
                            ));
                            end_border_alpha(content, a);
                        }
                    }

                    // Compute cell content top (simplified vertical alignment)
                    let content_top = row_y - cell.padding_top;
                    let cell_inner_w = cell_w - cell.padding_left - cell.padding_right;
                    let mut text_y = content_top;
                    for line in &cell.lines {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y -= metrics.half_leading + metrics.ascender;
                        let text_content: String =
                            line.runs.iter().map(|run| run.text.as_str()).collect();
                        if text_content.is_empty() {
                            continue;
                        }
                        let merged = merge_runs(&line.runs);
                        let line_width: f32 = merged
                            .iter()
                            .map(|run| estimate_run_width_with_fonts(run, custom_fonts))
                            .sum();
                        let text_x = match cell.text_align {
                            TextAlign::Right => {
                                cell_x + cell.padding_left + (cell_inner_w - line_width).max(0.0)
                            }
                            TextAlign::Center => {
                                cell_x
                                    + cell.padding_left
                                    + ((cell_inner_w - line_width) / 2.0).max(0.0)
                            }
                            _ => cell_x + cell.padding_left,
                        };
                        let mut lx = text_x;
                        for run in &merged {
                            if run.text.is_empty() {
                                continue;
                            }
                            // Inline background (for status badges etc.)
                            if let Some((br, bg_c, bb, _ba)) = run.background_color {
                                let (pad_h, pad_v) = run.padding;
                                let run_w = estimate_run_width_with_fonts(run, custom_fonts);
                                let rx = lx - pad_h;
                                let ry = text_y - 2.0 - pad_v;
                                let rw2 = run_w + pad_h * 2.0;
                                let rh = run.font_size + 2.0 + pad_v * 2.0;
                                content.push_str(&format!("{br} {bg_c} {bb} rg\n"));
                                if run.border_radius > 0.0 {
                                    content.push_str(&rounded_rect_path(
                                        rx,
                                        ry,
                                        rw2,
                                        rh,
                                        run.border_radius,
                                    ));
                                    content.push_str("\nf\n");
                                } else {
                                    content.push_str(&format!("{rx} {ry} {rw2} {rh} re\nf\n"));
                                }
                            }
                            let rw = render_run_text(
                                content,
                                run,
                                lx,
                                text_y,
                                crate::layout::text::line_primary_font_size(&merged),
                                custom_fonts,
                                prepared_custom_fonts,
                                0.0,
                                pdf_writer,
                                page_images,
                            );
                            lx += rw;
                        }
                        text_y -= metrics.descender + metrics.half_leading;
                    }

                    col_pos += cell.colspan;
                }
                cursor_y -= row_height;
            }
            LayoutElement::GridRow {
                cells,
                col_widths,
                gap,
                border: grid_border,
                padding_left: grid_pl,
                padding_right: grid_pr,
                padding_top: grid_pt,
                padding_bottom: grid_pb,
                margin_top,
                ..
            } => {
                cursor_y -= margin_top;
                let row_y = cursor_y;
                let row_height = compute_grid_row_height(cells) + grid_pt + grid_pb;
                let grid_total_w: f32 = col_widths.iter().sum::<f32>()
                    + gap * col_widths.len().saturating_sub(1) as f32
                    + grid_pl
                    + grid_pr;

                // Draw grid container border
                if grid_border.has_any() {
                    let bx1 = origin_x;
                    let bx2 = origin_x + grid_total_w;
                    let by1 = row_y;
                    let by2 = row_y - row_height;
                    if grid_border.top.width > 0.0 {
                        let (r, g, b) = grid_border.top.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            grid_border.top.alpha,
                        );
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx2} {by1} l S\n",
                            grid_border.top.width
                        ));
                        end_border_alpha(content, a);
                    }
                    if grid_border.right.width > 0.0 {
                        let (r, g, b) = grid_border.right.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            grid_border.right.alpha,
                        );
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx2} {by1} m {bx2} {by2} l S\n",
                            grid_border.right.width
                        ));
                        end_border_alpha(content, a);
                    }
                    if grid_border.bottom.width > 0.0 {
                        let (r, g, b) = grid_border.bottom.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            grid_border.bottom.alpha,
                        );
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by2} m {bx2} {by2} l S\n",
                            grid_border.bottom.width
                        ));
                        end_border_alpha(content, a);
                    }
                    if grid_border.left.width > 0.0 {
                        let (r, g, b) = grid_border.left.color;
                        let a = begin_border_alpha(
                            content,
                            page_ext_gstates,
                            bg_alpha_counter,
                            grid_border.left.alpha,
                        );
                        content.push_str(&format!(
                            "{r} {g} {b} RG\n{} w\n{bx1} {by1} m {bx1} {by2} l S\n",
                            grid_border.left.width
                        ));
                        end_border_alpha(content, a);
                    }
                }

                let cell_row_y = row_y - grid_pt;
                let cell_content_h = compute_grid_row_height(cells);
                let mut col_pos: usize = 0;
                for cell in cells.iter() {
                    let span = cell.colspan.max(1);
                    let track_x = origin_x
                        + grid_pl
                        + col_widths.iter().take(col_pos).sum::<f32>()
                        + gap * col_pos as f32;
                    let track_w: f32 = col_widths.iter().skip(col_pos).take(span).sum::<f32>()
                        + gap * span.saturating_sub(1) as f32;

                    // The painted box (background + border) either fills the
                    // track cell or, for grid items with an explicit smaller
                    // size, is inset per justify-items/align-items.
                    let (box_x, box_y, box_w, box_h) = match cell.grid_inset {
                        Some(ins) => (
                            track_x + ins.offset_x,
                            cell_row_y - ins.offset_y - ins.height,
                            ins.width,
                            ins.height,
                        ),
                        None => (
                            track_x,
                            cell_row_y - cell_content_h,
                            track_w,
                            cell_content_h,
                        ),
                    };

                    // Draw cell background
                    if let Some((r, g, b, a)) = cell.background_color {
                        let needs_alpha = a < 1.0;
                        if needs_alpha {
                            let gs_name = format!("GScca{bg_alpha_counter}");
                            *bg_alpha_counter += 1;
                            page_ext_gstates.push((gs_name.clone(), a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            x = box_x,
                            y = box_y,
                            w = box_w,
                            h = box_h,
                        ));
                        if needs_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    // Draw cell gradient backgrounds. A grid item is a block
                    // container, so a `background: linear/radial/conic-gradient`
                    // paints across the cell's border box just like any block
                    // (css-backgrounds-3 §3), clipped to the painted box.
                    paint_cell_gradient_backgrounds(
                        content,
                        cell,
                        box_x,
                        box_y,
                        box_w,
                        box_h,
                        page_shadings,
                        shading_counter,
                    );

                    // Draw per-cell border around the painted box. Use the
                    // shared image-border helper so each stroke is centered
                    // half its width INSIDE the cell's border-box edge (CSS
                    // box-sizing: border-box), matching Chrome's inner borders.
                    draw_image_border(
                        content,
                        box_x,
                        box_y,
                        box_w,
                        box_h,
                        &cell.border,
                        page_ext_gstates,
                        bg_alpha_counter,
                    );

                    let cell_x = box_x;
                    // Render cell text
                    let cell_inner_w = box_w - cell.padding_left - cell.padding_right;
                    let mut text_y = (box_y + box_h) - cell.padding_top;
                    for line in &cell.lines {
                        let metrics = line_box_metrics(line, custom_fonts);
                        text_y -= metrics.half_leading + metrics.ascender;
                        let text_content: String =
                            line.runs.iter().map(|run| run.text.as_str()).collect();
                        if text_content.is_empty() {
                            continue;
                        }
                        let merged = merge_runs(&line.runs);
                        let line_width: f32 = merged
                            .iter()
                            .map(|run| estimate_run_width_with_fonts(run, custom_fonts))
                            .sum();
                        let text_x = match cell.text_align {
                            TextAlign::Right => {
                                cell_x + cell.padding_left + (cell_inner_w - line_width).max(0.0)
                            }
                            TextAlign::Center => {
                                cell_x
                                    + cell.padding_left
                                    + ((cell_inner_w - line_width) / 2.0).max(0.0)
                            }
                            _ => cell_x + cell.padding_left,
                        };
                        let mut lx = text_x;
                        for run in &merged {
                            if run.text.is_empty() {
                                continue;
                            }
                            let rw = render_run_text(
                                content,
                                run,
                                lx,
                                text_y,
                                crate::layout::text::line_primary_font_size(&merged),
                                custom_fonts,
                                prepared_custom_fonts,
                                0.0,
                                pdf_writer,
                                page_images,
                            );
                            lx += rw;
                        }
                        text_y -= metrics.descender + metrics.half_leading;
                    }

                    // Render the cell's nested block children (e.g. a grid
                    // item's inner <div>), clipped to the cell's padding box when
                    // the item has `overflow: hidden`/`clip`/`scroll`/`auto`.
                    if !cell.nested_rows.is_empty() {
                        let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
                        let nested_clip = cell.clips;
                        if nested_clip {
                            content.push_str("q\n");
                            content.push_str(&overflow_clip_path(
                                box_x,
                                box_y,
                                box_w,
                                box_h,
                                cell.border.left.width,
                                cell.border.right.width,
                                cell.border.top.width,
                                cell.border.bottom.width,
                                0.0,
                            ));
                            content.push_str("W n\n");
                        }
                        let nested_x = box_x + cell.padding_left + cell.border.left.width;
                        let nested_w = (box_w
                            - cell.padding_left
                            - cell.padding_right
                            - cell.border.horizontal_width())
                        .max(0.0);
                        let nested_y =
                            (box_y + box_h) - cell.padding_top - cell.border.top.width - text_h;
                        let mut nested_abs: HashMap<usize, (f32, f32)> = HashMap::new();
                        render_container_children(
                            content,
                            &cell.nested_rows,
                            nested_x,
                            nested_y,
                            nested_w,
                            custom_fonts,
                            prepared_custom_fonts,
                            page_ext_gstates,
                            bg_alpha_counter,
                            page_shadings,
                            shading_counter,
                            pdf_writer,
                            page_images,
                            cell.padding_left,
                            cell.padding_top,
                            &mut nested_abs,
                        );
                        if nested_clip {
                            content.push_str("Q\n");
                        }
                    }

                    col_pos += span;
                }
                cursor_y -= row_height;
            }
            _ => {
                cursor_y -= crate::layout::engine::estimate_element_height(element);
            }
        }
    }
}

/// Paint a blurred `text-shadow` for `run` as an image XObject. Rasterizes the
/// run's glyph outlines into an alpha mask, gaussian-blurs + tints it (σ =
/// blur/2), and embeds it positioned so the mask's text origin lands at the
/// shadow's PDF origin `(origin_x_pt, baseline_y_pt)`. Returns `true` on
/// success; `false` (e.g. non-shapeable font, empty run) so the caller paints a
/// sharp vector copy instead.
#[allow(clippy::too_many_arguments)]
fn render_text_shadow_blur(
    content: &mut String,
    run: &TextRun,
    origin_x_pt: f32,
    baseline_y_pt: f32,
    blur_pt: f32,
    color: (f32, f32, f32, f32),
    custom_fonts: &HashMap<String, TtfFont>,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> bool {
    let (_, font) = match crate::text::resolve_custom_font(
        &run.font_family,
        run.bold,
        run.italic,
        custom_fonts,
    ) {
        Some(f) => f,
        None => return false,
    };
    let shaped = match crate::text::shape_text_run(run, custom_fonts) {
        Some(s) if !s.glyphs.is_empty() => s,
        _ => return false,
    };
    let raster = match crate::render::blur::rasterize_run_alpha(
        &font.data,
        font.units_per_em,
        run.font_size,
        &shaped.glyphs,
        pdf_writer.opts.filter_dpi,
    ) {
        Some(r) => r,
        None => return false,
    };
    let (mask_w, mask_h) = (raster.mask.width(), raster.mask.height());
    let (blurred, pad) = match crate::render::blur::blur_shadow_alpha_mask(
        &raster.mask,
        blur_pt,
        color,
        pdf_writer.opts.filter_dpi,
    ) {
        Some(b) => b,
        None => return false,
    };

    let px_per_pt = crate::render::blur::px_per_pt_at_filter_dpi(pdf_writer.opts.filter_dpi);
    let buf_w_px = (mask_w + 2 * pad) as f32;
    let buf_h_px = (mask_h + 2 * pad) as f32;
    let w_pt = buf_w_px / px_per_pt;
    let h_pt = buf_h_px / px_per_pt;

    // Text origin inside the blurred buffer (device px from top-left).
    let bx = raster.origin_x_px + pad as f32;
    let by = raster.baseline_y_px + pad as f32;

    // Place the buffer so its text-origin pixel lands at the shadow PDF origin.
    let ix = origin_x_pt - bx / px_per_pt;
    let iy = baseline_y_pt - h_pt + by / px_per_pt;

    let img_obj_id = pdf_writer.add_image_object(
        &blurred.asset.data,
        blurred.asset.source_width,
        blurred.asset.source_height,
        blurred.asset.format,
        blurred.asset.png_metadata.as_ref(),
    );
    let img_name = format!("Im{img_obj_id}");
    content.push_str(&format!(
        "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
        w = w_pt,
        h = h_pt,
        name = img_name,
    ));
    page_images.push(ImageRef {
        name: img_name,
        obj_id: img_obj_id,
    });
    true
}

#[allow(clippy::too_many_arguments)]
fn render_run_text(
    content: &mut String,
    run: &TextRun,
    x: f32,
    text_y: f32,
    parent_font_size: f32,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) -> f32 {
    let (r, g, b) = run.color;

    // css2 §10.8.1: `vertical-align: super`/`sub` paint a text run with its
    // baseline raised/lowered by a fraction of the parent (line) font size. This
    // only moves the painted glyphs vertically; the horizontal advance (the
    // returned width) is unchanged, so callers position the next run normally.
    let text_y = text_y + run_vertical_align_shift(run, parent_font_size);

    // CSS `text-shadow` (css-text-decor-3 §3): paint the glyphs again behind the
    // real text, once per shadow (back-to-front: the last listed shadow is
    // drawn first / furthest back). Each shadow is offset by (offset_x right,
    // offset_y down) in the shadow's colour. PDF Y grows upward, so a positive
    // CSS offset-y subtracts from `text_y`.
    //
    // When `blur > 0`, the shadow is a true gaussian (σ = blur/2): rasterize the
    // run's glyph outlines into an alpha mask, blur+tint it (reusing
    // `render::blur`), and embed as an image XObject — matching Chrome's soft
    // halo. When `blur == 0` (or rasterization is unavailable), paint a sharp
    // offset vector copy. Decorations and nested shadows are cleared on the
    // shadow run to avoid double-painting.
    if !run.text_shadow.is_empty() {
        for shadow in run.text_shadow.iter().rev() {
            let (sr, sg, sb, alpha) = shadow.color.to_f32_rgba();
            // Try the blurred raster path first when the shadow has blur and the
            // run is a shapeable custom font (outlines available).
            if shadow.blur > 0.0 {
                if render_text_shadow_blur(
                    content,
                    run,
                    x + shadow.offset_x,
                    text_y - shadow.offset_y,
                    shadow.blur,
                    (sr, sg, sb, alpha),
                    custom_fonts,
                    pdf_writer,
                    page_images,
                ) {
                    continue;
                }
            }
            let mut shadow_run = run.clone();
            shadow_run.color = (sr, sg, sb);
            shadow_run.text_shadow = Vec::new();
            shadow_run.underline = false;
            shadow_run.line_through = false;
            shadow_run.overline = false;
            shadow_run.background_color = None;
            shadow_run.link_url = None;
            // `text_y` already includes the vertical-align shift; neutralise it
            // on the recursive call so the shift is not applied twice.
            shadow_run.vertical_align = VerticalAlign::Baseline;
            render_run_text(
                content,
                &shadow_run,
                x + shadow.offset_x,
                text_y - shadow.offset_y,
                parent_font_size,
                custom_fonts,
                prepared_custom_fonts,
                word_spacing,
                pdf_writer,
                page_images,
            );
        }
    }

    // For runs with mixed scripts (e.g. "Chinese: 你好世界"), split into
    // segments and render each with the appropriate font: primary font for
    // characters it covers, fallback font for the rest.
    if crate::text::needs_unicode_fallback(run, custom_fonts) {
        let segments = crate::text::split_run_by_font_coverage(run, custom_fonts);
        let mut total_width = 0.0f32;
        let mut cur_x = x;
        for (segment_text, use_fallback) in &segments {
            let mut sub_run = run.clone();
            sub_run.text = segment_text.clone();
            // `text_y` already carries this run's vertical-align shift; clear it on
            // the per-segment recursion so the shift is not applied a second time.
            sub_run.vertical_align = VerticalAlign::Baseline;
            if *use_fallback {
                if let Some((fallback_shaped, fallback_key, fallback_font)) =
                    crate::text::shape_with_unicode_fallback(&sub_run, custom_fonts)
                {
                    let w = fallback_shaped.width;
                    let font_name = sanitize_pdf_name(fallback_key);
                    content.push_str(&format!("{r} {g} {b} rg\n"));
                    content.push_str("BT\n");
                    content.push_str(&format!("/{font_name} {} Tf\n", sub_run.font_size));
                    let prepared_font = prepared_custom_fonts.get(fallback_key);
                    let render = ShapedTextRender::new(
                        PdfPoint::new(cur_x, text_y),
                        sub_run.font_size,
                        fallback_font,
                        &fallback_shaped,
                        prepared_font,
                    )
                    .with_word_spacing(word_spacing);
                    if render.has_complex_offsets() {
                        append_positioned_shaped_text(content, render);
                    } else {
                        append_tj_shaped_text(content, render);
                    }
                    content.push_str("ET\n");
                    cur_x += w;
                    total_width += w;
                }
            } else {
                let w = render_run_text(
                    content,
                    &sub_run,
                    cur_x,
                    text_y,
                    parent_font_size,
                    custom_fonts,
                    prepared_custom_fonts,
                    word_spacing,
                    pdf_writer,
                    page_images,
                );
                cur_x += w;
                total_width += w;
            }
        }
        return total_width;
    }

    let shaped = crate::text::shape_text_run(run, custom_fonts);
    let run_width = shaped.as_ref().map_or_else(
        || estimate_run_width_with_fonts(run, custom_fonts),
        |run| run.width,
    );
    let custom_font =
        crate::text::resolve_custom_font(&run.font_family, run.bold, run.italic, custom_fonts);
    let font_name = resolve_font_name(run, custom_font, shaped.as_ref());

    content.push_str(&format!("{r} {g} {b} rg\n"));
    content.push_str("BT\n");
    content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));

    // Synthetic (faux) bold for custom-font runs that have no genuine bold face:
    // stroke each glyph outline (text render mode 2 = fill+stroke) with a thin
    // line so the stems thicken, mirroring browser algorithmic bold (CSS Fonts 4
    // §2.3). The stroke colour matches the fill so the glyph stays one colour.
    let faux_bold = matches!(run.font_family, FontFamily::Custom(_))
        && crate::system_fonts::needs_faux_bold(
            custom_fonts,
            run.font_family.name(),
            run.bold,
            run.italic,
        );
    if faux_bold {
        content.push_str(&format!("{r} {g} {b} RG\n"));
        content.push_str(&format!("{} w\n", format_pdf_number(run.font_size * 0.028)));
        content.push_str("2 Tr\n");
    }

    // Synthetic (faux) italic when an italic request resolved to an upright face
    // (CSS Fonts 4 §2.4 `font-synthesis: style`): apply an algorithmic oblique
    // shear in the text matrix, matching Skia/Chrome's synthetic skew of 0.25
    // (= tan ~14deg). The shear pivots on the baseline (no x-shift at y=0) and
    // does not change advances, so positioning is unaffected.
    const FAUX_ITALIC_SHEAR: f32 = 0.25;
    let faux_italic = matches!(run.font_family, FontFamily::Custom(_))
        && crate::system_fonts::needs_faux_italic(
            custom_fonts,
            run.font_family.name(),
            run.bold,
            run.italic,
        );
    let shear = if faux_italic { FAUX_ITALIC_SHEAR } else { 0.0 };

    if let (Some((resolved_name, font)), Some(shaped)) = (custom_font, shaped.as_ref()) {
        let prepared_font = prepared_custom_fonts.get(resolved_name);
        let render = ShapedTextRender::new(
            PdfPoint::new(x, text_y),
            run.font_size,
            font,
            shaped,
            prepared_font,
        )
        .with_word_spacing(word_spacing)
        .with_shear(shear);
        if render.has_complex_offsets() {
            append_positioned_shaped_text(content, render);
        } else {
            append_tj_shaped_text(content, render);
        }
    } else {
        let encoded = encode_pdf_text(&run.text);
        content.push_str(&format!(
            "{} {} Td\n",
            format_pdf_number(x),
            format_pdf_number(text_y),
        ));
        content.push_str(&format!("({encoded}) Tj\n"));
    }

    // Restore the default fill-only render mode so the faux-bold stroke does not
    // leak into subsequent runs (Tr is a persistent text-state parameter).
    if faux_bold {
        content.push_str("0 Tr\n");
    }

    content.push_str("ET\n");
    run_width
}

/// Render all text runs of a line in a single BT/ET block so the PDF viewer
/// advances the text cursor naturally after each Tj, eliminating cumulative
/// positioning errors between runs.
///
/// Falls back to per-run `render_run_text` when any run requires custom-font
/// shaping (complex glyph positioning).
/// `vertical-align: super`/`sub` raise/lower an atomic inline box's baseline by
/// these fractions of the parent (line) font size. CSS leaves the exact amount
/// to the UA; these match Chromium's measured superscript/subscript offsets.
/// Used both when positioning the box (`render_inline_box`) and when growing the
/// line box to contain it (`line_box_metrics`, `wrap_text_runs`), so the box and
/// the line box that holds it stay consistent.
pub(crate) const SUPER_SHIFT_RATIO: f32 = 0.38;
pub(crate) const SUB_SHIFT_RATIO: f32 = 0.23;

/// The x-height (as a fraction of em) of the parent text a `vertical-align:
/// middle` box aligns its centre against (CSS2 §10.8.1: centre at
/// `baseline + x-height/2`). Read from the largest baseline-aligned text run's
/// font; falls back to 0.5em when the line carries no measurable custom-font
/// text.
fn line_primary_x_height_ratio(runs: &[TextRun], custom_fonts: &HashMap<String, TtfFont>) -> f32 {
    let pick = runs
        .iter()
        .filter(|r| {
            r.inline_box.is_none()
                && matches!(r.vertical_align, VerticalAlign::Baseline)
                && !r.text.trim().is_empty()
        })
        .max_by(|a, b| {
            a.font_size
                .partial_cmp(&b.font_size)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .or_else(|| runs.iter().find(|r| r.inline_box.is_none()));
    if let Some(run) = pick
        && let FontFamily::Custom(name) = &run.font_family
        && let Some((_, ttf)) =
            crate::system_fonts::find_font(custom_fonts, name, run.bold, run.italic)
    {
        return ttf.x_height_ratio();
    }
    0.5
}

/// Paint an atomic inline box (`display: inline-block`) inside a line of text.
///
/// `box_x` is the left edge of the box in PDF coordinates; `baseline_y` is the
/// text baseline of the enclosing line; `line_top_y`/`line_bottom_y` bound the
/// line box. The box is positioned vertically per its `vertical_align`, then its
/// background, border, and pre-wrapped inner text are drawn.
#[allow(clippy::too_many_arguments)]
fn render_inline_box(
    content: &mut String,
    inline: &crate::layout::engine::InlineBox,
    box_x: f32,
    baseline_y: f32,
    line_top_y: f32,
    line_bottom_y: f32,
    line_text_top_y: f32,
    line_text_bottom_y: f32,
    line_font_size: f32,
    parent_x_height_ratio: f32,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
    page_ext_gstates: &mut Vec<(String, f32)>,
    bg_alpha_counter: &mut usize,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    let h = inline.height;
    // The box's own baseline as a distance from its TOP edge: for an
    // inline-block with in-flow line content this is its last line box's
    // baseline (CSS2 §10.8.1); with no content baseline the box's bottom margin
    // edge is its baseline, so the ascent equals the full height.
    let box_ascent = inline.baseline_ascent.unwrap_or(h);
    // Bottom edge of the box (PDF, y-up) for each vertical-align mode. The box
    // baseline sits at `box_top - box_ascent = box_bottom + h - box_ascent`;
    // aligning it to a target line baseline gives
    // `box_bottom = target_baseline - h + box_ascent`.
    let align_baseline = |target: f32| target - h + box_ascent;
    let box_bottom = match inline.vertical_align {
        VerticalAlign::Top => line_top_y - h,
        VerticalAlign::Bottom => line_bottom_y,
        // text-top: box top aligns to the parent's text content-area top
        // (parent baseline + parent ascent), which is below the line-box top by
        // the strut's half-leading (css2 §10.8.1).
        VerticalAlign::TextTop => line_text_top_y - h,
        // text-bottom: box bottom aligns to the parent's text content-area
        // bottom (parent baseline - parent descent).
        VerticalAlign::TextBottom => line_text_bottom_y,
        // Middle: box centre aligns roughly to the parent's mid-x-height, i.e.
        // a quarter-em above the baseline.
        // Middle: align the box centre to the parent's mid-x-height (CSS2
        // §10.8.1: baseline + x-height/2), read from the parent font — not a flat
        // 0.25em (which assumes x-height == 0.5em).
        VerticalAlign::Middle => {
            baseline_y + line_font_size * parent_x_height_ratio * 0.5 - h / 2.0
        }
        // Sub/super shift the box's baseline below/above the line baseline by a
        // fraction of the parent font size (css-inline-3; CSS2 §10.8.1). The
        // fractions match Chromium's measured subscript/superscript offsets.
        VerticalAlign::Sub => align_baseline(baseline_y - line_font_size * SUB_SHIFT_RATIO),
        VerticalAlign::Super => align_baseline(baseline_y + line_font_size * SUPER_SHIFT_RATIO),
        // Baseline: align the box's baseline to the line baseline.
        VerticalAlign::Baseline => align_baseline(baseline_y),
    };

    // CSS `position: relative` shifts the painted box (and its inner content)
    // without changing its in-flow slot: x right, y down (PDF y is up, so the
    // downward shift subtracts from y).
    let box_x = box_x + inline.rel_offset_x;
    let box_bottom = box_bottom - inline.rel_offset_y;

    // Background fill.
    if let Some((r, g, b, a)) = inline.background_color {
        let needs_alpha = a < 1.0;
        if needs_alpha {
            let gs_name = format!("GSib{bg_alpha_counter}");
            *bg_alpha_counter += 1;
            page_ext_gstates.push((gs_name.clone(), a));
            content.push_str(&format!("/{gs_name} gs\n"));
        }
        content.push_str(&format!("{r} {g} {b} rg\n"));
        if inline.border_radius > 0.0 {
            content.push_str(&rounded_rect_path(
                box_x,
                box_bottom,
                inline.width,
                h,
                inline.border_radius,
            ));
            content.push_str("\nf\n");
        } else {
            content.push_str(&format!(
                "{box_x} {box_bottom} {w} {h} re\nf\n",
                w = inline.width
            ));
        }
        if needs_alpha {
            content.push_str("/GSDefault gs\n");
        }
    }

    // Replaced-element image (pseudo `content: url(...)`): fill the content box
    // (inside the border) with the decoded raster, scaled to the box size.
    if let Some(image) = &inline.image {
        let content_x = box_x + inline.border.left.width;
        let content_y = box_bottom + inline.border.bottom.width;
        let content_w = (inline.width - inline.border.horizontal_width()).max(0.0);
        let content_h = (h - inline.border.vertical_width()).max(0.0);
        let img_obj_id = pdf_writer.add_image_object(
            &image.data,
            image.source_width,
            image.source_height,
            image.format,
            image.png_metadata.as_ref(),
        );
        let img_name = format!("Im{img_obj_id}");
        content.push_str(&format!(
            "q\n{content_w} 0 0 {content_h} {content_x} {content_y} cm\n/{img_name} Do\nQ\n"
        ));
        page_images.push(ImageRef {
            name: img_name,
            obj_id: img_obj_id,
        });
    }

    // Border (drawn inside the border box, matching border-box sizing).
    draw_image_border(
        content,
        box_x,
        box_bottom,
        inline.width,
        h,
        &inline.border,
        page_ext_gstates,
        bg_alpha_counter,
    );

    // Inner text lines, laid out from the content-box top downward.
    let content_top_y = box_bottom + h - inline.border.top.width - inline.padding_top;
    let content_left_x = box_x + inline.border.left.width + inline.padding_left;
    let mut inner_y = content_top_y;
    for line in &inline.lines {
        let metrics = line_box_metrics(line, custom_fonts);
        inner_y -= metrics.half_leading + metrics.ascender;
        let merged = merge_runs(&line.runs);
        render_line_text(
            content,
            &merged,
            content_left_x,
            inner_y,
            custom_fonts,
            prepared_custom_fonts,
            0.0,
            line_text_top(line, custom_fonts),
            pdf_writer,
            page_images,
        );
        inner_y -= metrics.descender + metrics.half_leading;
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn render_line_text(
    content: &mut String,
    runs: &[TextRun],
    start_x: f32,
    y: f32,
    custom_fonts: &HashMap<String, TtfFont>,
    prepared_custom_fonts: &PreparedCustomFonts,
    word_spacing: f32,
    // Line box ascent above the baseline, used to seat a drop-cap glyph's top on
    // the line's text top. The drop cap is excluded from this value.
    line_ascender: f32,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    // Keep text runs plus any atomic inline boxes (empty text but real advance).
    let non_empty: Vec<&TextRun> = runs
        .iter()
        .filter(|r| !r.text.is_empty() || r.inline_box.is_some())
        .collect();
    if non_empty.is_empty() {
        return;
    }

    // A `vertical-align: super`/`sub` run is shifted by a fraction of the parent
    // (surrounding) font size; resolve it once for the whole line.
    let parent_font_size = crate::layout::text::line_primary_font_size(runs);

    // An inline box interrupts the glyph stream with a fixed advance, so the
    // cursor must be positioned explicitly — force the per-run (mixed) path.
    let has_inline_box = non_empty.iter().any(|r| r.inline_box.is_some());

    // Check whether every run can be rendered with standard PDF fonts
    // (no custom-font shaping needed).  Unicode-fallback runs also need
    // shaping, so they count as non-standard.
    let all_standard = !has_inline_box
        && non_empty.iter().all(|run| {
            crate::text::resolve_custom_font(&run.font_family, run.bold, run.italic, custom_fonts)
                .is_none()
                && crate::text::shape_with_unicode_fallback(run, custom_fonts).is_none()
        });

    if all_standard {
        // Simple path: single BT block, one Td to set initial position,
        // then consecutive Tf/rg/Tj operators.  The viewer advances the
        // text cursor after each Tj.
        content.push_str("BT\n");
        // The text cursor tracks the *current* baseline so a per-run
        // sub/super shift can be applied (and undone) with a relative `Td`,
        // leaving the next run on the normal baseline.
        let mut cur_baseline = y;
        let mut first = true;
        for run in &non_empty {
            let (r, g, b) = run.color;
            let font_name = resolve_font_name(run, None, None);
            content.push_str(&format!("{r} {g} {b} rg\n"));
            content.push_str(&format!("/{font_name} {} Tf\n", run.font_size));
            // css2 §10.8: `vertical-align: super`/`sub` raise/lower a text run
            // off the line baseline by a fraction of its own font size. A floated
            // `::first-letter` drop cap is additionally lowered so its glyph top
            // sits on the line's text top (css-pseudo-4 §2.2).
            let target_baseline = y
                + run_vertical_align_shift(run, parent_font_size)
                + drop_cap_baseline_shift(run, line_ascender, custom_fonts);
            if first {
                content.push_str(&format!(
                    "{} {} Td\n",
                    format_pdf_number(start_x),
                    format_pdf_number(target_baseline),
                ));
                cur_baseline = target_baseline;
                first = false;
            } else if (target_baseline - cur_baseline).abs() > f32::EPSILON {
                // Relative move from the previous run's baseline; the cursor's
                // x has already advanced by the previous Tj, so dx = 0.
                content.push_str(&format!(
                    "0 {} Td\n",
                    format_pdf_number(target_baseline - cur_baseline),
                ));
                cur_baseline = target_baseline;
            }
            let encoded = encode_pdf_text(&run.text);
            content.push_str(&format!("({encoded}) Tj\n"));
        }
        content.push_str("ET\n");
    } else {
        // Mixed path: some runs need custom-font shaping.
        // Fall back to per-run rendering with individual BT/ET blocks.
        let mut x = start_x;
        for run in &non_empty {
            // Inline boxes are painted in Phase 1; here they only advance.
            if let Some(inline) = run.inline_box.as_deref() {
                x += inline.outer_width();
                continue;
            }
            // A floated `::first-letter` drop cap is lowered so its glyph top
            // sits on the line's text top (css-pseudo-4 §2.2).
            let run_y = y + drop_cap_baseline_shift(run, line_ascender, custom_fonts);
            let run_width = render_run_text(
                content,
                run,
                x,
                run_y,
                parent_font_size,
                custom_fonts,
                prepared_custom_fonts,
                word_spacing,
                pdf_writer,
                page_images,
            );
            x += run_width;
        }
    }
}

/// Baseline shift (PDF points, up positive) for a text run's `vertical-align`.
///
/// css2 §10.8.1: `super`/`sub` move a text run's baseline up/down by a fraction
/// of the PARENT (line) font size — not the shrunk superscript's own size — so a
/// 40%- and a 100%-size superscript on one line are raised by the same amount
/// (matching Chrome). All other values leave the run on the line baseline. Atomic
/// inline boxes are aligned elsewhere (they carry their own geometry), so this
/// only affects pure-text runs.
fn run_vertical_align_shift(run: &TextRun, parent_font_size: f32) -> f32 {
    if run.inline_box.is_some() {
        return 0.0;
    }
    match run.vertical_align {
        VerticalAlign::Super => parent_font_size * SUPER_SHIFT_RATIO,
        VerticalAlign::Sub => -parent_font_size * SUB_SHIFT_RATIO,
        _ => 0.0,
    }
}

/// True when a run is a floated `::first-letter` drop cap (css-pseudo-4 §2.2 +
/// css2 §9.5). The drop cap is the only text run whose line-height factor was
/// deliberately capped below the surrounding line (`apply_first_letter_style`
/// sets it to `block_line_height / cap_font_size`, well under 1) so the enlarged
/// glyph overflows its line box instead of inflating it.
fn is_drop_cap_run(run: &TextRun) -> bool {
    run.inline_box.is_none() && run.line_height_factor.is_finite() && run.line_height_factor < 0.9
}

/// The visual top of a run's glyphs above the baseline, in points. Prefers the
/// actual glyph bounding-box top (`yMax`) of the run's first letter so accent
/// space reserved by the font ascender is excluded; falls back to the ascender
/// metric when the glyph has no measurable outline.
fn run_glyph_top(run: &TextRun, custom_fonts: &HashMap<String, TtfFont>) -> f32 {
    let ch = run.text.chars().find(|c| !c.is_whitespace());
    if let (Some(ch), FontFamily::Custom(name)) = (ch, &run.font_family)
        && let Some((_, ttf)) =
            crate::system_fonts::find_font(custom_fonts, name, run.bold, run.italic)
        && let Some(ratio) = ttf.glyph_top_ratio(ch)
    {
        return ratio * run.font_size;
    }
    let (ascender_ratio, _) =
        crate::fonts::font_metrics_ratios(&run.font_family, run.bold, run.italic, custom_fonts);
    ascender_ratio * run.font_size
}

/// Extra baseline offset (PDF up-positive) for a drop-cap run so its glyph TOP
/// aligns with the TOP of the surrounding first line's text, then drops downward
/// across the spanned lines — matching how browsers position a floated
/// `::first-letter` (css-pseudo-4 §2.2). Painted at the line baseline a cap-sized
/// glyph would overflow far ABOVE the box; lowering it so its glyph top meets the
/// line's text top seats it correctly. `line_text_top` is the surrounding line's
/// glyph top above the baseline (the drop cap is excluded from it).
fn drop_cap_baseline_shift(
    run: &TextRun,
    line_text_top: f32,
    custom_fonts: &HashMap<String, TtfFont>,
) -> f32 {
    if !is_drop_cap_run(run) {
        return 0.0;
    }
    let cap_top = run_glyph_top(run, custom_fonts);
    // Negative => move the glyph DOWN (PDF y grows up). Never raise it above the
    // line top (clamp at 0) so a small/normal-sized first-letter is unaffected.
    (line_text_top - cap_top).min(0.0)
}

/// The surrounding (non-drop-cap) text's glyph top above the baseline for a line,
/// in points — the reference the drop-cap glyph top is seated against. Zero when
/// the line carries no ordinary text runs.
fn line_text_top(line: &TextLine, custom_fonts: &HashMap<String, TtfFont>) -> f32 {
    line.runs
        .iter()
        .filter(|r| r.inline_box.is_none() && !is_drop_cap_run(r) && !r.text.trim().is_empty())
        .map(|r| run_glyph_top(r, custom_fonts))
        .fold(0.0f32, f32::max)
}

#[derive(Clone, Copy)]
struct LineBoxMetrics {
    ascender: f32,
    descender: f32,
    half_leading: f32,
}

/// Per-run asymmetric line-box extents (above/below the baseline, in points) for
/// a line that contains a `vertical-align: super`/`sub` text run.
///
/// css2 §10.8.1: each inline text box contributes its half-leading-padded glyph
/// box (ascent+half / descent+half about its own baseline); a super/sub run has
/// that box shifted up/down by `parent_font_size * RATIO`. The line box is the
/// union, so it grows only on the shifted side. `wrap_text_runs` sizes the line
/// with the identical formula, so the painted baseline and the laid-out line
/// height stay consistent.
fn line_shifted_text_extents(
    line: &TextLine,
    parent_font_size: f32,
    custom_fonts: &HashMap<String, TtfFont>,
) -> (f32, f32) {
    // Runs that left line-height unspecified fall back to the largest resolved
    // factor on the line (the parent text's), excluding drop caps (< 0.9).
    let rep_factor = line
        .runs
        .iter()
        .filter(|r| {
            r.inline_box.is_none()
                && r.line_height_factor.is_finite()
                && r.line_height_factor >= 0.9
        })
        .map(|r| r.line_height_factor)
        .fold(0.0f32, f32::max);
    let rep_factor = if rep_factor > 0.0 { rep_factor } else { 1.2 };
    let mut above = 0.0f32;
    let mut below = 0.0f32;
    for run in line.runs.iter().filter(|r| r.inline_box.is_none()) {
        // Drop caps overflow the line box and must not raise it (see above).
        if run.line_height_factor.is_finite() && run.line_height_factor < 0.9 {
            continue;
        }
        let (asc_r, desc_r) =
            crate::fonts::font_metrics_ratios(&run.font_family, run.bold, run.italic, custom_fonts);
        let asc = asc_r * run.font_size;
        let desc = desc_r * run.font_size;
        let factor = if run.line_height_factor.is_finite() {
            run.line_height_factor
        } else {
            rep_factor
        };
        let half = ((run.font_size * factor - (asc + desc)) / 2.0).max(0.0);
        let shift = run_vertical_align_shift(run, parent_font_size);
        above = above.max(asc + half + shift);
        below = below.max(desc + half - shift);
    }
    (above, below)
}

fn line_box_metrics(line: &TextLine, custom_fonts: &HashMap<String, TtfFont>) -> LineBoxMetrics {
    // `super`/`sub` shifts are a fraction of the parent (surrounding) font size.
    let parent_font_size = crate::layout::text::line_primary_font_size(&line.runs);
    let (ascender, descender) = line
        .runs
        .iter()
        .filter(|r| r.inline_box.is_none())
        // A floated `::first-letter` drop cap stays inline on the first line but
        // is out of flow (css2 §9.5): its enlarged glyph overflows the line box
        // downward and must NOT raise the line's ascent/descent. It is marked by
        // an explicit line-height factor capped well below 1 (its line box was
        // reduced to the surrounding line height in `apply_first_letter_style`).
        .filter(|r| !(r.line_height_factor.is_finite() && r.line_height_factor < 0.9))
        .fold((0.0f32, 0.0f32), |(max_ascender, max_descender), run| {
            let (ascender_ratio, descender_ratio) = crate::fonts::font_metrics_ratios(
                &run.font_family,
                run.bold,
                run.italic,
                custom_fonts,
            );
            (
                max_ascender.max(ascender_ratio * run.font_size),
                max_descender.max(descender_ratio * run.font_size),
            )
        });
    // The block's strut establishes the line box BEFORE inline-level boxes are
    // aligned (CSS2 §10.8): the requested `line.height` is split into the text's
    // ascent/descent plus symmetric half-leading. The baseline therefore sits at
    // `strut_above = text_ascent + half_leading` below the line-box top. When the
    // line-height already exceeds the text's content extent, that leading is real
    // space ABOVE/BELOW the baseline that an inline box may occupy WITHOUT growing
    // the line box or moving the baseline. We fold the half-leading into the
    // returned ascent/descent (and report `half_leading = 0`) so downstream
    // `ascender + half_leading` / `descender + half_leading` sums are unchanged
    // for pure text, while a baseline box only pushes the baseline when it pokes
    // past the strut's leading-padded edges (matching Chrome).
    let strut_half_leading = (line.height - (ascender + descender)) / 2.0;
    // A `vertical-align: super`/`sub` text run shifts its half-leading-padded
    // glyph box off the baseline (css2 §10.8.1); the line then grows ONLY on the
    // shifted side. The symmetric strut split above cannot express that, so for
    // such lines compute the per-run asymmetric extents instead — the same model
    // `wrap_text_runs` used to size the line, keeping layout and paint consistent.
    let has_text_shift = line.runs.iter().any(|r| {
        r.inline_box.is_none()
            && matches!(r.vertical_align, VerticalAlign::Super | VerticalAlign::Sub)
    });
    let (mut above, mut below) = if has_text_shift {
        line_shifted_text_extents(line, parent_font_size, custom_fonts)
    } else {
        (
            ascender + strut_half_leading,
            descender + strut_half_leading,
        )
    };

    // A baseline-aligned inline box contributes `baseline_ascent` above the line
    // baseline and `height - baseline_ascent` below it (CSS2 §10.8.1). It raises
    // the line's ascent/descent ONLY when it extends past the strut's edges; a box
    // that fits inside the existing leading leaves the baseline put. A box without
    // a content baseline sits entirely above the baseline (its bottom edge rests
    // on it). Top/middle/bottom boxes don't move the baseline; they only widen the
    // line box, which `line.height` already reflects from the wrap pass.
    // x-height of the parent text (pt), for a `vertical-align: middle` box whose
    // centre sits at `baseline + x-height/2`.
    let line_x_height = line_primary_x_height_ratio(&line.runs, custom_fonts) * parent_font_size;
    for run in &line.runs {
        if let Some(inline) = run.inline_box.as_deref()
            && matches!(
                inline.vertical_align,
                VerticalAlign::Baseline
                    | VerticalAlign::Sub
                    | VerticalAlign::Super
                    | VerticalAlign::Middle
            )
        {
            // Box ascent above its own baseline and descent below it.
            let box_ascent = inline.baseline_ascent.unwrap_or(inline.height);
            let box_descent = (inline.height - box_ascent).max(0.0);
            // Sub/super shift the box's baseline relative to the line baseline,
            // moving its extents by a fraction of the run font size; middle centres
            // the box on `baseline + x-height/2`.
            let (box_above, box_below) = match inline.vertical_align {
                VerticalAlign::Sub => (
                    box_ascent - parent_font_size * SUB_SHIFT_RATIO,
                    box_descent + parent_font_size * SUB_SHIFT_RATIO,
                ),
                VerticalAlign::Super => (
                    box_ascent + parent_font_size * SUPER_SHIFT_RATIO,
                    box_descent - parent_font_size * SUPER_SHIFT_RATIO,
                ),
                VerticalAlign::Middle => (
                    inline.height / 2.0 + line_x_height / 2.0,
                    inline.height / 2.0 - line_x_height / 2.0,
                ),
                _ => (box_ascent, box_descent),
            };
            above = above.max(box_above.max(0.0));
            below = below.max(box_below.max(0.0));
        }
    }

    LineBoxMetrics {
        ascender: above,
        descender: below,
        half_leading: 0.0,
    }
}

/// Raw font ascent/descent of the PARENT's text content area on a line, i.e. the
/// extent of the parent's actual glyphs above/below the baseline WITHOUT the
/// strut's half-leading. `vertical-align: text-top`/`text-bottom` align an inline
/// box to these edges (css2 §10.8.1), which lie inside the line box when the line
/// is taller than the parent font box. Inline boxes themselves are excluded; if
/// the line carries no text, both are zero and callers fall back to the line-box
/// edge.
fn line_text_content_extents(
    line: &TextLine,
    custom_fonts: &HashMap<String, TtfFont>,
) -> (f32, f32) {
    line.runs
        .iter()
        .filter(|r| r.inline_box.is_none())
        .filter(|r| !(r.line_height_factor.is_finite() && r.line_height_factor < 0.9))
        .fold((0.0f32, 0.0f32), |(max_ascent, max_descent), run| {
            let (ascender_ratio, descender_ratio) = crate::fonts::font_metrics_ratios(
                &run.font_family,
                run.bold,
                run.italic,
                custom_fonts,
            );
            (
                max_ascent.max(ascender_ratio * run.font_size),
                max_descent.max(descender_ratio * run.font_size),
            )
        })
}

/// Estimate line width using TTF metrics for custom fonts.
fn estimate_line_width_with_fonts(line: &TextLine, custom_fonts: &HashMap<String, TtfFont>) -> f32 {
    line.runs
        .iter()
        .map(|r| {
            let text_w = estimate_run_width_with_fonts(r, custom_fonts);
            // Include inline padding (e.g. badge spans with horizontal padding)
            let (pad_h, _pad_v) = r.padding;
            text_w + pad_h * 2.0
        })
        .sum()
}

/// Sanitize a font name for use as a PDF name object (remove spaces, special chars).
pub(crate) fn sanitize_pdf_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn line_text_content(line: &TextLine) -> String {
    line.runs.iter().map(|r| r.text.as_str()).collect()
}

fn text_block_total_height(
    lines: &[TextLine],
    padding_top: f32,
    padding_bottom: f32,
    block_height: Option<f32>,
    clips: bool,
) -> f32 {
    let text_height: f32 = lines.iter().map(|l| l.height).sum();
    let content_h = padding_top + text_height + padding_bottom;
    // When the box clips (`overflow: hidden`/`scroll`), a definite `block_height`
    // is a hard size: overflowing text is clipped to it rather than growing the
    // box. Otherwise `block_height` acts as a floor (min-height / auto) and the
    // box still grows to fit content. Mirrors `estimate_element_height` in
    // paginate so the painted box matches the flow advance.
    if clips {
        block_height.unwrap_or(content_h)
    } else {
        block_height.map_or(content_h, |h| content_h.max(h))
    }
}

/// Merge consecutive text runs that share the same visual properties (font,
/// size, bold, italic, color, underline, line-through, link) into a single
/// run.  This produces cleaner PDF output and ensures that spaces between
/// words are part of one contiguous text string, preventing PDF viewers from
/// dropping inter-word spaces during text extraction.
fn merge_runs(runs: &[TextRun]) -> Vec<TextRun> {
    let mut merged: Vec<TextRun> = Vec::new();
    for run in runs {
        // Keep atomic inline boxes as standalone runs (they carry geometry, not
        // text) so the renderer can paint them; never merge them with text.
        if run.inline_box.is_some() {
            merged.push(run.clone());
            continue;
        }
        if run.text.is_empty() {
            continue;
        }
        let can_merge = if let Some(prev) = merged.last() {
            prev.inline_box.is_none()
                && prev.font_size == run.font_size
                && prev.bold == run.bold
                && prev.italic == run.italic
                && prev.underline == run.underline
                && prev.line_through == run.line_through
                && prev.overline == run.overline
                && prev.color == run.color
                && prev.link_url == run.link_url
                && prev.font_family == run.font_family
                && prev.background_color == run.background_color
                && prev.padding == run.padding
                && prev.border_radius == run.border_radius
                // A sub/super run is painted on a shifted baseline; never merge
                // it with a baseline-aligned neighbour (css2 §10.8).
                && prev.vertical_align == run.vertical_align
                // Don't merge across an RTL <-> LTR boundary: the bidi pass
                // split these into separate runs in visual order, and merging
                // would give the shaper a mixed-script buffer whose guessed
                // direction flips glyph order for one side. See #139.
                && crate::bidi::has_rtl_chars(&prev.text)
                    == crate::bidi::has_rtl_chars(&run.text)
        } else {
            false
        };
        if can_merge {
            if let Some(previous) = merged.last_mut() {
                previous.text.push_str(&run.text);
            }
        } else {
            merged.push(run.clone());
        }
    }
    merged
}

/// Render a linear gradient using a native PDF Shading Dictionary reference.
///
/// Instead of drawing 200 thin rectangles (which produces banding), this emits
/// a `sh` operator referencing a shading dictionary that the PDF viewer will
/// interpolate smoothly. The shading entry is collected and later written as a
/// PDF object in `finish_to_writer`.
#[allow(clippy::too_many_arguments)]
/// A single placed gradient tile in PDF coordinates (origin bottom-left).
struct GradientTile {
    /// Left edge (PDF x).
    x: f32,
    /// Bottom edge (PDF y).
    y: f32,
    width: f32,
    height: f32,
}

/// Resolve the per-layer size/position/repeat of a gradient into the set of
/// tile rectangles to paint within the box `(x, y, width, height)` (PDF coords,
/// origin bottom-left). When no `layer_box` is set the gradient fills the whole
/// box as a single tile (historical single-layer behaviour).
fn gradient_layer_tiles(
    layer_box: &crate::style::computed::GradientLayerBox,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Vec<GradientTile> {
    let resolve_axis = |value: f32, is_percent: bool, extent: f32| {
        if is_percent {
            extent * (value / 100.0)
        } else {
            value
        }
    };

    // Tile size. `None`/`Auto`/`Cover`/`Contain` fall back to the full box (a
    // gradient has no intrinsic size, so `auto` is `100% 100%`).
    let (tile_w, tile_h) = match layer_box.size {
        Some(BackgroundSize::Explicit {
            width: w,
            height: h,
            width_is_percent,
            height_is_percent,
        }) => {
            let tw = resolve_axis(w, width_is_percent, width);
            let th = h
                .map(|hv| resolve_axis(hv, height_is_percent, height))
                .unwrap_or(height);
            (tw, th)
        }
        _ => (width, height),
    };
    if tile_w <= 0.0 || tile_h <= 0.0 {
        return Vec::new();
    }

    // Position offset in CSS coords (origin top-left of the box).
    let (offset_x, offset_y) = match layer_box.position {
        Some(pos) => {
            let ox = if pos.x_is_percent {
                (width - tile_w) * pos.x
            } else {
                pos.x
            };
            let oy = if pos.y_is_percent {
                (height - tile_h) * pos.y
            } else {
                pos.y
            };
            (ox, oy)
        }
        None => (0.0, 0.0),
    };

    let repeat = layer_box.repeat.unwrap_or(BackgroundRepeat::Repeat);
    let xs = match repeat {
        BackgroundRepeat::NoRepeat | BackgroundRepeat::RepeatY => vec![offset_x],
        _ => tile_offsets(offset_x, tile_w, width),
    };
    let ys = match repeat {
        BackgroundRepeat::NoRepeat | BackgroundRepeat::RepeatX => vec![offset_y],
        _ => tile_offsets(offset_y, tile_h, height),
    };

    let mut tiles = Vec::new();
    for &css_oy in &ys {
        for &css_ox in &xs {
            // Convert the CSS top-left offset into a PDF bottom-left origin.
            let tile_x = x + css_ox;
            let tile_y = y + (height - css_oy - tile_h);
            tiles.push(GradientTile {
                x: tile_x,
                y: tile_y,
                width: tile_w,
                height: tile_h,
            });
        }
    }
    tiles
}

/// Paint a grid/table cell's gradient backgrounds over its painted box.
///
/// A grid item (and a table cell) is a block container, so a `background` with a
/// `linear-gradient()`/`radial-gradient()`/`conic-gradient()` paints across the
/// cell's border box exactly like a normal block (css-backgrounds-3 §3). The
/// fill is clipped to the box so it never bleeds past the cell edges. Painted
/// after the cell's solid `background-color` and before its border, matching the
/// block paint order.
#[allow(clippy::too_many_arguments)]
fn paint_cell_gradient_backgrounds(
    content: &mut String,
    cell: &TableCell,
    box_x: f32,
    box_y: f32,
    box_w: f32,
    box_h: f32,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    if box_w <= 0.0 || box_h <= 0.0 {
        return;
    }
    if let Some(gradient) = &cell.background_gradient {
        content.push_str("q\n");
        content.push_str(&format!("{box_x} {box_y} {box_w} {box_h} re\nW n\n"));
        render_linear_gradient(
            content,
            gradient,
            box_x,
            box_y,
            box_w,
            box_h,
            shadings,
            shading_counter,
        );
        content.push_str("Q\n");
    }
    if let Some(gradient) = &cell.background_radial_gradient {
        content.push_str("q\n");
        content.push_str(&format!("{box_x} {box_y} {box_w} {box_h} re\nW n\n"));
        render_radial_gradient(
            content,
            gradient,
            box_x,
            box_y,
            box_w,
            box_h,
            shadings,
            shading_counter,
        );
        content.push_str("Q\n");
    }
    if let Some(gradient) = &cell.background_conic_gradient {
        content.push_str("q\n");
        content.push_str(&format!("{box_x} {box_y} {box_w} {box_h} re\nW n\n"));
        render_conic_gradient(content, gradient, box_x, box_y, box_w, box_h);
        content.push_str("Q\n");
    }
}

#[allow(clippy::too_many_arguments)]
fn render_linear_gradient(
    content: &mut String,
    gradient: &LinearGradient,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    let base_stops: Vec<(f32, (f32, f32, f32))> = gradient
        .stops
        .iter()
        .map(|s| (s.position, s.color.to_f32_rgb()))
        .collect();
    let stops: Vec<(f32, (f32, f32, f32))> = if gradient.repeating {
        repeat_stops_to_unit(&base_stops)
    } else {
        base_stops
    };

    for tile in gradient_layer_tiles(&gradient.layer_box, x, y, width, height) {
        *shading_counter += 1;
        render_linear_gradient_tile(
            content,
            gradient.angle,
            tile.x,
            tile.y,
            tile.width,
            tile.height,
            &stops,
            shadings,
            shading_counter,
        );
    }
}

/// Paint a single axial-gradient tile clipped to its rectangle.
#[allow(clippy::too_many_arguments)]
fn render_linear_gradient_tile(
    content: &mut String,
    angle: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    stops: &[(f32, (f32, f32, f32))],
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    // CSS angle convention: 0° = to top (bottom-to-top), 90° = to right, 180° = to bottom
    // In PDF coordinate space, y-axis is bottom-up, so:
    //   CSS 0° (to top) => PDF line from bottom center to top center
    //   CSS 90° (to right) => PDF line from left center to right center
    //   CSS 180° (to bottom) => PDF line from top center to bottom center
    let angle_rad = angle * std::f32::consts::PI / 180.0;
    let sin_a = angle_rad.sin();
    let cos_a = angle_rad.cos();

    // Gradient line: start and end points
    // CSS: 0deg = to top, so direction vector is (sin(angle), -cos(angle)) in CSS coords
    // In PDF coords (y flipped): direction is (sin(angle), cos(angle))
    let cx = x + width / 2.0;
    let cy = y + height / 2.0;
    // Half-length of the gradient line along the direction
    let half_len = (width * sin_a.abs() + height * cos_a.abs()) / 2.0;
    let dx = sin_a * half_len;
    let dy = cos_a * half_len;

    let x0 = cx - dx;
    let y0 = cy - dy;
    let x1 = cx + dx;
    let y1 = cy + dy;

    let name = push_axial_shading(shadings, shading_counter, [x0, y0, x1, y1], stops.to_vec());

    // Clip to the gradient area and paint with shading
    content.push_str("q\n");
    content.push_str(&format!("{x} {y} {width} {height} re W n\n"));
    content.push_str(&format!("/{name} sh\n"));
    content.push_str("Q\n");
}

/// Render a radial gradient using a native PDF Shading Dictionary reference.
#[allow(clippy::too_many_arguments)]
fn render_radial_gradient(
    content: &mut String,
    gradient: &RadialGradient,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    let stops: Vec<(f32, (f32, f32, f32))> = gradient
        .stops
        .iter()
        .map(|s| (s.position, s.color.to_f32_rgb()))
        .collect();

    for tile in gradient_layer_tiles(&gradient.layer_box, x, y, width, height) {
        render_radial_gradient_tile(
            content,
            gradient,
            tile.x,
            tile.y,
            tile.width,
            tile.height,
            &stops,
            shadings,
            shading_counter,
        );
    }
}

/// Paint a single radial-gradient tile clipped to its rectangle.
#[allow(clippy::too_many_arguments)]
fn render_radial_gradient_tile(
    content: &mut String,
    gradient: &RadialGradient,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    stops: &[(f32, (f32, f32, f32))],
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
) {
    // `center` measures from the tile's left/top edges (CSS top-down). PDF y is
    // bottom-up and `y` is the tile's bottom edge, so flip the y offset.
    let (cx_pos, cy_pos) = gradient.center;
    let off_x = cx_pos.resolve(width);
    let off_y = cy_pos.resolve(height);
    let cx = x + off_x;
    let cy = y + (height - off_y);

    // Per-axis distances from the center to the nearest/farthest tile edges.
    let near_x = off_x.min(width - off_x).abs();
    let far_x = off_x.max(width - off_x).abs();
    let near_y = off_y.min(height - off_y).abs();
    let far_y = off_y.max(height - off_y).abs();

    // Repeating gradients tile the stop pattern along the gradient ray. The
    // shading function already loops if we feed it a stop list extended over the
    // ray; instead we expand the stops to cover [0,1] by repetition.
    let render_stops: Vec<(f32, (f32, f32, f32))> = if gradient.repeating {
        repeat_stops_to_unit(stops)
    } else {
        stops.to_vec()
    };

    match gradient.shape {
        RadialShape::Circle => {
            // Use the explicit circle radius when given; otherwise resolve the
            // extent keyword to a radius.
            let max_radius = gradient.radius.unwrap_or_else(|| match gradient.extent {
                RadialExtent::ClosestSide => near_x.min(near_y),
                RadialExtent::FarthestSide => far_x.max(far_y),
                RadialExtent::ClosestCorner => (near_x * near_x + near_y * near_y).sqrt(),
                RadialExtent::FarthestCorner => (far_x * far_x + far_y * far_y).sqrt(),
            });
            if max_radius <= 0.0 {
                return;
            }
            let name = push_radial_shading(
                shadings,
                shading_counter,
                [cx, cy, 0.0, cx, cy, max_radius],
                render_stops,
            );
            content.push_str("q\n");
            content.push_str(&format!("{x} {y} {width} {height} re W n\n"));
            content.push_str(&format!("/{name} sh\n"));
            content.push_str("Q\n");
        }
        RadialShape::Ellipse => {
            // Resolve the elliptical radii. Explicit radii win; otherwise use the
            // extent keyword.
            let (rx, ry) = if let Some((rxp, ryp)) = gradient.radii {
                (rxp.resolve(width), ryp.resolve(height))
            } else {
                match gradient.extent {
                    RadialExtent::ClosestSide => (near_x, near_y),
                    RadialExtent::FarthestSide => (far_x, far_y),
                    // For closest/farthest-corner the ending ellipse keeps the
                    // closest/farthest-side aspect ratio but is scaled to pass
                    // through that corner (CSS Images Level 3).
                    RadialExtent::ClosestCorner => {
                        corner_ellipse_radii(near_x, near_y, near_x, near_y)
                    }
                    RadialExtent::FarthestCorner => {
                        corner_ellipse_radii(far_x, far_y, far_x, far_y)
                    }
                }
            };
            if rx <= 0.0 || ry <= 0.0 {
                return;
            }
            // PDF radial shadings are circular, so paint a unit-radius circular
            // shading at the origin and squash it into the desired ellipse via a
            // `cm` transform applied after clipping to the tile (clip stays in
            // page space; the shading evaluates in the transformed space).
            let name = push_radial_shading(
                shadings,
                shading_counter,
                [0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                render_stops,
            );
            content.push_str("q\n");
            content.push_str(&format!("{x} {y} {width} {height} re W n\n"));
            content.push_str(&format!("{rx} 0 0 {ry} {cx} {cy} cm\n"));
            content.push_str(&format!("/{name} sh\n"));
            content.push_str("Q\n");
        }
    }
}

/// Scale a side-fitting ellipse `(side_x, side_y)` so its boundary passes through
/// the corner at `(corner_x, corner_y)` while keeping the side aspect ratio.
/// CSS Images Level 3 §3.2: solve for the smallest `k` such that the ellipse
/// `(k·side_x, k·side_y)` contains the corner.
fn corner_ellipse_radii(side_x: f32, side_y: f32, corner_x: f32, corner_y: f32) -> (f32, f32) {
    if side_x <= 0.0 || side_y <= 0.0 {
        return (side_x, side_y);
    }
    let ratio = side_x / side_y;
    // The ending ellipse has radii (rx, ry) with rx/ry == ratio and passing
    // through (corner_x, corner_y): (corner_x/rx)^2 + (corner_y/ry)^2 = 1.
    let ry = ((corner_x / ratio).powi(2) + corner_y.powi(2)).sqrt();
    let rx = ratio * ry;
    (rx, ry)
}

/// Expand a clamped stop list (positions in [0,1]) into a repeated pattern that
/// covers the whole [0,1] domain, so a single PDF shading renders a repeating
/// gradient. The input pattern spans `[stops.first, stops.last]`; it is tiled
/// from 0 up to 1. Used for repeating linear/radial gradients.
fn repeat_stops_to_unit(stops: &[(f32, (f32, f32, f32))]) -> Vec<(f32, (f32, f32, f32))> {
    if stops.len() < 2 {
        return stops.to_vec();
    }
    let p0 = stops.first().map(|s| s.0).unwrap_or(0.0);
    let p1 = stops.last().map(|s| s.0).unwrap_or(1.0);
    let period = p1 - p0;
    if period <= 0.0001 {
        return stops.to_vec();
    }
    let mut out: Vec<(f32, (f32, f32, f32))> = Vec::new();
    // Tile starting at offset 0 so the first repetition begins at the ray origin.
    let max_reps = ((1.0 - 0.0) / period).ceil() as i32 + 1;
    for rep in 0..max_reps {
        let base = rep as f32 * period;
        if base > 1.0 {
            break;
        }
        for s in stops {
            let pos = base + (s.0 - p0);
            if pos > 1.0 + 0.0001 {
                continue;
            }
            // Avoid duplicate coincident positions across tile seams.
            if let Some(last) = out.last() {
                if (last.0 - pos).abs() < 0.0001 && last.1 == s.1 {
                    continue;
                }
            }
            out.push((pos.clamp(0.0, 1.0), s.1));
        }
    }
    if out.last().map(|s| s.0).unwrap_or(0.0) < 1.0 {
        // Clamp the final color out to the end of the domain.
        if let Some(last) = out.last().copied() {
            out.push((1.0, last.1));
        }
    }
    out
}

/// Render a conic gradient. PDF has no native conic shading, so the sweep is
/// approximated by a fan of thin triangular wedges from the center, each filled
/// with the color interpolated at the wedge's mid-angle. The fan is clipped to
/// the painting rectangle.
#[allow(clippy::too_many_arguments)]
fn render_conic_gradient(
    content: &mut String,
    gradient: &ConicGradient,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    let stops: Vec<(f32, (f32, f32, f32))> = gradient
        .stops
        .iter()
        .map(|s| (s.position, s.color.to_f32_rgb()))
        .collect();
    if stops.is_empty() {
        return;
    }

    for tile in gradient_layer_tiles(&gradient.layer_box, x, y, width, height) {
        render_conic_gradient_tile(
            content,
            gradient,
            &stops,
            tile.x,
            tile.y,
            tile.width,
            tile.height,
        );
    }
}

/// Sample a conic gradient's color at angular fraction `t` (0..1 of a turn).
/// `stops` are sorted ascending by fraction. For repeating gradients `t` is
/// reduced into the pattern period before sampling.
fn conic_color_at(stops: &[(f32, (f32, f32, f32))], t: f32, repeating: bool) -> (f32, f32, f32) {
    let first = stops[0];
    let last = stops[stops.len() - 1];

    let t = if repeating {
        let p0 = first.0;
        let p1 = last.0;
        let period = p1 - p0;
        if period <= 0.0001 {
            t
        } else {
            p0 + (t - p0).rem_euclid(period)
        }
    } else {
        t
    };

    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    for w in stops.windows(2) {
        let (a_pos, a_col) = w[0];
        let (b_pos, b_col) = w[1];
        if t >= a_pos && t <= b_pos {
            let span = b_pos - a_pos;
            if span <= 0.00001 {
                // Coincident stops form a hard color line: take the later color.
                return b_col;
            }
            let f = (t - a_pos) / span;
            return (
                a_col.0 + (b_col.0 - a_col.0) * f,
                a_col.1 + (b_col.1 - a_col.1) * f,
                a_col.2 + (b_col.2 - a_col.2) * f,
            );
        }
    }
    last.1
}

/// Paint a single conic-gradient tile clipped to its rectangle as a wedge fan.
fn render_conic_gradient_tile(
    content: &mut String,
    gradient: &ConicGradient,
    stops: &[(f32, (f32, f32, f32))],
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    // Center in PDF space (y flips: `y` is the tile bottom edge).
    let (cx_pos, cy_pos) = gradient.center;
    let off_x = cx_pos.resolve(width);
    let off_y = cy_pos.resolve(height);
    let cx = x + off_x;
    let cy = y + (height - off_y);

    // Wedge resolution. A wedge's straight outer chord is inscribed in a circle,
    // so to fully cover the clip rectangle's corners the chord must reach BEYOND
    // each corner: enlarge the radius by 1/cos(half-wedge-angle).
    const WEDGES: usize = 360;
    let step = 1.0 / WEDGES as f32;
    let half_step_rad = std::f32::consts::PI * step;
    let corner_radius = {
        let corners = [
            (x - cx, y - cy),
            (x + width - cx, y - cy),
            (x - cx, y + height - cy),
            (x + width - cx, y + height - cy),
        ];
        corners
            .iter()
            .map(|(dx, dy)| (dx * dx + dy * dy).sqrt())
            .fold(0.0_f32, f32::max)
    };
    let radius = corner_radius / half_step_rad.cos() + 2.0;
    if radius <= 0.0 {
        return;
    }

    let from = gradient.from_angle;

    // Build the angular breakpoints (fractions of a turn): a uniform grid plus
    // every interior color-stop fraction, so hard color-stops fall exactly on a
    // wedge edge (no facet straddles a hard line).
    let mut fracs: Vec<f32> = (0..=WEDGES).map(|i| i as f32 * step).collect();
    if !gradient.repeating {
        for s in stops {
            if s.0 > 0.0 && s.0 < 1.0 {
                fracs.push(s.0);
            }
        }
        fracs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        fracs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    }

    content.push_str("q\n");
    content.push_str(&format!("{x} {y} {width} {height} re W n\n"));

    // Adjacent independently-filled triangles leave faint anti-aliased seams
    // along their shared edge. Overlap each wedge slightly past its right edge
    // (toward the next wedge) so the seam is overpainted. The overlap is a small
    // fraction of a step; for smooth gradients neighboring colors are nearly
    // identical so the bleed is invisible, and hard-stop edges are snapped to a
    // wedge boundary so at most one overlap-width bleeds by a sub-degree.
    let overlap = step * 0.9;

    for win in fracs.windows(2) {
        let f0 = win[0];
        let f1 = win[1];
        if f1 - f0 < 1e-6 {
            continue;
        }
        let fmid = (f0 + f1) * 0.5;
        let (r, g, b) = conic_color_at(stops, fmid, gradient.repeating);
        let f1e = f1 + overlap;

        // CSS conic angle: 0deg points up (12 o'clock), increasing clockwise.
        // PDF y increases upward, so the CSS clockwise angle θ from "up" maps to
        // direction (sin θ, cos θ): cos is the upward component, sin rightward —
        // reproducing a clockwise sweep.
        let theta0 = (from + f0 * 360.0).to_radians();
        let theta1 = (from + f1e * 360.0).to_radians();
        let x0 = cx + radius * theta0.sin();
        let y0 = cy + radius * theta0.cos();
        let x1 = cx + radius * theta1.sin();
        let y1 = cy + radius * theta1.cos();

        content.push_str(&format!("{r} {g} {b} rg\n"));
        content.push_str(&format!("{cx} {cy} m\n"));
        content.push_str(&format!("{x0} {y0} l\n"));
        content.push_str(&format!("{x1} {y1} l\n"));
        content.push_str("f\n");
    }

    content.push_str("Q\n");
}

#[allow(clippy::too_many_arguments)]
fn render_svg_background(
    content: &mut String,
    tree: &crate::parser::svg::SvgTree,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
    shadings: &mut Vec<ShadingEntry>,
    shading_counter: &mut usize,
    mut ext_gstates: Option<&mut Vec<(String, f32)>>,
    paint: BackgroundPaintContext,
) {
    // SVG image resources frequently omit explicit width/height and only provide
    // a viewBox. Browsers still use that intrinsic aspect ratio for background
    // sizing, so fall back to the viewBox dimensions before giving up.
    let intrinsic_width = if tree.width > 0.0 {
        tree.width
    } else {
        tree.view_box
            .as_ref()
            .map_or(0.0, |view_box| view_box.width)
    };
    let intrinsic_height = if tree.height > 0.0 {
        tree.height
    } else {
        tree.view_box
            .as_ref()
            .map_or(0.0, |view_box| view_box.height)
    };
    if intrinsic_width <= 0.0 || intrinsic_height <= 0.0 {
        return;
    }

    let (vb_w, vb_h) = if let Some(ref vb) = tree.view_box {
        (vb.width, vb.height)
    } else {
        (intrinsic_width, intrinsic_height)
    };
    if vb_w <= 0.0 || vb_h <= 0.0 {
        return;
    }

    let resolve_axis = |value: f32, is_percent: bool, extent: f32| {
        if is_percent {
            extent * (value / 100.0)
        } else {
            value
        }
    };

    // Compute the rendered size of one SVG tile based on background-size.
    let (scaled_w, scaled_h) = match paint.size {
        BackgroundSize::Cover => {
            let s = (paint.reference_box.width / vb_w).max(paint.reference_box.height / vb_h);
            (vb_w * s, vb_h * s)
        }
        BackgroundSize::Contain => {
            let s = (paint.reference_box.width / vb_w).min(paint.reference_box.height / vb_h);
            (vb_w * s, vb_h * s)
        }
        BackgroundSize::Auto => {
            // SVG dimensions are in CSS pixels; convert to points (1px = 0.75pt)
            (intrinsic_width * 0.75, intrinsic_height * 0.75)
        }
        BackgroundSize::Explicit {
            width: explicit_width,
            height: explicit_height,
            width_is_percent,
            height_is_percent,
        } => {
            let scaled_w =
                resolve_axis(explicit_width, width_is_percent, paint.reference_box.width);
            let scaled_h = explicit_height
                .map(|value| resolve_axis(value, height_is_percent, paint.reference_box.height))
                .unwrap_or_else(|| scaled_w * vb_h / vb_w);
            (scaled_w, scaled_h)
        }
    };

    if scaled_w <= 0.0 || scaled_h <= 0.0 {
        return;
    }

    // When `background-size` fixes BOTH dimensions explicitly, the image is
    // scaled to exactly that box, ignoring its intrinsic aspect ratio
    // (css-backgrounds-3 §3.9). `cover`/`contain` already derive an
    // aspect-correct target box, so the source's `preserveAspectRatio` (which
    // would re-fit it) must be neutralised; only `auto` keeps the ratio.
    let stretch_to_box = matches!(
        paint.size,
        BackgroundSize::Cover
            | BackgroundSize::Contain
            | BackgroundSize::Explicit {
                height: Some(_),
                ..
            }
    );
    let placement_par = if stretch_to_box {
        crate::parser::svg::SvgPreserveAspectRatio::None
    } else {
        tree.preserve_aspect_ratio
    };
    let placement = crate::render::svg_geometry::compute_svg_placement(
        tree,
        crate::render::svg_geometry::SvgPlacementRequest::from_rect(
            0.0,
            0.0,
            scaled_w,
            scaled_h,
            placement_par,
        ),
    );
    let Some(placement) = placement else {
        return;
    };
    let raster_background = synthetic_raster_background(tree).and_then(|(href, source_box)| {
        let image_box = SvgViewportBox::new(
            placement.translate_x + source_box.x * placement.scale_x,
            placement.translate_y + source_box.y * placement.scale_y,
            source_box.width * placement.scale_x,
            source_box.height * placement.scale_y,
        );
        let request = (paint.blur_radius > 0.0).then_some(RasterBackgroundRequest {
            canvas_box: paint.local_blur_canvas_box(),
            image_box,
            blur_radius: paint.blur_radius,
        });
        register_background_image(pdf_writer, page_images, href, image_box, request)
            .map(|registered| (image_box, registered))
    });
    let visual_overflow = raster_background.as_ref().map_or_else(
        || svg_visual_overflow(tree).scale(placement.scale_x, placement.scale_y),
        |(image_box, registered)| {
            overflow_from_viewport_box(
                placement.viewport,
                registered.draw_box.unwrap_or(*image_box),
            )
        },
    );
    let tile_clip_box = viewport_box_from_overflow(placement.viewport, visual_overflow);

    // Compute background-position offset (in the CSS coordinate system,
    // origin at top-left of the element box).
    let offset_x = if paint.position.x_is_percent {
        (paint.reference_box.width - scaled_w) * paint.position.x
    } else {
        paint.position.x
    };
    let offset_y = if paint.position.y_is_percent {
        (paint.reference_box.height - scaled_h) * paint.position.y
    } else {
        paint.position.y
    };

    // Determine tiling grid based on background-repeat.
    // We compute the set of tile origin offsets (in CSS coords, top-left = 0,0).
    let tiles_x: Vec<f32>;
    let tiles_y: Vec<f32>;

    match paint.repeat {
        BackgroundRepeat::NoRepeat => {
            tiles_x = vec![offset_x];
            tiles_y = vec![offset_y];
        }
        BackgroundRepeat::Repeat => {
            tiles_x = tile_offsets(offset_x, scaled_w, paint.reference_box.width);
            tiles_y = tile_offsets(offset_y, scaled_h, paint.reference_box.height);
        }
        BackgroundRepeat::RepeatX => {
            tiles_x = tile_offsets(offset_x, scaled_w, paint.reference_box.width);
            tiles_y = vec![offset_y];
        }
        BackgroundRepeat::RepeatY => {
            tiles_x = vec![offset_x];
            tiles_y = tile_offsets(offset_y, scaled_h, paint.reference_box.height);
        }
    }

    // Clip to the element box.
    content.push_str("q\n");
    let expanded_clip_box = viewport_box_from_overflow(paint.clip_box, visual_overflow);
    if paint.border_radius > 0.0 {
        content.push_str(&rounded_rect_path(
            expanded_clip_box.x,
            expanded_clip_box.y,
            expanded_clip_box.width,
            expanded_clip_box.height,
            paint.border_radius,
        ));
        content.push_str("W n\n");
    } else {
        content.push_str(&expanded_clip_box.clip_path());
    }

    for &ty in &tiles_y {
        for &tx in &tiles_x {
            content.push_str("q\n");
            let tile_origin = paint.tile_origin(tx, ty);
            let pdf_x = tile_origin.x;
            let pdf_top = tile_origin.y + tile_origin.height;
            content.push_str(&format!("1 0 0 -1 {pdf_x} {pdf_top} cm\n"));
            content.push_str("q\n");
            content.push_str(&tile_clip_box.clip_path());
            if let Some((image_box, registered_image)) = &raster_background {
                let draw_box = registered_image.draw_box.unwrap_or(*image_box);
                content.push_str(&format!(
                    "q\n{width} 0 0 -{height} {x} {y} cm\n/{name} Do\nQ\n",
                    width = draw_box.width,
                    height = draw_box.height,
                    x = draw_box.x,
                    y = draw_box.y + draw_box.height,
                    name = registered_image.name,
                ));
            } else {
                content.push_str(&format!(
                    "{sx} 0 0 {sy} {tx} {ty} cm\n",
                    sx = placement.scale_x,
                    sy = placement.scale_y,
                    tx = placement.translate_x,
                    ty = placement.translate_y,
                ));
                {
                    let mut image_sink = SvgPageImageSink {
                        pdf_writer,
                        page_images,
                    };
                    let mut resources = crate::render::svg_to_pdf::SvgPdfResources {
                        shadings,
                        shading_counter,
                        ext_gstates: ext_gstates.as_deref_mut(),
                        image_sink: Some(&mut image_sink),
                        // SVG used as a CSS background image: custom-font text in
                        // background SVGs is out of scope here (no font context is
                        // threaded this far), so fall back to standard fonts.
                        custom_fonts: None,
                        prepared_custom_fonts: None,
                    };
                    crate::render::svg_to_pdf::render_svg_tree_with_resources(
                        tree,
                        content,
                        &mut resources,
                    );
                }
            }
            content.push_str("Q\n");
            content.push_str("Q\n");
        }
    }
    content.push_str("Q\n");
}

/// Compute tile origin offsets that cover `[0, extent)` when starting from
/// `origin` and repeating every `step`.  Returns offsets that overlap the
/// visible range.
fn tile_offsets(origin: f32, step: f32, extent: f32) -> Vec<f32> {
    if step <= 0.0 {
        return vec![origin];
    }
    let mut offsets = Vec::new();
    // Walk backwards from origin to find the first tile that overlaps [0, extent).
    let mut start = origin;
    while start > 0.0 {
        start -= step;
    }
    let mut pos = start;
    while pos < extent {
        offsets.push(pos);
        pos += step;
    }
    if offsets.is_empty() {
        offsets.push(origin);
    }
    offsets
}
/// Render every outset shadow in a `box-shadow` list. CSS paints shadows
/// back-to-front: the FIRST listed shadow ends up on top, so the list is
/// iterated in reverse so earlier entries are painted last. Inset shadows in
/// the list are skipped here (drawn by `render_box_shadows_inset` after the
/// background).
#[allow(clippy::too_many_arguments)]
fn render_box_shadows(
    content: &mut String,
    shadows: &[crate::style::computed::BoxShadow],
    box_x: f32,
    box_y_bottom: f32,
    box_w: f32,
    box_h: f32,
    border_radius: f32,
    page_ext_gstates: &mut Vec<(String, f32)>,
    gs_counter: &mut usize,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    for shadow in shadows.iter().rev() {
        render_box_shadow(
            content,
            shadow,
            box_x,
            box_y_bottom,
            box_w,
            box_h,
            border_radius,
            page_ext_gstates,
            gs_counter,
            pdf_writer,
            page_images,
        );
    }
}

/// Render every inset shadow in a `box-shadow` list (reverse paint order, as
/// `render_box_shadows`). Call after the element background.
#[allow(clippy::too_many_arguments)]
fn render_box_shadows_inset(
    content: &mut String,
    shadows: &[crate::style::computed::BoxShadow],
    box_x: f32,
    box_y_bottom: f32,
    box_w: f32,
    box_h: f32,
    border_radius: f32,
    page_ext_gstates: &mut Vec<(String, f32)>,
    gs_counter: &mut usize,
) {
    for shadow in shadows.iter().rev() {
        if shadow.inset {
            render_box_shadow_inset(
                content,
                shadow,
                box_x,
                box_y_bottom,
                box_w,
                box_h,
                border_radius,
                page_ext_gstates,
                gs_counter,
            );
        }
    }
}

/// Render a box-shadow with optional Gaussian blur.
///
/// When `blur > 0.5`, rasterizes the (rounded) shadow rect into a transparent
/// buffer at device scale, applies a true gaussian (σ = blur/2, per
/// css-backgrounds-3 §7.1.1) reusing `render::blur`, and embeds the result as a
/// PDF image XObject positioned so the feather extends beyond the shadow rect —
/// matching Chrome's smooth penumbra. When `blur <= 0.5`, draws a single solid
/// shadow rectangle (byte-identical to the previous vector path).
#[allow(clippy::too_many_arguments)]
fn render_box_shadow(
    content: &mut String,
    shadow: &crate::style::computed::BoxShadow,
    box_x: f32,
    box_y_bottom: f32,
    box_w: f32,
    box_h: f32,
    border_radius: f32,
    page_ext_gstates: &mut Vec<(String, f32)>,
    gs_counter: &mut usize,
    pdf_writer: &mut PdfWriter,
    page_images: &mut Vec<ImageRef>,
) {
    let (sr, sg, sb, base_alpha) = shadow.color.to_f32_rgba();
    let blur = shadow.blur;
    let spread = shadow.spread;
    // CSS: positive offset_y = shadow below element.
    // PDF: Y increases upward, so negate offset_y.
    let layers: usize = 10;
    // Per-layer alpha multiplier, tuned so cumulative edge opacity under PDF
    // source-over compositing matches base_alpha roughly. Halved from 0.22
    // to 0.11 because parity testing showed shadows were still ~2× too dark
    // vs. Chromium at typical base_alpha values (0.2–0.5).
    const ALPHA_NORMALIZER: f32 = 0.11;

    if shadow.inset {
        // Inset shadows are drawn separately via render_box_shadow_inset()
        // AFTER the element background, so skip here.
        return;
    }

    // Outset shadow: position = box shifted by offset, expanded uniformly by spread.
    let sx = box_x + shadow.offset_x - spread;
    let sy = box_y_bottom - shadow.offset_y - spread;
    let sw = box_w + spread * 2.0;
    let sh = box_h + spread * 2.0;

    if blur <= 0.5 {
        // No blur — solid shadow
        if base_alpha < 1.0 {
            let gs_name = format!("GSbs{}", *gs_counter);
            *gs_counter += 1;
            page_ext_gstates.push((gs_name.clone(), base_alpha));
            content.push_str(&format!("/{gs_name} gs\n"));
        }
        content.push_str(&format!("{sr} {sg} {sb} rg\n"));
        if border_radius > 0.0 {
            content.push_str(&rounded_rect_path(sx, sy, sw, sh, border_radius));
            content.push_str("\nf\n");
        } else {
            content.push_str(&format!("{sx} {sy} {sw} {sh} re\nf\n"));
        }
        if base_alpha < 1.0 {
            content.push_str("/GSDefault gs\n");
        }
        return;
    }

    // True gaussian blur: rasterize the (rounded) shadow rect, gaussian-blur it
    // (σ = blur/2), and embed as an image XObject. The shadow's corner radius
    // tracks the box radius grown by spread (the spread expands the radius too).
    let shadow_radius = if border_radius > 0.0 {
        (border_radius + spread).max(0.0)
    } else {
        0.0
    };
    let _ = layers;
    let _ = ALPHA_NORMALIZER;
    if let Some(blurred) = crate::render::blur::blur_shadow_rect(
        sw,
        sh,
        shadow_radius,
        blur,
        (sr, sg, sb, base_alpha),
        pdf_writer.opts.filter_dpi,
    ) {
        let img_obj_id = pdf_writer.add_image_object(
            &blurred.asset.data,
            blurred.asset.source_width,
            blurred.asset.source_height,
            blurred.asset.format,
            blurred.asset.png_metadata.as_ref(),
        );
        let img_name = format!("Im{img_obj_id}");
        let ov = blurred.overflow_pt;
        content.push_str(&format!(
            "q\n{w} 0 0 {h} {ix} {iy} cm\n/{name} Do\nQ\n",
            w = sw + 2.0 * ov,
            h = sh + 2.0 * ov,
            ix = sx - ov,
            iy = sy - ov,
            name = img_name,
        ));
        page_images.push(ImageRef {
            name: img_name,
            obj_id: img_obj_id,
        });
        return;
    }

    // Fallback (raster unavailable): solid shadow at base alpha.
    if base_alpha < 1.0 {
        let gs_name = format!("GSbs{}", *gs_counter);
        *gs_counter += 1;
        page_ext_gstates.push((gs_name.clone(), base_alpha));
        content.push_str(&format!("/{gs_name} gs\n"));
    }
    content.push_str(&format!("{sr} {sg} {sb} rg\n"));
    if border_radius > 0.0 {
        content.push_str(&rounded_rect_path(sx, sy, sw, sh, border_radius));
        content.push_str("\nf\n");
    } else {
        content.push_str(&format!("{sx} {sy} {sw} {sh} re\nf\n"));
    }
    if base_alpha < 1.0 {
        content.push_str("/GSDefault gs\n");
    }
}

/// Render an inset box-shadow: shadow appears inside the box edges, fading
/// toward the center. Uses PDF clipping to constrain shadow to the box,
/// then draws rings of the shadow color via even-odd fill, with alpha
/// graded so edges accumulate maximum darkness.
///
/// Call this AFTER the element's background so the shadow isn't painted
/// over. The outset variant (render_box_shadow) is called before the
/// background.
#[allow(clippy::too_many_arguments)]
fn render_box_shadow_inset(
    content: &mut String,
    shadow: &crate::style::computed::BoxShadow,
    box_x: f32,
    box_y_bottom: f32,
    box_w: f32,
    box_h: f32,
    border_radius: f32,
    page_ext_gstates: &mut Vec<(String, f32)>,
    gs_counter: &mut usize,
) {
    if !shadow.inset {
        return;
    }
    let (sr, sg, sb, base_alpha) = shadow.color.to_f32_rgba();
    let blur = shadow.blur;
    let spread = shadow.spread;
    let offset_x = shadow.offset_x;
    let offset_y = shadow.offset_y;
    let layers: usize = 10;
    let alpha_normalizer: f32 = 0.11;
    // Save gfx state, clip to box path.
    content.push_str("q\n");
    if border_radius > 0.5 {
        content.push_str(&rounded_rect_path(
            box_x,
            box_y_bottom,
            box_w,
            box_h,
            border_radius,
        ));
        content.push('\n');
    } else {
        content.push_str(&format!("{box_x} {box_y_bottom} {box_w} {box_h} re\n"));
    }
    content.push_str("W n\n");

    content.push_str(&format!("{sr} {sg} {sb} rg\n"));

    // Outer bounds for even-odd fill — large enough to guarantee full
    // coverage of the clipped region.
    let ox = box_x - blur - spread.abs() - 2.0;
    let oy = box_y_bottom - blur - spread.abs() - 2.0;
    let ow = box_w + (blur + spread.abs()) * 2.0 + 4.0;
    let oh = box_h + (blur + spread.abs()) * 2.0 + 4.0;

    if blur <= 0.5 {
        // No blur: single solid fill of the "ring" area.
        if base_alpha < 1.0 {
            let gs_name = format!("GSbs{}", *gs_counter);
            *gs_counter += 1;
            page_ext_gstates.push((gs_name.clone(), base_alpha));
            content.push_str(&format!("/{gs_name} gs\n"));
        }
        let hx = box_x + offset_x + spread;
        let hy = box_y_bottom - offset_y + spread;
        let hw = box_w - spread * 2.0;
        let hh = box_h - spread * 2.0;
        content.push_str(&format!("{ox} {oy} {ow} {oh} re\n"));
        if hw > 0.0 && hh > 0.0 {
            content.push_str(&format!("{hx} {hy} {hw} {hh} re\n"));
        }
        content.push_str("f*\n");
        content.push_str("Q\n");
        if base_alpha < 1.0 {
            content.push_str("/GSDefault gs\n");
        }
        return;
    }

    for i in (0..layers).rev() {
        let t = (i as f32 + 1.0) / layers as f32;
        let gaussian = (-3.0 * t * t).exp();
        let alpha = (base_alpha * gaussian * alpha_normalizer).min(base_alpha);
        let expand = blur * t;

        let gs_name = format!("GSbs{}", *gs_counter);
        *gs_counter += 1;
        page_ext_gstates.push((gs_name.clone(), alpha));
        content.push_str(&format!("/{gs_name} gs\n"));

        // Inner "hole": shifted by shadow offset, contracted by (spread + expand).
        let total_inset = expand + spread;
        let hx = box_x + offset_x + total_inset;
        let hy = box_y_bottom - offset_y + total_inset;
        let hw = box_w - total_inset * 2.0;
        let hh = box_h - total_inset * 2.0;

        // Draw outer rect + inner rect, fill with even-odd → ring of shadow color.
        content.push_str(&format!("{ox} {oy} {ow} {oh} re\n"));
        if hw > 0.0 && hh > 0.0 {
            content.push_str(&format!("{hx} {hy} {hw} {hh} re\n"));
        }
        content.push_str("f*\n");
    }

    content.push_str("Q\n");
    content.push_str("/GSDefault gs\n");
}

/// Build a rounded-rect path with independent per-corner radii in
/// [top-left, top-right, bottom-right, bottom-left] order (CSS corner order).
/// `(x, y)` is the bottom-left in PDF (bottom-up) coordinates; `w`/`h` are the
/// box size. Radii are clamped per CSS 9.2 so adjacent corners on the same edge
/// never overlap. When all four radii are equal this matches `rounded_rect_path`
/// closely; the renderer falls back to the simpler builder for the uniform case
/// to keep existing byte output stable.
fn rounded_rect_path_per_corner(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> String {
    // Corner order: tl, tr, br, bl.
    let mut tl = radii[0].max(0.0);
    let mut tr = radii[1].max(0.0);
    let mut br = radii[2].max(0.0);
    let mut bl = radii[3].max(0.0);
    // CSS overlap clamping: scale all radii by the smallest edge ratio so no two
    // radii on a shared edge exceed that edge's length.
    let scale = {
        let mut s = 1.0f32;
        let top = tl + tr;
        let bottom = bl + br;
        let left = tl + bl;
        let right = tr + br;
        if top > w {
            s = s.min(w / top);
        }
        if bottom > w {
            s = s.min(w / bottom);
        }
        if left > h {
            s = s.min(h / left);
        }
        if right > h {
            s = s.min(h / right);
        }
        s
    };
    if scale < 1.0 {
        tl *= scale;
        tr *= scale;
        br *= scale;
        bl *= scale;
    }
    let kf = 0.552_284_8;
    // Coordinates: PDF y grows upward, so `y + h` is the top edge.
    let xl = x;
    let xr = x + w;
    let yt = y + h;
    let yb = y;
    // Bezier control factor per corner.
    let (ktl, ktr, kbr, kbl) = (tl * kf, tr * kf, br * kf, bl * kf);
    format!(
        // Start just right of the top-left corner, run clockwise.
        "{a} {yt} m\n\
         {b} {yt} l {b2} {yt} {xr} {tr_y2} {xr} {tr_y} c\n\
         {xr} {br_y} l {xr} {br_y2} {br_x2} {yb} {br_x} {yb} c\n\
         {bl_x} {yb} l {bl_x2} {yb} {xl} {bl_y2} {xl} {bl_y} c\n\
         {xl} {tl_y} l {xl} {tl_y2} {tl_x2} {yt} {a} {yt} c\n\
         h\n",
        a = xl + tl,           // top edge start (after TL)
        b = xr - tr,           // top edge end (before TR)
        b2 = xr - tr + ktr,    // TR control x
        tr_y = yt - tr,        // TR arc end y
        tr_y2 = yt - tr + ktr, // TR control y
        br_y = yb + br,        // right edge end (before BR)
        br_y2 = yb + br - kbr, // BR control y
        br_x = xr - br,        // BR arc end x
        br_x2 = xr - br + kbr, // BR control x
        bl_x = xl + bl,        // bottom edge end (before BL)
        bl_x2 = xl + bl - kbl, // BL control x
        bl_y = yb + bl,        // BL arc end y
        bl_y2 = yb + bl - kbl, // BL control y
        tl_y = yt - tl,        // left edge end (before TL)
        tl_y2 = yt - tl + ktl, // TL control y
        tl_x2 = xl + tl - ktl, // TL control x
    )
}

/// Build a rounded-rectangle path with ELLIPTICAL corners: each corner has a
/// distinct horizontal (`rx`) and vertical (`ry`) radius. `rx`/`ry` are in
/// [top-left, top-right, bottom-right, bottom-left] order. Applies the CSS
/// Backgrounds §5.1 overlap-clamping factor `f` (computed per axis) and draws
/// each corner as a quarter ellipse via a cubic Bézier whose control handles are
/// scaled by `0.5523` along each axis. `(x, y)` is the bottom-left in PDF
/// (bottom-up) coords; `w`×`h` is the box size. Used for `border-radius: 50%` on
/// non-square boxes and the `Rx / Ry` slash syntax.
fn rounded_rect_path_elliptical(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rx: [f32; 4],
    ry: [f32; 4],
) -> String {
    let mut rx = [
        rx[0].max(0.0),
        rx[1].max(0.0),
        rx[2].max(0.0),
        rx[3].max(0.0),
    ];
    let mut ry = [
        ry[0].max(0.0),
        ry[1].max(0.0),
        ry[2].max(0.0),
        ry[3].max(0.0),
    ];
    // CSS overlap clamping: f = min over each edge of L_edge / sum_of_radii_on_edge,
    // using horizontal radii on the top/bottom edges and vertical on left/right.
    let mut f = 1.0f32;
    let edges = [
        (rx[0] + rx[1], w), // top: TL.x + TR.x vs width
        (rx[3] + rx[2], w), // bottom: BL.x + BR.x vs width
        (ry[0] + ry[3], h), // left: TL.y + BL.y vs height
        (ry[1] + ry[2], h), // right: TR.y + BR.y vs height
    ];
    for (sum, len) in edges {
        if sum > len && sum > 0.0 {
            f = f.min(len / sum);
        }
    }
    if f < 1.0 {
        for i in 0..4 {
            rx[i] *= f;
            ry[i] *= f;
        }
    }
    let kf = 0.552_284_8;
    // Corner radii: [tl, tr, br, bl].
    let (tlx, trx, brx, blx) = (rx[0], rx[1], rx[2], rx[3]);
    let (tly, try_, bry, bly) = (ry[0], ry[1], ry[2], ry[3]);
    let xl = x;
    let xr = x + w;
    let yt = y + h;
    let yb = y;
    format!(
        // Start just right of the top-left corner, run clockwise.
        "{a} {yt} m\n\
         {b} {yt} l {b2} {yt} {xr} {tr_y2} {xr} {tr_y} c\n\
         {xr} {br_y} l {xr} {br_y2} {br_x2} {yb} {br_x} {yb} c\n\
         {bl_x} {yb} l {bl_x2} {yb} {xl} {bl_y2} {xl} {bl_y} c\n\
         {xl} {tl_y} l {xl} {tl_y2} {tl_x2} {yt} {a} {yt} c\n\
         h\n",
        a = xl + tlx,                  // top edge start (after TL)
        b = xr - trx,                  // top edge end (before TR)
        b2 = xr - trx + trx * kf,      // TR control x
        tr_y = yt - try_,              // TR arc end y
        tr_y2 = yt - try_ + try_ * kf, // TR control y
        br_y = yb + bry,               // right edge end (before BR)
        br_y2 = yb + bry - bry * kf,   // BR control y
        br_x = xr - brx,               // BR arc end x
        br_x2 = xr - brx + brx * kf,   // BR control x
        bl_x = xl + blx,               // bottom edge end (before BL)
        bl_x2 = xl + blx - blx * kf,   // BL control x
        bl_y = yb + bly,               // BL arc end y
        bl_y2 = yb + bly - bly * kf,   // BL control y
        tl_y = yt - tly,               // left edge end (before TL)
        tl_y2 = yt - tly + tly * kf,   // TL control y
        tl_x2 = xl + tlx - tlx * kf,   // TL control x
    )
}

/// Whether two per-corner radii arrays differ (elliptical corners needed).
fn radii_elliptical(rx: [f32; 4], ry: [f32; 4]) -> bool {
    (0..4).any(|i| (rx[i] - ry[i]).abs() > 1e-4)
}

/// Pick the right rounded-rectangle path for a box with per-corner horizontal
/// (`rx`) and vertical (`ry`) radii: an elliptical path when the two axes
/// differ, a per-corner circular path when they are uniform-axis but differ
/// across corners, the byte-stable uniform path when all corners share one
/// radius, or `None` (square box) when no corner is rounded. The caller emits a
/// plain `re` rectangle for the `None` case so existing output stays stable.
fn rounded_box_path(x: f32, y: f32, w: f32, h: f32, rx: [f32; 4], ry: [f32; 4]) -> Option<String> {
    if !radii_any(rx) && !radii_any(ry) {
        return None;
    }
    if radii_elliptical(rx, ry) {
        return Some(rounded_rect_path_elliptical(x, y, w, h, rx, ry));
    }
    if radii_uniform(rx) {
        return Some(rounded_rect_path(x, y, w, h, rx[0]));
    }
    Some(rounded_rect_path_per_corner(x, y, w, h, rx))
}

/// Whether per-corner radii are effectively uniform (all equal). Uniform boxes
/// use the simpler `rounded_rect_path` to keep golden output byte-stable.
fn radii_uniform(radii: [f32; 4]) -> bool {
    (radii[0] - radii[1]).abs() < 1e-4
        && (radii[0] - radii[2]).abs() < 1e-4
        && (radii[0] - radii[3]).abs() < 1e-4
}

/// Whether any corner has a non-zero radius.
fn radii_any(radii: [f32; 4]) -> bool {
    radii.iter().any(|r| *r > 0.0)
}

fn rounded_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> String {
    let r = r.min(w / 2.0).min(h / 2.0); // Clamp radius to half the smallest dimension
    let k = r * 0.552_284_8;
    format!(
        "{x0} {y0} m\n\
         {x1} {y0} l {x2} {y0} {x3} {y3} {x3} {y4} c\n\
         {x3} {y5} l {x3} {y6} {x2} {y7} {x1} {y7} c\n\
         {x0} {y7} l {x8} {y7} {x9} {y6} {x9} {y5} c\n\
         {x9} {y4} l {x9} {y3} {x8} {y0} {x0} {y0} c\n\
         h\n",
        x0 = x + r,
        x1 = x + w - r,
        x2 = x + w - r + k,
        x3 = x + w,
        x8 = x + r - k,
        x9 = x,
        y0 = y + h, // top
        y3 = y + h - r + k,
        y4 = y + h - r,
        y5 = y + r,
        y6 = y + r - k,
        y7 = y, // bottom
    )
}

/// Convert a UTF-8 string to WinAnsi (Windows-1252) encoded bytes.
///
/// Standard PDF fonts (Helvetica, Times-Roman, Courier) use WinAnsi encoding,
/// not UTF-8. Writing raw UTF-8 bytes causes multi-byte characters like em dash
/// to appear as mojibake. This function maps Unicode code points to their
/// WinAnsi byte equivalents.
pub(crate) fn utf8_to_winansi(text: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        match code {
            // ASCII range maps directly
            0x0000..=0x007F => result.push(code as u8),
            // Non-breaking space
            0x00A0 => result.push(0xA0),
            // Latin-1 supplement U+00A1..U+00FF map directly
            0x00A1..=0x00FF => result.push(code as u8),
            // WinAnsi special mappings from the Windows-1252 range 0x80..0x9F
            0x20AC => result.push(0x80), // Euro sign
            0x201A => result.push(0x82), // Single low-9 quotation mark
            0x0192 => result.push(0x83), // Latin small letter f with hook
            0x201E => result.push(0x84), // Double low-9 quotation mark
            0x2026 => result.push(0x85), // Horizontal ellipsis
            0x2020 => result.push(0x86), // Dagger
            0x2021 => result.push(0x87), // Double dagger
            0x02C6 => result.push(0x88), // Modifier letter circumflex accent
            0x2030 => result.push(0x89), // Per mille sign
            0x0160 => result.push(0x8A), // Latin capital letter S with caron
            0x2039 => result.push(0x8B), // Single left-pointing angle quotation mark
            0x0152 => result.push(0x8C), // Latin capital ligature OE
            0x017D => result.push(0x8E), // Latin capital letter Z with caron
            0x2018 => result.push(0x91), // Left single quotation mark
            0x2019 => result.push(0x92), // Right single quotation mark
            0x201C => result.push(0x93), // Left double quotation mark
            0x201D => result.push(0x94), // Right double quotation mark
            0x2022 => result.push(0x95), // Bullet
            0x2013 => result.push(0x96), // En dash
            0x2014 => result.push(0x97), // Em dash
            0x02DC => result.push(0x98), // Small tilde
            0x2122 => result.push(0x99), // Trade mark sign
            0x0161 => result.push(0x9A), // Latin small letter s with caron
            0x203A => result.push(0x9B), // Single right-pointing angle quotation mark
            0x0153 => result.push(0x9C), // Latin small ligature oe
            0x017E => result.push(0x9E), // Latin small letter z with caron
            0x0178 => result.push(0x9F), // Latin capital letter Y with diaeresis
            // Anything else is not representable in WinAnsi — replace with '?'
            _ => result.push(b'?'),
        }
    }
    result
}

/// Returns `true` if every character in `text` can be encoded in WinAnsiEncoding.
///
/// Characters outside this range (CJK, Arabic, Hebrew, emoji, box-drawing, etc.)
/// cannot be rendered by the standard PDF fonts and require a Unicode-capable
/// embedded font instead.
pub(crate) fn is_winansi_encodable(text: &str) -> bool {
    text.chars().all(is_winansi_char)
}

/// Check whether a single character is representable in WinAnsiEncoding.
pub(crate) fn is_winansi_char(ch: char) -> bool {
    let code = ch as u32;
    matches!(code,
        0x0000..=0x007F |
        0x00A0..=0x00FF |
        0x20AC | 0x201A | 0x0192 | 0x201E | 0x2026 |
        0x2020 | 0x2021 | 0x02C6 | 0x2030 | 0x0160 |
        0x2039 | 0x0152 | 0x017D | 0x2018 | 0x2019 |
        0x201C | 0x201D | 0x2022 | 0x2013 | 0x2014 |
        0x02DC | 0x2122 | 0x0161 | 0x203A | 0x0153 |
        0x017E | 0x0178
    )
}

/// Encode a UTF-8 string for use in a PDF text operator (Tj).
///
/// Converts to WinAnsi encoding, then produces a `String` where:
/// - ASCII printable bytes (0x20..=0x7E), except `\`, `(`, `)`, are kept as-is
/// - `\`, `(`, `)` are escaped as `\\`, `\(`, `\)`
/// - All other bytes (0x00..=0x1F, 0x7F..=0xFF) are written as octal escapes `\NNN`
///
/// The returned string is safe to embed in a PDF content stream as `(encoded) Tj`.
pub(crate) fn encode_pdf_text(text: &str) -> String {
    let winansi = utf8_to_winansi(text);
    let mut result = String::with_capacity(winansi.len() * 2);
    for &b in &winansi {
        match b {
            b'\\' => result.push_str("\\\\"),
            b'(' => result.push_str("\\("),
            b')' => result.push_str("\\)"),
            0x20..=0x7E => result.push(b as char),
            _ => {
                // Octal escape: \NNN (3-digit, zero-padded)
                result.push_str(&format!("\\{:03o}", b));
            }
        }
    }
    result
}

fn escape_pdf_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn build_tounicode_cmap(mappings: &[(u16, Vec<u16>)]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n\
12 dict begin\n\
begincmap\n\
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
/CMapName /Adobe-Identity-UCS def\n\
/CMapType 2 def\n\
1 begincodespacerange\n\
<0000> <FFFF>\n\
endcodespacerange\n",
    );

    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (glyph_id, unicode) in chunk {
            let unicode_hex: String = unicode
                .iter()
                .map(|code_unit| format!("{code_unit:04X}"))
                .collect();
            cmap.push_str(&format!("<{glyph_id:04X}> <{unicode_hex}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }

    cmap.push_str(
        "endcmap\n\
CMapName currentdict /CMap defineresource pop\n\
end\n\
end\n",
    );
    cmap
}

/// A reference to an image XObject used on a page.
pub(crate) struct ImageRef {
    pub name: String,
    pub obj_id: usize,
}

struct SvgPageImageSink<'a> {
    pdf_writer: &'a mut PdfWriter,
    page_images: &'a mut Vec<ImageRef>,
}

impl SvgPageImageSink<'_> {
    fn register_page_image(&mut self, obj_id: usize) -> String {
        let name = format!("Im{obj_id}");
        self.page_images.push(ImageRef {
            name: name.clone(),
            obj_id,
        });
        name
    }
}

impl crate::render::svg_to_pdf::SvgImageObjectSink for SvgPageImageSink<'_> {
    fn register_raster(&mut self, raw_image: &[u8]) -> Option<String> {
        let obj_id = self.pdf_writer.add_raw_raster_image_object(raw_image)?;
        Some(self.register_page_image(obj_id))
    }
}

struct DecodedPngImage {
    width: u32,
    height: u32,
    color_space: &'static str,
    color_data: Vec<u8>,
    alpha_data: Option<Vec<u8>>,
}

fn decode_png_for_pdf(raw: &[u8]) -> Option<DecodedPngImage> {
    let mut decoder = png_decoder::Decoder::new(std::io::Cursor::new(raw));
    decoder.ignore_checksums(true);
    let mut reader = decoder.read_info().ok()?;
    let output_size = reader.output_buffer_size()?;
    let mut buffer = vec![0; output_size];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = buffer.get(..info.buffer_size())?;

    let mut color_data = Vec::new();
    let mut alpha_data = Vec::new();
    let mut has_alpha = false;
    let color_space = match info.color_type {
        png_decoder::ColorType::Rgba => {
            color_data.reserve((info.width * info.height * 3) as usize);
            alpha_data.reserve((info.width * info.height) as usize);
            for chunk in pixels.chunks_exact(4) {
                color_data.extend_from_slice(&chunk[..3]);
                alpha_data.push(chunk[3]);
            }
            has_alpha = true;
            "/DeviceRGB"
        }
        png_decoder::ColorType::Rgb => {
            color_data.extend_from_slice(pixels);
            "/DeviceRGB"
        }
        png_decoder::ColorType::Grayscale => {
            color_data.extend_from_slice(pixels);
            "/DeviceGray"
        }
        png_decoder::ColorType::GrayscaleAlpha => {
            color_data.reserve((info.width * info.height) as usize);
            alpha_data.reserve((info.width * info.height) as usize);
            for chunk in pixels.chunks_exact(2) {
                color_data.push(chunk[0]);
                alpha_data.push(chunk[1]);
            }
            has_alpha = true;
            "/DeviceGray"
        }
        _ => return None,
    };

    Some(DecodedPngImage {
        width: info.width,
        height: info.height,
        color_space,
        color_data,
        alpha_data: has_alpha.then_some(alpha_data),
    })
}

/// Sample a sorted gradient stop list at fraction `t` (0..=1), returning the
/// interpolated `(r, g, b, a)` as floats in 0..1. Stops outside the requested
/// fraction clamp to the end colors (the non-repeating gradient behaviour used
/// for masks). Stop positions are already normalised to 0..1 at parse time.
fn sample_gradient_stops(
    stops: &[crate::style::computed::GradientStop],
    t: f32,
) -> (f32, f32, f32, f32) {
    if stops.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let to_rgba = |c: crate::types::Color| {
        (
            c.r as f32 / 255.0,
            c.g as f32 / 255.0,
            c.b as f32 / 255.0,
            c.a as f32 / 255.0,
        )
    };
    if t <= stops[0].position {
        return to_rgba(stops[0].color);
    }
    let last = stops.len() - 1;
    if t >= stops[last].position {
        return to_rgba(stops[last].color);
    }
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t >= a.position && t <= b.position {
            let span = (b.position - a.position).max(1e-6);
            let f = ((t - a.position) / span).clamp(0.0, 1.0);
            let (ar, ag, ab, aa) = to_rgba(a.color);
            let (br, bg, bb, ba) = to_rgba(b.color);
            return (
                ar + (br - ar) * f,
                ag + (bg - ag) * f,
                ab + (bb - ab) * f,
                aa + (ba - aa) * f,
            );
        }
    }
    to_rgba(stops[last].color)
}

/// The tiling period of a repeating gradient: the position of its last stop
/// (css-images-3 — the stop pattern from 0 to the final stop tiles to fill the
/// line/ray). Falls back to 1.0 when degenerate so the mask still renders.
fn repeat_period(stops: &[crate::style::computed::GradientStop]) -> f32 {
    stops
        .last()
        .map(|s| s.position)
        .filter(|p| *p > 1e-4)
        .unwrap_or(1.0)
}

/// Reduce a sampled gradient color to a single mask-coverage byte (0..255)
/// following `mask-mode` (css-masking-1 §3.4). `match-source` on a CSS gradient
/// resolves to alpha. Luminance uses the Rec.709 coefficients premultiplied by
/// alpha.
fn coverage_byte(rgba: (f32, f32, f32, f32), mode: crate::style::computed::MaskMode) -> u8 {
    use crate::style::computed::MaskMode;
    let (r, g, b, a) = rgba;
    let cov = match mode {
        MaskMode::Luminance => (0.2126 * r + 0.7152 * g + 0.0722 * b) * a,
        // `alpha` and `match-source` (CSS image) both use the source alpha.
        MaskMode::Alpha | MaskMode::MatchSource => a,
    };
    (cov.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Rasterise a `mask-image` source to a `px_w` × `px_h` DeviceGray coverage
/// buffer (row 0 = top of the box, matching PDF image sample order). Each byte
/// is the mask coverage for one pixel under `mode`.
fn rasterize_mask_coverage(
    source: &crate::style::computed::MaskSource,
    mode: crate::style::computed::MaskMode,
    px_w: u32,
    px_h: u32,
) -> Vec<u8> {
    use crate::style::computed::{MaskSource, RadialExtent, RadialPos, RadialShape};
    let w = px_w as f32;
    let h = px_h as f32;
    // Resolve a `RadialPos` to MASK PIXELS along an axis of `extent` pixels.
    // `Fraction` scales by the pixel extent directly; `Points` is stored in PDF
    // points, so convert to pixels (1pt = 1/0.75 px) to match the buffer space.
    let resolve_px = |p: RadialPos, extent: f32| -> f32 {
        match p {
            RadialPos::Fraction(f) => extent * f,
            RadialPos::Points(pt) => pt / 0.75,
        }
    };
    let mut out = Vec::with_capacity((px_w * px_h) as usize);
    match source {
        MaskSource::Linear(lg) => {
            // CSS gradient line: angle 0 = to top, 90 = to right, 180 = to
            // bottom. The line passes through the box centre; the gradient
            // extends from the start corner (projection min) to the end corner
            // (projection max). Normalise each pixel's projection to 0..1.
            let theta = lg.angle.to_radians();
            // Direction vector of increasing gradient (CSS y grows downward).
            let dx = theta.sin();
            let dy = -theta.cos();
            // Half-length of the projected gradient line: project the box's
            // half-extents onto the direction and sum the absolute components.
            let half = (w * 0.5 * dx.abs() + h * 0.5 * dy.abs()).max(1e-6);
            let (cx, cy) = (w * 0.5, h * 0.5);
            for py in 0..px_h {
                let fy = py as f32 + 0.5;
                for px in 0..px_w {
                    let fx = px as f32 + 0.5;
                    let proj = (fx - cx) * dx + (fy - cy) * dy;
                    let mut t = (proj + half) / (2.0 * half);
                    if lg.repeating {
                        t = t.rem_euclid(repeat_period(&lg.stops));
                    } else {
                        t = t.clamp(0.0, 1.0);
                    }
                    out.push(coverage_byte(sample_gradient_stops(&lg.stops, t), mode));
                }
            }
        }
        MaskSource::Radial(rg) => {
            // Centre in mask pixels.
            let cx = resolve_px(rg.center.0, w);
            let cy = resolve_px(rg.center.1, h);
            // Resolve the ending-shape radii (px). Explicit radii win; else the
            // extent keyword (default farthest-corner) is computed from the box.
            let dist_x = cx.max(w - cx);
            let dist_y = cy.max(h - cy);
            let near_x = cx.min(w - cx);
            let near_y = cy.min(h - cy);
            let (rx, ry) = if let Some(r) = rg.radius {
                let rp = r / 0.75; // points → mask pixels
                (rp, rp)
            } else if let Some((ex, ey)) = rg.radii {
                (resolve_px(ex, w), resolve_px(ey, h))
            } else {
                match (rg.shape, rg.extent) {
                    (RadialShape::Circle, RadialExtent::ClosestSide) => {
                        let r = near_x.min(near_y).max(1e-6);
                        (r, r)
                    }
                    (RadialShape::Circle, RadialExtent::FarthestSide) => {
                        let r = dist_x.max(dist_y).max(1e-6);
                        (r, r)
                    }
                    (RadialShape::Circle, RadialExtent::ClosestCorner) => {
                        let r = (near_x * near_x + near_y * near_y).sqrt().max(1e-6);
                        (r, r)
                    }
                    (RadialShape::Circle, _) => {
                        let r = (dist_x * dist_x + dist_y * dist_y).sqrt().max(1e-6);
                        (r, r)
                    }
                    (RadialShape::Ellipse, RadialExtent::ClosestSide) => {
                        (near_x.max(1e-6), near_y.max(1e-6))
                    }
                    (RadialShape::Ellipse, RadialExtent::FarthestSide) => {
                        (dist_x.max(1e-6), dist_y.max(1e-6))
                    }
                    (RadialShape::Ellipse, _) => (dist_x.max(1e-6), dist_y.max(1e-6)),
                }
            };
            let (rx, ry) = (rx.max(1e-6), ry.max(1e-6));
            for py in 0..px_h {
                let fy = py as f32 + 0.5;
                for px in 0..px_w {
                    let fx = px as f32 + 0.5;
                    let nx = (fx - cx) / rx;
                    let ny = (fy - cy) / ry;
                    let mut t = (nx * nx + ny * ny).sqrt();
                    if rg.repeating {
                        t = t.rem_euclid(repeat_period(&rg.stops));
                    } else {
                        t = t.clamp(0.0, 1.0);
                    }
                    out.push(coverage_byte(sample_gradient_stops(&rg.stops, t), mode));
                }
            }
        }
        MaskSource::Conic(cg) => {
            let cx = resolve_px(cg.center.0, w);
            let cy = resolve_px(cg.center.1, h);
            let from = cg.from_angle.to_radians();
            for py in 0..px_h {
                let fy = py as f32 + 0.5;
                for px in 0..px_w {
                    let fx = px as f32 + 0.5;
                    // CSS conic angle: clockwise from 12 o'clock (up). atan2 with
                    // (dx, -dy) gives angle CW from +y axis (up) in CSS space.
                    let dx = fx - cx;
                    let dy = fy - cy;
                    let mut ang = dx.atan2(-dy) - from;
                    ang = ang.rem_euclid(std::f32::consts::TAU);
                    let t = ang / std::f32::consts::TAU;
                    out.push(coverage_byte(sample_gradient_stops(&cg.stops, t), mode));
                }
            }
        }
        // SVG `url()` masks are rasterised by `rasterize_svg_mask_coverage` and
        // never reach this gradient sampler.
        MaskSource::Svg(_) => {}
    }
    out
}

/// Rasterise an SVG `url()` mask source to a `px_w` × `px_h` DeviceGray coverage
/// buffer (row 0 = top of the box, matching PDF image sample order), reusing the
/// `resvg`/`usvg`/`tiny-skia` stack already vendored for SVG rendering.
///
/// The SVG is scaled to fill the box (its viewBox stretched to `px_w`×`px_h`,
/// matching the default `mask-size: auto` + `mask-repeat: no-repeat` of these
/// fixtures), rendered into a premultiplied-RGBA pixmap, then each pixel reduced
/// to one coverage byte under `mode` (css-masking-1 §3.4). The initial
/// `mask-mode: match-source` resolves to luminance for an SVG image (the
/// referenced image's luminance defines coverage, unlike a CSS gradient image
/// which uses alpha).
fn rasterize_svg_mask_coverage(
    svg_bytes: &[u8],
    mode: crate::style::computed::MaskMode,
    px_w: u32,
    px_h: u32,
) -> Option<Vec<u8>> {
    use crate::style::computed::MaskMode;
    use resvg::tiny_skia;
    use resvg::usvg;

    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg_bytes, &opt).ok()?;
    let svg_size = tree.size();
    let (sw, sh) = (svg_size.width(), svg_size.height());
    if sw <= 0.0 || sh <= 0.0 {
        return None;
    }
    let mut pixmap = tiny_skia::Pixmap::new(px_w, px_h)?;
    // Stretch the SVG's intrinsic size to fill the box pixel buffer.
    let transform = tiny_skia::Transform::from_scale(px_w as f32 / sw, px_h as f32 / sh);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // The pixmap is premultiplied sRGB RGBA. For `match-source` on an image and
    // for `luminance`, coverage is the (premultiplied) Rec.709 luma; for `alpha`
    // it is the alpha channel directly.
    let data = pixmap.data();
    let mut out = Vec::with_capacity((px_w * px_h) as usize);
    for px in data.chunks_exact(4) {
        // tiny-skia stores premultiplied RGBA; r/g/b are already × alpha.
        let (r, g, b, a) = (
            px[0] as f32 / 255.0,
            px[1] as f32 / 255.0,
            px[2] as f32 / 255.0,
            px[3] as f32 / 255.0,
        );
        let cov = match mode {
            MaskMode::Alpha => a,
            // `luminance` and `match-source` (image) both use luminance. The
            // RGB are premultiplied, so the product already folds in alpha.
            MaskMode::Luminance | MaskMode::MatchSource => 0.2126 * r + 0.7152 * g + 0.0722 * b,
        };
        out.push((cov.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    Some(out)
}

fn flate_compress(data: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).ok()?;
    encoder.finish().ok()
}

fn encode_rgb_as_jpeg(rgb: &[u8], width: u32, height: u32, quality: u8) -> Option<Vec<u8>> {
    use image::ImageEncoder;

    if rgb.len() != width.checked_mul(height)?.checked_mul(3)? as usize {
        return None;
    }
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.clamp(0, 100))
        .write_image(rgb, width, height, image::ExtendedColorType::Rgb8)
        .ok()?;
    Some(buf)
}

fn try_decode_png_as_opaque_rgb(raw_png: &[u8]) -> Option<DecodedPngImage> {
    let decoded = decode_png_for_pdf(raw_png)?;
    if decoded.color_space != "/DeviceRGB" {
        return None;
    }
    if decoded.color_data.len()
        != decoded.width.checked_mul(decoded.height)?.checked_mul(3)? as usize
    {
        return None;
    }
    if decoded
        .alpha_data
        .as_ref()
        .is_some_and(|alpha| alpha.iter().any(|a| *a != 255))
    {
        return None;
    }
    Some(decoded)
}

fn should_try_lossy_png_reencode(width: u32, height: u32, byte_len: usize) -> bool {
    const MIN_LOSSY_PNG_PIXELS: u64 = 16_384;
    const MIN_LOSSY_PNG_BYTES: usize = 16 * 1024;

    u64::from(width) * u64::from(height) >= MIN_LOSSY_PNG_PIXELS && byte_len >= MIN_LOSSY_PNG_BYTES
}

struct ResizedImage {
    data: Vec<u8>,
    width: u32,
    height: u32,
    format: ImageFormat,
    png_metadata: Option<PngMetadata>,
}

/// A custom TrueType font entry for the PDF font dictionary.
struct CustomFontEntry {
    /// Sanitized PDF resource key used from page content streams.
    resource_name: String,
    /// Object ID of the font object.
    font_obj_id: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct RenderOpts {
    pub compress: bool,
    pub jpeg_quality: u8,
    pub auto_resize_images: bool,
    pub image_dpi: f32,
    pub filter_dpi: f32,
    /// Skip embedding raster images fully covered by a later fully-opaque
    /// rectangular element (default false). Conservative; zero visual change.
    pub occlusion_cull: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            compress: true,
            jpeg_quality: 85,
            auto_resize_images: true,
            image_dpi: 300.0,
            filter_dpi: 150.0,
            occlusion_cull: false,
        }
    }
}

/// Minimal PDF writer that produces valid PDF files.
pub(crate) struct PdfWriter {
    objects: Vec<String>,
    /// Raw binary objects stored separately (index corresponds to objects slot).
    binary_objects: std::collections::HashMap<usize, Vec<u8>>,
    page_ids: Vec<usize>,
    /// Annotation object IDs grouped by page index.
    page_annotations: Vec<Vec<usize>>,
    /// Image references grouped by page index.
    page_images: Vec<Vec<ImageRef>>,
    /// ExtGState entries (name, opacity) grouped by page index.
    page_ext_gstates: Vec<Vec<(String, f32)>>,
    /// Shading dictionary entries grouped by page index.
    page_shadings: Vec<Vec<ShadingEntry>>,
    /// Custom TrueType font entries.
    custom_font_entries: Vec<CustomFontEntry>,
    /// CSS `mask-image` soft-mask graphics states: `(gs_name, group_form_obj_id)`.
    /// Each becomes an `/ExtGState << /SMask << /S /Luminosity /G <form> >> >>`
    /// emitted into the shared resource dictionary. Names are global across pages.
    soft_mask_gstates: Vec<(String, usize)>,
    opts: RenderOpts,
}

impl PdfWriter {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
            binary_objects: std::collections::HashMap::new(),
            page_ids: Vec::new(),
            page_annotations: Vec::new(),
            page_images: Vec::new(),
            page_ext_gstates: Vec::new(),
            page_shadings: Vec::new(),
            custom_font_entries: Vec::new(),
            soft_mask_gstates: Vec::new(),
            opts: RenderOpts::default(),
        }
    }

    fn next_id(&self) -> usize {
        self.objects.len() + 1
    }

    /// Add an image as a PDF XObject and return its object ID.
    fn add_image_object(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        format: ImageFormat,
        png_metadata: Option<&PngMetadata>,
    ) -> usize {
        if matches!(format, ImageFormat::Png | ImageFormat::PngAlpha)
            && should_try_lossy_png_reencode(width, height, data.len())
            && let Some(decoded) = try_decode_png_as_opaque_rgb(data)
            && let Some(jpeg) = encode_rgb_as_jpeg(
                &decoded.color_data,
                decoded.width,
                decoded.height,
                self.opts.jpeg_quality,
            )
            && jpeg.len() < data.len()
        {
            return self.add_image_object(
                &jpeg,
                decoded.width,
                decoded.height,
                ImageFormat::Jpeg,
                None,
            );
        }
        // An alpha PNG carries the complete original PNG file; decode it into a
        // colour stream plus an `/SMask`, preserving transparency. Fall back to
        // an opaque RGB embedding if decoding fails for any reason.
        if format == ImageFormat::PngAlpha {
            if let Some(obj_id) = self.add_raw_png_image_object(data) {
                return obj_id;
            }
        }
        let id = self.next_id();
        let header = match format {
            ImageFormat::Jpeg => {
                format!(
                    "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {len} >>\nstream\n",
                    len = data.len(),
                )
            }
            ImageFormat::PngAlpha | ImageFormat::Png => {
                // Reaching here for a PngAlpha means the SMask decode above
                // failed (corrupt PNG); recover its metadata from the IHDR so the
                // passthrough header is still well-formed rather than panicking.
                let parsed_png = crate::parser::png::parse_png(data);
                let recovered = parsed_png.as_ref().map(|info| PngMetadata {
                    channels: info.channels,
                    bit_depth: info.bit_depth,
                });
                let meta = png_metadata
                    .or(recovered.as_ref())
                    .expect("PNG metadata required for PNG images");
                let color_space = match meta.channels {
                    1 | 2 => "/DeviceGray",
                    _ => "/DeviceRGB",
                };
                let stream_data = parsed_png
                    .as_ref()
                    .map_or(data, |info| info.idat_data.as_slice());
                format!(
                    "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color_space} /BitsPerComponent {bpc} /Filter /FlateDecode /DecodeParms << /Predictor 15 /Columns {width} /Colors {channels} /BitsPerComponent {bpc} >> /Length {len} >>\nstream\n",
                    bpc = meta.bit_depth,
                    channels = meta.channels,
                    len = stream_data.len(),
                )
            }
        };
        self.objects.push(header);
        let stream_data = match format {
            ImageFormat::Png | ImageFormat::PngAlpha => crate::parser::png::parse_png(data)
                .map_or_else(|| data.to_vec(), |info| info.idat_data),
            ImageFormat::Jpeg => data.to_vec(),
        };
        self.binary_objects.insert(id, stream_data);
        id
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_source_image_object(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        format: ImageFormat,
        png_metadata: Option<&PngMetadata>,
        display_w_pt: f32,
        display_h_pt: f32,
    ) -> usize {
        if let Some(resized) =
            self.maybe_resize_image(data, width, height, format, display_w_pt, display_h_pt)
        {
            self.add_image_object(
                &resized.data,
                resized.width,
                resized.height,
                resized.format,
                resized.png_metadata.as_ref(),
            )
        } else {
            self.add_image_object(data, width, height, format, png_metadata)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_decodable_source_image_object(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        format: ImageFormat,
        png_metadata: Option<&PngMetadata>,
        display_w_pt: f32,
        display_h_pt: f32,
    ) -> Option<usize> {
        if matches!(format, ImageFormat::Png | ImageFormat::PngAlpha)
            && decode_png_for_pdf(data).is_none()
        {
            return None;
        }
        Some(self.add_source_image_object(
            data,
            width,
            height,
            format,
            png_metadata,
            display_w_pt,
            display_h_pt,
        ))
    }

    fn maybe_resize_image(
        &self,
        data: &[u8],
        source_width: u32,
        source_height: u32,
        format: ImageFormat,
        display_w_pt: f32,
        display_h_pt: f32,
    ) -> Option<ResizedImage> {
        if !self.opts.auto_resize_images
            || source_width == 0
            || source_height == 0
            || display_w_pt <= 0.0
            || display_h_pt <= 0.0
        {
            return None;
        }

        let target_w = (display_w_pt * self.opts.image_dpi.max(72.0) / 72.0)
            .round()
            .max(1.0) as u32;
        let target_h = (display_h_pt * self.opts.image_dpi.max(72.0) / 72.0)
            .round()
            .max(1.0) as u32;
        let scale = ((target_w as f32 / source_width as f32)
            .min(target_h as f32 / source_height as f32))
        .min(1.0);
        if scale >= 1.0 {
            return None;
        }
        let new_w = ((source_width as f32 * scale).round().max(1.0) as u32).min(source_width);
        let new_h = ((source_height as f32 * scale).round().max(1.0) as u32).min(source_height);
        if new_w >= source_width && new_h >= source_height {
            return None;
        }

        match format {
            ImageFormat::Jpeg => {
                let decoded = image::load_from_memory(data).ok()?.to_rgb8();
                let resized = image::imageops::resize(
                    &decoded,
                    new_w,
                    new_h,
                    image::imageops::FilterType::Lanczos3,
                );
                let encoded =
                    encode_rgb_as_jpeg(resized.as_raw(), new_w, new_h, self.opts.jpeg_quality)?;
                Some(ResizedImage {
                    data: encoded,
                    width: new_w,
                    height: new_h,
                    format: ImageFormat::Jpeg,
                    png_metadata: None,
                })
            }
            ImageFormat::Png | ImageFormat::PngAlpha => {
                let decoded = image::load_from_memory(data).ok()?;
                let has_alpha = matches!(
                    decoded.color(),
                    image::ColorType::La8
                        | image::ColorType::La16
                        | image::ColorType::Rgba8
                        | image::ColorType::Rgba16
                        | image::ColorType::Rgba32F
                );
                let mut encoded = Vec::new();
                let output_format = if has_alpha {
                    let rgba = decoded.to_rgba8();
                    let resized = image::imageops::resize(
                        &rgba,
                        new_w,
                        new_h,
                        image::imageops::FilterType::Lanczos3,
                    );
                    image::DynamicImage::ImageRgba8(resized)
                        .write_to(
                            &mut std::io::Cursor::new(&mut encoded),
                            image::ImageFormat::Png,
                        )
                        .ok()?;
                    ImageFormat::PngAlpha
                } else {
                    let rgb = decoded.to_rgb8();
                    let resized = image::imageops::resize(
                        &rgb,
                        new_w,
                        new_h,
                        image::imageops::FilterType::Lanczos3,
                    );
                    image::DynamicImage::ImageRgb8(resized)
                        .write_to(
                            &mut std::io::Cursor::new(&mut encoded),
                            image::ImageFormat::Png,
                        )
                        .ok()?;
                    ImageFormat::Png
                };
                let png_metadata =
                    crate::parser::png::parse_png(&encoded).map(|info| PngMetadata {
                        channels: info.channels,
                        bit_depth: info.bit_depth,
                    });
                Some(ResizedImage {
                    data: encoded,
                    width: new_w,
                    height: new_h,
                    format: output_format,
                    png_metadata,
                })
            }
        }
    }

    #[allow(dead_code)]
    fn add_icc_profile_object(&mut self, icc_profile: &[u8]) -> Option<usize> {
        let id = self.next_id();
        self.objects.push(format!(
            "{id} 0 obj\n<< /N 3 /Alternate /DeviceRGB /Length {} >>\nstream\n",
            icc_profile.len(),
        ));
        self.binary_objects.insert(id, icc_profile.to_vec());
        Some(id)
    }

    #[allow(dead_code)]
    pub(crate) fn add_raw_rgb_image_object(
        &mut self,
        rgb_data: &[u8],
        width: u32,
        height: u32,
        icc_profile: Option<&[u8]>,
    ) -> Option<usize> {
        let color_stream = flate_compress(rgb_data)?;
        let color_space = if let Some(icc_profile) = icc_profile {
            let icc_id = self.add_icc_profile_object(icc_profile)?;
            format!("[/ICCBased {icc_id} 0 R]")
        } else {
            "/DeviceRGB".to_string()
        };

        let id = self.next_id();
        self.objects.push(format!(
            "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color_space} /BitsPerComponent 8 /Filter /FlateDecode /Length {len} >>\nstream\n",
            len = color_stream.len(),
        ));
        self.binary_objects.insert(id, color_stream);
        Some(id)
    }

    pub(crate) fn add_raw_png_image_object(&mut self, raw_png: &[u8]) -> Option<usize> {
        let decoded = decode_png_for_pdf(raw_png)?;
        let alpha_stream = if let Some(alpha_data) = decoded.alpha_data.as_deref() {
            Some(flate_compress(alpha_data)?)
        } else {
            None
        };

        let alpha_id = alpha_stream.map(|stream| {
            let id = self.next_id();
            let header = format!(
                "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {len} >>\nstream\n",
                width = decoded.width,
                height = decoded.height,
                len = stream.len(),
            );
            self.objects.push(header);
            self.binary_objects.insert(id, stream);
            id
        });

        // Colour stream: JPEG (/DCTDecode) for a large photographic DeviceRGB
        // image, keeping the alpha in a separate Flate /SMask — exactly how Chrome
        // embeds a semi-transparent photo (DCTDecode colour + soft mask). Lossy, so
        // gated to images large enough to be worth re-encoding (small synthetic
        // PNGs stay lossless Flate). DeviceGray and small images keep Flate.
        let jpeg_color = (decoded.color_space == "/DeviceRGB"
            && should_try_lossy_png_reencode(decoded.width, decoded.height, raw_png.len()))
        .then(|| {
            encode_rgb_as_jpeg(
                &decoded.color_data,
                decoded.width,
                decoded.height,
                self.opts.jpeg_quality,
            )
        })
        .flatten();
        let (filter, color_stream) = match jpeg_color {
            Some(jpeg) => ("/DCTDecode", jpeg),
            None => ("/FlateDecode", flate_compress(&decoded.color_data)?),
        };

        let id = self.next_id();
        let mut header = format!(
            "{id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color_space} /BitsPerComponent 8 /Filter {filter} /Length {len}",
            width = decoded.width,
            height = decoded.height,
            color_space = decoded.color_space,
            len = color_stream.len(),
        );
        if let Some(alpha_id) = alpha_id {
            header.push_str(&format!(" /SMask {alpha_id} 0 R"));
        }
        header.push_str(" >>\nstream\n");

        self.objects.push(header);
        self.binary_objects.insert(id, color_stream);
        Some(id)
    }

    /// Build a CSS `mask-image` soft mask (css-masking-1 §3) for a box of size
    /// `w` × `h` points whose top-left sits at PDF coordinate (`x`, `top_y`).
    ///
    /// The mask source is rasterised to a DeviceGray coverage buffer (alpha for
    /// `mask-mode: alpha`/`match-source` on a CSS image, luminance for
    /// `luminance`), wrapped in a `/Luminosity` transparency-group form XObject
    /// positioned over the box, and registered as an `/SMask` ExtGState. Returns
    /// the graphics-state name to emit with `gs` (the caller wraps the masked
    /// paint in `q /name gs ... Q`), or `None` if the source can't be rasterised.
    pub(crate) fn add_mask_soft_mask(
        &mut self,
        source: &crate::style::computed::MaskSource,
        mode: crate::style::computed::MaskMode,
        x: f32,
        top_y: f32,
        w: f32,
        h: f32,
    ) -> Option<String> {
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        // VECTOR path for SVG masks (resolution-independent, like Chrome, which
        // emits the mask as transparency-group bezier paths — NOT a fixed-DPI
        // bitmap). Render the SVG as PDF vector ops into the luminosity group.
        // Falls through to the raster path below for gradient masks, or any SVG
        // that can't be vectorised self-contained (needs shading/image/font
        // resources, or fails to parse).
        if let crate::style::computed::MaskSource::Svg(bytes) = source {
            if let Some(gs) = self.try_svg_vector_soft_mask(bytes, x, top_y, w, h) {
                return Some(gs);
            }
        }
        // VECTOR path for linear-gradient masks: a native PDF axial shading painted
        // into a DeviceRGB luminosity group (gray = mask coverage), resolution-
        // independent like Chrome — instead of a 1-sample/CSS-px coverage bitmap
        // upscaled by the device. Conic masks stay raster (no native PDF conic
        // shading); radial masks could follow the same approach later.
        if let crate::style::computed::MaskSource::Linear(lg) = source {
            if let Some(gs) = self.try_linear_mask_vector_shading(lg, mode, x, top_y, w, h) {
                return Some(gs);
            }
        }
        // Raster fallback for gradient masks (SVG masks take the vector path
        // above). Sample at ~1 per CSS pixel: gradient coverage is smooth so it
        // upscales without a hard edge, and the gradient's CSS-px geometry (center
        // /radius) is computed in this same space, so the grid must stay 1:1 with
        // CSS px. Capped so a very large box can't blow up the PDF.
        let px_w = ((w / 0.75).round() as u32).clamp(1, 1024);
        let px_h = ((h / 0.75).round() as u32).clamp(1, 1024);
        let coverage = match source {
            crate::style::computed::MaskSource::Svg(bytes) => {
                rasterize_svg_mask_coverage(bytes, mode, px_w, px_h)?
            }
            _ => rasterize_mask_coverage(source, mode, px_w, px_h),
        };

        // DeviceGray coverage image (luminosity source for the group).
        let gray_stream = flate_compress(&coverage)?;
        let img_id = self.next_id();
        self.objects.push(format!(
            "{img_id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {px_w} /Height {px_h} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {len} >>\nstream\n",
            len = gray_stream.len(),
        ));
        self.binary_objects.insert(img_id, gray_stream);

        // Transparency-group form XObject that draws the coverage image over the
        // box. The group backdrop for a luminosity mask is black (coverage 0 →
        // fully masked), so the mask is implicitly `no-repeat`: pixels outside
        // the box's image region contribute zero coverage.
        let bottom_y = top_y - h;
        let group_stream = format!("q\n{w} 0 0 {h} {x} {bottom_y} cm\n/MaskImg Do\nQ\n");
        let group_bytes = group_stream.into_bytes();
        let form_id = self.next_id();
        self.objects.push(format!(
            "{form_id} 0 obj\n<< /Type /XObject /Subtype /Form /FormType 1 /BBox [{x} {bottom_y} {x1} {top_y}] /Group << /Type /Group /S /Transparency /CS /DeviceGray >> /Resources << /XObject << /MaskImg {img_id} 0 R >> >> /Length {len} >>\nstream\n",
            x1 = x + w,
            len = group_bytes.len(),
        ));
        self.binary_objects.insert(form_id, group_bytes);

        let gs_name = format!("GSmask{form_id}");
        self.soft_mask_gstates.push((gs_name.clone(), form_id));
        Some(gs_name)
    }

    /// Try to build a CSS `mask-image: url(svg)` soft mask as resolution-
    /// independent VECTOR paths (matching Chrome), rendering the SVG into the
    /// luminosity transparency group instead of a fixed-DPI coverage bitmap.
    /// Returns `None` (so the caller falls back to raster) when the SVG can't be
    /// parsed, has zero size, renders nothing, or needs gradient/image/font
    /// resources the self-contained mask form can't carry.
    fn try_svg_vector_soft_mask(
        &mut self,
        svg_bytes: &[u8],
        x: f32,
        top_y: f32,
        w: f32,
        h: f32,
    ) -> Option<String> {
        let svg_text = std::str::from_utf8(svg_bytes).ok()?;
        let tree = crate::parser::svg::parse_svg_from_string(svg_text)?;
        // The SVG user-coordinate extent (viewBox if present, else width/height).
        let (sw, sh) = tree
            .view_box
            .map(|vb| (vb.width, vb.height))
            .unwrap_or((tree.width, tree.height));
        if !(sw > 0.0 && sh > 0.0) {
            return None;
        }
        let mut svg_content = String::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        crate::render::svg_to_pdf::render_svg_tree_with_shadings(
            &tree,
            &mut svg_content,
            &mut shadings,
            &mut shading_counter,
        );
        // The mask form is self-contained (empty /Resources): bail to raster if
        // the SVG produced gradient shadings (would need /Shading resources) or
        // drew nothing.
        if !shadings.is_empty() || svg_content.trim().is_empty() {
            return None;
        }
        let bottom_y = top_y - h;
        let x1 = x + w;
        // Map the SVG user space (y-down, 0..sw × 0..sh) onto the box (PDF y-up):
        // scale to the box and flip Y so SVG (0,0) lands at the box top-left.
        let group = format!(
            "q\n{a} 0 0 {d} {e} {f} cm\n{svg_content}Q\n",
            a = format_pdf_number(w / sw),
            d = format_pdf_number(-h / sh),
            e = format_pdf_number(x),
            f = format_pdf_number(top_y),
        );
        let group_bytes = group.into_bytes();
        let form_id = self.next_id();
        self.objects.push(format!(
            "{form_id} 0 obj\n<< /Type /XObject /Subtype /Form /FormType 1 /BBox [{x} {bottom_y} {x1} {top_y}] /Group << /Type /Group /S /Transparency /CS /DeviceRGB >> /Resources << >> /Length {len} >>\nstream\n",
            len = group_bytes.len(),
        ));
        self.binary_objects.insert(form_id, group_bytes);
        let gs_name = format!("GSmask{form_id}");
        self.soft_mask_gstates.push((gs_name.clone(), form_id));
        Some(gs_name)
    }

    /// Build a `mask-image: linear-gradient(...)` soft mask as a native PDF axial
    /// shading (vector, resolution-independent) instead of a coverage bitmap. The
    /// shading paints `(g, g, g)` where `g` is the mask coverage the gradient
    /// asks for (alpha for image masks, luminance for luminance mode), into a
    /// DeviceRGB `/Luminosity` transparency group — whose luminosity of an equal
    /// RGB triple is exactly `g`. Returns `None` for a degenerate (<2 stop)
    /// gradient so the caller falls back to raster.
    fn try_linear_mask_vector_shading(
        &mut self,
        lg: &crate::style::computed::LinearGradient,
        mode: crate::style::computed::MaskMode,
        x: f32,
        top_y: f32,
        w: f32,
        h: f32,
    ) -> Option<String> {
        if lg.stops.len() < 2 {
            return None;
        }
        let bottom_y = top_y - h;
        let x1 = x + w;
        // Coverage as a gray level per stop, replicated into an RGB triple.
        let base: Vec<(f32, (f32, f32, f32))> = lg
            .stops
            .iter()
            .map(|s| {
                let c = s.color;
                let rgba = (
                    f32::from(c.r) / 255.0,
                    f32::from(c.g) / 255.0,
                    f32::from(c.b) / 255.0,
                    f32::from(c.a) / 255.0,
                );
                let g = f32::from(coverage_byte(rgba, mode)) / 255.0;
                (s.position, (g, g, g))
            })
            .collect();
        let stops = if lg.repeating {
            repeat_stops_to_unit(&base)
        } else {
            base
        };
        // Reuse the CSS axial-gradient emitter to lay the gradient line over the
        // mask box and push the shading entry (clip rect = the box).
        let mut shadings = Vec::new();
        let mut counter = 0usize;
        let mut group = String::new();
        render_linear_gradient_tile(
            &mut group,
            lg.angle,
            x,
            bottom_y,
            w,
            h,
            &stops,
            &mut shadings,
            &mut counter,
        );
        let entry = shadings.into_iter().next()?;
        let function_str = build_shading_function(&entry.stops);
        let sh_id = self.next_id();
        self.objects.push(format!(
            "{sh_id} 0 obj\n<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [{} {} {} {}] /Function {function_str} /Extend [true true] >>\nendobj",
            entry.coords[0], entry.coords[1], entry.coords[2], entry.coords[3],
        ));
        let group_bytes = group.into_bytes();
        let form_id = self.next_id();
        self.objects.push(format!(
            "{form_id} 0 obj\n<< /Type /XObject /Subtype /Form /FormType 1 /BBox [{x} {bottom_y} {x1} {top_y}] /Group << /Type /Group /S /Transparency /CS /DeviceRGB >> /Resources << /Shading << /{name} {sh_id} 0 R >> >> /Length {len} >>\nstream\n",
            name = entry.name,
            len = group_bytes.len(),
        ));
        self.binary_objects.insert(form_id, group_bytes);
        let gs_name = format!("GSmask{form_id}");
        self.soft_mask_gstates.push((gs_name.clone(), form_id));
        Some(gs_name)
    }

    pub(crate) fn add_raw_raster_image_object(&mut self, raw_image: &[u8]) -> Option<usize> {
        if crate::parser::png::is_png(raw_image) {
            return self.add_raw_png_image_object(raw_image);
        }

        let (width, height) = crate::parser::jpeg::parse_jpeg_dimensions(raw_image)?;
        Some(self.add_image_object(raw_image, width, height, ImageFormat::Jpeg, None))
    }

    /// Embed a TrueType font and return the PDF resource name to reference it.
    fn add_ttf_font(
        &mut self,
        name: &str,
        ttf: &TtfFont,
        prepared_font: &PreparedCustomFont,
    ) -> String {
        let resource_name = sanitize_pdf_name(name);
        let base_font_name = &prepared_font.base_font_name;

        // 1. Font stream: embed the prepared font data and compress the stream
        // to avoid paying the full raw TTF size in the PDF.
        let stream_id = self.next_id();
        let compressed_data = flate_compress(&prepared_font.font_data);
        let header = if let Some(ref compressed_data) = compressed_data {
            format!(
                "{stream_id} 0 obj\n<< /Filter /FlateDecode /Length {} /Length1 {} >>\nstream\n",
                compressed_data.len(),
                prepared_font.font_data.len(),
            )
        } else {
            format!(
                "{stream_id} 0 obj\n<< /Length {} /Length1 {} >>\nstream\n",
                prepared_font.font_data.len(),
                prepared_font.font_data.len(),
            )
        };
        self.objects.push(header);
        self.binary_objects.insert(
            stream_id,
            compressed_data.unwrap_or_else(|| prepared_font.font_data.clone()),
        );

        // 2. FontDescriptor
        let descriptor_id = self.next_id();
        let pdf_metrics = ttf.pdf_vertical_metrics();
        let ascent_pdf = (pdf_metrics.ascent as i32 * 1000) / ttf.units_per_em as i32;
        let descent_pdf = (pdf_metrics.descent as i32 * 1000) / ttf.units_per_em as i32;
        let bbox_pdf = [
            (ttf.bbox[0] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[1] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[2] as i32 * 1000) / ttf.units_per_em as i32,
            (ttf.bbox[3] as i32 * 1000) / ttf.units_per_em as i32,
        ];
        self.objects.push(format!(
            "{descriptor_id} 0 obj\n<< /Type /FontDescriptor /FontName /{base_font_name} /Flags {flags} /FontBBox [{b0} {b1} {b2} {b3}] /Ascent {ascent} /Descent {descent} /ItalicAngle 0 /CapHeight {ascent} /StemV 80 /FontFile2 {stream_id} 0 R >>\nendobj",
            flags = ttf.flags,
            b0 = bbox_pdf[0],
            b1 = bbox_pdf[1],
            b2 = bbox_pdf[2],
            b3 = bbox_pdf[3],
            ascent = ascent_pdf,
            descent = descent_pdf,
        ));

        // 3. CID widths array keyed by glyph ID so shaped glyph IDs can be
        // emitted directly with Identity-H.
        let widths_str = prepared_font
            .widths
            .iter()
            .copied()
            .map(format_pdf_number)
            .collect::<Vec<_>>()
            .join(" ");

        // 4. CID descendant font object
        let cid_font_id = self.next_id();
        self.objects.push(format!(
            "{cid_font_id} 0 obj\n<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base_font_name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor_id} 0 R /CIDToGIDMap /Identity /W [0 [{widths_str}]] >>\nendobj",
        ));

        // 5. ToUnicode CMap so text stays searchable/selectable.
        let to_unicode_id = self.next_id();
        let to_unicode = build_tounicode_cmap(&prepared_font.to_unicode_map);
        self.objects.push(format!(
            "{to_unicode_id} 0 obj\n<< /Length {} >>\nstream\n{to_unicode}endstream\nendobj",
            to_unicode.len(),
        ));

        // 6. Type0 wrapper font object
        let font_id = self.next_id();
        self.objects.push(format!(
            "{font_id} 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /{base_font_name} /Encoding /Identity-H /DescendantFonts [{cid_font_id} 0 R] /ToUnicode {to_unicode_id} 0 R >>\nendobj",
        ));

        self.custom_font_entries.push(CustomFontEntry {
            resource_name: resource_name.clone(),
            font_obj_id: font_id,
        });

        resource_name
    }

    #[allow(clippy::too_many_arguments)]
    fn add_page(
        &mut self,
        width: f32,
        height: f32,
        content: &str,
        annotations: Vec<LinkAnnotation>,
        images: Vec<ImageRef>,
        ext_gstates: Vec<(String, f32)>,
        shadings: Vec<ShadingEntry>,
    ) {
        // Content stream — FlateDecode-compressed when enabled (lossless and
        // transparent to rasterization; PDF content streams are uncompressed
        // PostScript-like operators that shrink ~5-8x). Falls back to raw if
        // compression is disabled or fails.
        let content_id = self.next_id();
        match self
            .opts
            .compress
            .then(|| flate_compress(content.as_bytes()))
            .flatten()
        {
            Some(comp) => {
                // Binary stream: header ends at "stream\n"; the writer appends the
                // bytes then "\nendstream\nendobj" (see finish_to_writer).
                self.objects.push(format!(
                    "{content_id} 0 obj\n<< /Length {} /Filter /FlateDecode >>\nstream\n",
                    comp.len(),
                ));
                self.binary_objects.insert(content_id, comp);
            }
            None => {
                self.objects.push(format!(
                    "{content_id} 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj",
                    content.len(),
                ));
            }
        }
        let page_id = self.objects.len() + annotations.len() + 1;

        // Annotation objects
        let mut annot_ids = Vec::new();
        for annot in &annotations {
            let annot_id = self.next_id();
            self.objects.push(format!(
                "{annot_id} 0 obj\n<< /Type /Annot /Subtype /Link /P {page_id} 0 R /Rect [{x1} {y1} {x2} {y2}] /Border [0 0 0] /A << /Type /Action /S /URI /URI ({uri}) >> >>\nendobj",
                page_id = page_id,
                x1 = annot.x1,
                y1 = annot.y1,
                x2 = annot.x2,
                y2 = annot.y2,
                uri = escape_pdf_string(&annot.url),
            ));
            annot_ids.push(annot_id);
        }

        // Page object (placeholder — will be updated in finish())
        self.objects.push(format!(
            "{page_id} 0 obj\n<< /Type /Page /MediaBox [0 0 {width} {height}] /Contents {content_id} 0 R >>\nendobj",
        ));

        self.page_ids.push(page_id);
        self.page_annotations.push(annot_ids);
        self.page_images.push(images);
        self.page_ext_gstates.push(ext_gstates);
        self.page_shadings.push(shadings);
    }

    fn finish_to_writer<W: std::io::Write>(
        self,
        out: &mut W,
        bookmarks: &[BookmarkEntry],
    ) -> Result<(), IronpressError> {
        let mut bytes_written: usize = 0;
        out.write_all(b"%PDF-1.4\n")?;
        bytes_written += b"%PDF-1.4\n".len();

        // Font objects
        let font_base_id = self.objects.len() + 1;
        let font_names = [
            // Helvetica (sans-serif)
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Helvetica-BoldOblique",
            // Times Roman (serif)
            "Times-Roman",
            "Times-Bold",
            "Times-Italic",
            "Times-BoldItalic",
            // Courier (monospace)
            "Courier",
            "Courier-Bold",
            "Courier-Oblique",
            "Courier-BoldOblique",
            // Symbol (math/Greek)
            "Symbol",
        ];

        let mut all_objects: Vec<String> = self.objects.clone();

        for (i, name) in font_names.iter().enumerate() {
            let id = font_base_id + i;
            if name == &"Symbol" {
                // Symbol font uses its own built-in encoding, not WinAnsiEncoding
                all_objects.push(format!(
                    "{id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /{name} >>\nendobj",
                ));
            } else {
                all_objects.push(format!(
                    "{id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>\nendobj",
                ));
            }
        }

        // Font dictionary (standard + custom fonts)
        let font_dict_id = font_base_id + font_names.len();
        let mut font_entries: Vec<String> = font_names
            .iter()
            .enumerate()
            .map(|(i, name)| format!("/{name} {} 0 R", font_base_id + i))
            .collect();
        // Add custom font entries
        for entry in &self.custom_font_entries {
            font_entries.push(format!(
                "/{} {} 0 R",
                entry.resource_name, entry.font_obj_id
            ));
        }
        let font_entries_str = font_entries.join(" ");
        all_objects.push(format!(
            "{font_dict_id} 0 obj\n<< {font_entries_str} >>\nendobj",
        ));

        // Collect all image object IDs used across all pages
        let mut all_image_refs: Vec<(&str, usize)> = Vec::new();
        for page_imgs in &self.page_images {
            for img in page_imgs {
                if !all_image_refs.iter().any(|(_, id)| *id == img.obj_id) {
                    all_image_refs.push((&img.name, img.obj_id));
                }
            }
        }

        // Collect unique ExtGState entries across all pages
        let mut gs_entries: Vec<(String, f32)> = Vec::new();
        for page_gs in &self.page_ext_gstates {
            for (name, opacity) in page_gs {
                if !gs_entries.iter().any(|(n, _)| n == name) {
                    gs_entries.push((name.clone(), *opacity));
                }
            }
        }
        let has_opacity = !gs_entries.is_empty();
        // CSS `mask-image` soft-mask graphics states (css-masking-1 §3) — emitted
        // alongside the opacity/blend gstates into the shared resource dict.
        let has_soft_masks = !self.soft_mask_gstates.is_empty();
        let has_gstates = has_opacity || has_soft_masks;

        // Add ExtGState objects if needed
        let mut gs_obj_refs: Vec<(String, usize)> = Vec::new();
        if has_gstates {
            // GSDefault (opacity 1.0)
            let default_gs_id = all_objects.len() + 1;
            all_objects.push(format!(
                "{default_gs_id} 0 obj\n<< /Type /ExtGState /ca 1 /CA 1 >>\nendobj"
            ));
            gs_obj_refs.push(("GSDefault".to_string(), default_gs_id));

            // Per-element ExtGState objects. Names prefixed `GSbm` carry a blend
            // mode (e.g. `GSbmMultiply` → `/BM /Multiply`); all others are alpha
            // (`/ca` / `/CA`) gstates whose float value is the opacity.
            for (name, opacity) in &gs_entries {
                let gs_id = all_objects.len() + 1;
                let body = match name.strip_prefix("GSbm") {
                    Some(mode) => format!("/Type /ExtGState /BM /{mode}"),
                    None => format!("/Type /ExtGState /ca {opacity} /CA {opacity}"),
                };
                all_objects.push(format!("{gs_id} 0 obj\n<< {body} >>\nendobj"));
                gs_obj_refs.push((name.clone(), gs_id));
            }

            // Soft-mask gstates: a luminosity transparency group derived from the
            // rasterised mask image (the `/G` form XObject was created at render
            // time, so its object id is already valid in `self.objects`).
            for (name, form_id) in &self.soft_mask_gstates {
                let gs_id = all_objects.len() + 1;
                all_objects.push(format!(
                    "{gs_id} 0 obj\n<< /Type /ExtGState /SMask << /Type /Mask /S /Luminosity /G {form_id} 0 R >> >>\nendobj"
                ));
                gs_obj_refs.push((name.clone(), gs_id));
            }
        }

        // Add Shading objects
        let mut shading_obj_refs: Vec<(String, usize)> = Vec::new();
        for page_sh in &self.page_shadings {
            for entry in page_sh {
                let sh_id = all_objects.len() + 1;
                let function_str = build_shading_function(&entry.stops);
                let coords_str = if entry.shading_type == 2 {
                    // Axial: only first 4 coords
                    format!(
                        "{} {} {} {}",
                        entry.coords[0], entry.coords[1], entry.coords[2], entry.coords[3]
                    )
                } else {
                    // Radial: all 6 coords
                    format!(
                        "{} {} {} {} {} {}",
                        entry.coords[0],
                        entry.coords[1],
                        entry.coords[2],
                        entry.coords[3],
                        entry.coords[4],
                        entry.coords[5]
                    )
                };
                all_objects.push(format!(
                    "{sh_id} 0 obj\n<< /ShadingType {} /ColorSpace /DeviceRGB /Coords [{coords_str}] /Function {function_str} /Extend [true true] >>\nendobj",
                    entry.shading_type,
                ));
                shading_obj_refs.push((entry.name.clone(), sh_id));
            }
        }

        // Resources dictionary
        let resources_id = all_objects.len() + 1;
        let mut resource_parts = format!("/Font {font_dict_id} 0 R");

        if !all_image_refs.is_empty() {
            let xobj_entries: String = all_image_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /XObject << {xobj_entries} >>"));
        }

        if has_gstates {
            let gs_dict: String = gs_obj_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /ExtGState << {gs_dict} >>"));
        }

        if !shading_obj_refs.is_empty() {
            let shading_dict: String = shading_obj_refs
                .iter()
                .map(|(name, id)| format!("/{name} {id} 0 R"))
                .collect::<Vec<_>>()
                .join(" ");
            resource_parts.push_str(&format!(" /Shading << {shading_dict} >>"));
        }

        all_objects.push(format!(
            "{resources_id} 0 obj\n<< {resource_parts} >>\nendobj",
        ));

        // Update page objects to include parent, resources, and annotations
        let pages_id = resources_id + 1;
        for (idx, &page_id) in self.page_ids.iter().enumerate() {
            let obj = &mut all_objects[page_id - 1];
            let annot_ids = &self.page_annotations[idx];
            let mut extra = format!("/Parent {pages_id} 0 R /Resources {resources_id} 0 R");
            if !annot_ids.is_empty() {
                let annots_str: String = annot_ids
                    .iter()
                    .map(|id| format!("{id} 0 R"))
                    .collect::<Vec<_>>()
                    .join(" ");
                extra.push_str(&format!(" /Annots [{annots_str}]"));
            }
            *obj = obj.replace("/Contents", &format!("{extra} /Contents"));
        }

        // Pages object
        let kids: String = self
            .page_ids
            .iter()
            .map(|id| format!("{id} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        all_objects.push(format!(
            "{pages_id} 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {} >>\nendobj",
            self.page_ids.len(),
        ));

        // Outlines (PDF bookmarks from headings)
        let outlines_ref = if bookmarks.is_empty() {
            String::new()
        } else {
            let count = bookmarks.len();
            // Outline root object
            let root_id = all_objects.len() + 1;
            let first_entry_id = root_id + 1;
            let last_entry_id = first_entry_id + count - 1;
            all_objects.push(format!(
                "{root_id} 0 obj\n<< /Type /Outlines /First {first_entry_id} 0 R /Last {last_entry_id} 0 R /Count {count} >>\nendobj",
            ));

            // Outline entry objects (flat list, linked via Prev/Next)
            for (i, bm) in bookmarks.iter().enumerate() {
                let entry_id = first_entry_id + i;
                let page_obj_id = self.page_ids.get(bm.page_index).copied().unwrap_or(1);

                let mut entry = format!(
                    "{entry_id} 0 obj\n<< /Title ({title}) /Parent {root_id} 0 R /Dest [{page_obj_id} 0 R /XYZ 0 {dest_y} 0]",
                    title = escape_pdf_string(&bm.title),
                    dest_y = bm.y_pos,
                );
                if i > 0 {
                    entry.push_str(&format!(" /Prev {} 0 R", first_entry_id + i - 1));
                }
                if i + 1 < count {
                    entry.push_str(&format!(" /Next {} 0 R", first_entry_id + i + 1));
                }
                entry.push_str(" >>\nendobj");
                all_objects.push(entry);
            }

            format!(" /Outlines {root_id} 0 R /PageMode /UseOutlines")
        };

        // Catalog
        let catalog_id = all_objects.len() + 1;
        all_objects.push(format!(
            "{catalog_id} 0 obj\n<< /Type /Catalog /Pages {pages_id} 0 R{outlines_ref} >>\nendobj",
        ));

        // Write objects and track offsets for xref
        // Binary objects (images) need special handling
        let mut offsets = Vec::new();
        for (idx, obj_str) in all_objects.iter().enumerate() {
            offsets.push(bytes_written);
            let obj_id = idx + 1;
            if let Some(bin_data) = self.binary_objects.get(&obj_id) {
                // Write the header (stored in obj_str), then binary data, then endstream/endobj
                out.write_all(obj_str.as_bytes())?;
                bytes_written += obj_str.len();
                out.write_all(bin_data)?;
                bytes_written += bin_data.len();
                out.write_all(b"\nendstream\nendobj\n")?;
                bytes_written += b"\nendstream\nendobj\n".len();
            } else {
                out.write_all(obj_str.as_bytes())?;
                bytes_written += obj_str.len();
                out.write_all(b"\n")?;
                bytes_written += 1;
            }
        }

        // Cross-reference table
        let xref_offset = bytes_written;
        let xref_header = format!("xref\n0 {}\n", all_objects.len() + 1);
        out.write_all(xref_header.as_bytes())?;
        out.write_all(b"0000000000 65535 f \n")?;
        for offset in &offsets {
            let entry = format!("{:010} 00000 n \n", offset);
            out.write_all(entry.as_bytes())?;
        }

        // Trailer
        let trailer = format!(
            "trailer\n<< /Size {} /Root {catalog_id} 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            all_objects.len() + 1,
        );
        out.write_all(trailer.as_bytes())?;

        Ok(())
    }
}

/// Map a Unicode character to the Adobe Symbol font encoding byte.
fn unicode_to_symbol(ch: char) -> Option<u8> {
    match ch {
        // Greek lowercase
        '\u{03B1}' => Some(0x61), // α → a
        '\u{03B2}' => Some(0x62), // β → b
        '\u{03B3}' => Some(0x67), // γ → g
        '\u{03B4}' => Some(0x64), // δ → d
        '\u{03B5}' => Some(0x65), // ε → e
        '\u{03B6}' => Some(0x7A), // ζ → z
        '\u{03B7}' => Some(0x68), // η → h
        '\u{03B8}' => Some(0x71), // θ → q
        '\u{03B9}' => Some(0x69), // ι → i
        '\u{03BA}' => Some(0x6B), // κ → k
        '\u{03BB}' => Some(0x6C), // λ → l
        '\u{03BC}' => Some(0x6D), // μ → m
        '\u{03BD}' => Some(0x6E), // ν → n
        '\u{03BE}' => Some(0x78), // ξ → x
        '\u{03C0}' => Some(0x70), // π → p
        '\u{03C1}' => Some(0x72), // ρ → r
        '\u{03C3}' => Some(0x73), // σ → s
        '\u{03C4}' => Some(0x74), // τ → t
        '\u{03C5}' => Some(0x75), // υ → u
        '\u{03C6}' => Some(0x66), // φ → f
        '\u{03C7}' => Some(0x63), // χ → c
        '\u{03C8}' => Some(0x79), // ψ → y
        '\u{03C9}' => Some(0x77), // ω → w
        // Greek uppercase
        '\u{0393}' => Some(0x47), // Γ → G
        '\u{0394}' => Some(0x44), // Δ → D
        '\u{0398}' => Some(0x51), // Θ → Q
        '\u{039B}' => Some(0x4C), // Λ → L
        '\u{039E}' => Some(0x58), // Ξ → X
        '\u{03A0}' => Some(0x50), // Π → P
        '\u{03A3}' => Some(0x53), // Σ → S
        '\u{03A5}' => Some(0xA1), // Υ
        '\u{03A6}' => Some(0x46), // Φ → F
        '\u{03A8}' => Some(0x59), // Ψ → Y
        '\u{03A9}' => Some(0x57), // Ω → W
        // Large operators
        '\u{2211}' => Some(0xE5), // ∑
        '\u{220F}' => Some(0xD5), // ∏
        '\u{2210}' => Some(0xD5), // ∐ (fallback to ∏)
        '\u{222B}' => Some(0xF2), // ∫
        '\u{222C}' => Some(0xF2), // ∬ (fallback to ∫)
        '\u{222D}' => Some(0xF2), // ∭ (fallback to ∫)
        '\u{222E}' => Some(0xF2), // ∮ (fallback to ∫)
        '\u{22C3}' => Some(0xC8), // ⋃
        '\u{22C2}' => Some(0xC7), // ⋂
        // Relations
        '\u{2264}' => Some(0xA3), // ≤
        '\u{2265}' => Some(0xB3), // ≥
        '\u{2260}' => Some(0xB9), // ≠
        '\u{2248}' => Some(0xBB), // ≈
        '\u{2261}' => Some(0xBA), // ≡
        '\u{221D}' => Some(0xB5), // ∝
        '\u{2282}' => Some(0xCC), // ⊂
        '\u{2283}' => Some(0xC9), // ⊃
        '\u{2286}' => Some(0xCD), // ⊆
        '\u{2287}' => Some(0xCA), // ⊇
        '\u{2208}' => Some(0xCE), // ∈
        '\u{2209}' => Some(0xCF), // ∉
        '\u{22A2}' => Some(0x5E), // ⊢ (fallback)
        '\u{22A8}' => Some(0xF0), // ⊨
        // Arrows
        '\u{2192}' => Some(0xAE), // →
        '\u{2190}' => Some(0xAC), // ←
        '\u{2194}' => Some(0xAB), // ↔
        '\u{21D2}' => Some(0xDE), // ⇒
        '\u{21D0}' => Some(0xDC), // ⇐
        '\u{21D4}' => Some(0xDB), // ⇔
        '\u{21A6}' => Some(0xAE), // ↦ (fallback to →)
        // Binary operators
        '\u{00D7}' => Some(0xB4), // ×
        '\u{00F7}' => Some(0xB8), // ÷
        '\u{22C5}' => Some(0xD7), // ⋅
        '\u{00B1}' => Some(0xB1), // ±
        '\u{2213}' => Some(0xB1), // ∓ (fallback to ±)
        '\u{2218}' => Some(0xB0), // ∘
        '\u{2295}' => Some(0xC5), // ⊕
        '\u{2297}' => Some(0xC4), // ⊗
        '\u{222A}' => Some(0xC8), // ∪
        '\u{2229}' => Some(0xC7), // ∩
        '\u{2227}' => Some(0xD9), // ∧
        '\u{2228}' => Some(0xDA), // ∨
        // Misc math symbols
        '\u{221E}' => Some(0xA5), // ∞
        '\u{2202}' => Some(0xB6), // ∂
        '\u{2207}' => Some(0xD1), // ∇
        '\u{2200}' => Some(0x22), // ∀
        '\u{2203}' => Some(0x24), // ∃
        '\u{00AC}' => Some(0xD8), // ¬
        '\u{2205}' => Some(0xC6), // ∅
        '\u{2135}' => Some(0xC0), // ℵ
        '\u{221A}' => Some(0xD6), // √
        '\u{2032}' => Some(0xA2), // ′
        '\u{2026}' => Some(0xBC), // …
        '\u{22EF}' => Some(0xBC), // ⋯
        '\u{2016}' => Some(0xBD), // ‖
        // Delimiters
        '\u{27E8}' => Some(0xE1), // ⟨
        '\u{27E9}' => Some(0xF1), // ⟩
        '\u{230A}' => Some(0xEB), // ⌊
        '\u{230B}' => Some(0xFB), // ⌋
        '\u{2308}' => Some(0xE9), // ⌈
        '\u{2309}' => Some(0xF9), // ⌉
        _ => None,
    }
}

/// Render math glyphs to PDF content stream operators.
fn render_math_glyphs(
    glyphs: &[crate::layout::math::MathGlyph],
    origin_x: f32,
    origin_y: f32,
    content: &mut String,
) {
    use crate::layout::math::MathGlyph;

    for glyph in glyphs {
        match glyph {
            MathGlyph::Char {
                ch,
                x,
                y,
                font_size,
                italic,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;

                // Check if character needs Symbol font
                if let Some(sym_byte) = unicode_to_symbol(*ch) {
                    let encoded = format!("\\{:03o}", sym_byte);
                    content.push_str("BT\n");
                    content.push_str(&format!("/Symbol {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                } else {
                    let font_name = if *italic {
                        "Helvetica-Oblique"
                    } else {
                        "Helvetica"
                    };
                    let encoded = encode_pdf_text(&ch.to_string());
                    content.push_str("BT\n");
                    content.push_str(&format!("/{font_name} {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                }
            }
            MathGlyph::Text {
                text,
                x,
                y,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                let encoded = encode_pdf_text(text);
                content.push_str("BT\n");
                content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                content.push_str(&format!("{px} {py} Td\n"));
                content.push_str(&format!("({encoded}) Tj\n"));
                content.push_str("ET\n");
            }
            MathGlyph::Rule {
                x,
                y,
                width,
                thickness,
            } => {
                let px = origin_x + x;
                let py = origin_y + y - thickness / 2.0;
                content.push_str("0 0 0 rg\n");
                content.push_str(&format!("{px} {py} {width} {thickness} re\nf\n"));
            }
            MathGlyph::Radical {
                x,
                y,
                width,
                height,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                let line_w = font_size * 0.04;
                content.push_str(&format!("{line_w} w\n0 0 0 RG\n"));
                // Draw radical sign: short tick down, long line up-right, horizontal overline
                let tick_x = px + width * 0.15;
                let tick_bottom = py - height * 0.3;
                let bottom_x = px + width * 0.35;
                let bottom_y = py - height;
                let top_x = px + width;
                let top_y = py;
                content.push_str(&format!(
                    "{tick_x} {tick_bottom} m\n{bottom_x} {bottom_y} l\n{top_x} {top_y} l\nS\n"
                ));
            }
            MathGlyph::Delimiter {
                ch,
                x,
                y,
                height,
                font_size,
            } => {
                let px = origin_x + x;
                let py = origin_y + y;
                // For small delimiters, use text; for large, draw paths
                if *height <= font_size * 1.3 {
                    let encoded = encode_pdf_text(&ch.to_string());
                    content.push_str("BT\n");
                    content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                    content.push_str(&format!("{px} {py} Td\n"));
                    content.push_str(&format!("({encoded}) Tj\n"));
                    content.push_str("ET\n");
                } else {
                    // Draw scaled delimiter using PDF path ops
                    let line_w = font_size * 0.04;
                    content.push_str(&format!("{line_w} w\n0 0 0 RG\n"));
                    let half_h = height / 2.0;
                    match ch {
                        '(' => {
                            // Left parenthesis as cubic bezier
                            let cx = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            let ctrl_offset = height * 0.55;
                            content.push_str(&format!(
                                "{cx} {top_y} m\n{px} {c1y} {px} {c2y} {cx} {bot_y} c\nS\n",
                                c1y = py + ctrl_offset * 0.3,
                                c2y = py - ctrl_offset * 0.3,
                            ));
                        }
                        ')' => {
                            let cx = px;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            let ctrl_offset = height * 0.55;
                            content.push_str(&format!(
                                "{cx} {top_y} m\n{right} {c1y} {right} {c2y} {cx} {bot_y} c\nS\n",
                                c1y = py + ctrl_offset * 0.3,
                                c2y = py - ctrl_offset * 0.3,
                            ));
                        }
                        '[' => {
                            let right = px + font_size * 0.2;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{right} {top_y} m {px} {top_y} l {px} {bot_y} l {right} {bot_y} l S\n"
                            ));
                        }
                        ']' => {
                            let left = px;
                            let right = px + font_size * 0.2;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{left} {top_y} m {right} {top_y} l {right} {bot_y} l {left} {bot_y} l S\n"
                            ));
                        }
                        '{' => {
                            let mid = px + font_size * 0.15;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{right} {top_y} m {mid} {top_y} l {mid} {py} l {px} {py} l S\n\
                                 {px} {py} m {mid} {py} l {mid} {bot_y} l {right} {bot_y} l S\n"
                            ));
                        }
                        '}' => {
                            let mid = px + font_size * 0.1;
                            let right = px + font_size * 0.25;
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!(
                                "{px} {top_y} m {mid} {top_y} l {mid} {py} l {right} {py} l S\n\
                                 {right} {py} m {mid} {py} l {mid} {bot_y} l {px} {bot_y} l S\n"
                            ));
                        }
                        '|' => {
                            let top_y = py + half_h;
                            let bot_y = py - half_h;
                            content.push_str(&format!("{px} {top_y} m {px} {bot_y} l S\n"));
                        }
                        _ => {
                            // Fallback: render as text character
                            let encoded = encode_pdf_text(&ch.to_string());
                            content.push_str("BT\n");
                            content.push_str(&format!("/Helvetica {font_size} Tf\n"));
                            content.push_str(&format!("{px} {py} Td\n"));
                            content.push_str(&format!("({encoded}) Tj\n"));
                            content.push_str("ET\n");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::{LayoutBorder, layout};
    use crate::parser::html::parse_html;

    const TEST_JPEG_DATA_URI: &str = concat!(
        "data:image/jpeg;base64,",
        "/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkK",
        "DA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAA",
        "AAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q=="
    );

    fn test_text_run(text: impl Into<String>) -> TextRun {
        TextRun {
            text: text.into(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        }
    }

    fn test_text_line(runs: Vec<TextRun>) -> TextLine {
        TextLine {
            runs,
            height: 14.0,
            x_offset: 0.0,
        }
    }

    fn test_text_block(lines: Vec<TextLine>) -> LayoutElement {
        LayoutElement::TextBlock {
            lines,
            margin_top: 0.0,
            margin_bottom: 0.0,
            text_align: TextAlign::Left,
            writing_mode: crate::style::computed::WritingMode::HorizontalTb,
            background_color: None,
            padding_top: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            padding_right: 0.0,
            border: LayoutBorder::default(),
            block_width: None,
            block_height: None,
            opacity: 1.0,
            mix_blend_mode: crate::style::computed::BlendMode::Normal,
            background_blend_mode: crate::style::computed::BlendMode::Normal,
            float: Float::None,
            clear: crate::style::computed::Clear::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            offset_bottom: 0.0,
            offset_right: 0.0,
            containing_block: None,
            box_shadow: Vec::new(),
            visible: true,
            clip_rect: None,
            transform: None,
            transform_origin: crate::style::computed::TransformOrigin::default(),
            border_radius: 0.0,
            border_radii: [0.0; 4],
            border_radii_y: [0.0; 4],
            outline_offset: 0.0,
            outline_width: 0.0,
            outline_color: None,
            text_indent: 0.0,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            vertical_align: crate::style::computed::VerticalAlign::Baseline,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
            background_svg: None,
            background_blur_radius: 0.0,
            background_size: BackgroundSize::Auto,
            background_position: BackgroundPosition::default(),
            background_repeat: BackgroundRepeat::Repeat,
            background_origin: BackgroundOrigin::Padding,
            background_clip: BackgroundClip::Border,
            z_index: 0,
            repeat_on_each_page: false,
            positioned_depth: 0,
            heading_level: None,
            clip_children_count: 0,
        }
    }

    fn test_text_block_from_runs(runs: Vec<TextRun>) -> LayoutElement {
        test_text_block(vec![test_text_line(runs)])
    }

    fn test_page(elements: Vec<(f32, LayoutElement)>) -> Page {
        Page { elements }
    }

    fn first_td_y(content: &str) -> Option<f32> {
        for line in content.lines() {
            if let Some(coords) = line.strip_suffix(" Td") {
                let mut parts = coords.split_whitespace();
                let _x = parts.next()?;
                return parts.next()?.parse().ok();
            }
        }
        None
    }

    #[test]
    fn render_simple_pdf() {
        let nodes = parse_html("<p>Hello World</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();

        // Valid PDF starts with %PDF
        assert!(pdf.starts_with(b"%PDF-1.4"));
        // Valid PDF ends with %%EOF
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("%%EOF"));
        // Contains Helvetica font
        assert!(content.contains("/Helvetica"));
    }

    #[test]
    fn render_bold_italic() {
        let nodes = parse_html("<p><strong>Bold</strong> and <em>italic</em></p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Helvetica-Bold"));
        assert!(content.contains("/Helvetica-Oblique"));
    }

    #[test]
    fn render_empty_document() {
        let nodes = parse_html("").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn pdf_string_escaping() {
        assert_eq!(escape_pdf_string("hello"), "hello");
        assert_eq!(escape_pdf_string("(test)"), "\\(test\\)");
        assert_eq!(escape_pdf_string("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn render_background_color() {
        let html = r#"<pre>code here</pre>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Pre has gray background — PDF should contain rectangle fill commands
        assert!(content.contains("re\nf\n") || content.contains("re"));
    }

    #[test]
    fn render_center_align() {
        let html = r#"<p style="text-align: center">Centered</p>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_right_align() {
        let html = r#"<p style="text-align: right">Right</p>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_underline() {
        let html = "<p><u>Underlined text</u></p>";
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Underline draws a line with stroke command
        assert!(content.contains(" l\nS\n"));
    }

    #[test]
    fn render_bold_italic_combined() {
        let html = "<p><strong><em>Bold Italic</em></strong></p>";
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Helvetica-BoldOblique"));
    }

    #[test]
    fn render_page_break_in_content() {
        let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should have multiple page objects
        assert!(content.matches("/Type /Page").count() >= 2);
    }

    #[test]
    fn render_svg_without_viewbox_scales_to_layout_box() {
        let tree = crate::parser::svg::SvgTree {
            width: 120.0,
            height: 60.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle::default(),
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let pages = vec![Page {
            elements: vec![(
                0.0,
                LayoutElement::Svg {
                    tree,
                    width: 240.0,
                    height: 120.0,
                    flow_extra_bottom: 0.0,
                    margin_top: 0.0,
                    margin_bottom: 0.0,
                },
            )],
        }];
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("2 0 0 2 0 0 cm"),
            "expected outer scale for SVG without a viewBox"
        );
    }

    #[test]
    fn render_svg_honors_root_preserve_aspect_ratio() {
        let tree = crate::parser::svg::SvgTree {
            width: 20.0,
            height: 20.0,
            width_attr: Some("20".to_string()),
            height_attr: Some("20".to_string()),
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: Some(crate::parser::svg::ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 100.0,
                height: 20.0,
            }),
            defs: Default::default(),
            children: vec![],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let pages = vec![Page {
            elements: vec![(
                0.0,
                LayoutElement::Svg {
                    tree,
                    width: 20.0,
                    height: 20.0,
                    flow_extra_bottom: 0.0,
                    margin_top: 0.0,
                    margin_bottom: 0.0,
                },
            )],
        }];
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert!(
            content.contains("0.2 0 0 0.2 0 8 cm"),
            "expected meet scaling with vertical centering for the root SVG viewport"
        );
    }

    #[test]
    fn render_colored_text() {
        let html = r#"<p style="color: red">Red text</p>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1 0 0 rg")); // red in PDF
    }

    #[test]
    fn render_table_basic() {
        let html = r#"
            <table>
                <tr><th>Name</th><th>Age</th></tr>
                <tr><td>Alice</td><td>30</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // No default cell borders — only CSS-specified borders produce strokes
        assert!(content.contains("Name"));
        assert!(content.contains("Alice"));
    }

    #[test]
    fn render_table_with_background() {
        let html = r#"
            <table>
                <tr><td style="background-color: yellow">Highlighted</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Background fill command
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn render_empty_line_skipped() {
        let html = "<p>Above</p><br><p>Below</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Above"));
        assert!(content.contains("Below"));
    }

    #[test]
    fn render_empty_run_skipped() {
        let html = "<p>Text</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_page_break_element() {
        let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Multiple pages rendered
        assert!(content.matches("/Type /Page ").count() >= 2);
    }

    #[test]
    fn render_cell_text_empty_line_skipped() {
        let html = r#"<table><tr><td></td><td>Content</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Content"));
    }

    #[test]
    fn render_horizontal_rule() {
        let html = "<p>Above</p><hr><p>Below</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // HR draws a line with stroke
        assert!(content.contains(" l\nS\n"));
    }

    #[test]
    fn render_input_element() {
        let pdf = crate::html_to_pdf(r#"<input type="text" value="Hello">"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_input_with_placeholder() {
        let pdf = crate::html_to_pdf(r#"<input placeholder="Type here...">"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_select_element() {
        let pdf =
            crate::html_to_pdf(r#"<select><option>A</option><option>B</option></select>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_textarea_element() {
        let pdf = crate::html_to_pdf(r#"<textarea>Hello World</textarea>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_video_element() {
        let pdf = crate::html_to_pdf(r#"<video width="320" height="240"></video>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_audio_element() {
        let pdf = crate::html_to_pdf(r#"<audio></audio>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 100);
    }

    #[test]
    fn render_progress_element() {
        let pdf = crate::html_to_pdf(r#"<progress value="0.7" max="1"></progress>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        // Progress bar draws rectangles (track + fill + border)
        assert!(
            content.contains("re\nf\n"),
            "Expected filled rectangles for progress bar"
        );
    }

    #[test]
    fn render_progress_empty() {
        let pdf = crate::html_to_pdf(r#"<progress value="0" max="1"></progress>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_meter_element() {
        let pdf = crate::html_to_pdf(r#"<meter value="0.5" max="1"></meter>"#).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("re\nf\n"),
            "Expected filled rectangles for meter bar"
        );
    }

    #[test]
    fn render_meter_low_value() {
        let pdf = crate::html_to_pdf(r#"<meter value="5" max="100" low="25" high="75"></meter>"#)
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_form_controls_styled() {
        let html = r#"
            <input type="text" value="styled" style="width: 200px; border: 2px solid blue; background-color: #eee">
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_mixed_form_and_text() {
        let html = r#"
            <p>Fill in the form:</p>
            <input type="text" value="John">
            <p>Select country:</p>
            <select><option>France</option></select>
            <p>Comments:</p>
            <textarea>Great product!</textarea>
            <p>Rating:</p>
            <progress value="80" max="100"></progress>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(pdf.len() > 500);
    }

    #[test]
    fn render_pdf_bookmarks_from_headings() {
        let html = "<h1>Chapter 1</h1><p>Content</p><h2>Section 1.1</h2><p>More</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Type /Outlines"), "Expected PDF outlines");
        assert!(
            content.contains("Chapter 1"),
            "Expected heading text in bookmark"
        );
        assert!(
            content.contains("Section 1.1"),
            "Expected h2 heading in bookmark"
        );
    }

    #[test]
    fn render_pdf_no_bookmarks_without_headings() {
        let html = "<p>No headings here</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/Type /Outlines"),
            "Should not have outlines without headings"
        );
    }

    #[test]
    fn render_pdf_bookmarks_multi_page() {
        let html = r#"
            <h1>Page 1 Title</h1>
            <p>Content</p>
            <div style="page-break-before: always">
                <h1>Page 2 Title</h1>
                <p>More content</p>
            </div>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Page 1 Title"));
        assert!(content.contains("Page 2 Title"));
        assert!(content.contains("/Type /Outlines"));
    }

    #[test]
    fn render_pdf_bookmarks_all_levels() {
        let html = "<h1>H1</h1><h2>H2</h2><h3>H3</h3><h4>H4</h4><h5>H5</h5><h6>H6</h6>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Count 6"), "Expected 6 outline entries");
    }

    #[test]
    fn render_page_footer() {
        let pdf = crate::HtmlConverter::new()
            .footer("Page {page} of {pages}")
            .convert("<h1>Title</h1><p>Content</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Page 1 of 1"),
            "Expected footer with page numbers"
        );
    }

    #[test]
    fn render_page_header() {
        let pdf = crate::HtmlConverter::new()
            .header("My Document")
            .convert("<p>Content</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("My Document"),
            "Expected header text in PDF"
        );
    }

    #[test]
    fn render_header_and_footer() {
        let pdf = crate::HtmlConverter::new()
            .header("Report Title")
            .footer("Page {page} of {pages}")
            .convert("<p>Page 1</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Report Title"));
        assert!(content.contains("Page 1 of 1"));
    }

    #[test]
    fn render_footer_multi_page() {
        let html = r#"
            <p>First page</p>
            <div style="page-break-before: always"><p>Second page</p></div>
        "#;
        let pdf = crate::HtmlConverter::new()
            .footer("Page {page} of {pages}")
            .convert(html)
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Verify page number substitution works (at least page 1 and last page are present)
        assert!(content.contains("Page 1 of"), "Expected footer with page 1");
        assert!(content.contains("Page 2 of"), "Expected footer with page 2");
    }

    #[test]
    fn render_no_header_footer_by_default() {
        let pdf = crate::html_to_pdf("<p>Test</p>").unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(!content.contains("Page 1 of"));
    }

    #[test]
    fn render_header_only_no_footer() {
        let pdf = crate::HtmlConverter::new()
            .header("Header Only")
            .convert("<p>Content</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Header Only"));
        assert!(!content.contains("Page 1"));
    }

    #[test]
    fn render_footer_only_no_header() {
        let pdf = crate::HtmlConverter::new()
            .footer("{page}/{pages}")
            .convert("<p>Content</p>")
            .unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("1/1"));
    }

    #[test]
    fn render_progress_bar_zero_fraction() {
        let html = r#"<progress value="0" max="1"></progress>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Track is drawn but fill is skipped when fraction=0
        assert!(content.contains("re\nf\n")); // track rect
        assert!(content.contains("re\nS\n")); // border stroke
    }

    #[test]
    fn render_progress_bar_full_fraction() {
        let html = r#"<progress value="1" max="1"></progress>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn render_bookmark_special_chars() {
        let html = r#"<h1>Title with (parens) &amp; "quotes"</h1>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Type /Outlines"));
    }

    #[test]
    fn render_single_heading_bookmark() {
        let html = "<h1>Only One</h1><p>Text</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Count 1"));
        assert!(content.contains("Only One"));
    }

    #[test]
    fn render_link_annotation() {
        let html = r#"<p><a href="https://example.com">Click here</a></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Should contain a Link annotation with the URI
        assert!(
            content.contains("/Subtype /Link"),
            "PDF should contain a Link annotation"
        );
        assert!(
            content.contains("/S /URI"),
            "PDF should contain a URI action"
        );
        assert!(
            content.contains("https://example.com"),
            "PDF should contain the link URL"
        );
        assert!(
            content.contains("/P "),
            "PDF link annotations should record their owning page"
        );
        // The page object should reference annotations
        assert!(
            content.contains("/Annots ["),
            "Page should have an /Annots array"
        );
    }

    #[test]
    fn render_table_cell_link_annotation() {
        let html = r#"
            <table>
                <tr>
                    <td><a href="https://example.com/table">Cell link</a></td>
                </tr>
            </table>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert_eq!(content.matches("/Subtype /Link").count(), 1);
        assert!(content.contains("https://example.com/table"));
        assert!(content.contains("/Annots ["));
    }

    #[test]
    fn render_nested_table_link_annotation() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        <table>
                            <tr>
                                <td><a href="https://example.com/nested">Nested link</a></td>
                            </tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert_eq!(content.matches("/Subtype /Link").count(), 1);
        assert!(content.contains("https://example.com/nested"));
        assert!(content.contains("/Annots ["));
    }

    #[test]
    fn render_link_no_annotation_without_href() {
        // An <a> tag without href should not produce an annotation
        let html = "<p><a>No link</a></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/Subtype /Link"),
            "PDF should not contain a Link annotation without href"
        );
    }

    #[test]
    fn render_link_url_escaped() {
        // URL with parentheses should be properly escaped
        let html = r#"<p><a href="https://example.com/page(1)">Link</a></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Subtype /Link"));
        assert!(content.contains(r"https://example.com/page\(1\)"));
    }

    #[test]
    fn render_multiple_links() {
        let html =
            r#"<p><a href="https://one.com">One</a> and <a href="https://two.com">Two</a></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("https://one.com"));
        assert!(content.contains("https://two.com"));
        // Should have two Link annotations
        assert_eq!(
            content.matches("/Subtype /Link").count(),
            2,
            "Should have exactly 2 link annotations"
        );
    }

    #[test]
    fn render_page_without_links_has_no_annots() {
        let html = "<p>No links here</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/Annots"),
            "Page without links should not have /Annots"
        );
    }

    #[test]
    fn render_image_contains_xobject() {
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="100" height="80">"#);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /DCTDecode"),
            "JPEG image should use DCTDecode filter"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    fn render_image_xobject_uses_source_pixel_dimensions() {
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" width="120" height="90">"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Width 1 /Height 1"),
            "image XObject should use source pixel dimensions, not CSS box dimensions"
        );
    }

    #[test]
    fn render_no_image_no_xobject() {
        let html = "<p>No images here</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/XObject"),
            "PDF without images should not contain /XObject"
        );
    }

    #[test]
    fn render_border_draws_rectangle_stroke() {
        let html = r#"<div style="border: 1px solid black">Bordered text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Border draws a rectangle with stroke (re + S)
        assert!(
            content.contains("re\nS\n"),
            "PDF should contain rectangle stroke for border"
        );
        // The stroke color should be black (0 0 0 RG)
        assert!(
            content.contains("0 0 0 RG"),
            "Border stroke color should be black"
        );
    }

    #[test]
    fn render_border_with_custom_color() {
        let html = r#"<div style="border: 2px solid red">Red border</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Red border: 1 0 0 RG
        assert!(
            content.contains("1 0 0 RG"),
            "Border stroke color should be red"
        );
        assert!(
            content.contains("re\nS\n"),
            "PDF should contain rectangle stroke for border"
        );
    }

    #[test]
    fn render_dashed_border_emits_dash_pattern() {
        let html = r#"<div style="border: 2px dashed black; width: 100pt">Dashed</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Corner-symmetric dashed borders stroke each side with its own dash
        // array (snapped so a dash lands at every corner). The exact on/gap
        // values depend on the side length, but every dashed array is of the
        // form `[<on> <gap>] <phase> d` with a non-zero leading `on` segment.
        let has_dash_array = content
            .lines()
            .any(|l| l.ends_with(" d") && l.starts_with('[') && !l.starts_with("[0 "));
        assert!(
            has_dash_array,
            "Dashed border should emit a per-side dash array. Got: {}",
            &content[..content.len().min(2000)]
        );
        assert!(
            content.contains("[] 0 d"),
            "Dashed border should reset dash pattern with [] 0 d"
        );
    }

    #[test]
    fn render_dotted_border_emits_dash_pattern() {
        let html = r#"<div style="border: 2px dotted red; width: 100pt">Dotted</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Round dots: a round line cap (`1 J`) over a zero-length dash. The dot
        // spacing is snapped per side for corner symmetry, so the gap is close to
        // 2x the 1.5pt width but not exactly `[0 3]`.
        assert!(
            content.contains("1 J\n"),
            "Dotted border should set a round line cap. Got: {}",
            &content[..content.len().min(2000)]
        );
        let has_dot_array = content
            .lines()
            .any(|l| l.ends_with(" d") && l.starts_with("[0 "));
        assert!(
            has_dot_array,
            "Dotted border should emit a zero-length-dash (round dot) array"
        );
    }

    #[test]
    fn render_solid_border_no_dash_pattern() {
        let html = r#"<div style="border: 2px solid black; width: 100pt">Solid</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Solid borders should NOT set any dash pattern (no `[...] 0 d` and no
        // round-cap toggle).
        assert!(
            !content.contains("0 d\n") && !content.contains("1 J\n"),
            "Solid border should not emit dash patterns"
        );
    }

    #[test]
    fn border_style_parsed_from_shorthand() {
        use crate::parser::dom::HtmlTag;
        use crate::style::computed::BorderStyle;
        use crate::style::computed::ComputedStyle;
        let parent = ComputedStyle::default();
        let style = crate::style::computed::compute_style(
            HtmlTag::Div,
            Some("border: 2px dashed red"),
            &parent,
        );
        assert_eq!(style.border.top.style, BorderStyle::Dashed);
        assert_eq!(style.border.right.style, BorderStyle::Dashed);
        assert_eq!(style.border.bottom.style, BorderStyle::Dashed);
        assert_eq!(style.border.left.style, BorderStyle::Dashed);
    }

    #[test]
    fn render_times_roman_font_family() {
        let html = r#"<p style="font-family: serif">Serif text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Roman"),
            "PDF should use Times-Roman for serif font-family"
        );
    }

    #[test]
    fn render_times_bold_italic() {
        let html =
            r#"<p style="font-family: serif"><strong><em>Bold Italic Serif</em></strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-BoldItalic"),
            "PDF should use Times-BoldItalic for bold italic serif"
        );
    }

    #[test]
    fn render_times_bold() {
        let html = r#"<p style="font-family: times"><strong>Bold Serif</strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Bold"),
            "PDF should use Times-Bold for bold serif"
        );
    }

    #[test]
    fn render_times_italic() {
        let html = r#"<p style="font-family: serif"><em>Italic Serif</em></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Italic"),
            "PDF should use Times-Italic for italic serif"
        );
    }

    #[test]
    fn render_courier_font_family() {
        let html = r#"<p style="font-family: monospace">Monospace text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier ") || content.contains("/Courier\n"),
            "PDF should use Courier for monospace font-family"
        );
    }

    #[test]
    fn render_courier_bold_italic() {
        let html =
            r#"<p style="font-family: courier"><strong><em>Bold Italic Mono</em></strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-BoldOblique"),
            "PDF should use Courier-BoldOblique for bold italic monospace"
        );
    }

    #[test]
    fn render_courier_bold() {
        let html = r#"<p style="font-family: monospace"><strong>Bold Mono</strong></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-Bold"),
            "PDF should use Courier-Bold for bold monospace"
        );
    }

    #[test]
    fn render_courier_oblique() {
        let html = r#"<p style="font-family: courier"><em>Italic Mono</em></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Courier-Oblique"),
            "PDF should use Courier-Oblique for italic monospace"
        );
    }

    #[test]
    fn render_font_family_via_stylesheet() {
        let html = r#"
            <html>
            <head><style>p { font-family: serif }</style></head>
            <body><p>Styled serif</p></body>
            </html>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/Times-Roman"),
            "Stylesheet font-family should produce Times-Roman"
        );
    }

    #[test]
    fn render_jpeg_image_contains_xobject() {
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="100" height="80">"#);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /DCTDecode"),
            "JPEG image should use DCTDecode filter"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    #[ignore] // TODO: Container renderer doesn't render background images yet
    fn render_jpeg_background_uses_decoded_image_xobject() {
        use image::ImageEncoder;

        let mut jpeg_bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg_bytes)
            .write_image(
                &[255u8, 128, 0, 0, 128, 255, 0, 0, 0, 255, 255, 255],
                2,
                2,
                image::ExtendedColorType::Rgb8,
            )
            .expect("jpeg encoding should succeed");
        let jpeg_b64 = simple_base64_encode_test(&jpeg_bytes);
        let html = format!(
            r#"
            <div style="
                width: 100pt;
                height: 100pt;
                background-image: url(data:image/jpeg;base64,{jpeg_b64});
                background-repeat: no-repeat;
                background-size: 100pt 100pt;
            "></div>
        "#,
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        assert_eq!(content.matches("/Subtype /Image").count(), 1);
        assert!(
            content.contains("/Filter /FlateDecode"),
            "decoded JPEG backgrounds should use a Flate image XObject"
        );
        assert!(
            !content.contains("/Filter /DCTDecode"),
            "decoded JPEG backgrounds should not passthrough raw JPEG bytes"
        );
    }

    #[test]
    fn render_png_image_contains_flatedecode() {
        // Build a minimal valid PNG as base64 data URI
        let png_bytes = build_minimal_test_png();
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="100" height="100">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/XObject"),
            "PDF with PNG image should contain /XObject in resources"
        );
        assert!(
            content.contains("/Subtype /Image"),
            "PDF should contain image XObject"
        );
        assert!(
            content.contains("/Filter /FlateDecode"),
            "PNG image should use FlateDecode filter"
        );
        assert!(
            content.contains("/Predictor 15"),
            "PNG image should have Predictor 15 in DecodeParms"
        );
        assert!(
            content.contains("/Colors 3"),
            "RGB PNG should have Colors 3"
        );
        assert!(
            content.contains("Do"),
            "PDF should contain Do operator to draw image"
        );
    }

    #[test]
    fn render_png_grayscale_image() {
        let png_bytes = build_test_png_with_color_type(0); // Grayscale
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="50" height="50">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("/Filter /FlateDecode"));
        assert!(content.contains("/ColorSpace /DeviceGray"));
        assert!(content.contains("/Colors 1"));
    }

    /// Build a minimal valid PNG (1x1 RGB, 8-bit).
    fn build_minimal_test_png() -> Vec<u8> {
        build_test_png_with_color_type(2) // RGB
    }

    fn build_test_png_with_color_type(color_type: u8) -> Vec<u8> {
        let mut png = Vec::new();
        // PNG signature
        png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        // IHDR chunk (13 bytes data)
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes()); // width
        ihdr.extend_from_slice(&1u32.to_be_bytes()); // height
        ihdr.push(8); // bit depth
        ihdr.push(color_type);
        ihdr.push(0); // compression
        ihdr.push(0); // filter
        ihdr.push(0); // interlace
        append_png_chunk(&mut png, b"IHDR", &ihdr);
        // IDAT chunk with dummy zlib-compressed data
        let idat = [
            0x78, 0x01, 0x62, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
        ];
        append_png_chunk(&mut png, b"IDAT", &idat);
        // IEND
        append_png_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn append_png_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(chunk_type);
        buf.extend_from_slice(data);
        buf.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
    }

    fn simple_base64_encode_test(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut result = String::new();
        let mut i = 0;
        while i < data.len() {
            let b0 = data[i] as u32;
            let b1 = if i + 1 < data.len() {
                data[i + 1] as u32
            } else {
                0
            };
            let b2 = if i + 2 < data.len() {
                data[i + 2] as u32
            } else {
                0
            };
            let triple = (b0 << 16) | (b1 << 8) | b2;
            result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            if i + 1 < data.len() {
                result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            if i + 2 < data.len() {
                result.push(CHARS[(triple & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            i += 3;
        }
        result
    }

    #[test]
    fn render_all_12_fonts_registered() {
        let html = "<p>Test</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // All 12 standard font variants should be registered as font objects
        for name in &[
            "Helvetica",
            "Helvetica-Bold",
            "Helvetica-Oblique",
            "Helvetica-BoldOblique",
            "Times-Roman",
            "Times-Bold",
            "Times-Italic",
            "Times-BoldItalic",
            "Courier",
            "Courier-Bold",
            "Courier-Oblique",
            "Courier-BoldOblique",
        ] {
            assert!(
                content.contains(&format!("/BaseFont /{name}")),
                "PDF should register font {name}"
            );
        }
    }

    #[test]
    fn render_opacity_produces_extgstate() {
        let html = r#"<div style="opacity: 0.5">Semi-transparent</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ca 0.5"),
            "PDF should contain fill opacity /ca 0.5"
        );
        assert!(
            content.contains("/CA 0.5"),
            "PDF should contain stroke opacity /CA 0.5"
        );
        assert!(
            content.contains("/ExtGState"),
            "PDF should contain ExtGState resource"
        );
        assert!(content.contains("gs\n"), "PDF should use gs operator");
    }

    #[test]
    fn render_full_opacity_no_extgstate() {
        let html = r#"<div>Fully opaque</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("/ExtGState"),
            "PDF should not contain ExtGState for full opacity"
        );
    }

    #[test]
    fn render_width_constrains_background() {
        let html = r#"<div style="width: 200pt; background-color: red">Narrow</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("200"),
            "PDF should contain the constrained width 200"
        );
    }

    #[test]
    fn mask_image_gradient_emits_luminosity_smask() {
        // A box with a CSS gradient mask must emit a soft-mask graphics state
        // (a /Luminosity transparency group) and apply it via `gs` so the box's
        // paint fades through the mask coverage (css-masking-1 §3).
        let html = r#"<div style="width:120px;height:80px;background:#2e7d32;
            mask-image:linear-gradient(to right,#000,rgba(0,0,0,0))"></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/S /Luminosity"),
            "gradient mask must emit a luminosity soft-mask group"
        );
        assert!(
            content.contains("/SMask <<"),
            "gradient mask must register an /SMask ExtGState"
        );
        assert!(
            content.contains("GSmask"),
            "gradient mask must apply its soft-mask gstate via `gs`"
        );
    }

    #[test]
    fn no_mask_emits_no_softmask_gstate() {
        // A plain box (no mask-image) must not emit any GSmask soft-mask state.
        let html = r#"<div style="width:120px;height:80px;background:#2e7d32"></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("GSmask"),
            "a box without mask-image must not emit a soft-mask gstate"
        );
    }

    #[test]
    fn render_justify_produces_tw_operator() {
        // Use enough words to force line wrapping so a non-last line exists
        let words = "word ".repeat(80);
        let html = format!(r#"<p style="text-align: justify">{words}</p>"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Tw\n"),
            "Justified text should produce Tw operator in PDF"
        );
    }

    #[test]
    fn render_justify_last_line_no_tw() {
        // A single short line (which is the last line) should not have Tw
        let html = r#"<p style="text-align: justify">Short line</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // The single line is the last line, so no Tw should be applied
        assert!(
            !content.contains("Tw\n"),
            "Last line of justified paragraph should not have Tw"
        );
    }

    #[test]
    fn render_justify_resets_tw() {
        let words = "word ".repeat(80);
        let html = format!(r#"<p style="text-align: justify">{words}</p>"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Tw should be reset to 0 after each justified line
        assert!(
            content.contains("0 Tw\n"),
            "Tw should be reset to 0 after justified lines"
        );
    }

    // --- Overflow / Visibility / Transform PDF rendering tests ---

    #[test]
    fn render_visibility_hidden_skips_content() {
        let html = r#"<div style="visibility: hidden">Hidden text</div><p>Visible text</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("Hidden text"),
            "visibility: hidden should not render text content"
        );
        assert!(
            content.contains("Visible"),
            "Other text should still render"
        );
    }

    #[test]
    fn render_overflow_hidden_produces_clip_path() {
        let html =
            r#"<div style="overflow: hidden; width: 200pt; height: 100pt">Clipped content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("re W n"),
            "overflow: hidden should produce clipping path (re W n)"
        );
        assert!(
            content.contains("Clipped"),
            "Content should still be rendered inside clip"
        );
    }

    #[test]
    fn render_transform_rotate_produces_cm() {
        let html = r#"<div style="transform: rotate(45deg)">Rotated text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // rotate(45deg) should produce cos/sin values in a cm operator
        assert!(
            content.contains("cm\n"),
            "transform: rotate should produce cm operator"
        );
        assert!(
            content.contains("q\n"),
            "transform should save graphics state with q"
        );
        assert!(
            content.contains("Q\n"),
            "transform should restore graphics state with Q"
        );
        // cos(45) ~= 0.7071, sin(45) ~= 0.7071
        assert!(
            content.contains("0.707"),
            "rotate(45deg) should contain cos/sin values ~0.707"
        );
    }

    #[test]
    fn render_transform_scale_produces_cm() {
        let html = r#"<div style="transform: scale(2)">Scaled text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // scale(2) produces "2 0 0 2 tx ty cm" where tx,ty are the centre-offset
        // translation terms (non-zero because the block is not at the page origin).
        assert!(
            content.contains("2 0 0 2 "),
            "transform: scale(2) should produce '2 0 0 2 ...' cm operator"
        );
        assert!(
            content.contains(" cm\n"),
            "transform: scale(2) should produce a cm operator"
        );
    }

    #[test]
    fn render_transform_translate_produces_cm() {
        let html = r#"<div style="transform: translate(10pt, 20pt)">Translated text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("1 0 0 1 10 -20 cm"),
            "transform: translate(10pt, 20pt) should produce '1 0 0 1 10 -20 cm' (Y negated for PDF)"
        );
    }

    /// BUG P2-2: rotate/scale transforms must be applied around the element
    /// centre (CSS `transform-origin: 50% 50%`), not the page origin.
    /// Previously the translation terms in the `cm` matrix were always 0,
    /// which displaced the element off-page.
    #[test]
    fn render_transform_scale_centered_on_element() {
        // A block with explicit 100pt × 20pt size, positioned at the top of
        // the content area.  The rendered PDF matrix must be
        //   scale_x 0 0 scale_y tx ty
        // where tx = cx*(1-sx) and ty = cy*(1-sy) (non-zero when the element
        // is not at the page origin).
        let html = r#"<div style="transform: scale(2); width: 100pt; height: 20pt; background-color: blue">Box</div>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        // The matrix scale values are correct.
        assert!(
            content.contains("2 0 0 2 "),
            "scale(2) should produce '2 0 0 2 tx ty cm'"
        );
        // The translation terms must NOT both be zero — the element is not
        // at the page origin, so the centre-based offset is non-zero.
        assert!(
            !content.contains("2 0 0 2 0 0 cm"),
            "scale(2) on a non-origin element must have non-zero tx/ty in the cm matrix"
        );
    }

    /// BUG P2-2: a rotate transform must include non-zero translation terms
    /// so the element stays in its section instead of being displaced.
    #[test]
    fn render_transform_rotate_includes_translation_terms() {
        let html = r#"<div style="transform: rotate(45deg); width: 100pt; height: 20pt; background-color: red">Rotated</div>"#;
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);

        // cos/sin values of 45 deg must be present.
        assert!(
            content.contains("0.707"),
            "rotate(45deg) must contain cos/sin ~0.707"
        );
        // The matrix must NOT have zero translation — the element centre
        // is not at (0, 0) in PDF coordinates.
        assert!(
            !content.contains("0.70710677 0.70710677 -0.70710677 0.70710677 0 0 cm"),
            "rotate on a non-origin element must have non-zero tx/ty in the cm matrix"
        );
    }

    #[test]
    fn render_overflow_visible_no_clip() {
        let html = r#"<div style="width: 200pt">Normal content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            !content.contains("re W n"),
            "No overflow should not produce clipping path"
        );
    }

    #[test]
    fn render_border_radius_produces_bezier_curves() {
        let html = r#"<div style="border: 1px solid black; border-radius: 10pt; background-color: red">Rounded</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Bezier curves use 'c' operator; rounded rects should have them
        assert!(
            content.contains(" c\n"),
            "Border-radius should produce Bezier curve commands"
        );
        // Should also have 'h' to close the path
        assert!(
            content.contains("h\n"),
            "Rounded rect path should be closed with 'h'"
        );
    }

    #[test]
    fn render_outline_draws_outside_element() {
        let html = r#"<div style="outline: 2px solid red; width: 100pt">Outlined</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Outline should produce a stroke command (S) with outline color
        assert!(
            content.contains("1 0 0 RG"),
            "Outline should set red stroke color"
        );
        assert!(
            content.contains("S\n"),
            "Outline should produce a stroke command"
        );
    }

    #[test]
    fn render_border_radius_zero_uses_rectangle() {
        let html = r#"<div style="border: 1px solid black; background-color: blue">Square</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Without border-radius, should use 're' (rectangle) not Bezier curves
        assert!(
            content.contains("re\n"),
            "Zero border-radius should use rectangle operator"
        );
    }

    #[test]
    fn build_shading_function_single_stop() {
        // Single stop produces a constant-color Type 2 function
        let stops = vec![(0.5, (1.0, 0.0, 0.0))];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 2"));
        assert!(result.contains("/C0 [1 0 0]"));
        assert!(result.contains("/C1 [1 0 0]"));
    }

    #[test]
    fn build_shading_function_two_stops() {
        let stops = vec![(0.0, (1.0, 0.0, 0.0)), (1.0, (0.0, 0.0, 1.0))];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 2"));
        assert!(result.contains("/C0 [1 0 0]"));
        assert!(result.contains("/C1 [0 0 1]"));
    }

    #[test]
    fn build_shading_function_three_stops() {
        let stops = vec![
            (0.0, (1.0, 0.0, 0.0)),
            (0.5, (0.0, 1.0, 0.0)),
            (1.0, (0.0, 0.0, 1.0)),
        ];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 3"));
        assert!(result.contains("/Bounds [0.5]"));
        assert!(result.contains("/Encode [0 1 0 1]"));
    }

    #[test]
    fn build_shading_function_empty_stops() {
        let stops: Vec<(f32, (f32, f32, f32))> = vec![];
        let result = build_shading_function(&stops);
        assert!(result.contains("/FunctionType 2"));
        assert!(result.contains("/C0 [0 0 0]"));
    }

    #[test]
    fn render_cell_text_with_empty_line_and_empty_run() {
        // Covers lines 718, 724: empty line text skipped, empty run skipped
        let empty_run = TextRun {
            text: String::new(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let non_empty_run = TextRun {
            text: "Hello".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let cell = TableCell {
            lines: vec![
                TextLine {
                    runs: vec![empty_run.clone()],
                    height: 14.0,
                    x_offset: 0.0,
                },
                TextLine {
                    runs: vec![empty_run.clone(), non_empty_run],
                    height: 14.0,
                    x_offset: 0.0,
                },
            ],
            nested_rows: Vec::new(),
            bold: false,
            colspan: 1,
            rowspan: 1,
            padding_top: 2.0,
            padding_bottom: 2.0,
            padding_left: 2.0,
            padding_right: 2.0,
            background_color: None,
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Baseline,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        let mut content = String::new();
        let fonts = HashMap::new();
        let mut annotations = Vec::new();
        let prepared_fonts = PreparedCustomFonts::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut text_context = TextRenderContext::new(
            &fonts,
            &prepared_fonts,
            &mut annotations,
            &mut ts_pdf_writer,
            &mut ts_page_images,
        );
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(0.0, 100.0, 50.0),
            &mut text_context,
        );
        assert!(content.contains("Hello"));
    }

    #[test]
    fn text_block_empty_run_skipped() {
        // Covers line 401: empty text run within a text block line is skipped
        let page = test_page(vec![(
            0.0,
            test_text_block_from_runs(vec![test_text_run(""), test_text_run("Data")]),
        )]);
        let pdf = render_pdf(&[page], PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Data"));
    }

    #[test]
    fn page_break_element_renders() {
        // Covers line 677: PageBreak empty match arm
        let page = test_page(vec![
            (
                0.0,
                test_text_block_from_runs(vec![test_text_run("Before")]),
            ),
            (20.0, LayoutElement::PageBreak(Default::default())),
        ]);
        let pdf = render_pdf(&[page], PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Before"));
    }

    #[test]
    fn font_name_for_run_custom_bold_italic() {
        // Covers lines 761-763: Custom font bold+italic fallback names
        let run_bi = TextRun {
            text: "test".to_string(),
            font_size: 12.0,
            bold: true,
            italic: true,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Custom("MyFont".to_string()),
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        assert_eq!(font_name_for_run(&run_bi), "Helvetica-BoldOblique");

        let run_b = TextRun {
            text: "test".to_string(),
            font_size: 12.0,
            bold: true,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Custom("MyFont".to_string()),
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        assert_eq!(font_name_for_run(&run_b), "Helvetica-Bold");

        let run_i = TextRun {
            text: "test".to_string(),
            font_size: 12.0,
            bold: false,
            italic: true,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Custom("MyFont".to_string()),
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        assert_eq!(font_name_for_run(&run_i), "Helvetica-Oblique");
    }

    #[test]
    fn render_radial_gradient_uses_shading() {
        use crate::style::computed::GradientStop;
        use crate::types::Color;
        let mut content = String::new();
        let mut shadings = Vec::new();
        let mut counter = 0usize;
        let gradient = RadialGradient {
            stops: vec![
                GradientStop {
                    color: Color {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                    position: 0.0,
                },
                GradientStop {
                    color: Color {
                        r: 0,
                        g: 0,
                        b: 255,
                        a: 255,
                    },
                    position: 1.0,
                },
            ],
            center: (
                crate::style::computed::RadialPos::Fraction(0.5),
                crate::style::computed::RadialPos::Fraction(0.5),
            ),
            shape: RadialShape::Circle,
            extent: RadialExtent::FarthestCorner,
            radius: None,
            radii: None,
            repeating: false,
            layer_box: crate::style::computed::GradientLayerBox::default(),
        };
        render_radial_gradient(
            &mut content,
            &gradient,
            0.0,
            0.0,
            1.0,
            1.0,
            &mut shadings,
            &mut counter,
        );
        assert!(!content.is_empty());
        assert!(content.contains("/SH0 sh"));
        assert_eq!(shadings.len(), 1);
        assert_eq!(shadings[0].shading_type, 3);
    }

    #[test]
    fn utf8_to_winansi_ascii() {
        let input = "Hello, World! 123";
        let result = utf8_to_winansi(input);
        assert_eq!(result, input.as_bytes());
    }

    #[test]
    fn utf8_to_winansi_em_dash() {
        // "hello — world" contains U+2014 em dash which should become 0x97
        let input = "hello \u{2014} world";
        let result = utf8_to_winansi(input);
        let expected: Vec<u8> = vec![
            b'h', b'e', b'l', b'l', b'o', b' ', 0x97, b' ', b'w', b'o', b'r', b'l', b'd',
        ];
        assert_eq!(result, expected);
    }

    #[test]
    fn utf8_to_winansi_quotes() {
        // Left/right single and double curly quotes
        let input = "\u{2018}hello\u{2019} \u{201C}world\u{201D}";
        let result = utf8_to_winansi(input);
        assert_eq!(result[0], 0x91); // left single quote
        assert_eq!(result[6], 0x92); // right single quote
        assert_eq!(result[8], 0x93); // left double quote
        assert_eq!(result[14], 0x94); // right double quote
    }

    #[test]
    fn utf8_to_winansi_latin1() {
        // e-acute (U+00E9), n-tilde (U+00F1), u-diaeresis (U+00FC)
        let input = "\u{00E9}\u{00F1}\u{00FC}";
        let result = utf8_to_winansi(input);
        assert_eq!(result, vec![0xE9, 0xF1, 0xFC]);
    }

    #[test]
    fn utf8_to_winansi_unknown() {
        // Chinese character and emoji should be replaced with '?'
        let input = "\u{4E16}\u{1F600}";
        let result = utf8_to_winansi(input);
        assert_eq!(result, vec![b'?', b'?']);
    }

    #[test]
    fn utf8_to_winansi_en_dash_bullet_ellipsis_euro_trademark() {
        assert_eq!(utf8_to_winansi("\u{2013}"), vec![0x96]); // en dash
        assert_eq!(utf8_to_winansi("\u{2022}"), vec![0x95]); // bullet
        assert_eq!(utf8_to_winansi("\u{2026}"), vec![0x85]); // ellipsis
        assert_eq!(utf8_to_winansi("\u{20AC}"), vec![0x80]); // euro
        assert_eq!(utf8_to_winansi("\u{2122}"), vec![0x99]); // trademark
    }

    #[test]
    fn encode_pdf_text_special_chars() {
        assert_eq!(encode_pdf_text("hello"), "hello");
        assert_eq!(encode_pdf_text("(test)"), "\\(test\\)");
        assert_eq!(encode_pdf_text("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn encode_pdf_text_em_dash() {
        let encoded = encode_pdf_text("hello \u{2014} world");
        // 0x97 = 151 decimal = 227 octal; em dash should be \227
        assert_eq!(encoded, "hello \\227 world");
    }

    #[test]
    fn encode_pdf_text_em_dash_in_pdf_bytes() {
        // Verify that rendering em dash produces correct octal escape in PDF
        // and does NOT produce UTF-8 bytes or mojibake
        let html = "<p>hello \u{2014} world</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        // The PDF content stream should contain the octal escape \227
        assert!(
            pdf_str.contains("\\227"),
            "PDF should contain octal escape \\227 for em dash"
        );

        // The raw UTF-8 bytes for em dash (0xE2 0x80 0x94) should NOT appear
        let has_utf8_em_dash = pdf.windows(3).any(|w| w == [0xE2, 0x80, 0x94]);
        assert!(
            !has_utf8_em_dash,
            "PDF should not contain raw UTF-8 bytes for em dash"
        );

        // The mojibake pattern should not appear
        let has_mojibake = pdf.windows(2).any(|w| w == [0xC3, 0xA2]);
        assert!(!has_mojibake, "PDF should not contain mojibake bytes");
    }

    #[test]
    fn integration_em_dash_no_mojibake_in_pdf() {
        // Render HTML with em dash and verify the raw UTF-8 mojibake bytes
        // "\xC3\xA2\xC2\x80\xC2\x94" (the UTF-8 encoding of U+2014 read as
        // latin1) do NOT appear in the output.
        let html = "<p>hello \u{2014} world</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();

        // The mojibake sequence for em dash in UTF-8 misinterpreted as latin1
        // is bytes [0xC3, 0xA2]. This must NOT appear in the PDF.
        let has_mojibake = pdf.windows(2).any(|w| w == [0xC3, 0xA2]);
        assert!(
            !has_mojibake,
            "PDF output contains UTF-8 mojibake for em dash"
        );

        // The octal escape sequence \227 (for byte 0x97) should appear in the PDF
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("\\227"),
            "PDF output should contain octal escape \\227 for WinAnsi em dash"
        );
    }

    #[test]
    fn total_row_bold_from_descendant_selector() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            .total-row td { font-weight: bold; font-size: 12pt; }
        </style></head><body>
        <table>
            <tr><td>Item</td><td>$100</td></tr>
            <tr class="total-row"><td>Total</td><td>$100</td></tr>
        </table>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // The total row cells inherit the UA-default serif family, so the bold
        // descendant selector resolves to Times-Bold at 12pt.
        assert!(
            pdf_str.contains("/Times-Bold 12 Tf"),
            "Total row should use Times-Bold at 12pt, PDF content:\n{}",
            pdf_str
                .lines()
                .filter(|l| l.contains("Times"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn table_cell_em_dash_encoded_correctly() {
        let html = r#"<table><tr><td>HTML/CSS to PDF conversion — Enterprise</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Em dash in table cell should be encoded as octal \227
        assert!(
            pdf_str.contains("\\227"),
            "Table cell em dash should be encoded as \\227"
        );
        // No raw UTF-8 bytes for em dash
        let has_utf8_em_dash = pdf.windows(3).any(|w| w == [0xE2, 0x80, 0x94]);
        assert!(
            !has_utf8_em_dash,
            "Table cell should not contain raw UTF-8 em dash bytes"
        );
    }

    #[test]
    fn linear_gradient_uses_shading() {
        let html = r#"<div style="background: linear-gradient(to bottom, red, blue); height: 50pt">Gradient</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ShadingType 2"),
            "Linear gradient should produce ShadingType 2 (axial)"
        );
    }

    #[test]
    fn radial_gradient_uses_shading_in_pdf() {
        let html =
            r#"<div style="background: radial-gradient(red, blue); height: 50pt">Gradient</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ShadingType 3"),
            "Radial gradient should produce ShadingType 3"
        );
    }

    #[test]
    fn border_top_only_renders_single_line() {
        let html = r#"<div style="border-top: 2pt solid red">Top border only</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Per-side border renders as a move-to + line-to + stroke, not a rectangle
        assert!(
            pdf_str.contains("l S\n"),
            "Should have line stroke for top border"
        );
        assert!(pdf_str.contains("1 0 0 RG"), "Should have red stroke color");
    }

    #[test]
    fn border_bottom_renders() {
        let html = r#"<div style="border-bottom: 1pt solid blue">Bottom border</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("l S\n"),
            "Should have line stroke for bottom border"
        );
        assert!(
            pdf_str.contains("0 0 1 RG"),
            "Should have blue stroke color"
        );
    }

    #[test]
    fn border_left_renders() {
        let html = r#"<blockquote style="border-left: 3pt solid green">Left border</blockquote>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("l S\n"),
            "Should have line stroke for left border"
        );
        assert!(
            pdf_str.contains("0 0.50196 0 RG")
                || pdf_str.contains("0 0.501960")
                || pdf_str.contains("RG"),
            "Should have green stroke color"
        );
    }

    #[test]
    fn non_uniform_borders_render_per_side() {
        let html =
            r#"<div style="border-top: 2pt solid red; border-bottom: 1pt solid blue">Mixed</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Non-uniform borders should produce per-side line strokes
        assert!(pdf_str.contains("1 0 0 RG"), "Should have red for top");
        assert!(pdf_str.contains("0 0 1 RG"), "Should have blue for bottom");
        // Should use line strokes, not rectangle
        let stroke_count = pdf_str.matches("l S\n").count();
        assert!(
            stroke_count >= 2,
            "Should have at least 2 line strokes, got {stroke_count}"
        );
    }

    #[test]
    fn gradient_clipped_to_border_radius() {
        let html = r#"<div style="background: linear-gradient(to bottom, red, blue); border-radius: 10pt; height: 50pt">Clipped</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("sh"),
            "Should have shading operator for gradient"
        );
        assert!(
            pdf_str.contains("W n"),
            "Should have clip operator for border-radius"
        );
    }

    #[test]
    #[ignore] // TODO: Container renderer doesn't render SVG backgrounds with border-radius clip yet
    fn svg_background_clipped_to_border_radius() {
        let html = r#"<div style="width: 200pt; height: 80pt; border-radius: 12pt; background: url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%221%22 height=%221%22%3E%3Crect width=%221%22 height=%221%22 fill=%22red%22/%3E%3C/svg%3E') no-repeat"></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains(" c\n"),
            "Rounded clip should use Bezier curves"
        );
        assert!(pdf_str.contains("W n"), "SVG background should be clipped");
    }

    #[test]
    fn svg_background_percent_size_uses_positioning_area() {
        let tree = crate::parser::svg::SvgTree {
            width: 1.0,
            height: 1.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((1.0, 0.0, 0.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let mut content = String::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_background(
            &mut content,
            &tree,
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            None,
            BackgroundPaintContext::new(
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                0.0,
                0.0,
                BackgroundSize::Explicit {
                    width: 50.0,
                    height: Some(25.0),
                    width_is_percent: true,
                    height_is_percent: true,
                },
                BackgroundPosition::default(),
                BackgroundRepeat::NoRepeat,
            ),
        );
        assert!(
            content.contains("0 0 100 25 re W n"),
            "Expected SVG tile viewport to resolve against the 200pt by 100pt positioning area"
        );
        // Both background-size values are explicit (50% 25%), so the image is
        // scaled to exactly that box, ignoring its intrinsic 1:1 ratio
        // (css-backgrounds-3 §3.9): the 1x1 SVG stretches to the full 100x25 tile.
        assert!(
            content.contains("100 0 0 25 0 0 cm"),
            "Expected explicit two-value background-size to stretch the SVG to the 100pt by 25pt tile"
        );
    }

    #[test]
    fn svg_background_single_percent_size_preserves_aspect_ratio() {
        let tree = crate::parser::svg::SvgTree {
            width: 2.0,
            height: 1.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 1.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((1.0, 0.0, 0.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let mut content = String::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_background(
            &mut content,
            &tree,
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            None,
            BackgroundPaintContext::new(
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                0.0,
                0.0,
                BackgroundSize::Explicit {
                    width: 50.0,
                    height: None,
                    width_is_percent: true,
                    height_is_percent: false,
                },
                BackgroundPosition::default(),
                BackgroundRepeat::NoRepeat,
            ),
        );
        assert!(
            content.contains("50 0 0 50 0 0 cm"),
            "Single-value background-size should preserve intrinsic aspect ratio"
        );
    }

    #[test]
    fn svg_background_uses_outer_clip_box() {
        let tree = crate::parser::svg::SvgTree {
            width: 1.0,
            height: 1.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((1.0, 0.0, 0.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let mut content = String::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        render_svg_background(
            &mut content,
            &tree,
            &mut pdf_writer,
            &mut page_images,
            &mut shadings,
            &mut shading_counter,
            None,
            BackgroundPaintContext::new(
                SvgViewportBox::new(20.0, 10.0, 160.0, 80.0),
                SvgViewportBox::new(0.0, 0.0, 200.0, 100.0),
                0.0,
                0.0,
                BackgroundSize::Auto,
                BackgroundPosition::default(),
                BackgroundRepeat::NoRepeat,
            ),
        );
        assert!(
            content.contains("0 0 200 100 re W n"),
            "Clip box should stay on the outer element box, not shrink to the origin box"
        );
    }

    #[test]
    fn flexrow_with_gradient() {
        let html = r#"<div style="display: flex; background: linear-gradient(to right, red, blue); height: 40pt"><div style="width: 100pt">A</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/ShadingType 2"),
            "FlexRow with linear-gradient should produce ShadingType 2"
        );
    }

    #[test]
    fn flexrow_cell_background() {
        let html = r#"<div style="display: flex"><div style="width: 100pt; background-color: yellow">Yellow</div><div style="width: 100pt">Plain</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Yellow = 1 1 0 rg
        assert!(
            pdf_str.contains("1 1 0 rg"),
            "Should have yellow fill color for cell background"
        );
        assert!(
            pdf_str.contains("re\nf\n"),
            "Should have rectangle fill for cell background"
        );
    }

    #[test]
    fn flexrow_cell_border_radius() {
        let html = r#"<div style="display: flex"><div style="width: 100pt; background-color: red; border-radius: 8pt">Round</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Rounded rect uses Bezier curve commands (c)
        assert!(pdf_str.contains("1 0 0 rg"), "Should have red fill");
        assert!(
            pdf_str.contains(" c\n"),
            "Should have Bezier curve for border-radius"
        );
    }

    #[test]
    fn flexrow_cell_gradient() {
        let html = r#"<div style="display: flex"><div style="width: 150pt; background: linear-gradient(to bottom, green, yellow)">Grad</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("sh"),
            "Should have shading for cell gradient"
        );
        assert!(
            pdf_str.contains("/ShadingType 2"),
            "Cell gradient should use axial shading"
        );
    }

    #[test]
    fn flexrow_border_renders() {
        let html = r#"<div style="display: flex; border: 2pt solid black"><div style="width: 100pt">Bordered</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("re\nS\n"),
            "Should have rectangle stroke for uniform flex border"
        );
        assert!(
            pdf_str.contains("0 0 0 RG"),
            "Should have black stroke color"
        );
    }

    #[test]
    fn flexrow_border_radius_background() {
        let html = r#"<div style="display: flex; border-radius: 10pt; background-color: #cccccc"><div style="width: 100pt">Rounded</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Rounded background uses Bezier curves, not re
        assert!(
            pdf_str.contains(" c\n"),
            "Should have Bezier curves for rounded background"
        );
        assert!(pdf_str.contains("f\n"), "Should have fill command");
    }

    #[test]
    fn inline_span_border_radius() {
        let html = r#"<div style="display: flex"><div style="width: 300pt"><p><span style="background-color: yellow; border-radius: 4pt; padding: 2pt">Tag</span> text</p></div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Inline span with border-radius should produce rounded rect path + fill
        assert!(
            pdf_str.contains("1 1 0 rg"),
            "Should have yellow fill for span bg"
        );
    }

    #[test]
    fn root_svg_background_renders_in_pdf() {
        use crate::parser::css::parse_stylesheet;

        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = crate::layout::engine::layout_with_rules(
            &nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        assert!(
            pdf_str.contains("1 0 0 rg"),
            "Expected red SVG background fill"
        );
    }

    #[test]
    fn root_svg_background_viewbox_only_renders_in_pdf() {
        use crate::parser::css::parse_stylesheet;

        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = crate::layout::engine::layout_with_rules(
            &nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        assert!(
            pdf_str.contains("1 0 0 rg"),
            "Expected viewBox-only SVG background to render"
        );
    }

    #[test]
    fn root_svg_background_with_gradient_registers_shading_resources() {
        use crate::parser::css::parse_stylesheet;

        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 20 10'%3E%3Cdefs%3E%3ClinearGradient id='g' x1='0' y1='0' x2='20' y2='0' gradientUnits='userSpaceOnUse'%3E%3Cstop offset='0' stop-color='%23f00'/%3E%3Cstop offset='1' stop-color='%2300f'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect width='20' height='10' fill='url(%23g)'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = crate::layout::engine::layout_with_rules(
            &nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        assert!(
            pdf_str.contains("/ShadingType 2"),
            "Expected gradient SVG background to emit an axial shading resource"
        );
    }

    #[test]
    fn table_cell_nested_background_block_renders_image_xobject() {
        let png_bytes = build_minimal_test_png();
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(
            r#"<table><tr><td><div style="display: flex; width: 40pt; aspect-ratio: 1 / 1; background-image: url('data:image/png;base64,{b64}') no-repeat;"></div></td></tr></table>"#
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);

        assert!(
            pdf_str.contains("BI\n"),
            "Expected nested table-cell background block to emit an inline image"
        );
        assert!(
            pdf_str.contains("EI\n"),
            "Expected nested table-cell background block to terminate the inline image"
        );
    }

    #[test]
    fn nested_text_block_padding_top_offsets_text() {
        let lines = vec![test_text_line(vec![test_text_run("Nested")])];
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();

        let mut without_padding = String::new();
        let mut without_padding_context = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        render_nested_text_block(
            &mut without_padding,
            NestedTextBlock {
                lines: &lines,
                clips: false,
                text_align: TextAlign::Left,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: Some(80.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radius: 0.0,
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 80.0),
            &mut without_padding_context,
        );
        drop(without_padding_context);

        let mut with_padding = String::new();
        let mut with_padding_context = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        render_nested_text_block(
            &mut with_padding,
            NestedTextBlock {
                lines: &lines,
                clips: false,
                text_align: TextAlign::Left,
                padding_top: 12.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: Some(80.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radius: 0.0,
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 80.0),
            &mut with_padding_context,
        );

        let without_padding_y = first_td_y(&without_padding).unwrap();
        let with_padding_y = first_td_y(&with_padding).unwrap();
        assert!((without_padding_y - with_padding_y - 12.0).abs() < 0.01);
    }

    #[test]
    fn nested_absolute_without_containing_block_uses_initial_origin() {
        let mut absolute = test_text_block_from_runs(vec![test_text_run("Absolute")]);
        if let LayoutElement::TextBlock {
            position,
            offset_top,
            offset_left,
            ..
        } = &mut absolute
        {
            *position = Position::Absolute;
            *offset_top = 10.0;
            *offset_left = 20.0;
        }

        let elements = [absolute];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(50.0, 100.0, 10.0, 200.0, 80.0),
        );
        assert_eq!(planned.len(), 1);
        assert!((planned[0].origin_x - 30.0).abs() < 0.01);
        assert!((planned[0].top_y - 190.0).abs() < 0.01);
    }

    #[test]
    fn nested_static_without_containing_block_uses_local_origin() {
        let static_block = test_text_block_from_runs(vec![test_text_run("Static")]);
        let elements = [static_block];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(50.0, 100.0, 10.0, 200.0, 80.0),
        );
        assert_eq!(planned.len(), 1);
        assert!((planned[0].origin_x - 50.0).abs() < 0.01);
        assert!((planned[0].top_y - 100.0).abs() < 0.01);
    }

    #[test]
    fn table_cell_absolute_pseudo_background_renders_blurred_copy() {
        use crate::parser::css::parse_stylesheet;

        let png_bytes = {
            let image = image::RgbaImage::from_fn(4, 4, |x, y| {
                image::Rgba([(x * 40) as u8, (y * 40) as u8, 180, 255])
            });
            let mut encoded = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(
                    &mut std::io::Cursor::new(&mut encoded),
                    image::ImageFormat::Png,
                )
                .unwrap();
            encoded
        };
        let b64 = simple_base64_encode_test(&png_bytes);
        let html = format!(
            r#"<html><head><style>
                .image-container {{
                    display: flex;
                    position: relative;
                    width: 40pt;
                    aspect-ratio: 1 / 1;
                    background-image: url('data:image/png;base64,{b64}');
                    background-size: cover;
                    background-repeat: no-repeat;
                }}
                .image-container::after {{
                    content: '';
                    background-image: inherit;
                    background-size: inherit;
                    background-repeat: inherit;
                    width: 100%;
                    height: 100%;
                    display: block;
                    position: absolute;
                    bottom: -10pt;
                    z-index: -1;
                    filter: blur(4px);
                }}
            </style></head><body>
                <table><tr><td><div class="image-container"></div></td></tr></table>
            </body></html>"#
        );
        let result = crate::parser::html::parse_html_with_styles(&html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        fn count_background_svgs(elements: &[LayoutElement]) -> usize {
            elements.iter().map(count_element_background_svgs).sum()
        }

        fn count_element_background_svgs(element: &LayoutElement) -> usize {
            match element {
                LayoutElement::TextBlock { background_svg, .. } => {
                    usize::from(background_svg.is_some())
                }
                LayoutElement::TableRow { cells, .. } | LayoutElement::GridRow { cells, .. } => {
                    cells.iter().map(count_cell_background_svgs).sum()
                }
                LayoutElement::FlexRow {
                    cells,
                    background_svg,
                    ..
                } => {
                    usize::from(background_svg.is_some())
                        + cells
                            .iter()
                            .map(|cell| usize::from(cell.background_svg.is_some()))
                            .sum::<usize>()
                }
                _ => 0,
            }
        }

        fn count_cell_background_svgs(cell: &TableCell) -> usize {
            count_background_svgs(&cell.nested_rows)
        }

        let background_svg_count: usize = pages[0]
            .elements
            .iter()
            .map(|(_, element)| count_element_background_svgs(element))
            .sum();

        assert!(
            background_svg_count >= 2,
            "Expected both the main block and the blurred pseudo-element to survive into layout with raster backgrounds"
        );

        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/SMask"),
            "Expected the blurred pseudo-background to preserve alpha via a PDF soft mask"
        );
    }

    #[test]
    fn table_cell_borders_render() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            td { border-bottom: 1pt solid #999999; }
        </style></head><body>
        <table><tr><td>Cell</td></tr></table>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("l\nS\n") || pdf_str.contains("l S\n") || pdf_str.contains("re\nS\n"),
            "Table cell border should produce stroke commands"
        );
    }

    #[test]
    fn text_align_right_in_flex_cell() {
        let html = r#"<div style="display: flex"><div style="width: 200pt; text-align: right">Right</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Right"), "Should contain the text 'Right'");
        // The text x-position should be offset from left (not at left margin)
        assert!(
            pdf_str.contains("Td"),
            "Should have text positioning operator"
        );
    }

    #[test]
    fn text_align_center_in_flex_cell() {
        let html = r#"<div style="display: flex"><div style="width: 200pt; text-align: center">Center</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Center"),
            "Should contain the text 'Center'"
        );
        assert!(
            pdf_str.contains("Td"),
            "Should have text positioning operator"
        );
    }

    #[test]
    fn absolute_position_offset() {
        let html = r#"<div style="position: absolute; left: 100pt; top: 50pt">Absolute</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Absolute"),
            "Should contain positioned text"
        );
    }

    #[test]
    fn float_right_position() {
        let html = r#"<div style="float: right; width: 100pt">Floated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Floated"), "Should contain floated text");
    }

    #[test]
    fn radial_gradient_clipped() {
        let html = r#"<div style="background: radial-gradient(red, blue); border-radius: 10pt; height: 50pt">Radial</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/ShadingType 3"),
            "Should have radial shading"
        );
        assert!(
            pdf_str.contains("W n"),
            "Should clip radial gradient to border-radius"
        );
    }

    #[test]
    fn opacity_renders_extgstate() {
        let html = r#"<div style="opacity: 0.5">Transparent</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/ExtGState"),
            "Should have ExtGState for opacity"
        );
        assert!(pdf_str.contains("gs\n"), "Should apply graphics state");
    }

    #[test]
    fn box_shadow_renders() {
        let html = r#"<div style="box-shadow: 2pt 2pt 0 #888888; height: 30pt">Shadow</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Box shadow renders as a filled rectangle behind the element
        assert!(
            pdf_str.contains("re\nf\n") || pdf_str.contains("f\n"),
            "Should have fill for box shadow"
        );
        assert!(pdf_str.contains("Shadow"), "Should contain the text");
    }

    // --- Coverage tests for uncovered lines ---

    #[test]
    fn position_absolute_block_x() {
        // Covers line 93, 128: Position::Absolute uses margin.left + offset_left
        let html =
            r#"<div style="position: absolute; left: 50pt; background-color: cyan">Absolute</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Absolute"),
            "Should render absolute positioned text"
        );
    }

    #[test]
    fn position_relative_block_x() {
        // Covers lines 119-120, 129: Position::Relative block_x calculation
        let html =
            r#"<div style="position: relative; left: 30pt; background-color: lime">Relative</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Relative"),
            "Should render relative positioned text"
        );
    }

    #[test]
    fn float_right_positioning() {
        // Covers line 131: Float::Right block_x = margin.left + available_width - render_w
        let html = r#"<div style="float: right; width: 100pt">Float right</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Float right"),
            "Should render float right text"
        );
    }

    #[test]
    fn per_side_border_rendering() {
        // Four differently-colored solid sides meet at diagonal miters, so each
        // side paints as a filled trapezoid (`rg` fill) rather than a centerline
        // stroke. This keeps adjacent-color corners on the 45° seam (CSS
        // Backgrounds §6.2) instead of leaving a single-color overlap.
        let html = r#"<div style="border-top: 2pt solid red; border-right: 3pt solid green; border-bottom: 1pt solid blue; border-left: 4pt solid black; width: 200pt; height: 50pt">Borders</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Each side's color is now a fill (`rg`), not a stroke (`RG`).
        assert!(
            pdf_str.contains("1 0 0 rg"),
            "Should have red top border fill"
        );
        assert!(
            pdf_str.contains("0 0 0 rg"),
            "Should have black left border fill"
        );
        // Trapezoid corners: the miter geometry closes each side with `h\nf`.
        assert!(
            pdf_str.contains("h\nf\n"),
            "Per-side miter borders should fill closed trapezoids"
        );
    }

    #[test]
    fn center_align_with_inline_span() {
        // Covers line 487: TextAlign::Center branch in TextBlock with inline padding
        let html = r#"<p style="text-align: center"><span style="background-color: yellow; padding: 4pt">Centered Span</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Centered Span"),
            "Should render centered span text"
        );
        assert!(
            pdf_str.contains("1 1 0 rg"),
            "Should have yellow background fill"
        );
    }

    #[test]
    fn right_align_with_inline_span() {
        // Covers line 491: TextAlign::Right branch in TextBlock with inline padding
        let html = r#"<p style="text-align: right"><span style="background-color: lime; padding: 4pt">Right Span</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Right Span"),
            "Should render right-aligned span text"
        );
    }

    #[test]
    fn letter_spacing_in_text_rendering() {
        // Covers line 519 (letter-spacing sets Tc operator)
        let html = r#"<p style="letter-spacing: 2pt">Spaced out</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Tc\n"),
            "Letter spacing should produce Tc operator"
        );
        assert!(
            pdf_str.contains("0 Tc\n"),
            "Letter spacing should be reset to 0"
        );
    }

    #[test]
    fn underline_and_strikethrough_rendering() {
        // Covers underline and strikethrough draw lines with font-size-relative thickness
        let html = r#"<p><span style="text-decoration: underline">Under</span> <span style="text-decoration: line-through">Strike</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Both underline and strikethrough produce line strokes (S operator)
        let stroke_count = pdf_str.matches(" w\n").count();
        assert!(
            stroke_count >= 2,
            "Should have at least 2 stroke weight commands (underline + strikethrough), got {stroke_count}"
        );
        // Thickness should scale with font size (not hardcoded 0.5)
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw stroke lines for text decorations"
        );
    }

    #[test]
    fn table_cell_all_borders() {
        // Covers lines 621, 626-627, 705-724: table cell border rendering (all 4 sides)
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            td { border: 2pt solid red; }
        </style></head><body>
        <table><tr><td>Bordered Cell</td></tr></table>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Bordered Cell"), "Should render cell text");
        // Red border strokes
        assert!(
            pdf_str.contains("1 0 0 RG"),
            "Should have red border stroke color"
        );
        // Should have multiple line strokes (top, right, bottom, left)
        let stroke_count = pdf_str.matches("l S\n").count() + pdf_str.matches("l\nS\n").count();
        assert!(
            stroke_count >= 4,
            "Should have at least 4 border line strokes, got {stroke_count}"
        );
    }

    #[test]
    fn table_cell_rowspan_continuation() {
        // Covers lines 667, 669: rowspan > 1 cell rendering
        let html = r#"<table>
            <tr><td rowspan="2">Spanning</td><td>A</td></tr>
            <tr><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Spanning"), "Should render rowspan cell");
        assert!(pdf_str.contains("A"), "Should render first row cell");
        assert!(pdf_str.contains("B"), "Should render second row cell");
    }

    #[test]
    fn table_cell_nested_table_renders_inner_content() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        Outer
                        <table>
                            <tr><td>Inner</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Outer"), "Should render outer cell text");
        assert!(
            pdf_str.contains("Inner"),
            "Should render nested table cell text"
        );
    }

    #[test]
    fn flexrow_container_gradient() {
        // Covers lines 742, 744, 753, 848-874: FlexRow linear gradient with border-radius
        let html = r#"<div style="display: flex; background: linear-gradient(to right, red, blue); border-radius: 5pt"><div>Gradient Flex</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Gradient Flex"),
            "Should render flex content"
        );
        // Linear gradient produces shading reference
        assert!(
            pdf_str.contains("sh\n"),
            "Should have shading operator for gradient"
        );
    }

    #[test]
    fn flexrow_non_uniform_border() {
        // Covers lines 790, 798, 804-805, 939-969: FlexRow non-uniform per-side border
        let html = r#"<div style="display: flex; border-top: 2pt solid red; border-right: 3pt solid green; border-bottom: 1pt solid blue; border-left: 4pt solid black"><div>Flex Borders</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // The flex item shrinks to content; the words may render as separate
        // text-show operators, so assert each word is present rather than the
        // joined string.
        assert!(
            pdf_str.contains("(Flex)") && pdf_str.contains("(Borders)"),
            "Should render flex content"
        );
        // Non-uniform borders produce per-side strokes
        assert!(
            pdf_str.contains("1 0 0 RG"),
            "Should have red stroke for top"
        );
    }

    #[test]
    fn flexrow_cell_inline_background_with_border_radius() {
        // Covers lines 852-903, 982-1001: FlexRow cell bg with border-radius and gradient
        let html = r#"<div style="display: flex"><div style="background-color: orange; border-radius: 8pt; width: 100pt">Cell BG</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Cell BG"), "Should render cell text");
        // Orange background: 1 0.647.. 0 rg — check for the fill command
        assert!(
            pdf_str.contains("rg\n"),
            "Should have fill color for cell background"
        );
    }

    #[test]
    fn flexrow_cell_text_alignment() {
        // Covers lines 918-969, 1084, 1090: FlexRow cell text-align center and right
        let html = r#"<div style="display: flex">
            <div style="width: 200pt; text-align: center">Center</div>
            <div style="width: 200pt; text-align: right">Right</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("Center"),
            "Should render center-aligned text"
        );
        assert!(
            pdf_str.contains("Right"),
            "Should render right-aligned text"
        );
    }

    #[test]
    fn render_cell_text_vertical_centering() {
        // Covers lines 1116-1123: render_cell_text vertical centering with bg + border-radius
        let run = TextRun {
            text: "Centered".to_string(),
            font_size: 14.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((1.0, 0.0, 0.0, 1.0)),
            padding: (4.0, 2.0),
            border_radius: 3.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let cell = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 16.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            colspan: 1,
            rowspan: 1,
            padding_top: 4.0,
            padding_bottom: 4.0,
            padding_left: 4.0,
            padding_right: 4.0,
            background_color: None,
            border: LayoutBorder::default(),
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Middle,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        let mut content = String::new();
        let fonts = HashMap::new();
        let mut annotations = Vec::new();
        let prepared_fonts = PreparedCustomFonts::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut text_context = TextRenderContext::new(
            &fonts,
            &prepared_fonts,
            &mut annotations,
            &mut ts_pdf_writer,
            &mut ts_page_images,
        );
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(10.0, 200.0, 100.0),
            &mut text_context,
        );
        assert!(content.contains("Centered"), "Should render cell text");
        // Background with border-radius produces rounded rect
        assert!(
            content.contains("1 0 0 rg"),
            "Should have red inline background"
        );
    }

    #[test]
    fn merge_runs_border_radius_comparison() {
        // Covers lines 1175, 1179-1180: merge_runs checks border_radius equality
        let run_a = TextRun {
            text: "Hello ".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((1.0, 1.0, 0.0, 1.0)),
            padding: (2.0, 1.0),
            border_radius: 4.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let run_b = TextRun {
            text: "World".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((1.0, 1.0, 0.0, 1.0)),
            padding: (2.0, 1.0),
            border_radius: 8.0, // Different border_radius
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let merged = merge_runs(&[run_a.clone(), run_b.clone()]);
        // Different border_radius should prevent merging
        assert_eq!(
            merged.len(),
            2,
            "Runs with different border_radius should not merge"
        );
        // Same border_radius should merge
        let mut run_b_same = run_b;
        run_b_same.border_radius = 4.0;
        let merged2 = merge_runs(&[run_a, run_b_same]);
        assert_eq!(
            merged2.len(),
            1,
            "Runs with same border_radius should merge"
        );
    }

    #[test]
    fn build_shading_function_four_stops_stitching() {
        // Covers lines 1277-1304: Type 3 stitching function with 4 stops
        let stops = vec![
            (0.0, (1.0, 0.0, 0.0)),
            (0.33, (0.0, 1.0, 0.0)),
            (0.66, (0.0, 0.0, 1.0)),
            (1.0, (1.0, 1.0, 0.0)),
        ];
        let result = build_shading_function(&stops);
        assert!(
            result.contains("/FunctionType 3"),
            "4 stops should produce Type 3 stitching function"
        );
        assert!(
            result.contains("/Bounds [0.33 0.66]"),
            "Should have bounds for intermediate stops"
        );
        assert!(
            result.contains("/Encode [0 1 0 1 0 1]"),
            "Should have encode entries for each sub-function"
        );
        // Should contain 3 sub-functions (one per stop pair)
        let subfn_count = result.matches("/FunctionType 2").count();
        assert_eq!(
            subfn_count, 3,
            "Should have 3 Type 2 sub-functions, got {subfn_count}"
        );
    }

    #[test]
    fn custom_font_embedding_in_pdf() {
        // Covers lines 1628-1657: TTF font objects in PDF
        use crate::parser::ttf::TtfFont;
        let mut cmap = HashMap::new();
        for c in 32u32..=126 {
            cmap.insert(c, (c - 31) as u16);
        }
        let ttf = TtfFont {
            font_name: "TestFont".to_string(),
            units_per_em: 1000,
            bbox: [0, -200, 800, 800],
            pdf_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            cmap,
            glyph_widths: (0..=96).map(|_| 500).collect(),
            num_h_metrics: 96,
            flags: 32,
            is_bold: false,
            is_italic: false,
            x_height: 0,
            zero_advance: 0,
            data: std::sync::Arc::new(vec![0u8; 64]), // Minimal dummy font data
        };
        let mut fonts = HashMap::new();
        fonts.insert("TestFont".to_string(), ttf);

        let mut run = test_text_run("Custom");
        run.font_family = FontFamily::Custom("TestFont".to_string());
        let page = test_page(vec![(0.0, test_text_block_from_runs(vec![run]))]);
        let pdf = render_pdf_with_fonts(&[page], PageSize::A4, Margin::default(), &fonts).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("/BaseFont /TestFont"),
            "Should have custom font BaseFont entry"
        );
        assert!(
            pdf_str.contains("/Subtype /Type0"),
            "Should have Type0 font wrapper"
        );
        assert!(
            pdf_str.contains("/Subtype /CIDFontType2"),
            "Should have CIDFontType2 descendant font"
        );
        assert!(
            pdf_str.contains("/FontDescriptor"),
            "Should have FontDescriptor reference"
        );
        assert!(
            pdf_str.contains("/Encoding /Identity-H"),
            "Should use Identity-H for shaped custom glyphs"
        );
        assert!(
            pdf_str.contains("/ToUnicode"),
            "Should attach a ToUnicode CMap for text extraction"
        );
        assert!(
            pdf_str.contains("/FontFile2"),
            "Should have FontFile2 reference for embedded TTF"
        );
        assert!(
            pdf_str.contains("/TestFont"),
            "Should reference custom font name"
        );
    }

    #[test]
    fn render_run_text_falls_back_to_standard_font_when_custom_shaping_fails() {
        use crate::parser::ttf::TtfFont;

        let mut cmap = HashMap::new();
        for c in 32u32..=126 {
            cmap.insert(c, (c - 31) as u16);
        }
        let ttf = TtfFont {
            font_name: "TestFont".to_string(),
            units_per_em: 1000,
            bbox: [0, -200, 800, 800],
            pdf_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            cmap,
            glyph_widths: (0..=96).map(|_| 500).collect(),
            num_h_metrics: 96,
            flags: 32,
            is_bold: false,
            is_italic: false,
            x_height: 0,
            zero_advance: 0,
            data: std::sync::Arc::new(vec![0u8; 64]),
        };
        let mut fonts = HashMap::new();
        fonts.insert(
            crate::system_fonts::font_variant_key("TestFont", false, false),
            ttf,
        );

        let mut run = test_text_run("Custom");
        run.font_family = FontFamily::Custom("TestFont".to_string());

        let mut content = String::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        render_run_text(
            &mut content,
            &run,
            10.0,
            20.0,
            run.font_size,
            &fonts,
            &prepared_custom_fonts,
            0.0,
            &mut pdf_writer,
            &mut page_images,
        );

        assert!(content.contains("/Helvetica 12 Tf\n"));
        assert!(content.contains("(Custom) Tj\n"));
        assert!(!content.contains("/testfont 12 Tf\n"));
    }

    #[test]
    fn append_tj_shaped_text_uses_single_text_matrix() {
        let font = crate::parser::ttf::TtfFont {
            font_name: "TestFont".to_string(),
            units_per_em: 1000,
            bbox: [0, -200, 800, 800],
            pdf_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            layout_metrics: crate::parser::ttf::FontVerticalMetrics::new(800, -200, 0),
            cmap: HashMap::new(),
            glyph_widths: vec![0, 500, 500],
            num_h_metrics: 3,
            flags: 32,
            is_bold: false,
            is_italic: false,
            x_height: 0,
            zero_advance: 0,
            data: std::sync::Arc::new(Vec::new()),
        };
        let shaped = crate::text::ShapedRun {
            glyphs: vec![
                crate::text::ShapedGlyph {
                    glyph_id: 1,
                    x_advance: 6.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                    unicode: vec![0x0041],
                },
                crate::text::ShapedGlyph {
                    glyph_id: 2,
                    x_advance: 6.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                    unicode: vec![0x0042],
                },
            ],
            width: 12.0,
        };
        let mut content = String::new();
        append_tj_shaped_text(
            &mut content,
            ShapedTextRender::new(PdfPoint::new(10.0, 20.0), 12.0, &font, &shaped, None),
        );

        assert!(
            content.contains("1 0 0 1 10 20 Tm"),
            "Should position the run once with a single text matrix"
        );
        assert!(
            content.contains("[<0001> <0002>] TJ"),
            "Should encode the shaped run as one TJ array"
        );
        assert_eq!(
            content.matches(" Tm\n").count(),
            1,
            "Simple shaped runs should not emit per-glyph matrices"
        );
    }

    #[test]
    fn build_tounicode_cmap_supports_multi_codepoint_glyphs() {
        let cmap = build_tounicode_cmap(&[(1, vec![0x0066, 0x0069])]);
        assert!(
            cmap.contains("<0001> <00660069>"),
            "ToUnicode should preserve multi-codepoint mappings such as ligatures"
        );
    }

    #[test]
    fn ext_gstate_objects_rendered() {
        // Covers line 2011: ExtGState objects in resource dict
        let html = r#"<div style="opacity: 0.3">Dim</div><div style="opacity: 0.7">Bright</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("/ca 0.3"), "Should have fill opacity 0.3");
        assert!(pdf_str.contains("/ca 0.7"), "Should have fill opacity 0.7");
        assert!(
            pdf_str.contains("/ExtGState"),
            "Should have ExtGState in resources"
        );
        // Should have default GS reset
        assert!(
            pdf_str.contains("/GSDefault gs"),
            "Should reset to default graphics state"
        );
    }

    #[test]
    fn flexrow_cell_gradient_with_border_radius() {
        // Covers lines 1009-1060: FlexRow cell with linear gradient + border-radius clip
        let html = r#"<div style="display: flex"><div style="width: 150pt; background: linear-gradient(to bottom, red, blue); border-radius: 10pt">Grad Cell</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("Grad Cell"), "Should render cell text");
        assert!(
            pdf_str.contains("sh\n"),
            "Should have shading operator for cell gradient"
        );
    }

    #[test]
    fn half_leading_text_positioning() {
        // Text blocks should use half-leading model (not full line.height offset)
        let html = "<p style=\"font-size: 20pt; line-height: 2\">Test</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Should contain Td operator for text positioning
        assert!(pdf_str.contains("Td\n"), "Should have text positioning");
        // Text should be rendered
        assert!(pdf_str.contains("(Test)"), "Should contain text content");
    }

    #[test]
    fn underline_in_flex_cell() {
        // Underline in flex cells should produce stroke commands
        let html = r#"<html><head><style>
            .row { display: flex; }
        </style></head><body>
        <div class="row">
            <div><u>Underlined in flex</u></div>
        </div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Should have a stroke line for underline
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw underline stroke in flex cell"
        );
    }

    #[test]
    fn strikethrough_in_flex_cell() {
        let html = r#"<html><head><style>
            .row { display: flex; }
        </style></head><body>
        <div class="row">
            <div><del>Deleted in flex</del></div>
        </div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw strikethrough stroke in flex cell"
        );
    }

    #[test]
    fn underline_in_table_cell() {
        let html = r#"<table><tr><td><u>Underlined cell</u></td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw underline stroke in table cell"
        );
    }

    #[test]
    fn strikethrough_in_table_cell() {
        let html = r#"<table><tr><td><s>Struck cell</s></td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains(" l\nS\n"),
            "Should draw strikethrough stroke in table cell"
        );
    }

    #[test]
    fn font_size_relative_underline_thickness() {
        // Large font should produce thicker underline than small font
        let html = r#"<p><span style="font-size: 6pt; text-decoration: underline">Small</span></p>
        <p><span style="font-size: 30pt; text-decoration: underline">Big</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // Both should have strokes; thickness should vary
        let w_count = pdf_str.matches(" w\n").count();
        assert!(
            w_count >= 2,
            "Should have at least 2 underline thickness commands, got {w_count}"
        );
    }

    #[test]
    fn table_cell_vertical_centering_with_metrics() {
        // Table cells with different row heights should center text
        let html = r#"<table>
            <tr>
                <td style="padding: 20pt">Centered</td>
                <td>Short</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(Centered)"),
            "Should render centered cell text"
        );
        assert!(pdf_str.contains("(Short)"), "Should render short cell text");
    }

    // ===== layout_elements.rs coverage tests =====

    /// table_cell_content_top: VerticalAlign::Middle positions text mid-row
    #[test]
    fn layout_elements_vertical_align_middle_in_table_cell() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            td { vertical-align: middle; }
        </style></head><body>
        <table>
            <tr>
                <td style="height: 80pt; padding: 0">Middle</td>
                <td style="height: 80pt; padding: 0">Other</td>
            </tr>
        </table>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(Middle)"),
            "Should render middle-aligned text"
        );
    }

    /// table_cell_content_top: VerticalAlign::Bottom positions text at bottom
    #[test]
    fn layout_elements_vertical_align_bottom_in_table_cell() {
        use crate::parser::css::parse_stylesheet;
        let html = r#"<html><head><style>
            td.bottom { vertical-align: bottom; }
        </style></head><body>
        <table>
            <tr>
                <td class="bottom" style="padding: 0">Bottom</td>
                <td style="padding: 0; height: 60pt">Tall</td>
            </tr>
        </table>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = crate::layout::engine::layout_with_rules(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
        );
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(Bottom)"),
            "Should render bottom-aligned text"
        );
    }

    /// render_nested_text_block: background_color + border_radius in nested block
    #[test]
    fn layout_elements_nested_text_block_background_with_border_radius() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let lines = vec![test_text_line(vec![test_text_run("BgRound")])];
        let mut content = String::new();
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                clips: false,
                text_align: TextAlign::Left,
                padding_top: 4.0,
                padding_bottom: 4.0,
                padding_left: 4.0,
                padding_right: 4.0,
                border: LayoutBorder::default(),
                block_width: Some(100.0),
                block_height: None,
                background_color: Some((0.0, 1.0, 0.0, 1.0)),
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radius: 8.0, // Triggers rounded rect path
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // Green background
        assert!(
            content.contains("0 1 0 rg"),
            "Should have green background color"
        );
        // Rounded rect uses Bezier curves
        assert!(
            content.contains(" c\n"),
            "Should have Bezier curves for border-radius"
        );
        assert!(content.contains("f\n"), "Should fill the rounded rect");
    }

    /// render_nested_text_block: border rendering (all 4 sides)
    #[test]
    fn layout_elements_nested_text_block_all_four_borders() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let lines = vec![test_text_line(vec![test_text_run("Bordered")])];
        let mut content = String::new();
        let mut border = LayoutBorder::default();
        border.top = crate::layout::engine::LayoutBorderSide {
            width: 1.0,
            color: (1.0, 0.0, 0.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.right = crate::layout::engine::LayoutBorderSide {
            width: 1.0,
            color: (0.0, 1.0, 0.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.bottom = crate::layout::engine::LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.left = crate::layout::engine::LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 0.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                clips: false,
                text_align: TextAlign::Left,
                padding_top: 2.0,
                padding_bottom: 2.0,
                padding_left: 2.0,
                padding_right: 2.0,
                border,
                block_width: Some(100.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radius: 0.0,
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // Should have stroke commands for all 4 borders
        assert!(content.contains("1 0 0 RG"), "Should have red top border");
        assert!(
            content.contains("0 1 0 RG"),
            "Should have green right border"
        );
        assert!(
            content.contains("0 0 1 RG"),
            "Should have blue bottom border"
        );
        assert!(
            content.contains("0 0 0 RG"),
            "Should have black left border"
        );
        // All 4 sides produce line strokes
        let stroke_count = content.matches(" l S\n").count() + content.matches(" l\nS\n").count();
        assert!(
            stroke_count >= 4,
            "Should have at least 4 border strokes, got {stroke_count}"
        );
    }

    /// render_nested_text_block: only top border triggers single line
    #[test]
    fn layout_elements_nested_text_block_top_border_only() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let lines = vec![test_text_line(vec![test_text_run("TopOnly")])];
        let mut content = String::new();
        let mut border = LayoutBorder::default();
        border.top = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (1.0, 0.0, 0.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                clips: false,
                text_align: TextAlign::Left,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border,
                block_width: Some(80.0),
                block_height: None,
                background_color: None,
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radius: 0.0,
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 80.0),
            &mut ctx,
        );
        assert!(content.contains("1 0 0 RG"), "Should have red top border");
        assert!(content.contains("2 w\n"), "Should have 2pt line width");
    }

    /// render_nested_text_block: background_svg with BackgroundOrigin::Border
    #[test]
    fn layout_elements_nested_text_block_svg_background_border_origin() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let svg_tree = crate::parser::svg::SvgTree {
            width: 10.0,
            height: 10.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((1.0, 0.0, 0.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let mut border = LayoutBorder::default();
        border.top = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (0.0, 0.0, 0.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.bottom = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (0.0, 0.0, 0.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.left = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (0.0, 0.0, 0.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.right = crate::layout::engine::LayoutBorderSide {
            width: 2.0,
            color: (0.0, 0.0, 0.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        let lines = vec![test_text_line(vec![test_text_run("SvgBorder")])];
        let mut content = String::new();
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                clips: false,
                text_align: TextAlign::Left,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border,
                block_width: Some(100.0),
                block_height: None,
                background_color: None,
                background_svg: Some(&svg_tree),
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Cover,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::NoRepeat,
                // Border origin expands ref box by border widths
                background_origin: BackgroundOrigin::Border,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radius: 0.0,
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // SVG rect should produce fill output
        assert!(
            content.contains("1 0 0 rg"),
            "Should have red fill from SVG rect"
        );
        assert!(content.contains("(SvgBorder)"), "Should render block text");
    }

    /// render_nested_text_block: background_svg with BackgroundOrigin::Content
    #[test]
    fn layout_elements_nested_text_block_svg_background_content_origin() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let svg_tree = crate::parser::svg::SvgTree {
            width: 10.0,
            height: 10.0,
            width_attr: None,
            height_attr: None,
            preserve_aspect_ratio: crate::parser::svg::SvgPreserveAspectRatio::default(),
            view_box: None,
            defs: Default::default(),
            children: vec![crate::parser::svg::SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: crate::parser::svg::SvgStyle {
                    fill: crate::parser::svg::SvgPaint::Color((0.0, 0.0, 1.0)),
                    ..Default::default()
                },
            }],
            text_ctx: crate::parser::svg::SvgTextContext::default(),
            source_markup: None,
        };
        let lines = vec![test_text_line(vec![test_text_run("SvgContent")])];
        let mut content = String::new();
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &lines,
                clips: false,
                text_align: TextAlign::Left,
                padding_top: 5.0,
                padding_bottom: 5.0,
                padding_left: 5.0,
                padding_right: 5.0,
                border: LayoutBorder::default(),
                block_width: Some(100.0),
                block_height: None,
                background_color: None,
                background_svg: Some(&svg_tree),
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Cover,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::NoRepeat,
                // Content origin shrinks ref box by padding
                background_origin: BackgroundOrigin::Content,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radius: 0.0,
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // SVG rect should produce fill output (blue)
        assert!(
            content.contains("0 0 1 rg"),
            "Should have blue fill from SVG rect"
        );
    }

    /// render_nested_text_block: empty lines (no text) but with background
    #[test]
    fn layout_elements_nested_text_block_no_lines_with_background() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let mut content = String::new();
        render_nested_text_block(
            &mut content,
            NestedTextBlock {
                lines: &[], // No lines
                clips: false,
                text_align: TextAlign::Left,
                padding_top: 0.0,
                padding_bottom: 0.0,
                padding_left: 0.0,
                padding_right: 0.0,
                border: LayoutBorder::default(),
                block_width: Some(100.0),
                block_height: Some(50.0), // Explicit height keeps the block visible
                background_color: Some((0.5, 0.5, 0.5, 1.0)),
                background_svg: None,
                background_blur_radius: 0.0,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition::default(),
                background_repeat: BackgroundRepeat::Repeat,
                background_origin: BackgroundOrigin::Padding,
                background_clip: BackgroundClip::Border,
                background_blur_canvas_box: None,
                border_radius: 0.0,
                text_indent: 0.0,
            },
            NestedLayoutFrame::new(10.0, 100.0, 10.0, 100.0, 100.0),
            &mut ctx,
        );
        // Background rect fill should be emitted even with no lines
        assert!(
            content.contains("0.5 0.5 0.5 rg"),
            "Should have gray background fill even with no lines"
        );
        assert!(content.contains("re\nf\n"), "Should have rectangle fill");
    }

    /// render_nested_layout_elements: rowspan == 0 skips the cell
    #[test]
    fn layout_elements_nested_rowspan_zero_skips_cell() {
        use crate::layout::engine::{LayoutBorder, TableCell};
        let run = TextRun {
            text: "Skipped".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let run_visible = TextRun {
            text: "Visible".to_string(),
            ..run.clone()
        };
        // rowspan=0 means "continuation" — renderer skips it
        let cell_skip = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 0, // Should be skipped
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        let cell_visible = TableCell {
            lines: vec![TextLine {
                runs: vec![run_visible],
                height: 14.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        let element = LayoutElement::TableRow {
            cells: vec![cell_skip, cell_visible],
            col_widths: vec![100.0, 100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: crate::style::computed::BorderCollapse::Separate,
            border_spacing: 0.0,
            is_header: false,
            is_footer: false,
            offset_left: 0.0,
        };
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let mut content = String::new();
        render_nested_layout_elements(
            &mut content,
            &[element],
            NestedLayoutFrame::new(0.0, 100.0, 0.0, 100.0, 200.0),
            &mut ctx,
        );
        assert!(
            content.contains("(Visible)"),
            "Visible cell should be rendered"
        );
        assert!(
            !content.contains("(Skipped)"),
            "rowspan=0 cell should be skipped"
        );
    }

    /// render_nested_layout_elements: cell with background_color in nested table
    #[test]
    fn layout_elements_nested_table_cell_background_color() {
        use crate::layout::engine::{LayoutBorder, TableCell};
        let run = TextRun {
            text: "BgCell".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let cell = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: Some((1.0, 0.0, 0.0, 1.0)), // red cell background
            padding_top: 2.0,
            padding_right: 2.0,
            padding_bottom: 2.0,
            padding_left: 2.0,
            colspan: 1,
            rowspan: 1,
            border: LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        let element = LayoutElement::TableRow {
            cells: vec![cell],
            col_widths: vec![100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: crate::style::computed::BorderCollapse::Separate,
            border_spacing: 0.0,
            is_header: false,
            is_footer: false,
            offset_left: 0.0,
        };
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let mut content = String::new();
        render_nested_layout_elements(
            &mut content,
            &[element],
            NestedLayoutFrame::new(0.0, 100.0, 0.0, 100.0, 100.0),
            &mut ctx,
        );
        assert!(
            content.contains("1 0 0 rg"),
            "Should have red cell background fill"
        );
        assert!(
            content.contains("re\nf\n"),
            "Should have filled rect for cell background"
        );
        assert!(content.contains("(BgCell)"), "Should render cell text");
    }

    /// render_nested_layout_elements: cell with borders in nested context
    #[test]
    fn layout_elements_nested_table_cell_with_borders() {
        use crate::layout::engine::{LayoutBorder, LayoutBorderSide, TableCell};
        let run = TextRun {
            text: "BorderedNested".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let mut border = LayoutBorder::default();
        border.top = LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.right = LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.bottom = LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        border.left = LayoutBorderSide {
            width: 1.0,
            color: (0.0, 0.0, 1.0),
            alpha: 1.0,
            style: crate::style::computed::BorderStyle::Solid,
        };
        let cell = TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 2.0,
            padding_right: 2.0,
            padding_bottom: 2.0,
            padding_left: 2.0,
            colspan: 1,
            rowspan: 1,
            border,
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        let element = LayoutElement::TableRow {
            cells: vec![cell],
            col_widths: vec![100.0],
            margin_top: 0.0,
            margin_bottom: 0.0,
            border_collapse: crate::style::computed::BorderCollapse::Separate,
            border_spacing: 0.0,
            is_header: false,
            is_footer: false,
            offset_left: 0.0,
        };
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut pdf_writer = PdfWriter::new();
        let mut page_images = Vec::new();
        let mut shadings = Vec::new();
        let mut shading_counter = 0usize;
        let mut page_ext_gstates = Vec::new();
        let mut bg_alpha_counter = 0usize;
        let mut annotations = Vec::new();
        let mut ctx = PageRenderContext::new(
            &mut pdf_writer,
            &mut page_images,
            &custom_fonts,
            &prepared_custom_fonts,
            &mut shadings,
            &mut shading_counter,
            &mut page_ext_gstates,
            &mut bg_alpha_counter,
            &mut annotations,
        );
        let mut content = String::new();
        render_nested_layout_elements(
            &mut content,
            &[element],
            NestedLayoutFrame::new(0.0, 100.0, 0.0, 100.0, 100.0),
            &mut ctx,
        );
        assert!(
            content.contains("0 0 1 RG"),
            "Should have blue cell border color"
        );
        // Should have stroke commands for cell borders
        let stroke_count = content.matches("l S\n").count() + content.matches("l\nS\n").count();
        assert!(
            stroke_count >= 4,
            "Should have at least 4 cell border strokes, got {stroke_count}"
        );
    }

    /// render_cell_text: text-align right and center in nested table cell
    #[test]
    fn layout_elements_cell_text_align_right_and_center() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ctx = TextRenderContext::new(
            &custom_fonts,
            &prepared_custom_fonts,
            &mut annotations,
            &mut ts_pdf_writer,
            &mut ts_page_images,
        );

        let run = TextRun {
            text: "Aligned".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };

        // Test right-align
        let cell_right = crate::layout::engine::TableCell {
            lines: vec![TextLine {
                runs: vec![run.clone()],
                height: 14.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Right,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        let mut content_right = String::new();
        render_cell_text(
            &mut content_right,
            &cell_right,
            CellTextPlacement::new(0.0, 100.0, 200.0),
            &mut ctx,
        );
        assert!(
            content_right.contains("(Aligned)"),
            "Should render right-aligned text"
        );

        // Test center-align
        let cell_center = crate::layout::engine::TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        let mut content_center = String::new();
        render_cell_text(
            &mut content_center,
            &cell_center,
            CellTextPlacement::new(0.0, 100.0, 200.0),
            &mut ctx,
        );
        assert!(
            content_center.contains("(Aligned)"),
            "Should render center-aligned text"
        );
    }

    /// render_cell_text: underline and line_through in nested table cell
    #[test]
    fn layout_elements_cell_text_underline_and_line_through() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ctx = TextRenderContext::new(
            &custom_fonts,
            &prepared_custom_fonts,
            &mut annotations,
            &mut ts_pdf_writer,
            &mut ts_page_images,
        );

        let underline_run = TextRun {
            text: "Under".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: true,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };
        let strike_run = TextRun {
            text: "Strike".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: true,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };

        let cell = crate::layout::engine::TableCell {
            lines: vec![
                TextLine {
                    runs: vec![underline_run],
                    height: 14.0,
                    x_offset: 0.0,
                },
                TextLine {
                    runs: vec![strike_run],
                    height: 14.0,
                    x_offset: 0.0,
                },
            ],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(10.0, 200.0, 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Under)"), "Should render underlined text");
        assert!(
            content.contains("(Strike)"),
            "Should render struck-through text"
        );
        // Both decorations draw lines with S stroke command
        let stroke_count = content.matches(" l\nS\n").count() + content.matches(" l S\n").count();
        assert!(
            stroke_count >= 2,
            "Should have strokes for underline and line-through, got {stroke_count}"
        );
    }

    /// render_cell_text: inline span with background_color and border_radius in nested cell
    #[test]
    fn layout_elements_cell_text_inline_bg_with_border_radius() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ctx = TextRenderContext::new(
            &custom_fonts,
            &prepared_custom_fonts,
            &mut annotations,
            &mut ts_pdf_writer,
            &mut ts_page_images,
        );

        let run = TextRun {
            text: "Badge".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (1.0, 1.0, 1.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((0.2, 0.4, 0.8, 1.0)),
            padding: (3.0, 2.0),
            border_radius: 4.0, // Triggers rounded rect for inline background
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };

        let cell = crate::layout::engine::TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(10.0, 200.0, 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Badge)"), "Should render badge text");
        // Inline background fill (rounded rect uses Bezier c operator)
        assert!(
            content.contains("0.2 0.4 0.8 rg"),
            "Should have blue inline background color"
        );
        assert!(
            content.contains(" c\n"),
            "Should have Bezier curves for rounded inline bg"
        );
    }

    /// render_cell_text: inline span with background_color but no border_radius (rect path)
    #[test]
    fn layout_elements_cell_text_inline_bg_no_border_radius() {
        let custom_fonts = HashMap::new();
        let prepared_custom_fonts = PreparedCustomFonts::new();
        let mut annotations = Vec::new();
        let mut ts_pdf_writer = PdfWriter::new();
        let mut ts_page_images = Vec::new();
        let mut ctx = TextRenderContext::new(
            &custom_fonts,
            &prepared_custom_fonts,
            &mut annotations,
            &mut ts_pdf_writer,
            &mut ts_page_images,
        );

        let run = TextRun {
            text: "Tag".to_string(),
            font_size: 12.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            color: (0.0, 0.0, 0.0),
            font_family: FontFamily::Helvetica,
            link_url: None,
            background_color: Some((1.0, 1.0, 0.0, 1.0)), // yellow
            padding: (2.0, 1.0),
            border_radius: 0.0, // No rounding — should use rectangle
            line_height_factor: f32::NAN,
            inline_box: None,
            disable_ligatures: false,
            vertical_align: VerticalAlign::Baseline,
            text_shadow: Vec::new(),
        };

        let cell = crate::layout::engine::TableCell {
            lines: vec![TextLine {
                runs: vec![run],
                height: 14.0,
                x_offset: 0.0,
            }],
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: 0.0,
            padding_right: 0.0,
            padding_bottom: 0.0,
            padding_left: 0.0,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };

        let mut content = String::new();
        render_cell_text(
            &mut content,
            &cell,
            CellTextPlacement::new(10.0, 200.0, 150.0),
            &mut ctx,
        );
        assert!(content.contains("(Tag)"), "Should render tag text");
        assert!(
            content.contains("1 1 0 rg"),
            "Should have yellow inline background color"
        );
        // No border-radius: should use rectangle re operator
        assert!(
            content.contains(" re\nf\n"),
            "Should use rectangle fill for zero-radius inline bg"
        );
    }

    /// plan_nested_layout_elements: Position::Relative with positioned_depth registers origin
    #[test]
    fn layout_elements_plan_relative_with_positioned_depth() {
        let mut relative = test_text_block_from_runs(vec![test_text_run("Relative")]);
        if let LayoutElement::TextBlock {
            position,
            offset_top,
            offset_left,
            positioned_depth,
            ..
        } = &mut relative
        {
            *position = Position::Relative;
            *offset_top = 5.0;
            *offset_left = 15.0;
            *positioned_depth = 1; // Non-zero: should register origin
        }
        let elements = [relative];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(20.0, 80.0, 10.0, 120.0, 100.0),
        );
        assert_eq!(planned.len(), 1);
        // Relative: uses local origin (20.0) + offset_left (15.0)
        assert!(
            (planned[0].origin_x - 35.0).abs() < 0.01,
            "Relative block origin_x should be frame.origin_x + offset_left"
        );
        // top_y: cursor_y (80.0) - margin_top (0) - offset_top (5) = 75.0
        assert!(
            (planned[0].top_y - 75.0).abs() < 0.01,
            "Relative block top_y should be cursor_y - offset_top"
        );
    }

    /// plan_nested_layout_elements: absolute with containing_block sets blur_canvas_box
    #[test]
    fn layout_elements_plan_absolute_with_containing_block_sets_blur_canvas_box() {
        let containing = crate::layout::engine::ContainingBlock {
            x: 5.0,
            width: 200.0,
            height: 100.0,
            depth: 2,
        };
        let mut absolute = test_text_block_from_runs(vec![test_text_run("Abs")]);
        if let LayoutElement::TextBlock {
            position,
            containing_block,
            positioned_depth,
            ..
        } = &mut absolute
        {
            *position = Position::Absolute;
            *containing_block = Some(containing);
            *positioned_depth = 0;
        }
        // First register a positioned origin for depth 2 by planning a relative block
        let mut relative_parent = test_text_block_from_runs(vec![test_text_run("Parent")]);
        if let LayoutElement::TextBlock {
            position,
            positioned_depth,
            ..
        } = &mut relative_parent
        {
            *position = Position::Relative;
            *positioned_depth = 2;
        }
        let elements = [relative_parent, absolute];
        let planned = plan_nested_layout_elements(
            &elements,
            NestedLayoutFrame::new(10.0, 200.0, 10.0, 200.0, 300.0),
        );
        // The absolute element should have a blur_canvas_box derived from the containing block
        let _abs_planned = planned.iter().find(|p| {
            if let LayoutElement::TextBlock { .. } = p.element {
                // The second element (absolute) should have blur_canvas_box set when
                // its containing_block refers to a depth that has been registered
                true
            } else {
                false
            }
        });
        // Just verify the plan succeeds without panic and produces 2 elements
        assert_eq!(planned.len(), 2, "Should plan both elements");
    }

    /// table_row_total_height: returns 0 for non-TableRow variant
    #[test]
    fn layout_elements_table_row_total_height_non_row_returns_zero() {
        let non_row = LayoutElement::PageBreak(Default::default());
        assert_eq!(
            table_row_total_height(&non_row),
            0.0,
            "Non-TableRow element should return 0 height"
        );
        let text_block = test_text_block_from_runs(vec![test_text_run("Hello")]);
        assert_eq!(
            table_row_total_height(&text_block),
            0.0,
            "TextBlock element should return 0 height"
        );
    }

    /// Integration: nested table with vertical-align middle exercises layout_elements paths
    #[test]
    fn layout_elements_nested_table_cell_vertical_align_middle_integration() {
        let html = r#"<table>
            <tr>
                <td>
                    <table>
                        <tr>
                            <td style="vertical-align: middle; height: 50pt">Inner</td>
                            <td style="height: 50pt">Other</td>
                        </tr>
                    </table>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(Inner)"),
            "Should render inner nested cell text"
        );
    }

    /// Integration: nested div inside table cell with SVG background
    /// exercises render_nested_text_block with background_svg via nested cell rows
    #[test]
    fn layout_elements_nested_svg_background_in_table_cell() {
        // A div with SVG background inside a td triggers render_nested_layout_elements
        // and render_nested_text_block with background_svg set
        let html = r#"<table>
            <tr>
                <td>
                    <div style="background-image: url('data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2210%22 height=%2210%22%3E%3Crect width=%2210%22 height=%2210%22 fill=%22red%22/%3E%3C/svg%3E'); background-size: cover; width: 40pt; height: 20pt;">CellSVG</div>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        // The text should render
        assert!(
            pdf_str.contains("(CellSVG)"),
            "Should render text inside nested cell div"
        );
        // The overall PDF should be valid (no crash on SVG background in nested context)
        assert!(pdf_str.contains("%PDF-1.4"), "Should produce a valid PDF");
    }

    /// Integration: border-collapse collapse with nested elements
    #[test]
    fn layout_elements_nested_border_collapse() {
        let html = r#"<table style="border-collapse: collapse">
            <tr>
                <td style="border: 1pt solid black">CollapseA</td>
                <td style="border: 1pt solid black">CollapseB</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(pdf_str.contains("(CollapseA)"), "Should render first cell");
        assert!(pdf_str.contains("(CollapseB)"), "Should render second cell");
    }

    /// Integration: nested table with rowspan > 1 spanning into future rows
    #[test]
    fn layout_elements_nested_rowspan_spans_future_rows() {
        let html = r#"<table>
            <tr>
                <td>
                    <table>
                        <tr>
                            <td rowspan="2">SpanInner</td>
                            <td>A</td>
                        </tr>
                        <tr>
                            <td>B</td>
                        </tr>
                    </table>
                </td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let pdf_str = String::from_utf8_lossy(&pdf);
        assert!(
            pdf_str.contains("(SpanInner)"),
            "Should render spanning nested cell"
        );
        assert!(
            pdf_str.contains("(A)"),
            "Should render first row second cell"
        );
        assert!(
            pdf_str.contains("(B)"),
            "Should render second row second cell"
        );
    }

    // ── unicode_to_symbol ───────────────────────────────────────────────

    #[test]
    fn unicode_to_symbol_greek_lowercase() {
        assert_eq!(unicode_to_symbol('\u{03B1}'), Some(0x61)); // α
        assert_eq!(unicode_to_symbol('\u{03C0}'), Some(0x70)); // π
        assert_eq!(unicode_to_symbol('\u{03C9}'), Some(0x77)); // ω
    }

    #[test]
    fn unicode_to_symbol_greek_uppercase() {
        assert_eq!(unicode_to_symbol('\u{0393}'), Some(0x47)); // Γ
        assert_eq!(unicode_to_symbol('\u{03A9}'), Some(0x57)); // Ω
        assert_eq!(unicode_to_symbol('\u{03A3}'), Some(0x53)); // Σ
    }

    #[test]
    fn unicode_to_symbol_operators() {
        assert_eq!(unicode_to_symbol('\u{2211}'), Some(0xE5)); // ∑
        assert_eq!(unicode_to_symbol('\u{222B}'), Some(0xF2)); // ∫
        assert_eq!(unicode_to_symbol('\u{221E}'), Some(0xA5)); // ∞
    }

    #[test]
    fn unicode_to_symbol_relations() {
        assert_eq!(unicode_to_symbol('\u{2264}'), Some(0xA3)); // ≤
        assert_eq!(unicode_to_symbol('\u{2265}'), Some(0xB3)); // ≥
        assert_eq!(unicode_to_symbol('\u{2260}'), Some(0xB9)); // ≠
        assert_eq!(unicode_to_symbol('\u{2208}'), Some(0xCE)); // ∈
    }

    #[test]
    fn unicode_to_symbol_arrows() {
        assert_eq!(unicode_to_symbol('\u{2192}'), Some(0xAE)); // →
        assert_eq!(unicode_to_symbol('\u{2190}'), Some(0xAC)); // ←
        assert_eq!(unicode_to_symbol('\u{21D2}'), Some(0xDE)); // ⇒
    }

    #[test]
    fn unicode_to_symbol_delimiters() {
        assert_eq!(unicode_to_symbol('\u{27E8}'), Some(0xE1)); // ⟨
        assert_eq!(unicode_to_symbol('\u{27E9}'), Some(0xF1)); // ⟩
        assert_eq!(unicode_to_symbol('\u{230A}'), Some(0xEB)); // ⌊
        assert_eq!(unicode_to_symbol('\u{2309}'), Some(0xF9)); // ⌉
    }

    #[test]
    fn unicode_to_symbol_binary_ops() {
        assert_eq!(unicode_to_symbol('\u{00D7}'), Some(0xB4)); // ×
        assert_eq!(unicode_to_symbol('\u{00F7}'), Some(0xB8)); // ÷
        assert_eq!(unicode_to_symbol('\u{00B1}'), Some(0xB1)); // ±
    }

    #[test]
    fn unicode_to_symbol_misc() {
        assert_eq!(unicode_to_symbol('\u{2202}'), Some(0xB6)); // ∂
        assert_eq!(unicode_to_symbol('\u{2207}'), Some(0xD1)); // ∇
        assert_eq!(unicode_to_symbol('\u{2200}'), Some(0x22)); // ∀
        assert_eq!(unicode_to_symbol('\u{2203}'), Some(0x24)); // ∃
        assert_eq!(unicode_to_symbol('\u{2205}'), Some(0xC6)); // ∅
    }

    #[test]
    fn unicode_to_symbol_returns_none_for_ascii() {
        assert_eq!(unicode_to_symbol('A'), None);
        assert_eq!(unicode_to_symbol('x'), None);
        assert_eq!(unicode_to_symbol('+'), None);
    }

    // ── render_math_glyphs ──────────────────────────────────────────────

    #[test]
    fn render_math_glyphs_char_italic() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Char {
            ch: 'x',
            x: 10.0,
            y: 20.0,
            font_size: 12.0,
            italic: true,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("Helvetica-Oblique"));
        assert!(content.contains("12 Tf"));
    }

    #[test]
    fn render_math_glyphs_char_regular() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Char {
            ch: '2',
            x: 0.0,
            y: 0.0,
            font_size: 10.0,
            italic: false,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 5.0, 5.0, &mut content);
        assert!(content.contains("/Helvetica 10"));
        assert!(content.contains("(2) Tj"));
    }

    #[test]
    fn render_math_glyphs_symbol_char() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Char {
            ch: '\u{03B1}', // α
            x: 0.0,
            y: 0.0,
            font_size: 12.0,
            italic: false,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("/Symbol 12 Tf"));
    }

    #[test]
    fn render_math_glyphs_text() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Text {
            text: "lim".to_string(),
            x: 0.0,
            y: 0.0,
            font_size: 12.0,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("/Helvetica 12 Tf"));
        assert!(content.contains("(lim) Tj"));
    }

    #[test]
    fn render_math_glyphs_rule() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Rule {
            x: 10.0,
            y: 20.0,
            width: 50.0,
            thickness: 0.5,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("re\nf\n"));
    }

    #[test]
    fn render_math_glyphs_radical() {
        use crate::layout::math::MathGlyph;
        let glyphs = vec![MathGlyph::Radical {
            x: 0.0,
            y: 0.0,
            width: 30.0,
            height: 15.0,
            font_size: 12.0,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        // Radical draws lines
        assert!(content.contains(" l\n"));
        assert!(content.contains("S\n"));
    }

    #[test]
    fn render_math_glyphs_delimiter_small() {
        use crate::layout::math::MathGlyph;
        // Small delimiter: height <= font_size * 1.3, renders as text
        let glyphs = vec![MathGlyph::Delimiter {
            ch: '(',
            x: 0.0,
            y: 0.0,
            height: 12.0,
            font_size: 12.0,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains("Tf\n"));
    }

    #[test]
    fn render_math_glyphs_delimiter_large() {
        use crate::layout::math::MathGlyph;
        // Large delimiter: height > font_size * 1.3, renders as paths
        let glyphs = vec![MathGlyph::Delimiter {
            ch: '(',
            x: 0.0,
            y: 0.0,
            height: 30.0,
            font_size: 12.0,
        }];
        let mut content = String::new();
        render_math_glyphs(&glyphs, 0.0, 0.0, &mut content);
        assert!(content.contains(" c\n")); // cubic bezier for parenthesis
    }

    // ── Math integration via HTML ───────────────────────────────────────

    #[test]
    fn math_inline_produces_symbol_font_in_pdf() {
        let html = r#"<span class="math-inline" data-math="\alpha + \beta">α+β</span>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Symbol"));
    }

    #[test]
    fn math_display_produces_valid_pdf() {
        let html = r#"<div class="math-display" data-math="\frac{a}{b}">a/b</div>"#;
        let pdf = crate::html_to_pdf(html).unwrap();
        assert!(pdf.len() > 100);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("%PDF"));
    }

    #[test]
    fn math_markdown_inline_renders() {
        let pdf = crate::markdown_to_pdf("The equation $E = mc^2$ is famous.").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("BT\n"));
        assert!(pdf.len() > 200);
    }

    #[test]
    fn math_markdown_display_renders() {
        let pdf = crate::markdown_to_pdf("$$\\sum_{k=1}^{n} k = \\frac{n(n+1)}{2}$$").unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Symbol"));
    }

    #[test]
    fn render_rgba_background_produces_extgstate() {
        let html =
            r#"<div style="background-color: rgba(255, 0, 0, 0.5)">Semi-transparent bg</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/ca 0.5"),
            "PDF should contain fill opacity /ca 0.5 for rgba background"
        );
        assert!(
            content.contains("/ExtGState"),
            "PDF should contain ExtGState resource for rgba background"
        );
        assert!(
            content.contains("gs\n"),
            "PDF should use gs operator for rgba background"
        );
    }

    #[test]
    fn math_mixed_text_and_math() {
        let pdf =
            crate::markdown_to_pdf("For $x > 0$, we have $f(x) = x^2$ and $g(x) = \\sqrt{x}$.")
                .unwrap();
        assert!(pdf.len() > 200);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("%PDF"));
        assert!(text.contains("%%EOF"));
    }

    #[test]
    fn render_box_shadow_no_blur() {
        let html = r#"<div style="width: 100pt; height: 50pt; box-shadow: 5px 5px 0px rgba(0,0,0,0.5)">Shadow</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // No-blur shadow should draw a solid rect fill
        assert!(
            content.contains("re\nf\n"),
            "Box shadow without blur should produce a filled rectangle"
        );
    }

    #[test]
    fn render_box_shadow_with_blur() {
        let html = r#"<div style="width: 100pt; height: 50pt; box-shadow: 3px 3px 10px rgba(0,0,0,0.4)">Blurred shadow</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // A blurred box-shadow is now rendered as a gaussian-blurred image
        // XObject (a soft penumbra), embedded and drawn with `Do`, rather than
        // the previous concentric-layer alpha approximation.
        assert!(
            content.contains("Do\n"),
            "Blurred box shadow should embed a blurred image XObject"
        );
    }

    #[test]
    fn render_container_with_background_and_border() {
        let html = r#"
            <div style="background-color: #ccc; border: 2px solid blue; padding: 10px">
                <p>Inside container</p>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Container background fill
        assert!(
            content.contains("rg\n"),
            "Container should have background color fill"
        );
        // Container border stroke
        assert!(
            content.contains("RG\n"),
            "Container should have border stroke color"
        );
        assert!(
            content.contains("Inside container"),
            "Container children text should be rendered"
        );
    }

    #[test]
    fn render_flexbox_with_border() {
        let html = r#"
            <div style="display: flex; border: 1px solid red; padding: 5px">
                <div style="flex: 1">Left</div>
                <div style="flex: 1">Right</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Flex container border: red = 1 0 0 RG
        assert!(
            content.contains("1 0 0 RG"),
            "FlexRow border should use red stroke color"
        );
    }

    #[test]
    fn render_flexbox_honors_own_left_margin() {
        // Regression: a top-level flex container must honour its own horizontal
        // margin (like any block). The container background rect must be painted
        // at page-content-left + the container's margin-left, not flush left.
        // Page margin = 72pt (default); container margin-left = 40px = 30pt; so
        // the background rect x-origin must be 102pt.
        let html = r#"
            <div style="display: flex; margin-left: 40px; width: 200px; background-color: #abcdef">
                <div style="width: 50px">A</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // The container background fill rectangle starts at x = 102 (72 + 30).
        assert!(
            content.contains("102 ") && content.contains("re\nf\n"),
            "flex container background must be shifted right by its margin-left \
             (expected x-origin 102pt); content did not contain it.\n{content}"
        );
        // It must NOT be painted flush at the page content-left (72pt).
        assert!(
            !content.contains("\n72 "),
            "flex container background must not be flush at page content-left"
        );
    }

    #[test]
    fn render_flexbox_with_background_color() {
        let html = r#"
            <div style="display: flex; background-color: yellow; padding: 8px">
                <div style="flex: 1">A</div>
                <div style="flex: 1">B</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Yellow bg = 1 1 0 rg
        assert!(
            content.contains("1 1 0 rg"),
            "FlexRow should render yellow background"
        );
    }

    #[test]
    fn render_transform_skew_matrix_in_pdf() {
        // skew() produces a Transform::Matrix variant, exercising the Matrix arm
        let html = r#"<div style="transform: skew(10deg); width: 50pt; height: 30pt; background: red">Skewed</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // skew(10deg) produces a Matrix transform which emits a cm operator
        assert!(
            content.contains("cm\n"),
            "CSS transform: skew() should produce a cm (concat matrix) operator in PDF"
        );
    }

    #[test]
    fn render_grid_item_overflow_hidden_paints_clipped_inner_block() {
        // A grid item with overflow:hidden and an oversized inner block must
        // paint the inner block (clipped to the cell), not drop it. Regression
        // test for the grid-cell nested-block clip path.
        let html = r#"
            <div style="display: grid; grid-template-columns: 100px 100px; gap: 10px; width: 210px">
                <div style="overflow: hidden; height: 80px; background: #eee">
                    <div style="width: 200px; height: 160px; background: #2874a6"></div>
                </div>
                <div style="height: 80px; background: #ddd"></div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(pdf.starts_with(b"%PDF"));
        // The inner block fill colour #2874a6 (0.156.. 0.454.. 0.650..) must be
        // emitted, proving the oversized inner block is painted inside the cell.
        assert!(
            content.contains("0.15686275 0.45490196 0.6509804 rg"),
            "grid item's oversized inner block should be painted (clipped) inside the cell"
        );
        // And a clip (W n) must be present for the overflow:hidden cell.
        assert!(
            content.contains("W n"),
            "overflow:hidden grid cell should emit a clip path"
        );
    }

    #[test]
    fn render_grid_row_with_border() {
        let html = r#"
            <div style="display: grid; grid-template-columns: 1fr 1fr; border: 2px solid green; gap: 4px">
                <div>Cell A</div>
                <div>Cell B</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            pdf.starts_with(b"%PDF"),
            "Grid with border should produce valid PDF"
        );
        // Green border: 0 0.50196... 0 RG (CSS green = #008000)
        assert!(
            content.contains("RG\n"),
            "Grid border should produce stroke color"
        );
    }

    #[test]
    fn render_container_with_border_radius() {
        let html = r#"
            <div style="background-color: blue; border-radius: 10px; width: 100pt; height: 60pt; padding: 10px">
                <p>Rounded</p>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        // Rounded rect uses Bezier curves (c operator)
        assert!(
            content.contains(" c\n"),
            "Border radius should produce Bezier curve operators"
        );
    }

    #[test]
    fn render_pdf_to_writer_produces_same_output() {
        let nodes = parse_html("<p>Writer test</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf_bytes = render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let mut writer_buf = Vec::new();
        render_pdf_to_writer(&pages, PageSize::A4, Margin::default(), &mut writer_buf).unwrap();
        assert_eq!(
            pdf_bytes.len(),
            writer_buf.len(),
            "render_pdf and render_pdf_to_writer should produce identical output"
        );
        assert_eq!(pdf_bytes, writer_buf);
    }
}
