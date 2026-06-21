use crate::parser::css::{AncestorInfo, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::{
    ComputedStyle, FontWeight, GridAlign, GridTrack, TextAlign, VerticalAlign, Visibility,
    compute_style_with_context,
};

use super::context::{LayoutContext, LayoutEnv};
use super::engine::{BackgroundFields, LayoutBorder, LayoutElement};
use super::table::{GridInset, TableCell};
use super::text::{
    FlexTextRunCollector, TextWrapOptions, resolved_line_height_factor, wrap_text_runs,
};

/// Resolve grid column widths from track definitions.
///
/// CSS Grid track-sizing semantics:
/// - Fixed(v): uses `v` directly.
/// - Auto: sized to the column's max-content intrinsic width (passed in via
///   `auto_intrinsic_widths`, indexed by track). When the sum of fixed + auto
///   exceeds the available space, auto columns shrink proportionally.
/// - Fr(v) / Minmax(min, max): flexible tracks. The space left after the
///   fixed/percent/auto tracks is divided among them by the CSS Grid
///   "find the size of an fr" algorithm — each flexible track resolves to
///   `flex_size × flex_factor`, floored at its base (`0` for a bare `fr`, the
///   `min` for a `minmax`) and capped at its `max`, with `flex_size` found by
///   iteratively freezing clamped tracks. Equal `fr` peers therefore resolve
///   to equal widths even when their `minmax` minimums differ. If no flexible
///   tracks exist and slack remains, Auto columns absorb it (so `auto auto`
///   fills the row like Chrome does).
///
/// `auto_intrinsic_widths` must have length == tracks.len(); the value at
/// each Auto track index is that column's max-content width. Non-Auto
/// entries are ignored.
fn resolve_grid_columns(
    tracks: &[GridTrack],
    available_width: f32,
    gap: f32,
    auto_intrinsic_widths: &[f32],
) -> Vec<f32> {
    if tracks.is_empty() {
        return vec![available_width];
    }

    let num_gaps = if tracks.len() > 1 {
        (tracks.len() - 1) as f32 * gap
    } else {
        0.0
    };
    let space = (available_width - num_gaps).max(0.0);

    // First pass: bucket totals.
    let mut fixed_total: f32 = 0.0;
    let mut fr_total: f32 = 0.0;
    let mut auto_total: f32 = 0.0;
    let mut auto_count: usize = 0;
    let mut minmax_count: usize = 0;

    for (i, track) in tracks.iter().enumerate() {
        match track {
            GridTrack::Fixed(v) => fixed_total += *v,
            GridTrack::Percent(p) => fixed_total += *p * space,
            GridTrack::Fr(v) => fr_total += *v,
            GridTrack::Auto => {
                auto_total += auto_intrinsic_widths.get(i).copied().unwrap_or(0.0);
                auto_count += 1;
            }
            GridTrack::Minmax(_, _) => {
                // `minmax` tracks are flexible: their base (`min`) participates
                // in the fr resolution below, not in `fixed_total`, so the
                // shared flex space includes the whole track.
                minmax_count += 1;
            }
        }
    }

    let after_fixed = (space - fixed_total).max(0.0);
    let has_fr = fr_total + minmax_count as f32 > 0.0;

    if has_fr {
        // Flexible-track regime (`fr` / `minmax(min, ...fr)` present). Auto
        // tracks size to their intrinsic max-content width; the rest of the
        // space is distributed among the flexible tracks by the CSS Grid
        // "find the size of an fr" algorithm (§12.7): every flexible track is
        // sized to `flex_size × flex_factor`, but no smaller than its base
        // minimum (0 for a bare `fr`, the `min` for a `minmax`) and no larger
        // than its `max` cap. `flex_size` is found by iteratively freezing
        // tracks whose floor/ceiling clamps them, then re-dividing the
        // remaining space among the still-flexible tracks. This makes equal
        // `1fr` peers resolve to equal widths even when their minimums differ
        // (e.g. `minmax(80px,1fr) minmax(120px,1fr)` → two equal tracks),
        // matching Chrome — unlike the old `min + share` formula which inflated
        // the larger-min track.
        let space_for_flex = (after_fixed - auto_total).max(0.0);

        // Per flexible track: (flex_factor, base_min, max_cap).
        struct Flex {
            factor: f32,
            base: f32,
            cap: f32,
        }
        let flex: Vec<Option<Flex>> = tracks
            .iter()
            .map(|track| match track {
                GridTrack::Fr(v) => Some(Flex {
                    factor: *v,
                    base: 0.0,
                    cap: f32::MAX,
                }),
                GridTrack::Minmax(min, max) => Some(Flex {
                    factor: 1.0,
                    base: *min,
                    cap: *max,
                }),
                _ => None,
            })
            .collect();

        // Iteratively resolve the shared flex size, freezing any track that
        // its base (floor) or cap (ceiling) pins, then re-dividing.
        let mut frozen = vec![false; tracks.len()];
        let mut resolved = vec![0.0_f32; tracks.len()];
        loop {
            let mut remaining = space_for_flex;
            let mut active_factor = 0.0_f32;
            for (i, f) in flex.iter().enumerate() {
                let Some(f) = f else { continue };
                if frozen[i] {
                    remaining -= resolved[i];
                } else {
                    active_factor += f.factor;
                }
            }
            remaining = remaining.max(0.0);
            if active_factor <= 0.0 {
                break;
            }
            let flex_size = remaining / active_factor;
            // Freeze the first track pinned below its base or above its cap;
            // restart so the freed/consumed space redistributes correctly.
            let mut changed = false;
            for (i, f) in flex.iter().enumerate() {
                let Some(f) = f else { continue };
                if frozen[i] {
                    continue;
                }
                let want = flex_size * f.factor;
                if want < f.base {
                    resolved[i] = f.base;
                    frozen[i] = true;
                    changed = true;
                    break;
                }
                if want > f.cap {
                    resolved[i] = f.cap;
                    frozen[i] = true;
                    changed = true;
                    break;
                }
            }
            if !changed {
                for (i, f) in flex.iter().enumerate() {
                    if let Some(f) = f {
                        if !frozen[i] {
                            resolved[i] = flex_size * f.factor;
                        }
                    }
                }
                break;
            }
        }

        let auto_shrink_scale = if auto_total > after_fixed && auto_total > 0.0 {
            after_fixed / auto_total
        } else {
            1.0
        };

        return tracks
            .iter()
            .enumerate()
            .map(|(i, track)| match track {
                GridTrack::Fixed(v) => *v,
                GridTrack::Percent(p) => *p * space,
                GridTrack::Fr(_) | GridTrack::Minmax(_, _) => resolved[i],
                GridTrack::Auto => {
                    let intrinsic = auto_intrinsic_widths.get(i).copied().unwrap_or(0.0);
                    intrinsic * auto_shrink_scale
                }
            })
            .collect();
    }

    // No flexible tracks: auto tracks take their intrinsic width, then split
    // the remaining space EQUALLY among themselves (additive), matching
    // Chrome's track-sizing for `auto auto` layouts.
    let (auto_extra, auto_shrink_scale) = if auto_count > 0 {
        let slack = after_fixed - auto_total;
        if slack >= 0.0 {
            (slack / auto_count as f32, 1.0)
        } else {
            // Overflow — shrink auto tracks proportionally so the row fits.
            let scale = if auto_total > 0.0 {
                after_fixed / auto_total
            } else {
                0.0
            };
            (0.0, scale)
        }
    } else {
        (0.0, 1.0)
    };

    tracks
        .iter()
        .enumerate()
        .map(|(i, track)| match track {
            GridTrack::Fixed(v) => *v,
            GridTrack::Percent(p) => *p * space,
            GridTrack::Fr(_) => 0.0,
            GridTrack::Auto => {
                let intrinsic = auto_intrinsic_widths.get(i).copied().unwrap_or(0.0);
                intrinsic * auto_shrink_scale + auto_extra
            }
            GridTrack::Minmax(min, _) => *min,
        })
        .collect()
}

/// Resolve a row track to a fixed height in points, if it is a definite size.
/// `fr`/`auto`/`minmax` rows return `None` (they fall back to auto sizing).
fn grid_track_fixed_height(track: &GridTrack) -> Option<f32> {
    match track {
        GridTrack::Fixed(v) => Some(*v),
        GridTrack::Minmax(min, _) => Some(*min),
        _ => None,
    }
}

/// The outer height a grid item wants: an explicit `height` (border-box) or
/// the measured text height plus vertical padding.
fn grid_item_outer_height(
    cs: &ComputedStyle,
    env: &mut LayoutEnv,
    child_el: &ElementNode,
    ancestors: &[AncestorInfo],
) -> f32 {
    if let Some(h) = cs.height {
        return h;
    }
    let mut runs = Vec::new();
    FlexTextRunCollector {
        runs: &mut runs,
        rules: env.rules,
        fonts: env.fonts,
    }
    .collect(&child_el.children, cs, None, (0.0, 0.0), ancestors);
    let line_h = cs.font_size * resolved_line_height_factor(cs, env.fonts);
    let text_h = if runs.is_empty() { 0.0 } else { line_h };
    text_h + cs.padding.top + cs.padding.bottom
}

/// An empty filler cell that still occupies the track height so the grid row
/// keeps its geometry when an item is absent in that column.
fn empty_grid_cell(track_h: f32) -> TableCell {
    TableCell {
        lines: Vec::new(),
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
        vertical_align: VerticalAlign::Baseline,
        min_content_height: track_h,
        hide_if_empty: false,
        grid_inset: None,
    }
}

/// Compute the painted-box inset of a grid item within its track cell from the
/// per-axis `justify-items` (inline) and `align-items` (block) keywords. Only
/// applies when the item has an explicit size smaller than the track; otherwise
/// the item stretches to fill (returns `None`).
fn compute_grid_inset(
    cs: &ComputedStyle,
    container: &ComputedStyle,
    track_w: f32,
    track_h: f32,
) -> Option<GridInset> {
    let item_w = cs.width;
    let item_h = cs.height;
    let justify = container.justify_items;
    let align = container.grid_align_items;

    // Stretch on both axes with no explicit size → fill the track (no inset).
    let stretch_w = item_w.is_none() && justify == GridAlign::Stretch;
    let stretch_h = item_h.is_none() && align == GridAlign::Stretch;
    if stretch_w && stretch_h {
        return None;
    }

    let box_w = item_w.unwrap_or(track_w).min(track_w);
    let box_h = item_h.unwrap_or(track_h).min(track_h);

    let offset_x = match justify {
        GridAlign::Start | GridAlign::Stretch => 0.0,
        GridAlign::End => (track_w - box_w).max(0.0),
        GridAlign::Center => ((track_w - box_w) / 2.0).max(0.0),
    };
    let offset_y = match align {
        GridAlign::Start | GridAlign::Stretch => 0.0,
        GridAlign::End => (track_h - box_h).max(0.0),
        GridAlign::Center => ((track_h - box_h) / 2.0).max(0.0),
    };
    // When stretching one axis, use the full track extent on that axis.
    let final_w = if stretch_w { track_w } else { box_w };
    let final_h = if stretch_h { track_h } else { box_h };

    Some(GridInset {
        offset_x,
        offset_y,
        width: final_w,
        height: final_h,
    })
}

/// Lay out a CSS Grid container into GridRow layout elements.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_grid_container(
    el: &ElementNode,
    style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    env: &mut LayoutEnv,
) {
    let available_width = ctx.available_width();
    // The track-sizing basis is the container's content-box width. When an
    // explicit `width` is set it wins (resolving box-sizing: a border-box
    // width already includes border+padding, so subtract them; a content-box
    // width is used directly). Otherwise fall back to the available width.
    let border_pad_w = style.border.left.width
        + style.border.right.width
        + style.padding.left
        + style.padding.right;
    let inner_width = match style.width {
        Some(w) => {
            if style.box_sizing == crate::style::computed::BoxSizing::BorderBox {
                (w - border_pad_w).max(0.0)
            } else {
                w
            }
        }
        None => available_width - style.padding.left - style.padding.right,
    };
    // The container's border-box width (used for the wrapping Container's
    // block width and to resolve horizontal margin / auto-centering).
    let border_box_w = inner_width + border_pad_w;
    // Horizontal offset of the grid container within the available width:
    // explicit `margin-left`, or centering when both side margins are auto and
    // the box is narrower than the line. Mirrors block-level positioning so the
    // grid lines up with where Chrome paints it.
    let h_offset = if style.width.is_some() && border_box_w < available_width {
        if style.margin_left_auto && style.margin_right_auto {
            (available_width - border_box_w) / 2.0
        } else if style.margin_left_auto {
            available_width - border_box_w
        } else {
            style.margin.left
        }
    } else {
        style.margin.left
    };
    let column_gap = style.column_gap;
    let row_gap = style.row_gap;

    // Number of columns is determined by the track list. Fall back to one
    // column when no explicit track definition exists.
    let num_cols = style.grid_template_columns.len().max(1);

    // Collect element children (skip text nodes) so we can measure intrinsic
    // widths per column before resolving track sizes.
    let element_children: Vec<&ElementNode> = el
        .children
        .iter()
        .filter_map(|child| {
            if let DomNode::Element(child_el) = child {
                Some(child_el)
            } else {
                None
            }
        })
        .collect();

    // Compute each child's style once and remember it alongside the element.
    let child_ancestors_base: Vec<AncestorInfo> = ancestors.to_vec();
    let mut child_ancestors: Vec<AncestorInfo> = child_ancestors_base.clone();
    child_ancestors.push(AncestorInfo {
        element: el,
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
    });

    let child_count = element_children.len();
    let child_styles: Vec<ComputedStyle> = element_children
        .iter()
        .enumerate()
        .map(|(idx, child_el)| {
            let classes = child_el.class_list();
            let selector_ctx = SelectorContext {
                ancestors: child_ancestors.clone(),
                child_index: idx,
                sibling_count: child_count,
                preceding_siblings: Vec::new(),
            };
            compute_style_with_context(
                child_el.tag,
                child_el.style_attr(),
                style,
                env.rules,
                child_el.tag_name(),
                &classes,
                child_el.id(),
                &child_el.attributes,
                &selector_ctx,
            )
        })
        .collect();

    // ---- Item placement (auto-flow) -------------------------------------
    // Each item occupies a rectangle in the column/row grid. We honour
    // explicit `grid-column: span N` and `grid-row: span N`, placing items
    // row-by-row (default) or column-by-column (`grid-auto-flow: column`).
    struct Placed {
        idx: usize,
        col: usize,
        row: usize,
        col_span: usize,
        row_span: usize,
    }
    let mut placed: Vec<Placed> = Vec::new();
    // occupancy[row][col] tracking via a HashSet-free dense grid.
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let ensure_row = |occ: &mut Vec<Vec<bool>>, r: usize, cols: usize| {
        while occ.len() <= r {
            occ.push(vec![false; cols]);
        }
    };
    let fits = |occ: &[Vec<bool>], r: usize, c: usize, rs: usize, cs: usize| -> bool {
        for rr in r..r + rs {
            if rr >= occ.len() {
                continue;
            }
            for cc in c..c + cs {
                if cc < occ[rr].len() && occ[rr][cc] {
                    return false;
                }
            }
        }
        true
    };

    if style.grid_auto_flow_column {
        // Column-major flow: fill down each column, then move right. Rows are
        // bounded by the explicit row-track count (fallback 1).
        let num_rows = style.grid_template_rows.len().max(1);
        let mut col = 0usize;
        let mut row = 0usize;
        for (idx, cs) in child_styles.iter().enumerate() {
            let row_span = cs.grid_row_span.max(1).min(num_rows);
            if row + row_span > num_rows {
                row = 0;
                col += 1;
            }
            placed.push(Placed {
                idx,
                col,
                row,
                col_span: 1,
                row_span,
            });
            row += row_span;
        }
    } else {
        // Row-major flow (default).
        let mut cursor_col = 0usize;
        let mut cursor_row = 0usize;
        for (idx, cs) in child_styles.iter().enumerate() {
            let col_span = cs.grid_column_span.max(1).min(num_cols);
            let row_span = cs.grid_row_span.max(1);
            // Advance to the next free slot that fits this item's span.
            loop {
                if cursor_col + col_span > num_cols {
                    cursor_col = 0;
                    cursor_row += 1;
                    continue;
                }
                ensure_row(&mut occupied, cursor_row + row_span - 1, num_cols);
                if fits(&occupied, cursor_row, cursor_col, row_span, col_span) {
                    break;
                }
                cursor_col += 1;
            }
            for rr in cursor_row..cursor_row + row_span {
                ensure_row(&mut occupied, rr, num_cols);
                occupied[rr][cursor_col..cursor_col + col_span].fill(true);
            }
            placed.push(Placed {
                idx,
                col: cursor_col,
                row: cursor_row,
                col_span,
                row_span,
            });
            cursor_col += col_span;
        }
    }

    let num_rows = placed.iter().map(|p| p.row + p.row_span).max().unwrap_or(0);

    // ---- Track sizing ---------------------------------------------------
    // Columns: existing resolver (auto tracks measure max-content width of
    // the items that start in that column).
    let mut auto_intrinsic_widths = vec![0.0_f32; num_cols];
    for (i, track) in style.grid_template_columns.iter().enumerate() {
        if !matches!(track, GridTrack::Auto) {
            continue;
        }
        let mut max_w: f32 = 0.0;
        for p in placed.iter().filter(|p| p.col == i && p.col_span == 1) {
            let cs = &child_styles[p.idx];
            let child_el = element_children[p.idx];
            let mut runs = Vec::new();
            FlexTextRunCollector {
                runs: &mut runs,
                rules: env.rules,
                fonts: env.fonts,
            }
            .collect(&child_el.children, cs, None, (0.0, 0.0), &child_ancestors);
            let text_w = super::helpers::measure_runs_width(&runs, env.fonts);
            let cell_w = text_w
                + cs.padding.left
                + cs.padding.right
                + cs.border.left.width
                + cs.border.right.width;
            max_w = max_w.max(cell_w);
        }
        auto_intrinsic_widths[i] = max_w;
    }

    let col_widths = resolve_grid_columns(
        &style.grid_template_columns,
        inner_width,
        column_gap,
        &auto_intrinsic_widths,
    );

    // Rows: explicit template-rows first, then grid-auto-rows for implicit
    // rows, then content height as a final fallback.
    let mut row_heights = vec![0.0_f32; num_rows];
    for (r, h) in row_heights.iter_mut().enumerate() {
        let explicit = style
            .grid_template_rows
            .get(r)
            .and_then(grid_track_fixed_height);
        *h = explicit.or(style.grid_auto_rows).unwrap_or(0.0);
    }
    // Grow rows to fit any item content / explicit item height that exceeds
    // the track height (auto rows, or items taller than their fixed track).
    for p in &placed {
        let cs = &child_styles[p.idx];
        let item_h = grid_item_outer_height(cs, env, element_children[p.idx], &child_ancestors);
        if p.row_span == 1 {
            let r = p.row;
            if r < row_heights.len() && item_h > row_heights[r] {
                row_heights[r] = item_h;
            }
        }
    }

    // Natural content-box height of the grid: the resolved row tracks plus the
    // row gaps between them. With fixed row tracks (no fr/auto growth), this is
    // the height the grid rows actually occupy; any surplus from an explicit
    // container `height` stays as blank free space below the last row (Chrome's
    // default `align-content: start` for definite tracks), rather than being
    // absorbed by stretching the tracks.
    let content_height: f32 =
        row_heights.iter().sum::<f32>() + row_gap * num_rows.saturating_sub(1) as f32;
    // Honour an explicit container `height` so the container's border-box ends
    // where Chrome paints it (and any free space below the last row is left
    // blank), mirroring the block-level convention where a Container's
    // `block_height` is a border-box value compared against a content height
    // that already includes the border.
    let border_v = style.border.top.width + style.border.bottom.width;
    let block_height = style.height.map(|_| {
        let padding_box_h = super::helpers::resolve_padding_box_height(
            content_height,
            style.height,
            style.padding.top,
            style.padding.bottom,
            border_v,
            style.box_sizing,
        );
        padding_box_h + border_v
    });

    // Helper to compute the x-offset of a column index.
    let col_x =
        |c: usize| -> f32 { col_widths.iter().take(c).sum::<f32>() + column_gap * c as f32 };
    let span_width = |c: usize, cs: usize| -> f32 {
        let w: f32 = col_widths.iter().skip(c).take(cs).sum();
        w + column_gap * cs.saturating_sub(1) as f32
    };

    // ---- Build one GridRow per grid row --------------------------------
    // Each GridRow holds cells positioned by column (using colspan for the
    // resolved per-cell widths) with min_content_height forcing the row's
    // track height. Items that start on a later row are emitted on that row;
    // multi-row items are approximated by emitting on their starting row with
    // a min height covering the spanned tracks.
    let mut grid_children: Vec<LayoutElement> = Vec::new();
    for row in 0..num_rows {
        let track_h = row_heights[row];
        let mut cells: Vec<TableCell> = Vec::new();
        let mut next_col = 0usize;

        // Items whose top-left lands on this row, in column order.
        let mut row_items: Vec<&Placed> = placed.iter().filter(|p| p.row == row).collect();
        row_items.sort_by_key(|p| p.col);

        for p in row_items {
            // Pad with empty filler cells up to this item's column.
            while next_col < p.col {
                cells.push(empty_grid_cell(track_h));
                next_col += 1;
            }
            let cs = &child_styles[p.idx];
            let child_el = element_children[p.idx];

            let track_w = span_width(p.col, p.col_span);
            // Height the item's cell box must occupy in the flow (covers the
            // spanned row tracks plus the gaps between them).
            let spanned_h: f32 = (row..(row + p.row_span).min(row_heights.len()))
                .map(|r| row_heights[r])
                .sum::<f32>()
                + row_gap * (p.row_span.saturating_sub(1)) as f32;

            let cell_inner = (track_w - cs.padding.left - cs.padding.right).max(1.0);
            let mut runs = Vec::new();
            FlexTextRunCollector {
                runs: &mut runs,
                rules: env.rules,
                fonts: env.fonts,
            }
            .collect(&child_el.children, cs, None, (0.0, 0.0), &child_ancestors);
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    cell_inner,
                    cs.font_size,
                    resolved_line_height_factor(cs, env.fonts),
                    cs.overflow_wrap,
                )
                .with_rtl(cs.direction_rtl),
                env.fonts,
            );

            let bg = cs
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgba());

            // Per-item alignment: when the item has an explicit smaller size
            // than its track, position the painted box per justify/align-items.
            // A row-spanning item must paint across its spanned tracks without
            // inflating the starting row, so it always carries an explicit
            // inset covering `spanned_h` (the row keeps the single track
            // height via `min_content_height`).
            let inset = if p.row_span > 1 {
                Some(
                    compute_grid_inset(cs, style, track_w, spanned_h).unwrap_or(GridInset {
                        offset_x: 0.0,
                        offset_y: 0.0,
                        width: track_w,
                        height: spanned_h,
                    }),
                )
            } else {
                compute_grid_inset(cs, style, track_w, spanned_h)
            };
            let cell_min_h = if p.row_span > 1 { track_h } else { spanned_h };

            cells.push(TableCell {
                lines,
                nested_rows: Vec::new(),
                bold: cs.font_weight == FontWeight::Bold,
                background_color: bg,
                padding_top: cs.padding.top,
                padding_right: cs.padding.right,
                padding_bottom: cs.padding.bottom,
                padding_left: cs.padding.left,
                colspan: p.col_span.max(1),
                rowspan: 1,
                border: LayoutBorder::from_computed(&cs.border),
                text_align: cs.text_align,
                vertical_align: cs.vertical_align,
                min_content_height: cell_min_h,
                hide_if_empty: false,
                grid_inset: inset,
            });
            next_col = p.col + p.col_span;
        }

        // Fill trailing columns.
        while next_col < num_cols {
            cells.push(empty_grid_cell(track_h));
            next_col += 1;
        }

        let margin_top = if row == 0 { 0.0 } else { row_gap };

        grid_children.push(LayoutElement::GridRow {
            cells,
            col_widths: col_widths.clone(),
            gap: column_gap,
            margin_top,
            margin_bottom: 0.0,
            border: LayoutBorder::default(),
            padding_left: 0.0,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
        });
    }
    let _ = col_x;

    // Wrap all grid rows in a Container that carries the border, padding,
    // and background of the grid container element.
    let bg = style
        .background_color
        .map(|c: crate::types::Color| c.to_f32_rgba());
    let BackgroundFields {
        gradient: background_gradient,
        radial_gradient: background_radial_gradient,
        svg: background_svg,
        blur_radius: background_blur_radius,
        size: background_size,
        position: background_position,
        repeat: background_repeat,
        origin: background_origin,
    } = BackgroundFields::from_style(style);
    output.push(LayoutElement::Container {
        children: grid_children,
        background_color: bg,
        border: LayoutBorder::from_computed(&style.border),
        border_radius: style.border_radius,
        padding_top: style.padding.top,
        padding_bottom: style.padding.bottom,
        padding_left: style.padding.left,
        padding_right: style.padding.right,
        margin_top: style.margin.top,
        margin_bottom: style.margin.bottom,
        block_width: Some(border_box_w),
        block_height,
        opacity: style.opacity,
        mix_blend_mode: style.mix_blend_mode,
        background_blend_mode: style.background_blend_mode,
        visible: style.visibility == Visibility::Visible,
        float: style.float,
        clear: style.clear,
        position: style.position,
        offset_top: 0.0,
        offset_left: h_offset,
        overflow: style.overflow,
        transform: style.transform,
        transform_origin: style.transform_origin,
        clip_path: style.clip_path.clone(),
        box_shadow: style.box_shadow.clone(),
        background_gradient,
        background_radial_gradient,
        background_svg,
        background_blur_radius,
        background_size,
        background_position,
        background_repeat,
        background_origin,
        outline_width: style.outline_width,
        outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
        z_index: style.z_index,
    });
}
