//! CSS multi-column layout (`column-count` / `column-width` / `columns`).
//!
//! Implements a column-major *balanced* flow: the container's block-level
//! children are laid out top-to-bottom filling column 1, then column 2, etc.,
//! with the content distributed so the columns end up roughly equal height.
//! Each column is emitted as an absolutely-positioned [`LayoutElement::Container`]
//! at its computed x-offset inside the multicol element's padding box, so the
//! columns sit side-by-side without participating in the parent's vertical flow.
//!
//! Supported:
//! - `column-count`, `column-width`, and the `columns` shorthand (used-column
//!   count derived per the CSS spec when both/either are present).
//! - `column-gap` (with a `normal` default of 1em).
//! - `column-rule` painted as a vertical stroke centered in each gap; `solid`
//!   paints as a filled bar, `dashed`/`dotted`/`double` as the matching styled
//!   line. The rule spans the full content box of a definite-height container.
//! - `column-span: all` — a child spans every column as a full-width band that
//!   breaks the balanced flow (content before/after balances independently).
//! - `column-fill`: `balance` (default, equal column heights) and `auto`
//!   (sequential fill to the container height, last column short). Under
//!   `auto` with a definite height, a simple block box that crosses a column
//!   boundary is *fragmented* (css-break-3 + css-multicol-1): the part that
//!   fits stays at the bottom of column N and the remainder continues at the
//!   top of N+1, with `box-decoration-break: slice` borders at the cut.
//! - Margins adjoining a column fragmentation break are truncated (css-break-3
//!   §4.2): the trailing `margin-bottom` of the last box in every column except
//!   the last (in document order) is dropped from the column's used height.
//! - In the `balance` path each top-level child is an atomic unit that is never
//!   split across a column boundary (so `break-inside: avoid` is honored).

use crate::parser::css::AncestorInfo;
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::{BorderStyle, ComputedStyle, Position, Visibility};

use super::context::{LayoutContext, LayoutEnv};
use super::engine::{BackgroundFields, LayoutBorder, LayoutElement, flatten_element};
use super::paginate::estimate_element_height;

/// A single laid-out top-level child of the multicol element.
struct MultiColItem {
    /// The flattened layout elements for this child (usually one Container or
    /// TextBlock, but text/anonymous content may produce several).
    elements: Vec<LayoutElement>,
    /// Outer (margin-box) height used for balancing.
    height: f32,
    /// The item's trailing `margin-bottom` (the last in-flow element's bottom
    /// margin). Truncated at a column-fragment break per css-break-3 §4.2.
    margin_bottom: f32,
    /// `column-span: all` — render as a full-width band, not inside a column.
    span_all: bool,
}

/// The trailing `margin-bottom` of a laid-out item: the bottom margin of its
/// last in-flow (non-absolute) layout element, which is what adjoins a column
/// fragmentation break. Returns 0.0 for elements that carry no bottom margin.
fn element_trailing_margin_bottom(elements: &[LayoutElement]) -> f32 {
    for el in elements.iter().rev() {
        match el {
            LayoutElement::TextBlock {
                margin_bottom,
                position,
                ..
            }
            | LayoutElement::Container {
                margin_bottom,
                position,
                ..
            } => {
                if *position == Position::Absolute {
                    continue;
                }
                return *margin_bottom;
            }
            LayoutElement::Image { margin_bottom, .. }
            | LayoutElement::Svg { margin_bottom, .. }
            | LayoutElement::HorizontalRule { margin_bottom, .. }
            | LayoutElement::ProgressBar { margin_bottom, .. }
            | LayoutElement::MathBlock { margin_bottom, .. }
            | LayoutElement::TableRow { margin_bottom, .. }
            | LayoutElement::GridRow { margin_bottom, .. }
            | LayoutElement::FlexRow { margin_bottom, .. } => return *margin_bottom,
            // No bottom margin to truncate (e.g. PageBreak); skip it.
            _ => continue,
        }
    }
    0.0
}

/// Lay out a multi-column container, replacing the previous grid-emulation path.
pub(crate) fn layout_multicol_container(
    el: &ElementNode,
    style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutElement>,
    ancestors: &[AncestorInfo],
    positioned_depth: usize,
    env: &mut LayoutEnv,
) {
    let available_width = ctx.available_width();
    let border_pad_w = style.border.left.width
        + style.border.right.width
        + style.padding.left
        + style.padding.right;

    // Content-box (inner) width: explicit `width` wins (resolving box-sizing),
    // else available width minus padding.
    let inner_width = match style.width {
        Some(w) => {
            if style.box_sizing == crate::style::computed::BoxSizing::BorderBox {
                (w - border_pad_w).max(0.0)
            } else {
                w
            }
        }
        None => (available_width - style.padding.left - style.padding.right).max(0.0),
    };
    let border_box_w = inner_width + border_pad_w;

    // Horizontal placement of the container within the available width: explicit
    // margin-left, or centering when both side margins are auto.
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

    // Column gap: `normal` resolves to 1em (the element's font-size, in pt).
    let gap = if style.column_gap_is_normal {
        style.font_size
    } else {
        style.column_gap
    };

    // Resolve the used number of columns and the per-column width, following the
    // CSS multicol "pseudo-algorithm" (simplified): with both column-count N and
    // column-width W, use up to N columns each at least W wide; with only one,
    // derive the other from the available inner width.
    let (num_cols, col_width) = resolve_columns(style, inner_width, gap);
    if num_cols < 1 {
        return;
    }

    // ---- Lay out each top-level child into its own buffer at column width ----
    let col_ctx = ctx.with_parent(col_width, None, style.font_size);
    let full_ctx = ctx.with_parent(inner_width, None, style.font_size);

    let mut child_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        element: el,
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    });

    let element_count = el
        .children
        .iter()
        .filter(|n| matches!(n, DomNode::Element(_)))
        .count();

    let mut items: Vec<MultiColItem> = Vec::new();
    let mut element_index = 0usize;
    let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
    for node in &el.children {
        let DomNode::Element(child_el) = node else {
            continue;
        };
        // Decide span-all by computing the child's style cheaply.
        let span_all = child_span_all(
            child_el,
            style,
            env,
            &child_ancestors,
            element_index,
            element_count,
            &preceding_siblings,
        );
        let item_ctx = if span_all { &full_ctx } else { &col_ctx };

        let mut buf: Vec<LayoutElement> = Vec::new();
        flatten_element(
            child_el,
            style,
            item_ctx,
            &mut buf,
            None,
            &child_ancestors,
            positioned_depth,
            element_index,
            element_count,
            &preceding_siblings,
            &[],
            env,
        );
        let height: f32 = buf.iter().map(estimate_element_height).sum();
        let margin_bottom = element_trailing_margin_bottom(&buf);
        items.push(MultiColItem {
            elements: buf,
            height,
            margin_bottom,
            span_all,
        });

        preceding_siblings.push((
            child_el.tag_name().to_string(),
            child_el
                .class_list()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        ));
        element_index += 1;
    }

    // ---- Distribute items into columns (balanced, span-all as a band) -------
    // The output is a sequence of "segments": each segment is either a
    // full-width band (one span-all item) or a balanced multicol run. We track
    // the running vertical cursor so successive segments stack.
    let pad_left = style.border.left.width + style.padding.left;
    let pad_top = style.border.top.width + style.padding.top;
    // Column/band/rule containers are emitted as `Position::Absolute` children of
    // the multicol wrapper. The renderer places absolute children at the wrapper's
    // PADDING-box origin (CSS §10.1), so their offsets must be padding-box-relative:
    // strip the wrapper border from the border-box-relative cursors below. The
    // height accounting (`cursor_y`/`max_bottom`) stays in border-box coordinates.
    let bl = style.border.left.width;
    let bt = style.border.top.width;

    // Explicit border-box height (if any) resolved up front so the column rule
    // can span the full content box of a definite-height multicol container
    // (CSS Multicol §6: the rule is as tall as the column box, and in a
    // definite-height container the columns fill the content box).
    let explicit_border_box_h = style.height.map(|h| {
        if style.box_sizing == crate::style::computed::BoxSizing::BorderBox {
            h
        } else {
            h + style.border.vertical_width() + style.padding.top + style.padding.bottom
        }
    });

    let mut column_children: Vec<LayoutElement> = Vec::new();
    // A pending rule span recorded per balanced run: (rule_x, run_top, run_h).
    // Emitted after the loop so a single-run, definite-height container can have
    // its rules stretched to the full content-box height.
    let mut rule_spans: Vec<(f32, f32, f32)> = Vec::new();
    let mut run_count = 0usize;
    let mut cursor_y = pad_top; // distance from border-box top to current band top
    let mut max_bottom = pad_top;

    let mut i = 0usize;
    while i < items.len() {
        if items[i].span_all {
            // Full-width band spanning all columns.
            let band_h = items[i].height;
            let band = make_band_container(
                std::mem::take(&mut items[i].elements),
                pad_left - bl,
                cursor_y - bt,
                inner_width,
                band_h,
            );
            column_children.push(band);
            cursor_y += band_h;
            max_bottom = max_bottom.max(cursor_y);
            i += 1;
            continue;
        }

        // Gather a run of consecutive non-span items.
        let run_start = i;
        while i < items.len() && !items[i].span_all {
            i += 1;
        }
        let run = &mut items[run_start..i];

        // `column-fill: auto` with a definite height fills each column to the
        // content-box height in turn, *fragmenting* a block that crosses a column
        // boundary (the part that fits stays in column N, the rest continues at
        // the top of N+1). Only used when every item in the run is a simple,
        // slice-able block box; otherwise (or for `balance`) fall back to the
        // atomic bucket distribution.
        let fill_h_auto = match (style.column_fill_auto, explicit_border_box_h) {
            (true, Some(bh)) => Some(
                (bh - style.padding.top - style.padding.bottom - style.border.vertical_width())
                    .max(0.0),
            ),
            _ => None,
        };
        let use_fragmentation = fill_h_auto.is_some()
            && num_cols > 1
            && run.iter().all(item_is_splittable)
            && !run.is_empty();

        let mut run_max_h = 0.0f32;
        if let (true, Some(fill_h)) = (use_fragmentation, fill_h_auto) {
            // ---- Fragmenting fill path (column-fill: auto) -----------------
            let run_indices: Vec<usize> = (0..run.len()).collect();
            let (frag_cols, used) = fragment_columns_auto(run, &run_indices, num_cols, fill_h);
            for (c, frags) in frag_cols.iter().enumerate() {
                if frags.is_empty() {
                    continue;
                }
                run_max_h = run_max_h.max(used[c]);
                let col_x = pad_left + c as f32 * (col_width + gap);
                let mut col_kids: Vec<LayoutElement> = Vec::new();
                for f in frags {
                    col_kids.push(make_fragment_box(
                        &run[f.item].elements[0],
                        0.0,
                        f.y,
                        col_width,
                        f.height,
                        f.is_first,
                        f.is_last,
                    ));
                }
                column_children.push(make_column_container(
                    col_kids,
                    col_x - bl,
                    cursor_y - bt,
                    col_width,
                    used[c],
                ));
            }
        } else {
            // ---- Atomic bucket path (balance, or non-slice-able auto) ------
            let heights: Vec<f32> = run.iter().map(|it| it.height).collect();
            let buckets = match fill_h_auto {
                Some(fill_h) => fill_columns(&heights, num_cols, fill_h),
                None => balance_columns(&heights, num_cols),
            };

            // The last non-empty column in document order is the natural end of
            // the run's content: its trailing margin is NOT adjoining a
            // fragmentation break and is kept. Every earlier column ends at a
            // column break, so the bottom margin of its last item is truncated
            // (css-break-3 §4.2). This shortens those columns' used height, which
            // is what the container's auto height (`run_max_h`) is measured
            // against — matching Chrome.
            let last_nonempty_col = buckets
                .iter()
                .rposition(|b| !b.is_empty())
                .unwrap_or(usize::MAX);

            for (c, bucket) in buckets.iter().enumerate() {
                if bucket.is_empty() {
                    continue;
                }
                let col_x = pad_left + c as f32 * (col_width + gap);
                let mut col_kids: Vec<LayoutElement> = Vec::new();
                let mut col_height = 0.0f32;
                for &idx in bucket {
                    col_height += run[idx].height;
                    col_kids.append(&mut run[idx].elements);
                }
                // Used column height for sizing: drop the trailing margin at a break.
                let used_col_height = if c != last_nonempty_col {
                    let last_idx = *bucket.last().unwrap();
                    (col_height - run[last_idx].margin_bottom).max(0.0)
                } else {
                    col_height
                };
                run_max_h = run_max_h.max(used_col_height);
                column_children.push(make_column_container(
                    col_kids,
                    col_x - bl,
                    cursor_y - bt,
                    col_width,
                    col_height,
                ));
            }
        }

        // Record one rule span per gap for this run; the final height is decided
        // after the loop (full content box for a single definite-height run,
        // otherwise the run's column-content height).
        if style.column_rule.width > 0.0
            && style.column_rule.style != BorderStyle::None
            && num_cols > 1
        {
            let rule_w = style.column_rule.width;
            for c in 0..num_cols - 1 {
                // Center of the gap between column c and c+1.
                let gap_center = pad_left + (c + 1) as f32 * col_width + c as f32 * gap + gap / 2.0;
                let rule_x = gap_center - rule_w / 2.0;
                rule_spans.push((rule_x, cursor_y, run_max_h));
            }
        }
        run_count += 1;

        cursor_y += run_max_h;
        max_bottom = max_bottom.max(cursor_y);
    }

    // Emit the recorded column rules. Per CSS Multicol §6 the rule is as tall as
    // the column box. In a definite-height container with a single balanced run
    // the columns fill the content box, so the rule spans from the content-box
    // top (`pad_top`) to its bottom (matching Chrome, which paints the rule the
    // full height of the box rather than only the filled content).
    if !rule_spans.is_empty() {
        let rule_w = style.column_rule.width;
        let rule_color = style.column_rule.color.unwrap_or(style.color).to_f32_rgba();
        // Content-box bottom (border-box coords) when the height is definite.
        let content_box_bottom =
            explicit_border_box_h.map(|bh| bh - style.padding.bottom - style.border.bottom.width);
        for (rule_x, run_top, run_h) in rule_spans {
            let (rule_top, rule_h) = match content_box_bottom {
                // Single balanced run + definite height: span the whole content
                // box (top padding edge → bottom padding edge).
                Some(bottom) if run_count == 1 => (pad_top, (bottom - pad_top).max(run_h)),
                // Multiple runs (broken by span-all bands) or auto height: the
                // rule is as tall as this run's columns.
                _ => (run_top, run_h),
            };
            column_children.push(make_rule_container(
                rule_x - bl,
                rule_top - bt,
                rule_w,
                rule_h,
                rule_color,
                style.column_rule.style,
            ));
        }
    }

    // ---- Outer container height --------------------------------------------
    // An explicit height wins; otherwise size to the tallest column run plus
    // the bottom padding.
    let content_box_h = max_bottom + style.padding.bottom + style.border.bottom.width;
    let block_height = Some(explicit_border_box_h.unwrap_or(content_box_h));

    // ---- Emit the wrapping container ---------------------------------------
    let bg = style
        .background_color
        .map(|c: crate::types::Color| c.to_f32_rgba());
    let BackgroundFields {
        gradient: background_gradient,
        radial_gradient: background_radial_gradient,
        conic_gradient: background_conic_gradient,
        svg: background_svg,
        blur_radius: background_blur_radius,
        size: background_size,
        position: background_position,
        repeat: background_repeat,
        origin: background_origin,
        clip: background_clip,
    } = BackgroundFields::from_style(style);

    output.push(LayoutElement::Container {
        box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
        children: column_children,
        background_color: bg,
        border: LayoutBorder::from_computed(&style.border),
        border_radius: style.border_radius,
        border_radii: style.border_radii,
        border_radii_y: style.border_radii_y,
        outline_offset: style.outline_offset,
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
        overflow_x: style.overflow_x,
        overflow_y: style.overflow_y,
        transform: style.transform,
        transform_origin: style.transform_origin,
        clip_path: style.clip_path.clone(),
        mask_image: style.mask_image.clone(),
        mask_mode: style.mask_mode,
        box_shadow: style.box_shadow.clone(),
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
        outline_width: style.outline_width,
        outline_color: style.outline_color.map(|c| c.to_f32_rgb()),
        z_index: style.z_index,
        positioned_depth: 0,
        containing_block: None,
    });
}

/// Assign items (by index, in document order) to `num_cols` columns so the
/// tallest column is as short as possible, breaking only at item boundaries
/// (an item is never split — honouring `break-inside: avoid`). This models CSS
/// `column-fill: balance`.
///
/// We binary-search the minimal feasible column height over the set of prefix
/// sums (the only heights at which a balanced fill can change), then greedily
/// pack at that height. Returns one bucket of item indices per column.
fn balance_columns(heights: &[f32], num_cols: usize) -> Vec<Vec<usize>> {
    let n = heights.len();
    if num_cols <= 1 || n == 0 {
        return vec![(0..n).collect()];
    }

    // Greedily fill columns, starting a new column whenever adding the next
    // item would exceed `limit` (a non-empty column). Returns the number of
    // columns used, or None if it doesn't fit in `num_cols`.
    let fits = |limit: f32| -> Option<usize> {
        let mut cols_used = 1usize;
        let mut col_h = 0.0f32;
        for &h in heights {
            if col_h > 0.0 && col_h + h > limit + 0.01 {
                cols_used += 1;
                col_h = 0.0;
                if cols_used > num_cols {
                    return None;
                }
            }
            col_h += h;
        }
        Some(cols_used)
    };

    // Candidate limits: each item height (a single item never splits, so the
    // limit must be at least the tallest item) and total/num_cols upward.
    let total: f32 = heights.iter().sum();
    let max_item = heights.iter().cloned().fold(0.0f32, f32::max);
    let mut lo = max_item.max(total / num_cols as f32);
    // Upper bound: the whole run in one column always fits.
    let hi = total.max(lo);
    // Search a fine grid between lo and hi for the smallest feasible limit.
    let mut best = hi;
    let steps = 256;
    let span = (hi - lo).max(0.0);
    for s in 0..=steps {
        let limit = lo + span * (s as f32 / steps as f32);
        if fits(limit).is_some() {
            best = limit;
            break;
        }
    }
    lo = best;

    // Pack at the chosen limit.
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); num_cols];
    let mut col = 0usize;
    let mut col_h = 0.0f32;
    for (idx, &h) in heights.iter().enumerate() {
        if col + 1 < num_cols && col_h > 0.0 && col_h + h > lo + 0.01 {
            col += 1;
            col_h = 0.0;
        }
        buckets[col].push(idx);
        col_h += h;
    }
    buckets
}

/// Atomic `column-fill: auto` fallback (used when a run contains a non-slice-able
/// item, e.g. an image or table): fill each column with whole items in document
/// order up to `fill_h`, then move to the next column; the last column is left
/// short. A block whose addition would overflow the current non-empty column
/// instead starts the next one (never split). The slice-able common case is
/// handled by [`fragment_columns_auto`], which fragments the crossing block.
/// Overflow past the last column piles into it.
fn fill_columns(heights: &[f32], num_cols: usize, fill_h: f32) -> Vec<Vec<usize>> {
    let n = heights.len();
    if num_cols <= 1 || n == 0 || fill_h <= 0.0 {
        return vec![(0..n).collect()];
    }
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); num_cols];
    let mut col = 0usize;
    let mut col_h = 0.0f32;
    for (idx, &h) in heights.iter().enumerate() {
        if col + 1 < num_cols && col_h > 0.0 && col_h + h > fill_h + 0.01 {
            col += 1;
            col_h = 0.0;
        }
        buckets[col].push(idx);
        col_h += h;
    }
    buckets
}

/// One placed fragment of an item inside a column for `column-fill: auto`.
struct AutoFragment {
    /// Index into the run of the source item this fragment belongs to.
    item: usize,
    /// Border-box top of the fragment, relative to the column content top (px).
    y: f32,
    /// Border-box height of this fragment (px).
    height: f32,
    /// This fragment contains the item's top edge (first slice → keep top border).
    is_first: bool,
    /// This fragment contains the item's bottom edge (last slice → keep bottom
    /// border + the item's trailing margin extends below it within the column).
    is_last: bool,
}

/// Distribute items across `num_cols` for `column-fill: auto`, *fragmenting* a
/// block that crosses a column boundary (css-break-3 + css-multicol-1): the part
/// that fits stays at the bottom of column N, the remainder continues at the top
/// of column N+1. Each column is filled to `fill_h` before the next is started.
///
/// A box's content height is its border-box height (`item.height - margin`); the
/// trailing margin follows the box and is *truncated* if it would cross the
/// column bottom (a margin adjoining a fragmentation break is dropped). Splitting
/// follows `box-decoration-break: slice`: the top slice keeps the top border, the
/// bottom slice keeps the bottom border, and the cut edge gets neither.
///
/// Returns one `Vec<AutoFragment>` per column (in document order) plus the used
/// content height of each column (the bottom of its last fragment, including a
/// kept trailing margin).
fn fragment_columns_auto(
    items: &[MultiColItem],
    indices: &[usize],
    num_cols: usize,
    fill_h: f32,
) -> (Vec<Vec<AutoFragment>>, Vec<f32>) {
    let mut cols: Vec<Vec<AutoFragment>> = (0..num_cols).map(|_| Vec::new()).collect();
    let mut used: Vec<f32> = vec![0.0; num_cols];
    if num_cols == 0 || fill_h <= 0.0 {
        // Degenerate: pile everything into column 0 unfragmented.
        let mut y = 0.0f32;
        for &idx in indices {
            let h = (items[idx].height - items[idx].margin_bottom).max(0.0);
            cols[0].push(AutoFragment {
                item: idx,
                y,
                height: h,
                is_first: true,
                is_last: true,
            });
            y += items[idx].height;
        }
        used[0] = y;
        return (cols, used);
    }

    let mut col = 0usize;
    let mut y = 0.0f32; // border-box fill cursor within the current column
    for &idx in indices {
        let margin = items[idx].margin_bottom.max(0.0);
        let box_h = (items[idx].height - margin).max(0.0);
        let mut remaining = box_h;
        let mut first_slice = true;
        // Place the box, splitting across columns as needed.
        loop {
            let space = fill_h - y;
            // Start a new column when the current one is full and this is not the
            // last column (the last column absorbs any overflow).
            if space <= 0.01 && col + 1 < num_cols {
                col += 1;
                y = 0.0;
                continue;
            }
            let space = fill_h - y;
            let last_col = col + 1 >= num_cols;
            let take = if last_col {
                remaining
            } else {
                remaining.min(space.max(0.0))
            };
            let is_last_slice = (remaining - take).abs() <= 0.01 || last_col;
            cols[col].push(AutoFragment {
                item: idx,
                y,
                height: take,
                is_first: first_slice,
                is_last: is_last_slice,
            });
            y += take;
            used[col] = used[col].max(y);
            remaining -= take;
            first_slice = false;
            if remaining <= 0.01 || last_col {
                break;
            }
            // Box continues in the next column.
            col += 1;
            y = 0.0;
        }
        // Trailing margin follows the box: kept if it fits in the column, else
        // truncated at the fragmentation break (do not carry it to the next col).
        if margin > 0.0 {
            if y + margin <= fill_h + 0.01 || col + 1 >= num_cols {
                y += margin;
                used[col] = used[col].max(y);
            } else {
                // Margin adjoins the column break → truncated; next item starts a
                // fresh column at the top.
                col += 1;
                y = 0.0;
            }
        }
    }
    (cols, used)
}

/// Resolve the used number of columns and per-column width from the
/// `column-count` / `column-width` properties and the inner content width.
fn resolve_columns(style: &ComputedStyle, inner_width: f32, gap: f32) -> (usize, f32) {
    let count = style.column_count;
    let width = style.column_width.filter(|w| *w > 0.0);

    let n = match (count, width) {
        (Some(c), Some(w)) => {
            // Use at most `c` columns, but no more than fit at the ideal width.
            let fit = ((inner_width + gap) / (w + gap)).floor() as i32;
            (c as i32).min(fit.max(1)).max(1) as usize
        }
        (Some(c), None) => (c.max(1)) as usize,
        (None, Some(w)) => {
            let fit = ((inner_width + gap) / (w + gap)).floor() as i32;
            fit.max(1) as usize
        }
        (None, None) => 1,
    };
    // Equal columns filling the inner width: colW = (inner - (n-1)*gap) / n.
    let col_width = ((inner_width - (n.saturating_sub(1)) as f32 * gap) / n as f32).max(0.0);
    (n, col_width)
}

/// Build an absolutely-positioned column container at `(off_left, off_top)`
/// from the multicol element's border-box top-left, holding `kids` in flow.
fn make_column_container(
    kids: Vec<LayoutElement>,
    off_left: f32,
    off_top: f32,
    width: f32,
    height: f32,
) -> LayoutElement {
    empty_abs_container(kids, off_left, off_top, width, height, None)
}

/// Whether an item is a single block box (one Container or TextBlock) that the
/// `column-fill: auto` fragmenter can geometrically slice across a column break.
/// Anything else (multiple flattened elements, images, tables, …) is treated as
/// atomic and never split.
fn item_is_splittable(item: &MultiColItem) -> bool {
    item.elements.len() == 1
        && matches!(
            item.elements[0],
            LayoutElement::Container { .. } | LayoutElement::TextBlock { .. }
        )
}

/// Build one positioned fragment box for a `column-fill: auto` slice of an item.
///
/// Clones the item's single block element, repositions it as an absolute box at
/// `off_top` (column-content-relative), forces its border-box height to the slice
/// height, and applies `box-decoration-break: slice` borders: the first slice
/// keeps the top border, the last slice keeps the bottom border, and any cut edge
/// drops its border. The slice is clipped to its own box so inner content does
/// not spill past the cut.
fn make_fragment_box(
    src: &LayoutElement,
    off_left: f32,
    off_top: f32,
    width: f32,
    height: f32,
    is_first: bool,
    is_last: bool,
) -> LayoutElement {
    let mut el = src.clone();
    match &mut el {
        LayoutElement::Container {
            border,
            block_width,
            block_height,
            margin_top,
            margin_bottom,
            position,
            offset_top,
            offset_left,
            overflow,
            overflow_x,
            overflow_y,
            padding_top,
            padding_bottom,
            ..
        } => {
            if !is_first {
                border.top = crate::layout::engine::LayoutBorderSide::default();
                *padding_top = 0.0;
            }
            if !is_last {
                border.bottom = crate::layout::engine::LayoutBorderSide::default();
                *padding_bottom = 0.0;
            }
            *block_width = Some(width);
            *block_height = Some(height);
            *margin_top = 0.0;
            *margin_bottom = 0.0;
            *position = Position::Absolute;
            *offset_top = off_top;
            *offset_left = off_left;
            // Clip the slice to its box so any inner content respects the cut.
            *overflow = crate::style::computed::Overflow::Hidden;
            *overflow_x = crate::style::computed::Overflow::Hidden;
            *overflow_y = crate::style::computed::Overflow::Hidden;
        }
        LayoutElement::TextBlock {
            border,
            block_width,
            block_height,
            margin_top,
            margin_bottom,
            position,
            offset_top,
            offset_left,
            padding_top,
            padding_bottom,
            clip_rect,
            ..
        } => {
            if !is_first {
                border.top = crate::layout::engine::LayoutBorderSide::default();
                *padding_top = 0.0;
            }
            if !is_last {
                border.bottom = crate::layout::engine::LayoutBorderSide::default();
                *padding_bottom = 0.0;
            }
            *block_width = Some(width);
            *block_height = Some(height);
            *margin_top = 0.0;
            *margin_bottom = 0.0;
            *position = Position::Absolute;
            *offset_top = off_top;
            *offset_left = off_left;
            // Clip the slice to its box so any text outside the cut is hidden.
            *clip_rect = Some((off_left, off_top, width, height));
        }
        _ => {}
    }
    el
}

/// Build a full-width band (for `column-span: all`) at the current cursor.
fn make_band_container(
    kids: Vec<LayoutElement>,
    off_left: f32,
    off_top: f32,
    width: f32,
    height: f32,
) -> LayoutElement {
    empty_abs_container(kids, off_left, off_top, width, height, None)
}

/// Build a rule box spanning a column gap.
///
/// A `solid` rule paints as a filled bar (the simplest faithful match). Other
/// border styles (`dashed`/`dotted`/`double`) are carried on the box's LEFT
/// border so the renderer's styled-line path (`paint_column_rule_line`) draws
/// them with the correct dash/dot/double pattern instead of a solid fill.
fn make_rule_container(
    off_left: f32,
    off_top: f32,
    width: f32,
    height: f32,
    color: (f32, f32, f32, f32),
    rule_style: BorderStyle,
) -> LayoutElement {
    if rule_style == BorderStyle::Solid {
        return empty_abs_container(Vec::new(), off_left, off_top, width, height, Some(color));
    }
    let mut el = empty_abs_container(Vec::new(), off_left, off_top, width, height, None);
    if let LayoutElement::Container { border, .. } = &mut el {
        border.left = crate::layout::engine::LayoutBorderSide {
            width,
            color: (color.0, color.1, color.2),
            style: rule_style,
            alpha: color.3,
        };
    }
    el
}

/// Shared constructor for an absolutely-positioned, border/padding-free
/// container used for columns, bands, and rules.
fn empty_abs_container(
    kids: Vec<LayoutElement>,
    off_left: f32,
    off_top: f32,
    width: f32,
    height: f32,
    bg: Option<(f32, f32, f32, f32)>,
) -> LayoutElement {
    LayoutElement::Container {
        box_decoration_break: crate::style::computed::BoxDecorationBreak::Slice,
        children: kids,
        background_color: bg,
        border: LayoutBorder::default(),
        border_radius: 0.0,
        border_radii: [0.0; 4],
        border_radii_y: [0.0; 4],
        outline_offset: 0.0,
        padding_top: 0.0,
        padding_bottom: 0.0,
        padding_left: 0.0,
        padding_right: 0.0,
        margin_top: 0.0,
        margin_bottom: 0.0,
        block_width: Some(width),
        block_height: Some(height),
        opacity: 1.0,
        mix_blend_mode: crate::style::computed::BlendMode::Normal,
        background_blend_mode: crate::style::computed::BlendMode::Normal,
        visible: true,
        float: crate::style::computed::Float::None,
        clear: crate::style::computed::Clear::None,
        position: Position::Absolute,
        offset_top: off_top,
        offset_left: off_left,
        overflow: crate::style::computed::Overflow::Visible,
        overflow_x: crate::style::computed::Overflow::Visible,
        overflow_y: crate::style::computed::Overflow::Visible,
        transform: None,
        transform_origin: crate::style::computed::TransformOrigin::default(),
        clip_path: None,
        mask_image: None,
        mask_mode: crate::style::computed::MaskMode::default(),
        box_shadow: Vec::new(),
        background_gradient: None,
        background_radial_gradient: None,
        background_conic_gradient: None,
        background_svg: None,
        background_blur_radius: 0.0,
        background_size: crate::style::computed::BackgroundSize::Auto,
        background_position: crate::style::computed::BackgroundPosition::default(),
        background_repeat: crate::style::computed::BackgroundRepeat::Repeat,
        background_origin: crate::style::computed::BackgroundOrigin::Padding,
        background_clip: crate::style::computed::BackgroundClip::Border,
        outline_width: 0.0,
        outline_color: None,
        z_index: 0,
        positioned_depth: 0,
        containing_block: None,
    }
}

/// Compute whether a child carries `column-span: all` by resolving its style.
#[allow(clippy::too_many_arguments)]
fn child_span_all(
    child_el: &ElementNode,
    parent_style: &ComputedStyle,
    env: &LayoutEnv,
    child_ancestors: &[AncestorInfo],
    child_index: usize,
    sibling_count: usize,
    preceding_siblings: &[(String, Vec<String>)],
) -> bool {
    use crate::parser::css::SelectorContext;
    use crate::style::computed::compute_style_with_context;
    let classes = child_el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: child_ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: preceding_siblings.to_vec(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let cs = compute_style_with_context(
        child_el.tag,
        child_el.style_attr(),
        parent_style,
        env.rules,
        child_el.tag_name(),
        &classes,
        child_el.id(),
        &child_el.attributes,
        &selector_ctx,
    );
    cs.column_span_all
}
