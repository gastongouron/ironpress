use super::engine::{
    decode_footnote_link, layout_element_paint_order, table_cell_content_height, FootnoteItem,
    LayoutElement, Page, PageBreakSide, FOOTNOTE_CALL_FONT_SCALE,
};
use crate::style::computed::{
    BorderCollapse, BoxDecorationBreak, Clear, Float, ObjectFit, Position,
};
use crate::types::{Margin, PageSize};
use std::collections::HashMap;

fn advance_positioned_ancestors_after_page_break(
    positioned_y_by_depth: &mut HashMap<usize, f32>,
    consumed_height: f32,
) {
    for y in positioned_y_by_depth.values_mut() {
        *y -= consumed_height;
    }
}

fn collect_footnotes_from_element(element: &LayoutElement, out: &mut Vec<FootnoteItem>) {
    match element {
        LayoutElement::TextBlock { lines, .. } => {
            for line in lines {
                for run in &line.runs {
                    let Some((marker, text)) =
                        run.link_url.as_deref().and_then(decode_footnote_link)
                    else {
                        continue;
                    };
                    out.push(FootnoteItem {
                        marker,
                        text,
                        font_size: run.font_size / FOOTNOTE_CALL_FONT_SCALE,
                        bold: run.bold,
                        italic: run.italic,
                        color: run.color,
                        font_family: run.font_family.clone(),
                        line_height_factor: run.line_height_factor,
                    });
                }
            }
        }
        LayoutElement::Container { children, .. } => {
            for child in children {
                collect_footnotes_from_element(child, out);
            }
        }
        LayoutElement::TableRow { cells, .. } | LayoutElement::GridRow { cells, .. } => {
            for cell in cells {
                for nested in &cell.nested_rows {
                    collect_footnotes_from_element(nested, out);
                }
            }
        }
        LayoutElement::FlexRow { cells, .. } => {
            for cell in cells {
                for nested in &cell.nested_elements {
                    collect_footnotes_from_element(nested, out);
                }
            }
        }
        _ => {}
    }
}

fn footnote_reserved_height(footnotes: &[FootnoteItem]) -> f32 {
    footnotes
        .iter()
        .map(|footnote| {
            let factor = if footnote.line_height_factor.is_finite() {
                footnote.line_height_factor
            } else {
                1.2
            };
            footnote.font_size * factor
        })
        .sum()
}

/// A tracked float region for simplified float layout.
#[derive(Debug, Clone)]
struct FloatRegion {
    #[allow(dead_code)]
    y_start: f32,
    y_end: f32,
    #[allow(dead_code)]
    side: Float,
}

/// Estimate the height of a layout element for wrapper sizing.
pub(crate) fn estimate_element_height(element: &LayoutElement) -> f32 {
    estimate_element_height_bounded(element, 50)
}

fn estimate_element_height_bounded(element: &LayoutElement, depth: usize) -> f32 {
    if depth == 0 {
        return 0.0;
    }
    match element {
        LayoutElement::TextBlock {
            lines,
            margin_top,
            margin_bottom,
            padding_top,
            padding_bottom,
            border,
            block_height,
            position,
            clip_rect,
            ..
        } => {
            if *position == Position::Absolute {
                return 0.0;
            }
            let text_height: f32 = lines.iter().map(|l| l.height).sum();
            let content_h = padding_top + text_height + padding_bottom;
            // When clipping (overflow:hidden), use the specified block_height
            // instead of expanding to fit content.
            let effective_h = if clip_rect.is_some() {
                block_height.unwrap_or(content_h)
            } else {
                block_height.map_or(content_h, |h| content_h.max(h))
            };
            margin_top + effective_h + margin_bottom + border.vertical_width()
        }
        LayoutElement::FlexRow {
            row_height,
            margin_top,
            margin_bottom,
            padding_top,
            padding_bottom,
            border,
            ..
        } => {
            margin_top
                + padding_top
                + row_height
                + padding_bottom
                + margin_bottom
                + border.vertical_width()
        }
        LayoutElement::TableRow {
            cells,
            margin_top,
            margin_bottom,
            ..
        } => {
            let row_h = cells
                .iter()
                .map(table_cell_content_height)
                .fold(0.0f32, f32::max);
            margin_top + row_h + margin_bottom
        }
        LayoutElement::GridRow {
            cells,
            margin_top,
            margin_bottom,
            padding_top,
            padding_bottom,
            ..
        } => {
            // A grid row occupies its resolved track height (css-grid-1 §11),
            // carried on each cell as `min_content_height`. A grid item with a
            // definite height does NOT grow its track when its content is taller
            // (the content overflows), so the row height must not be inflated by
            // the cells' intrinsic content height the way a table row's is.
            let row_h = cells
                .iter()
                .map(|cell| cell.min_content_height)
                .fold(0.0f32, f32::max);
            margin_top + padding_top + row_h + padding_bottom + margin_bottom
        }
        LayoutElement::Image {
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + height + flow_extra_bottom + margin_bottom,
        LayoutElement::HorizontalRule {
            margin_top,
            margin_bottom,
        } => margin_top + 1.0 + margin_bottom,
        LayoutElement::ProgressBar {
            height,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + height + margin_bottom,
        LayoutElement::Svg {
            height,
            flow_extra_bottom,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + height + flow_extra_bottom + margin_bottom,
        LayoutElement::MathBlock {
            layout,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + layout.height() + margin_bottom,
        LayoutElement::RunningElement { .. } => 0.0,
        LayoutElement::Container {
            children,
            padding_top,
            padding_bottom,
            border,
            margin_top,
            margin_bottom,
            block_height,
            position,
            ..
        } => {
            // Absolute containers are out of flow and contribute no height to
            // their parent (matches the TextBlock arm above).
            if *position == Position::Absolute {
                return 0.0;
            }
            // When any direct child floats, the auto content height excludes the
            // floats (they don't stretch the box) but includes any clearance gap;
            // `simulate_block_flow` is the shared source of truth for that. The
            // plain (no-float) sum is kept byte-for-byte to avoid regressions.
            let children_h: f32 = if children.iter().any(|c| element_float(c) != Float::None) {
                simulate_block_flow(children).height
            } else {
                children
                    .iter()
                    .map(|c| estimate_element_height_bounded(c, depth - 1))
                    .sum()
            };
            let content_h = padding_top + children_h + padding_bottom + border.vertical_width();
            // A definite `block_height` (set only for an explicit `height`) is a
            // hard border-box size: overflowing content spills past it rather than
            // growing the box, so honour it directly instead of `content_h.max(h)`.
            let effective_h = block_height.unwrap_or(content_h);
            margin_top + effective_h + margin_bottom
        }
        _ => 0.0,
    }
}

/// The CSS `float` value of a block-level layout element (`None` for anything
/// that cannot float, e.g. table rows).
pub(crate) fn element_float(element: &LayoutElement) -> Float {
    match element {
        LayoutElement::TextBlock { float, .. } | LayoutElement::Container { float, .. } => *float,
        _ => Float::None,
    }
}

/// The CSS `clear` value of a block-level layout element.
fn element_clear(element: &LayoutElement) -> Clear {
    match element {
        LayoutElement::TextBlock { clear, .. } | LayoutElement::Container { clear, .. } => *clear,
        _ => Clear::None,
    }
}

/// Whether a layout element is out of normal flow (absolutely positioned) and so
/// contributes no height to its container and does not advance the flow cursor.
fn element_is_absolute(element: &LayoutElement) -> bool {
    matches!(
        element,
        LayoutElement::TextBlock {
            position: Position::Absolute,
            ..
        } | LayoutElement::Container {
            position: Position::Absolute,
            ..
        }
    )
}

/// Whether an in-flow element participates in adjacent-sibling vertical margin
/// collapse. Table/grid rows and other non-block content do not (they break the
/// collapse chain), mirroring `collapse_role` in the renderer.
fn element_collapses_margins(element: &LayoutElement) -> bool {
    matches!(
        element,
        LayoutElement::TextBlock { .. }
            | LayoutElement::Container { .. }
            | LayoutElement::Image { .. }
            | LayoutElement::Svg { .. }
            | LayoutElement::FlexRow { .. }
            | LayoutElement::HorizontalRule { .. }
            | LayoutElement::ProgressBar { .. }
            | LayoutElement::MathBlock { .. }
    )
}

/// The top/bottom margins of a layout element, used for adjacent-sibling
/// vertical margin collapse. Returns `(margin_top, margin_bottom)`.
fn element_margins(element: &LayoutElement) -> (f32, f32) {
    match element {
        LayoutElement::TextBlock {
            margin_top,
            margin_bottom,
            ..
        }
        | LayoutElement::Container {
            margin_top,
            margin_bottom,
            ..
        }
        | LayoutElement::Image {
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
        } => (*margin_top, *margin_bottom),
        _ => (0.0, 0.0),
    }
}

/// The collapsed vertical gap between two adjacent block margins (CSS 2.1
/// § 8.3.1): positive margins overlap, negative margins overlap, and a mixed
/// pair sums.
fn collapse_pair(margin_top: f32, prev_margin_bottom: f32) -> f32 {
    if margin_top >= 0.0 && prev_margin_bottom >= 0.0 {
        margin_top.max(prev_margin_bottom)
    } else if margin_top < 0.0 && prev_margin_bottom < 0.0 {
        margin_top.min(prev_margin_bottom)
    } else {
        margin_top + prev_margin_bottom
    }
}

/// The placement of a single floated child, relative to the container's
/// content-box top-left, computed by [`simulate_block_flow`]. The float's side
/// is read from the element itself at paint time, so only its index and top are
/// recorded here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FloatPlacement {
    /// Index of the float within the original `children` slice.
    pub index: usize,
    /// Distance of the float's border-box top below the content-box top.
    pub top: f32,
}

/// Result of simulating normal-flow block layout with simplified CSS floats.
#[derive(Debug, Clone, Default)]
pub(crate) struct BlockFlowResult {
    /// Total in-flow content height (border-box heights of in-flow children plus
    /// collapsed margins and any `clear` offsets). Floats do not extend it.
    pub height: f32,
    /// Placement of each floated child, in source order.
    pub floats: Vec<FloatPlacement>,
    /// Lowest bottom edge of any left float, below the content-box top.
    pub left_float_bottom: f32,
    /// Lowest bottom edge of any right float, below the content-box top.
    pub right_float_bottom: f32,
}

/// Simulate normal-flow block layout of `children` with simplified floats and
/// `clear`, returning the in-flow content height and the resolved top of every
/// float. This is the single source of truth shared by the wrapper-height
/// estimate and the renderer's float placement, so the painted geometry always
/// matches the measured height.
///
/// The in-flow accumulation mirrors `collapsed_children_height` in the renderer
/// (sum of each child's outer `estimate_element_height` minus adjacent-sibling
/// margin-collapse overlap), so for the common no-float case the measured height
/// is byte-for-byte identical and nothing regresses.
///
/// Float model (block-sibling case): a `float: left|right` child is removed from
/// normal flow — it does not advance the flow cursor and does not stretch the
/// container — but it is pinned to the left/right content edge at the cursor's
/// current position (its top below the content origin). A later in-flow block
/// with `clear` is pushed below the bottom of the relevant float(s); that
/// clearance gap *does* extend the container because the cleared block is in
/// flow. Adjacent in-flow blocks collapse their vertical margins across
/// out-of-flow (float / absolute) siblings.
pub(crate) fn simulate_block_flow(children: &[LayoutElement]) -> BlockFlowResult {
    // Running in-flow content bottom, below the content-box top. Accumulated the
    // same way as `collapsed_children_height`: add each child's full outer
    // height, then back out the collapsed overlap with the previous sibling.
    let mut cursor = 0.0f32;
    // Previous in-flow sibling's margin-bottom for adjacent collapse; `None`
    // breaks the chain (start, after a float, or after clearance).
    let mut prev_mb: Option<f32> = None;
    // Bottom edges of placed floats per side (below the content origin), for
    // `clear` and for stacking successive same-side floats.
    let mut left_bottom = 0.0f32;
    let mut right_bottom = 0.0f32;
    let mut floats = Vec::new();

    for (index, child) in children.iter().enumerate() {
        if element_is_absolute(child) {
            // Out of flow: contributes nothing and leaves the collapse chain.
            continue;
        }
        let float = element_float(child);
        let outer_h = estimate_element_height(child);
        let (mt, mb) = element_margins(child);

        if float != Float::None {
            // Float: pinned at the current content bottom plus its margin-top,
            // stacked below any earlier same-side float. Floats don't collapse
            // margins and don't advance the in-flow cursor, but they do break
            // the running collapse chain for the next in-flow sibling.
            let side_bottom = if float == Float::Left {
                left_bottom
            } else {
                right_bottom
            };
            let float_top = (cursor + mt).max(side_bottom);
            let border_box_h = (outer_h - mt - mb).max(0.0);
            let float_bottom = float_top + border_box_h;
            if float == Float::Left {
                left_bottom = float_bottom;
            } else {
                right_bottom = float_bottom;
            }
            floats.push(FloatPlacement {
                index,
                top: float_top,
            });
            prev_mb = None;
            continue;
        }

        // In-flow block. Apply `clear` first: push the content bottom below the
        // relevant float(s). Clearance breaks the margin-collapse chain.
        let clear = element_clear(child);
        let clear_to = match clear {
            Clear::Left => left_bottom,
            Clear::Right => right_bottom,
            Clear::Both => left_bottom.max(right_bottom),
            Clear::None => f32::NEG_INFINITY,
        };
        if clear != Clear::None && clear_to > cursor {
            cursor = clear_to;
            prev_mb = None;
        }

        // Add the full outer box, then remove the collapse overlap with the
        // previous in-flow sibling (mirrors `collapsed_children_height`).
        cursor += outer_h;
        if element_collapses_margins(child) {
            if let Some(pmb) = prev_mb {
                cursor -= pmb + mt - collapse_pair(mt, pmb);
            }
            prev_mb = Some(mb);
        } else {
            // Non-collapsing in-flow content (table/grid rows): breaks the chain.
            prev_mb = None;
        }
    }

    BlockFlowResult {
        height: cursor,
        floats,
        left_float_bottom: left_bottom,
        right_float_bottom: right_bottom,
    }
}

pub(crate) fn table_row_content_width(element: &LayoutElement) -> f32 {
    match element {
        LayoutElement::TableRow {
            col_widths,
            border_collapse,
            border_spacing,
            ..
        } => {
            let spacing = if *border_collapse == BorderCollapse::Collapse {
                0.0
            } else {
                *border_spacing
            };
            col_widths.iter().sum::<f32>() + spacing * col_widths.len().saturating_sub(1) as f32
        }
        _ => 0.0,
    }
}

/// Split a too-tall in-flow `TextBlock` at a line boundary (CSS Fragmentation 3
/// §3) so its first fragment fills the remaining fragmentainer height and the
/// rest continues on the next page. `avail_below_box_top` is the page height
/// still available below this box's *border-box top* on the current page
/// (`content_height − cursor − collapsed margin-top`).
///
/// Returns `(first_fragment, continuation)` for `box-decoration-break: slice`
/// (the default): the first fragment keeps the box's TOP border/padding but
/// drops its bottom border/padding/margin; the continuation drops its top
/// margin/border/padding and keeps the original bottom decoration. `None` is
/// returned when the box cannot be cleanly split between lines — a definite
/// `height`/clipped (overflow) box, a positioned/floated box, fewer than two
/// lines, or a boundary where every line fits or none would move — in which case
/// the caller places it whole (the pre-existing, possibly-overflowing behavior).
fn split_text_block(
    element: &LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutElement, LayoutElement)> {
    let LayoutElement::TextBlock {
        lines,
        block_height,
        clip_rect,
        position,
        float,
        border,
        padding_top,
        padding_bottom,
        box_decoration_break,
        orphans,
        widows,
        ..
    } = element
    else {
        return None;
    };
    // Only a plain, auto-height, in-flow text block is splittable here. A box
    // with a definite height or `overflow` clip is a hard-sized box (treat as
    // monolithic); a positioned/floated box is out of normal flow and handled
    // elsewhere; a single line cannot be divided.
    if block_height.is_some()
        || clip_rect.is_some()
        || *position != Position::Static
        || *float != Float::None
        || lines.len() < 2
    {
        return None;
    }

    // `box-decoration-break: clone` re-wraps EVERY fragment with the full
    // top+bottom border/padding/margin and background, so the first fragment's
    // line area is reduced by the bottom decoration too (the box closes on this
    // page). `slice` (default) leaves the box open at the bottom, so its lines
    // may extend to the page edge.
    let clone = *box_decoration_break == BoxDecorationBreak::Clone;

    // Content-box height available for text lines on this page: the space below
    // the box's border-box top, minus the top border + top padding (and, under
    // `clone`, the bottom border + bottom padding the fragment also carries).
    let avail_lines = if clone {
        avail_below_box_top - border.vertical_width() - padding_top - padding_bottom
    } else {
        avail_below_box_top - border.top.width - padding_top
    };

    // Greedily keep whole lines that fit, but always retain at least one line on
    // this page — the forward-progress invariant: never leave a fragmentainer
    // empty / never break at the very top with zero content.
    let mut acc = 0.0f32;
    let mut idx = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let next = acc + line.height;
        if i > 0 && next > avail_lines {
            break;
        }
        acc = next;
        idx = i + 1;
    }
    if idx == 0 || idx >= lines.len() {
        // Every line fits (no overflow to split) or not even one would stay.
        return None;
    }

    // CSS Fragmentation 3 §3.4 (orphans / widows): a break between line N and
    // N+1 is permitted only when at least `orphans` lines remain on this
    // fragment (N >= orphans) AND at least `widows` lines move to the next
    // ((total − N) >= widows). `idx` is the greedy maximum that *fits*, so it
    // can only be REDUCED (move lines to the continuation) without overflowing;
    // it cannot be increased (the extra lines do not fit). Reducing it satisfies
    // widows. Orphans is then satisfied automatically when feasible — after the
    // reduction `idx >= n − widows >= orphans` — and is unsatisfiable only when
    // even the greedy maximum already keeps fewer than `orphans` lines (lines
    // taller than the fragmentainer). `split_text_block` is reached only for a
    // block taller than a full fragmentainer, so when the constraint cannot be
    // honoured it is DROPPED (split greedily) to guarantee forward progress,
    // exactly as the spec requires when a full page cannot satisfy it.
    let orphans = (*orphans).max(1) as usize;
    let widows = (*widows).max(1) as usize;
    let n = lines.len();
    if n >= orphans + widows {
        let max_idx = n - widows;
        if idx > max_idx {
            idx = max_idx;
        }
    }

    // First fragment: the lines that fit. Under `slice` it keeps the box's top
    // decoration but drops its bottom border/padding/margin (the box stays open
    // at the page bottom); under `clone` it keeps the FULL decoration and closes.
    let mut first = element.clone();
    if let LayoutElement::TextBlock {
        lines: f_lines,
        margin_bottom: f_mb,
        padding_bottom: f_pb,
        border: f_border,
        border_radii: f_radii,
        border_radii_y: f_radii_y,
        ..
    } = &mut first
    {
        *f_lines = lines[..idx].to_vec();
        if !clone {
            *f_mb = 0.0;
            *f_pb = 0.0;
            f_border.bottom.width = 0.0;
            // css-break-3 §5.4: the cut (bottom) edge is square under `slice`.
            f_radii[2] = 0.0;
            f_radii[3] = 0.0;
            f_radii_y[2] = 0.0;
            f_radii_y[3] = 0.0;
        }
    }

    // Continuation: the remaining lines. Under `slice` it drops the top
    // margin/border/padding (continuing the open box) and keeps the original
    // bottom decoration so the LAST fragment closes it; under `clone` it keeps
    // the FULL decoration so the fragment is independently wrapped.
    let mut rest = element.clone();
    if let LayoutElement::TextBlock {
        lines: r_lines,
        margin_top: r_mt,
        padding_top: r_pt,
        border: r_border,
        border_radii: r_radii,
        border_radii_y: r_radii_y,
        ..
    } = &mut rest
    {
        *r_lines = lines[idx..].to_vec();
        if !clone {
            *r_mt = 0.0;
            *r_pt = 0.0;
            r_border.top.width = 0.0;
            // css-break-3 §5.4: the cut (top) edge is square under `slice`.
            r_radii[0] = 0.0;
            r_radii[1] = 0.0;
            r_radii_y[0] = 0.0;
            r_radii_y[1] = 0.0;
        }
    }

    Some((first, rest))
}

/// Slice a definite-height in-flow text block at the fragmentainer edge. Unlike
/// [`split_text_block`], this handles boxes whose own height is monolithic and
/// taller than the page; the box background/border must continue on following
/// pages instead of overflowing and being clipped. Text lines that fit in the
/// first fragment stay there, and later lines move to the continuation.
fn split_fixed_height_text_block(
    element: &LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutElement, LayoutElement)> {
    let LayoutElement::TextBlock {
        lines,
        block_height,
        clip_rect,
        position,
        float,
        border,
        padding_top,
        padding_bottom,
        box_decoration_break,
        ..
    } = element
    else {
        return None;
    };
    let block_height = (*block_height)?;
    if clip_rect.is_some()
        || *position != Position::Static
        || *float != Float::None
        || block_height <= 0.0
    {
        return None;
    }

    let clone = *box_decoration_break == BoxDecorationBreak::Clone;
    let first_border_h = if clone {
        border.vertical_width()
    } else {
        border.top.width
    };
    let first_content_h = (avail_below_box_top - first_border_h).min(block_height);
    if first_content_h <= MIN_IMAGE_SLICE || block_height - first_content_h <= MIN_IMAGE_SLICE {
        return None;
    }

    let first_line_space = if clone {
        first_content_h - padding_top - padding_bottom
    } else {
        first_content_h - padding_top
    };
    let mut acc = 0.0f32;
    let mut idx = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let next = acc + line.height;
        if i > 0 && next > first_line_space {
            break;
        }
        if next > first_line_space && i == 0 {
            break;
        }
        acc = next;
        idx = i + 1;
    }

    let mut first = element.clone();
    if let LayoutElement::TextBlock {
        lines: f_lines,
        margin_bottom: f_mb,
        padding_bottom: f_pb,
        border: f_border,
        border_radii: f_radii,
        border_radii_y: f_radii_y,
        block_height: f_bh,
        ..
    } = &mut first
    {
        *f_lines = lines[..idx.min(lines.len())].to_vec();
        *f_bh = Some(first_content_h.max(0.0));
        if !clone {
            *f_mb = 0.0;
            *f_pb = 0.0;
            f_border.bottom.width = 0.0;
            f_radii[2] = 0.0;
            f_radii[3] = 0.0;
            f_radii_y[2] = 0.0;
            f_radii_y[3] = 0.0;
        }
    }

    let mut rest = element.clone();
    if let LayoutElement::TextBlock {
        lines: r_lines,
        margin_top: r_mt,
        padding_top: r_pt,
        border: r_border,
        border_radii: r_radii,
        border_radii_y: r_radii_y,
        block_height: r_bh,
        ..
    } = &mut rest
    {
        *r_lines = lines[idx.min(lines.len())..].to_vec();
        *r_bh = Some((block_height - first_content_h).max(0.0));
        if !clone {
            *r_mt = 0.0;
            *r_pt = 0.0;
            r_border.top.width = 0.0;
            r_radii[0] = 0.0;
            r_radii[1] = 0.0;
            r_radii_y[0] = 0.0;
            r_radii_y[1] = 0.0;
        }
    }

    Some((first, rest))
}

/// Minimum slice height (pt) below which a too-tall raster image is not sliced —
/// keeps a fragment from being a sliver and guarantees forward progress.
const MIN_IMAGE_SLICE: f32 = 1.0;

/// Slice a too-tall in-flow raster `Image` at the page boundary (CSS
/// Fragmentation 3 §4.1: monolithic content taller than the fragmentainer is
/// sliced at the fragmentainer edge rather than discarded). `avail_below_box_top`
/// is the page height still available below the image's border-box top on the
/// current page.
///
/// Returns `(first_fragment, continuation)`: the first fragment fills the rest of
/// this page with the TOP slice of the source raster (its `flow_extra_bottom` and
/// `margin_bottom` dropped), and the continuation displays the remainder on the
/// next page (its `margin_top` dropped, the original bottom decoration kept so the
/// FINAL fragment closes the box). Each fragment records the source-pixel
/// sub-rectangle it shows in `src_crop`, so the renderer emits only that slice as
/// the page's image XObject instead of a full copy behind a clip.
///
/// Returns `None` (caller places the image whole, the pre-existing overflow
/// behavior) when the image cannot be sliced cleanly: a non-`fill` `object-fit`
/// (the source does not map linearly onto the box, so a box slice is not a source
/// slice), a bordered box (the frame cannot be split here), a `filter` raster
/// (already feathered/padded), or no usable space on the page.
fn split_image_block(
    element: &LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutElement, LayoutElement)> {
    let LayoutElement::Image {
        image,
        height,
        object_fit,
        border,
        blur_overflow,
        src_crop,
        ..
    } = element
    else {
        return None;
    };
    if *object_fit != ObjectFit::Fill
        || border.vertical_width() != 0.0
        || *blur_overflow != 0.0
        || *height <= 0.0
    {
        return None;
    }

    // Display height of the TOP slice that fits on this page.
    let first_h = avail_below_box_top.min(*height);
    if first_h <= MIN_IMAGE_SLICE || *height - first_h <= MIN_IMAGE_SLICE {
        // No room for a meaningful slice, or the remainder would be a sliver —
        // (re)place the image whole (it already restarts on a fresh page).
        return None;
    }

    // The source sub-rectangle this element currently displays (the whole source
    // if it has not been sliced yet), mapped linearly onto `height` under
    // object-fit: fill. Slicing composes with any inherited crop.
    let [bx, by, bw, bh] = src_crop.unwrap_or([
        0.0,
        0.0,
        image.source_width as f32,
        image.source_height as f32,
    ]);
    let slice_src_h = bh * (first_h / *height);

    let mut first = element.clone();
    if let LayoutElement::Image {
        height: f_h,
        flow_extra_bottom: f_fe,
        margin_bottom: f_mb,
        src_crop: f_crop,
        ..
    } = &mut first
    {
        *f_h = first_h;
        *f_fe = 0.0;
        *f_mb = 0.0;
        *f_crop = Some([bx, by, bw, slice_src_h]);
    }

    let mut rest = element.clone();
    if let LayoutElement::Image {
        height: r_h,
        margin_top: r_mt,
        src_crop: r_crop,
        ..
    } = &mut rest
    {
        *r_h = *height - first_h;
        *r_mt = 0.0;
        *r_crop = Some([bx, by + slice_src_h, bw, bh - slice_src_h]);
        // `flow_extra_bottom` and `margin_bottom` stay on the continuation so the
        // final fragment keeps the original strut / bottom margin.
    }

    Some((first, rest))
}

fn svg_source_box(tree: &crate::parser::svg::SvgTree) -> Option<crate::parser::svg::ViewBox> {
    if let Some(view_box) = tree.view_box {
        if view_box.width > 0.0 && view_box.height > 0.0 {
            return Some(view_box);
        }
    }
    let width = tree
        .width_attr
        .as_deref()
        .and_then(crate::parser::svg::parse_absolute_length)
        .unwrap_or(tree.width);
    let height = tree
        .height_attr
        .as_deref()
        .and_then(crate::parser::svg::parse_absolute_length)
        .unwrap_or(tree.height);
    if width > 0.0 && height > 0.0 {
        Some(crate::parser::svg::ViewBox {
            min_x: 0.0,
            min_y: 0.0,
            width,
            height,
        })
    } else {
        None
    }
}

/// Slice a too-tall SVG replaced element by narrowing its source viewBox to the
/// rows that belong on each page. This is the SVG analogue of raster `src_crop`:
/// each fragment maps the relevant source slice into that page's fragment box.
fn split_svg_block(
    element: &LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutElement, LayoutElement)> {
    let LayoutElement::Svg { tree, height, .. } = element else {
        return None;
    };
    if *height <= 0.0 {
        return None;
    }

    let first_h = avail_below_box_top.min(*height);
    if first_h <= MIN_IMAGE_SLICE || *height - first_h <= MIN_IMAGE_SLICE {
        return None;
    }
    let source = svg_source_box(tree)?;
    let slice_src_h = source.height * (first_h / *height);

    let mut first = element.clone();
    if let LayoutElement::Svg {
        tree: f_tree,
        height: f_h,
        flow_extra_bottom: f_fe,
        margin_bottom: f_mb,
        ..
    } = &mut first
    {
        *f_h = first_h;
        *f_fe = 0.0;
        *f_mb = 0.0;
        f_tree.view_box = Some(crate::parser::svg::ViewBox {
            min_x: source.min_x,
            min_y: source.min_y,
            width: source.width,
            height: slice_src_h,
        });
    }

    let mut rest = element.clone();
    if let LayoutElement::Svg {
        tree: r_tree,
        height: r_h,
        margin_top: r_mt,
        ..
    } = &mut rest
    {
        *r_h = *height - first_h;
        *r_mt = 0.0;
        r_tree.view_box = Some(crate::parser::svg::ViewBox {
            min_x: source.min_x,
            min_y: source.min_y + slice_src_h,
            width: source.width,
            height: source.height - slice_src_h,
        });
    }

    Some((first, rest))
}

/// Dispatch a too-tall in-flow element to the right splitter: a `TextBlock`
/// splits at a line boundary, a raster `Image` slices at the page edge, a
/// `Container` splits between (or recurses into) its children. Returns `None`
/// for anything monolithic/out-of-flow that cannot be fragmented here. Shared by
/// `paginate` (top-level boxes) and `split_container` (a single too-tall child).
fn split_element(
    element: &LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutElement, LayoutElement)> {
    split_fixed_height_text_block(element, avail_below_box_top)
        .or_else(|| split_text_block(element, avail_below_box_top))
        .or_else(|| split_image_block(element, avail_below_box_top))
        .or_else(|| split_svg_block(element, avail_below_box_top))
        .or_else(|| split_table_row(element, avail_below_box_top))
        .or_else(|| split_flex_row(element, avail_below_box_top))
        .or_else(|| split_container(element, avail_below_box_top))
}

/// Split a table row that is taller than the current fragmentainer. CSS Tables
/// fragments row boxes by slicing each cell box at the page edge; under the
/// default `box-decoration-break: slice` the first fragment keeps the top
/// border/padding and the continuation keeps the bottom edge.
fn split_table_row(
    element: &LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutElement, LayoutElement)> {
    let LayoutElement::TableRow { cells, .. } = element else {
        return None;
    };
    let row_h = cells
        .iter()
        .map(table_cell_content_height)
        .fold(0.0f32, f32::max);
    if row_h <= 1.0 || avail_below_box_top <= 1.0 || row_h <= avail_below_box_top + 0.5 {
        return None;
    }

    let consumed_h = avail_below_box_top.min(row_h - 1.0).max(1.0);
    let rest_h = (row_h - consumed_h).max(0.0);
    if rest_h <= 0.5 {
        return None;
    }
    let top_edge_bleed = cells
        .iter()
        .map(|cell| cell.border.top.width)
        .fold(0.0f32, f32::max)
        / 2.0;
    let first_painted_h = (consumed_h - top_edge_bleed).max(1.0);

    let mut line_cut_by_cell: Vec<usize> = Vec::with_capacity(cells.len());
    for cell in cells {
        let available_lines = (first_painted_h - cell.padding_top).max(0.0);
        let mut acc = 0.0f32;
        let mut cut = 0usize;
        for (idx, line) in cell.lines.iter().enumerate() {
            let next = acc + line.height;
            if idx > 0 && next > available_lines + 0.01 {
                break;
            }
            acc = next;
            cut = idx + 1;
        }
        line_cut_by_cell.push(cut);
    }

    let mut first = element.clone();
    if let LayoutElement::TableRow {
        cells: first_cells,
        margin_bottom,
        ..
    } = &mut first
    {
        *margin_bottom = 0.0;
        for (cell, &cut) in first_cells.iter_mut().zip(&line_cut_by_cell) {
            cell.lines = cell.lines[..cut.min(cell.lines.len())].to_vec();
            cell.nested_rows.clear();
            cell.border.bottom.width = 0.0;
            cell.padding_bottom = 0.0;
            cell.min_content_height = first_painted_h;
        }
    }

    let mut rest = element.clone();
    if let LayoutElement::TableRow {
        cells: rest_cells,
        margin_top,
        ..
    } = &mut rest
    {
        *margin_top = 0.0;
        for (cell, &cut) in rest_cells.iter_mut().zip(&line_cut_by_cell) {
            let cut = cut.min(cell.lines.len());
            cell.lines = cell.lines[cut..].to_vec();
            cell.nested_rows.clear();
            cell.border.top.width = 0.0;
            cell.padding_top = 0.0;
            cell.min_content_height = rest_h;
        }
    }

    Some((first, rest))
}

/// Split a wrapped row-direction flex container at a flex-line boundary. Flex
/// lines are class-A break opportunities between sibling flex items; keeping
/// the cells grouped by their `y_offset` preserves each line's internal
/// main-axis layout while allowing the flex container's border/background to
/// continue on the next fragmentainer.
fn split_flex_row(
    element: &LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutElement, LayoutElement)> {
    let LayoutElement::FlexRow {
        cells,
        row_height,
        border,
        padding_top,
        padding_bottom: _,
        ..
    } = element
    else {
        return None;
    };
    if cells.is_empty() || *row_height <= 1.0 || avail_below_box_top <= 1.0 {
        return None;
    }
    let avail_inner = (avail_below_box_top - border.top.width - *padding_top).max(0.0);
    if *row_height <= avail_inner + 0.5 {
        return None;
    }

    let mut line_tops: Vec<f32> = cells.iter().map(|cell| cell.y_offset).collect();
    line_tops.sort_by(f32::total_cmp);
    line_tops.dedup_by(|a, b| (*a - *b).abs() <= 0.5);
    if line_tops.len() <= 1 {
        return None;
    }

    let line_extent = |line_top: f32| -> f32 {
        cells
            .iter()
            .filter(|cell| (cell.y_offset - line_top).abs() <= 0.5)
            .map(|cell| {
                if cell.line_cross_size > 0.0 {
                    cell.line_cross_size
                } else {
                    cell.natural_height
                }
            })
            .fold(0.0_f32, f32::max)
    };

    let mut cut_y = None;
    for (idx, &top) in line_tops.iter().enumerate() {
        let bottom = top + line_extent(top);
        if idx > 0 && bottom > avail_inner + 0.5 {
            cut_y = Some(top);
            break;
        }
    }
    let cut_y = cut_y?;
    if cut_y <= 0.5 || cut_y >= *row_height - 0.5 {
        return None;
    }

    let mut first_cells = Vec::new();
    let mut rest_cells = Vec::new();
    for cell in cells {
        if cell.y_offset < cut_y - 0.5 {
            first_cells.push(cell.clone());
        } else {
            let mut rest = cell.clone();
            rest.y_offset = (rest.y_offset - cut_y).max(0.0);
            rest_cells.push(rest);
        }
    }
    if first_cells.is_empty() || rest_cells.is_empty() {
        return None;
    }

    let mut first = element.clone();
    if let LayoutElement::FlexRow {
        cells,
        row_height,
        margin_bottom,
        padding_bottom,
        border,
        ..
    } = &mut first
    {
        *cells = first_cells;
        *row_height = (avail_below_box_top - border.top.width - *padding_top).max(cut_y);
        *margin_bottom = 0.0;
        *padding_bottom = 0.0;
        border.bottom.width = 0.0;
    }

    let mut rest = element.clone();
    if let LayoutElement::FlexRow {
        cells,
        row_height,
        margin_top,
        padding_top,
        border,
        ..
    } = &mut rest
    {
        *cells = rest_cells;
        *row_height = (*row_height - cut_y).max(0.0);
        *margin_top = 0.0;
        *padding_top = 0.0;
        border.top.width = 0.0;
    }

    Some((first, rest))
}

/// Split a too-tall in-flow `Container` between its children (CSS Fragmentation 3
/// §3, class-A break point) so its first fragment fills the remaining
/// fragmentainer height and the rest continues on the next page.
/// `avail_below_box_top` is the page height still available below this box's
/// *border-box top* on the current page.
///
/// Returns `(first_fragment, continuation)`. Under `box-decoration-break: slice`
/// (the default) the first fragment keeps the box's TOP border/padding but drops
/// its bottom border/padding/margin (the box stays open at the page bottom), and
/// the continuation drops its top margin/border/padding while keeping the bottom
/// so the LAST fragment closes the box. Under `clone` EVERY fragment is
/// independently wrapped with the full border/padding/margin and background.
///
/// Returns `None` — so the caller places the box whole, the pre-existing
/// (possibly-overflowing) behavior — for any container that cannot be cleanly
/// split: a definite-`height`/clipped (overflow) box, a positioned or floated
/// box, or an empty box. The split always keeps at least the first child on this
/// page (forward progress) and the continuation carries strictly less content
/// than the original, so re-enqueuing it terminates. When the first child is
/// ALONE taller than the fragmentainer the splitter RECURSES into it (rather than
/// leaving it whole to clip), so a deeply nested too-tall box still fragments
/// across pages instead of losing data.
fn split_container(
    element: &LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutElement, LayoutElement)> {
    let LayoutElement::Container {
        children,
        border,
        padding_top,
        padding_bottom,
        block_height,
        overflow,
        position,
        float,
        box_decoration_break,
        ..
    } = element
    else {
        return None;
    };
    // Only a plain, auto-height, in-flow container is splittable here. A definite
    // `height` or `overflow` clip makes it a hard-sized/monolithic box; a
    // positioned/floated box is out of normal flow; an empty box has nothing to
    // fragment. A single-child box has no between-children break point but may
    // still be split by RECURSING into that one (too-tall) child below.
    if block_height.is_some()
        || overflow.clips()
        || *position != Position::Static
        || *float != Float::None
        || children.is_empty()
    {
        return None;
    }

    // Any out-of-flow (absolutely positioned) child is anchored, not flowed, so it
    // must not become a break boundary or move to the continuation independently.
    // Keep the split path to the simple all-in-flow case; anything else is placed
    // whole (unchanged behavior).
    if children.iter().any(element_is_absolute) {
        return None;
    }

    let clone = *box_decoration_break == BoxDecorationBreak::Clone;

    // Content-box height available for children on this page: below the box's
    // border-box top, minus the top border + top padding (and, under `clone`, the
    // bottom border + bottom padding the fragment also carries).
    let avail_children = if clone {
        avail_below_box_top - border.vertical_width() - padding_top - padding_bottom
    } else {
        avail_below_box_top - border.top.width - padding_top
    };

    // The page-fit check that brought us here sums the children's outer heights
    // WITHOUT adjacent-sibling margin collapse, so a box whose children collapse
    // (CSS 2.1 §8.3.1) is over-measured and can look like it overflows when it
    // actually fits. Re-measure with the collapsed model the renderer uses
    // (`simulate_block_flow`): if the children genuinely fit, the box is not
    // overflowing — place it whole (unchanged behaviour) rather than spuriously
    // fragmenting a box that lands on a single page in Chrome.
    const FRAG_EPSILON: f32 = 0.5;
    if simulate_block_flow(children).height <= avail_children + FRAG_EPSILON {
        return None;
    }

    // Greedily keep whole children that fit, always retaining at least the first
    // (forward progress). Children heights are summed the same way the container's
    // own auto height is measured in `paginate` (plain outer-height sum), so the
    // boundary the fit-decision saw and the boundary we cut at agree.
    let mut acc = 0.0f32;
    let mut idx = 0usize;
    for (i, child) in children.iter().enumerate() {
        let next = acc + estimate_element_height(child);
        if i > 0 && next > avail_children {
            break;
        }
        acc = next;
        idx = i + 1;
    }

    // Partition the children into the first fragment's list and the continuation's
    // list. Normally the cut is between children at `idx`. But the first child is
    // always force-kept for forward progress, so when it ALONE overflows the
    // fragmentainer (idx == 1 and its height exceeds the space), placing it whole
    // would clip it (data loss). Instead RECURSE into that child — split it with
    // the same splitter — so its head fills this page and its tail continues. Only
    // the first child can be the too-tall one (every later kept child fit), so this
    // single check covers every nested-too-tall case (CSS Fragmentation 3 §3).
    let first_child_h = estimate_element_height(&children[0]);
    let (f_children_vec, r_children_vec) = if idx == 1 && first_child_h > avail_children {
        let first_child = &children[0];
        // The child's border-box top sits at the container's content-box top plus
        // its own margin-top, so it has that much less room than the content box.
        let child_avail = avail_children - element_margins(first_child).0;
        match split_element(first_child, child_avail) {
            Some((c_first, c_rest)) => {
                let mut rest_children = vec![c_rest];
                rest_children.extend_from_slice(&children[1..]);
                (vec![c_first], rest_children)
            }
            // The single too-tall child cannot be split (e.g. a definite-height /
            // clipped / replaced box). If there are later siblings, still cut after
            // it (it overflows, as before); otherwise nothing can be done — place
            // the whole container as-is (unchanged overflow behaviour).
            None if children.len() >= 2 => (children[..1].to_vec(), children[1..].to_vec()),
            None => return None,
        }
    } else if idx >= children.len() {
        // Every child fits at this boundary (nothing to move to a continuation):
        // not actually overflowing between children.
        return None;
    } else {
        (children[..idx].to_vec(), children[idx..].to_vec())
    };

    // First fragment: the children that fit, with the box's top decoration. Under
    // `slice` drop the bottom border/padding/margin (box stays open at the page
    // bottom); under `clone` keep the full decoration so the fragment closes.
    let mut first = element.clone();
    if let LayoutElement::Container {
        children: f_children,
        margin_bottom: f_mb,
        padding_bottom: f_pb,
        border: f_border,
        border_radii: f_radii,
        border_radii_y: f_radii_y,
        block_height: f_bh,
        ..
    } = &mut first
    {
        *f_children = f_children_vec;
        if !clone {
            *f_mb = 0.0;
            *f_pb = 0.0;
            f_border.bottom.width = 0.0;
            // css-break-3 §5.4: under `slice` the fragmentation CUT edge is
            // square — only the box's real corners stay rounded. This fragment's
            // bottom edge is the cut, so drop the bottom-right/bottom-left radii.
            f_radii[2] = 0.0;
            f_radii[3] = 0.0;
            f_radii_y[2] = 0.0;
            f_radii_y[3] = 0.0;
            // A box that continues onto the next fragmentainer occupies the FULL
            // remaining height of THIS one: its background and left/right borders
            // extend to the page bottom even though the children only fill part of
            // it (css-break-3 — the box is sliced at the fragmentainer edge, not
            // shrink-wrapped to the children that landed on this page). Pin the
            // first fragment's border-box height to that remaining space so the
            // background/side-borders reach the page bottom, matching Chrome. The
            // last fragment keeps auto height (block_height stays None) so it ends
            // at its natural content + bottom decoration.
            *f_bh = Some(avail_below_box_top);
        }
    }

    // Continuation: the remaining children. Under `slice` drop the top
    // margin/border/padding (the open box continues) and keep the bottom so the
    // LAST fragment closes it; under `clone` keep the full decoration.
    let mut rest = element.clone();
    if let LayoutElement::Container {
        children: r_children,
        margin_top: r_mt,
        padding_top: r_pt,
        border: r_border,
        border_radii: r_radii,
        border_radii_y: r_radii_y,
        ..
    } = &mut rest
    {
        *r_children = r_children_vec;
        if !clone {
            *r_mt = 0.0;
            *r_pt = 0.0;
            r_border.top.width = 0.0;
            // css-break-3 §5.4: the continuation's TOP edge is the cut, so it is
            // square — drop the top-left/top-right radii (the original bottom
            // corners stay rounded so the LAST fragment closes the box).
            r_radii[0] = 0.0;
            r_radii[1] = 0.0;
            r_radii_y[0] = 0.0;
            r_radii_y[1] = 0.0;
        }
    }

    Some((first, rest))
}

/// Geometry override for the first page (CSS Paged Media 3 §3.3 `@page :first`).
/// `content_height` is the page-1 content box height (page height minus the
/// first-page top/bottom margins); `margin` is the full first-page margin used
/// to tag the emitted [`Page`] so the renderer positions it correctly.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FirstPageGeom {
    pub content_height: f32,
    pub margin: Margin,
}

/// Spread margins for the `:left` / `:right` page pseudo-classes (CSS Paged Media
/// 3 §3.2). Each is the full page margin to tag pages of that spread side with,
/// resolved from the default margin plus the side's declared `margin-*`. In LTR
/// page 1 is a `:right` page, so odd 1-based pages are `:right` and even are
/// `:left`. `None` keeps the document-global margin for that side (the universal
/// corpus case), so behaviour is unchanged when no spread rule is present.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SpreadMargins {
    pub left: Option<Margin>,
    pub right: Option<Margin>,
}

/// Resolved declarations for a named `@page <name>` rule before pagination. The
/// margin always starts from the document-global margin; `page_size` is present
/// only when that named rule declares `size`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NamedPageOverride {
    pub margin: Margin,
    pub page_size: Option<PageSize>,
}

/// All per-page margin overrides resolved from `@page` pseudo-class rules (CSS
/// Paged Media 3 §3.2–3.4), bundled so the layout entry point threads one value
/// instead of several. `first` is the `:first` margin (page 1); `spread` carries
/// the `:left`/`:right` margins applied by page parity; `named` maps each
/// `@page <name>` to its margin (CSS Paged Media 3 §3.4 named pages), applied to
/// the page started by a `page: <name>` box. A `Default` value reproduces the
/// document-global margin on every page.
#[derive(Debug, Clone, Default)]
pub(crate) struct PageMarginOverrides {
    pub first: Option<Margin>,
    pub spread: SpreadMargins,
    pub named: HashMap<String, NamedPageOverride>,
}

/// Geometry of a named page (CSS Paged Media 3 §3.4), pre-resolved at the layout
/// entry point where the page size and document-default margin are known. The
/// `margin` tags the page so the renderer positions content against the named
/// margin; `content_height` is the resulting fragmentainer height (page height
/// minus the named top/bottom margin).
#[derive(Debug, Clone, Copy)]
pub(crate) struct NamedPageGeom {
    pub content_height: f32,
    pub margin: Margin,
    pub page_size: PageSize,
}

/// Paginate with a single global content height (no per-page geometry). Thin
/// wrapper over [`paginate_with_first_page`]; used by unit tests and any caller
/// that does not need an `@page :first`/`:left`/`:right` override.
#[allow(dead_code)]
pub(crate) fn paginate(
    elements: Vec<LayoutElement>,
    content_height: f32,
    root_margin_top: f32,
) -> Vec<Page> {
    paginate_with_first_page(
        elements,
        content_height,
        root_margin_top,
        None,
        SpreadMargins::default(),
        HashMap::new(),
    )
}

/// Paginate with an optional first-page geometry override and optional
/// `:left`/`:right` spread margins. When `first_page` is `None` and `spread` is
/// empty this is identical to a single global `content_height`/margin for every
/// page (the default path used by the whole corpus).
pub(crate) fn paginate_with_first_page(
    elements: Vec<LayoutElement>,
    default_content_height: f32,
    root_margin_top: f32,
    first_page: Option<FirstPageGeom>,
    spread: SpreadMargins,
    named_pages: HashMap<String, NamedPageGeom>,
) -> Vec<Page> {
    // The content height in force for the page currently being filled. Page 1
    // uses the first-page override (if any); every page after page 1 reverts to
    // the default. Updated to `default_content_height` immediately after the
    // first page is finalized.
    let mut content_height = first_page
        .map(|f| f.content_height)
        .unwrap_or(default_content_height);
    // The margin tag applied to the FIRST emitted page (page 1).
    let first_margin_override = first_page.map(|f| f.margin);
    // The per-page margin override for the page about to be pushed, chosen by
    // 1-based page number: `:first` wins on page 1, otherwise the spread side by
    // parity (odd = `:right`, even = `:left` in LTR). `None` => document-global
    // margin. `already_pushed` is `pages.len()` at the push site, so the new
    // page's number is `already_pushed + 1`.
    let page_margin_override = move |already_pushed: usize| -> Option<Margin> {
        let page_no = already_pushed + 1;
        if page_no == 1 {
            if let Some(m) = first_margin_override {
                return Some(m);
            }
        }
        if page_no % 2 == 1 {
            spread.right
        } else {
            spread.left
        }
    };
    // CSS Paged Media 3 §3.4 named-page margin (`page: <name>`) currently in
    // force. Set when a named `PageBreak` is consumed; it overrides the
    // parity/`:first` margin for every page pushed while active (the page the
    // named box starts and any continuation it overflows onto), and reverts
    // when a different named break — or the document end — is reached. `None`
    // means the default page geometry.
    let mut pending_named_page: Option<NamedPageGeom> = None;
    let mut pages: Vec<Page> = Vec::new();
    let mut current_elements: Vec<(f32, LayoutElement)> = Vec::new();
    let mut current_running_elements: HashMap<String, LayoutElement> = HashMap::new();
    let mut current_footnotes: Vec<FootnoteItem> = Vec::new();
    // Page 1 starts with body/html margin-top applied; continuation pages
    // start flush against the page margin (Chrome's print-model: body margin
    // opens the document, not every page).
    let mut y: f32 = root_margin_top;

    // Track active float regions for simplified float/clear behavior
    let mut left_floats: Vec<FloatRegion> = Vec::new();
    let mut right_floats: Vec<FloatRegion> = Vec::new();
    let mut prev_margin_bottom: f32 = 0.0;
    // CSS margin-collapse-through-root: the first in-flow block on a page has
    // its margin-top collapse with the body margin on page 1. On continuation
    // pages (after page break), the first block's margin-top applies as-is
    // because body is mid-flow and doesn't collapse with the viewport anymore.
    let mut first_on_page: bool = true;
    let mut on_first_page: bool = true;

    // Collect synthetic full-page background elements that should be repeated
    // across every page during pagination.
    let mut absolute_backgrounds: Vec<(f32, LayoutElement)> = Vec::new();
    // Track the y-position of positioned ancestors by depth so absolute descendants
    // resolve against the nearest positioned ancestor rather than the most recent one.
    let mut positioned_y_by_depth: HashMap<usize, f32> = HashMap::new();

    // Track the header rows of the currently-active table so pagination can
    // re-emit them at the top of each page the table spans (Chrome parity).
    // Cleared as soon as a non-TableRow element is encountered.
    let mut pending_table_headers: Vec<LayoutElement> = Vec::new();
    // Track the `<tfoot>` rows of the active table so pagination can repeat them
    // as a running footer at the bottom of every page the table spans, directly
    // after the last body row (Chrome's LayoutNG table fragmentation). Collected
    // by a forward scan when the table is first entered, so their height is known
    // (and reserved) while body rows are placed — even though, after the
    // thead->tbody->tfoot reorder, the footer rows arrive LAST in the stream.
    let mut pending_table_footers: Vec<LayoutElement> = Vec::new();
    // Total reserved height of `pending_table_footers` (content height of the
    // footer rows, mirroring the repeated-header advance). Subtracted from the
    // available page height when deciding whether a body row fits.
    let mut pending_footer_height: f32 = 0.0;
    // Whether the cursor is currently inside a table's row run (between the first
    // row and the next non-row element), used to detect table entry for the
    // footer pre-scan.
    let mut in_table = false;
    #[allow(unused_assignments)]
    let mut in_table_body = false;
    let mut previous_table_row_break_inside_avoid = false;

    // Content height of a table row = the tallest cell's content height (the same
    // measure used for the repeated-header advance), excluding row margins.
    let row_content_height = |element: &LayoutElement| -> f32 {
        match element {
            LayoutElement::TableRow { cells, .. } => cells
                .iter()
                .map(table_cell_content_height)
                .fold(0.0f32, f32::max),
            _ => 0.0,
        }
    };

    // Worklist of pending top-level elements. A box that is too tall for the
    // page is split (CSS Fragmentation 3 §3): its first fragment is placed and
    // the continuation is pushed back onto the FRONT so it resumes immediately
    // on the next page. Elements that already fit are processed exactly as
    // before (every existing `continue`/placement is unchanged), so the whole
    // single-page corpus is byte-for-byte identical.
    let mut work: std::collections::VecDeque<LayoutElement> = elements.into();
    while let Some(element) = work.pop_front() {
        if let LayoutElement::RunningElement { name, element } = element {
            current_running_elements.insert(name, *element);
            continue;
        }

        // When the FIRST row of a `break-inside: avoid` table cannot fit in the
        // space left on the current page but DOES fit on an empty one, the break
        // decision below uses the whole table's height (this value) instead of
        // just the first row's, so the entire row run is pushed to the next page
        // intact rather than split between rows (Chrome's table keep-together).
        let mut table_keep_break_height: Option<f32> = None;
        // Track <thead>/<tfoot> rows so we can repeat them across page breaks
        // that occur mid-table: the header at each page top, the footer at each
        // page bottom. Reset when leaving the table.
        let mut suppress_repeated_headers_after_break = false;
        match &element {
            LayoutElement::TableRow {
                is_header,
                is_footer,
                break_inside_avoid,
                ..
            } => {
                let table_avoid_group_starts_here =
                    *break_inside_avoid && !previous_table_row_break_inside_avoid;
                previous_table_row_break_inside_avoid = *break_inside_avoid;
                if !in_table {
                    // First row of a new table: scan ahead over the rest of this
                    // table's contiguous row run to collect the `<tfoot>` rows
                    // (which the thead->tbody->tfoot reorder places at the end of
                    // the run) so their height is reserved while body rows are
                    // placed and they can be repeated at each page bottom.
                    in_table = true;
                    pending_table_headers.clear();
                    pending_table_footers.clear();
                    pending_footer_height = 0.0;
                    for w in work.iter() {
                        match w {
                            LayoutElement::TableRow {
                                is_footer: w_foot, ..
                            } => {
                                if *w_foot {
                                    pending_footer_height += row_content_height(w);
                                    pending_table_footers.push(w.clone());
                                }
                            }
                            _ => break,
                        }
                    }
                    // `break-inside: avoid` table keep-together (CSS Fragmentation
                    // 3 §5.2 / legacy `page-break-inside: avoid`): sum the whole
                    // table's row run (this first row plus every contiguous row
                    // still queued). When the table is avoid-inside AND fits on a
                    // full page, arm the whole-table break height so a table that
                    // would straddle the boundary is moved WHOLE to the next page.
                    // A table taller than a full page cannot be kept together, so
                    // it falls back to the normal between-rows split.
                    if *break_inside_avoid {
                        let mut total = estimate_element_height(&element);
                        for w in work.iter() {
                            match w {
                                LayoutElement::TableRow { .. } => {
                                    total += estimate_element_height(w);
                                }
                                _ => break,
                            }
                        }
                        if total <= content_height {
                            table_keep_break_height = Some(total);
                        }
                    }
                }
                if table_keep_break_height.is_none() && table_avoid_group_starts_here {
                    let mut total = estimate_element_height(&element);
                    for w in work.iter() {
                        match w {
                            LayoutElement::TableRow {
                                break_inside_avoid: true,
                                is_header: w_header,
                                is_footer: w_footer,
                                ..
                            } if *w_header == *is_header && *w_footer == *is_footer => {
                                total += estimate_element_height(w);
                            }
                            _ => break,
                        }
                    }
                    if total <= content_height {
                        table_keep_break_height = Some(total);
                    }
                }
                suppress_repeated_headers_after_break =
                    table_keep_break_height.is_some() && table_avoid_group_starts_here;
                // A header is collected for repetition; a footer is handled by the
                // running-footer placement (below / at page breaks); only ordinary
                // body rows count as "table body" for fit/break decisions.
                in_table_body = !*is_header && !*is_footer;
                if *is_header {
                    pending_table_headers.push(element.clone());
                }
            }
            _ => {
                pending_table_headers.clear();
                pending_table_footers.clear();
                pending_footer_height = 0.0;
                in_table = false;
                in_table_body = false;
                previous_table_row_break_inside_avoid = false;
            }
        }

        // A `<tfoot>` row reaching the normal flow is the FINAL-page footer (the
        // reorder put it after every body row): place it directly after the last
        // body row on the current page. Its height was reserved while the body
        // rows were placed, so it always fits — skip the generic fit/break path.
        if matches!(
            &element,
            LayoutElement::TableRow {
                is_footer: true,
                ..
            }
        ) {
            let fh = row_content_height(&element);
            collect_footnotes_from_element(&element, &mut current_footnotes);
            current_elements.push((y, element));
            y += fh;
            prev_margin_bottom = 0.0;
            first_on_page = false;
            continue;
        }

        // Extract float/clear/position info from TextBlock elements
        let (
            elem_float,
            elem_clear,
            elem_position,
            elem_offset_top,
            _elem_offset_bottom,
            elem_containing_block,
            elem_positioned_depth,
        ) = match &element {
            LayoutElement::TextBlock {
                float,
                clear,
                position,
                offset_top,
                offset_bottom,
                containing_block,
                positioned_depth,
                ..
            } => (
                *float,
                *clear,
                *position,
                *offset_top,
                *offset_bottom,
                *containing_block,
                *positioned_depth,
            ),
            // A positioned Container (e.g. a `position: fixed`/`absolute` box
            // with a background/border/explicit size) must also be recognised so
            // it is removed from normal flow and anchored to its containing
            // block. A root-level box has `containing_block: None` and resolves
            // against the page content box; bottom/right are pre-resolved into
            // top/left at layout time. Reading the real fields (rather than
            // hardcoding None/0) keeps a top-level positioned Container that has
            // a positioned ancestor anchored correctly and lets its own
            // descendants resolve against it by depth.
            LayoutElement::Container {
                float,
                clear,
                position,
                offset_top,
                containing_block,
                positioned_depth,
                ..
            } => (
                *float,
                *clear,
                *position,
                *offset_top,
                0.0,
                *containing_block,
                *positioned_depth,
            ),
            _ => (
                Float::None,
                Clear::None,
                Position::Static,
                0.0,
                0.0,
                None,
                0,
            ),
        };

        // A flex container (emitted as a FlexRow) that establishes a containing
        // block for absolute children records its padding-box top under its
        // `positioned_depth`, so abs children emitted after it anchor correctly.
        // (`top: 0` of such a child is the padding-box edge.) The padding-box top
        // is the FlexRow's flowed border-box top plus its top border.
        let flex_cb_depth = match &element {
            LayoutElement::FlexRow {
                positioned_depth,
                border,
                ..
            } if *positioned_depth > 0 => Some((*positioned_depth, border.top.width)),
            _ => None,
        };

        // Handle clear: move y below active floats on the specified side
        match elem_clear {
            Clear::Left | Clear::Both => {
                for f in &left_floats {
                    if f.y_end > y {
                        y = f.y_end;
                    }
                }
                if elem_clear == Clear::Both {
                    for f in &right_floats {
                        if f.y_end > y {
                            y = f.y_end;
                        }
                    }
                }
            }
            Clear::Right => {
                for f in &right_floats {
                    if f.y_end > y {
                        y = f.y_end;
                    }
                }
            }
            Clear::None => {}
        }

        // Returns (content_height_without_margins, margin_top, margin_bottom)
        let (content_h_val, margin_top_val, margin_bottom_val) = match &element {
            LayoutElement::PageBreak(side, name) => {
                let mut side = *side;
                let mut name = name.clone();
                // CSS Fragmentation 3 break precedence is resolved at the
                // shared class-A break point. The layout flattener emits
                // `break-after` followed by `break-before`; coalesce adjacent
                // forced breaks here so the later-in-flow `break-before` value
                // wins instead of being ignored on the empty page just created.
                while let Some(LayoutElement::PageBreak(next_side, next_name)) = work.front() {
                    side = *next_side;
                    name = next_name.clone();
                    work.pop_front();
                }
                // A forced break before any real content on the current page is
                // ignored (CSS Fragmentation 3: a forced break at the very start
                // of the fragmentation flow produces no leading blank page).
                // Consecutive forced breaks likewise collapse to one. A page that
                // holds only repeated page-background elements counts as empty.
                let page_has_content = current_elements.iter().any(|(_, el)| {
                    !matches!(
                        el,
                        LayoutElement::TextBlock {
                            repeat_on_each_page: true,
                            ..
                        }
                    )
                });
                if !page_has_content {
                    // A named box that opens the document (no preceding content)
                    // still selects its page geometry: the leading break is
                    // suppressed but the first page adopts the named margin.
                    if let Some(geom) = name.as_ref().and_then(|n| named_pages.get(n)) {
                        pending_named_page = Some(*geom);
                        content_height = geom.content_height;
                    }
                    continue;
                }
                let consumed_height = y;
                // The page being finalized adopts the named margin in force while
                // it was filled (if any), else the parity/`:first` override.
                let margin_override = pending_named_page
                    .map(|geom| geom.margin)
                    .or_else(|| page_margin_override(pages.len()));
                let page_size_override = pending_named_page.map(|geom| geom.page_size);
                pages.push(Page {
                    elements: std::mem::take(&mut current_elements),
                    running_elements: current_running_elements.clone(),
                    footnotes: std::mem::take(&mut current_footnotes),
                    margin_override,
                    page_size_override,
                });
                // After page 1 is finalized, page 2+ use the default geometry —
                // unless this break starts a named page (resolved just below).
                content_height = default_content_height;
                // Duplicate root background onto the new page.
                for bg in &absolute_backgrounds {
                    current_elements.push(bg.clone());
                }
                // CSS Paged Media 3 §3.4: a `page: <name>` break starts a page
                // whose geometry is the matching `@page <name>` rule. Switch the
                // active named margin (and fragmentainer height) to it; a break
                // back to the default flow clears it.
                pending_named_page = None;
                if let Some(geom) = name.as_ref().and_then(|n| named_pages.get(n)) {
                    pending_named_page = Some(*geom);
                    content_height = geom.content_height;
                }
                // Sided break (`break-*: left|right|recto|verso`): force the
                // following content onto a page of the requested parity. Page 1
                // is a right/recto page (LTR), so odd 1-based pages are right and
                // even are left. When the natural next page is the wrong side,
                // insert ONE blank page (carrying any repeated background) so the
                // content lands correctly.
                if matches!(
                    side,
                    PageBreakSide::Left
                        | PageBreakSide::Right
                        | PageBreakSide::Recto
                        | PageBreakSide::Verso
                ) {
                    let next_page_no = pages.len() + 1; // 1-based content page
                    let wants_right = matches!(side, PageBreakSide::Right | PageBreakSide::Recto);
                    let next_is_right = next_page_no % 2 == 1;
                    if wants_right != next_is_right {
                        let mut blank: Vec<(f32, LayoutElement)> = Vec::new();
                        for bg in &absolute_backgrounds {
                            blank.push(bg.clone());
                        }
                        let margin_override = pending_named_page
                            .map(|geom| geom.margin)
                            .or_else(|| page_margin_override(pages.len()));
                        let page_size_override = pending_named_page.map(|geom| geom.page_size);
                        pages.push(Page {
                            elements: blank,
                            running_elements: current_running_elements.clone(),
                            footnotes: Vec::new(),
                            margin_override,
                            page_size_override,
                        });
                    }
                }
                y = 0.0;
                prev_margin_bottom = 0.0;
                first_on_page = true;
                on_first_page = false;
                left_floats.clear();
                right_floats.clear();
                advance_positioned_ancestors_after_page_break(
                    &mut positioned_y_by_depth,
                    consumed_height,
                );
                continue;
            }
            LayoutElement::HorizontalRule {
                margin_top,
                margin_bottom,
            } => (1.0, *margin_top, *margin_bottom),
            LayoutElement::TableRow {
                cells,
                margin_top,
                margin_bottom,
                ..
            } => {
                let row_height = cells
                    .iter()
                    .map(table_cell_content_height)
                    .fold(0.0f32, f32::max);
                (row_height, *margin_top, *margin_bottom)
            }
            LayoutElement::GridRow {
                cells,
                margin_top,
                margin_bottom,
                ..
            } => {
                // Grid track height (resolved at layout) — never grown by the
                // cells' intrinsic content height (css-grid-1 §11).
                let row_height = cells
                    .iter()
                    .map(|cell| cell.min_content_height)
                    .fold(0.0f32, f32::max);
                (row_height, *margin_top, *margin_bottom)
            }
            LayoutElement::FlexRow {
                row_height,
                margin_top,
                margin_bottom,
                padding_top,
                padding_bottom,
                border,
                ..
            } => {
                let content = padding_top + row_height + padding_bottom + border.vertical_width();
                (content, *margin_top, *margin_bottom)
            }
            LayoutElement::TextBlock {
                lines,
                margin_top,
                margin_bottom,
                padding_top,
                padding_bottom,
                border,
                block_height,
                clip_rect,
                ..
            } => {
                let text_height: f32 = lines.iter().map(|l| l.height).sum();
                let border_extra = border.vertical_width();
                let content_h = padding_top + text_height + padding_bottom;
                let effective_content_h = if clip_rect.is_some() {
                    // overflow:hidden — use specified height, don't expand
                    block_height.unwrap_or(content_h)
                } else {
                    match block_height {
                        Some(h) => content_h.max(*h),
                        None => content_h,
                    }
                };
                (
                    effective_content_h + border_extra,
                    *margin_top,
                    *margin_bottom,
                )
            }
            LayoutElement::Image {
                height,
                flow_extra_bottom,
                margin_top,
                margin_bottom,
                ..
            } => (*height + *flow_extra_bottom, *margin_top, *margin_bottom),
            LayoutElement::Svg {
                height,
                flow_extra_bottom,
                margin_top,
                margin_bottom,
                ..
            } => (*height + *flow_extra_bottom, *margin_top, *margin_bottom),
            LayoutElement::ProgressBar {
                height,
                margin_top,
                margin_bottom,
                ..
            } => (*height, *margin_top, *margin_bottom),
            LayoutElement::MathBlock {
                layout,
                margin_top,
                margin_bottom,
                ..
            } => (layout.height(), *margin_top, *margin_bottom),
            LayoutElement::RunningElement { .. } => (0.0, 0.0, 0.0),
            LayoutElement::Container {
                children,
                padding_top,
                padding_bottom,
                border,
                margin_top,
                margin_bottom,
                block_height,
                overflow,
                ..
            } => {
                let children_h: f32 = children
                    .iter()
                    .map(|c| estimate_element_height_bounded(c, 50))
                    .sum();
                let content_h = padding_top + children_h + padding_bottom + border.vertical_width();
                let effective_h = if overflow.clips() {
                    block_height.unwrap_or(content_h)
                } else {
                    block_height.map_or(content_h, |h| content_h.max(h))
                };
                (effective_h, *margin_top, *margin_bottom)
            }
        };

        // Collapse margins: adjacent vertical margins merge (larger wins for positive,
        // most negative for negative, sum for mixed).
        let collapsed_margin = if margin_top_val >= 0.0 && prev_margin_bottom >= 0.0 {
            margin_top_val.max(prev_margin_bottom)
        } else if margin_top_val < 0.0 && prev_margin_bottom < 0.0 {
            margin_top_val.min(prev_margin_bottom)
        } else {
            margin_top_val + prev_margin_bottom
        };
        // CSS margin collapse through the root applies ONLY on page 1 (where
        // body opens). On page 1, the first block's margin-top collapses with
        // body.margin.top: since paginate pre-seeded `y = root_margin_top`,
        // the *extra* to add is `(block_mt - root_mt).max(0)`. On continuation
        // pages (page 2+), body is already mid-flow — no collapse with root,
        // and no body margin-top at all.
        let collapsed_margin = if first_on_page && on_first_page {
            (collapsed_margin - root_margin_top).max(0.0)
        } else {
            collapsed_margin
        };
        let margin_top_val = collapsed_margin;
        let element_height = margin_top_val + content_h_val + margin_bottom_val;
        let mut pending_footnotes = Vec::new();
        collect_footnotes_from_element(&element, &mut pending_footnotes);
        let footnote_reserve = footnote_reserved_height(&current_footnotes)
            + footnote_reserved_height(&pending_footnotes);
        let effective_content_height = (content_height - footnote_reserve).max(0.0);

        // Handle position: absolute -- place at fixed position, don't affect flow
        if elem_position == Position::Absolute {
            let abs_y = if let Some(cb) = elem_containing_block {
                // Position relative to the containing block (nearest positioned ancestor).
                // bottom/right offsets are pre-resolved into top/left in build_pseudo_block.
                positioned_y_by_depth.get(&cb.depth).copied().unwrap_or(0.0) + elem_offset_top
            } else {
                // No containing block — position relative to page (legacy behavior).
                elem_offset_top
            };
            if elem_positioned_depth > 0 {
                positioned_y_by_depth.insert(elem_positioned_depth, abs_y);
            }
            let repeats_on_each_page = match &element {
                LayoutElement::TextBlock {
                    repeat_on_each_page,
                    ..
                } => *repeat_on_each_page,
                _ => false,
            };
            if repeats_on_each_page {
                absolute_backgrounds.push((abs_y, element.clone()));
            }
            collect_footnotes_from_element(&element, &mut current_footnotes);
            current_elements.push((abs_y, element));
            continue;
        }

        // Reserve the repeated running-footer height while placing table body
        // rows so a body row is never laid where the footer will be re-emitted
        // at the page bottom (Chrome reserves the tfoot on every spanned page).
        let footer_reserve = if in_table_body {
            pending_footer_height
        } else {
            0.0
        };
        // For the first row of a `break-inside: avoid` table that fits a full
        // page, decide the break against the WHOLE table's height (footer
        // already included in that sum) so the entire table moves to the next
        // page intact; otherwise decide against this row plus the reserved
        // running-footer height as before.
        let (break_decision_height, break_footer_reserve) = match table_keep_break_height {
            Some(total) => (total, 0.0),
            None => (element_height, footer_reserve),
        };
        const PAGE_BREAK_EPSILON: f32 = 1.0;
        let mut page_broke_mid_loop = y + break_decision_height + break_footer_reserve
            > effective_content_height + PAGE_BREAK_EPSILON
            && y > 0.0;
        if page_broke_mid_loop
            && current_footnotes.is_empty()
            && pending_footnotes.is_empty()
            && elem_position == Position::Static
            && elem_float == Float::None
            && !in_table_body
        {
            let movable = current_elements
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (_, el))| {
                    !matches!(
                        el,
                        LayoutElement::TextBlock {
                            repeat_on_each_page: true,
                            ..
                        }
                    ) && !element_is_absolute(el)
                });
            if let Some((idx, (last_y, last_element))) = movable {
                let earlier_real_content = current_elements[..idx].iter().any(|(_, el)| {
                    !matches!(
                        el,
                        LayoutElement::TextBlock {
                            repeat_on_each_page: true,
                            ..
                        }
                    )
                });
                let moved_flow_height = (y - *last_y).max(0.0);
                let moved_footnote_free = {
                    let mut notes = Vec::new();
                    collect_footnotes_from_element(last_element, &mut notes);
                    notes.is_empty()
                };
                if earlier_real_content
                    && moved_footnote_free
                    && moved_flow_height > 0.0
                    && moved_flow_height < element_height * 0.9
                    && moved_flow_height <= effective_content_height * 0.35
                    && moved_flow_height + element_height
                        <= effective_content_height + PAGE_BREAK_EPSILON
                {
                    let (moved_y, moved_element) = current_elements.remove(idx);
                    y = moved_y;
                    let consumed_height = y;
                    let margin_override = pending_named_page
                        .map(|geom| geom.margin)
                        .or_else(|| page_margin_override(pages.len()));
                    let page_size_override = pending_named_page.map(|geom| geom.page_size);
                    pages.push(Page {
                        elements: std::mem::take(&mut current_elements),
                        running_elements: current_running_elements.clone(),
                        footnotes: std::mem::take(&mut current_footnotes),
                        margin_override,
                        page_size_override,
                    });
                    content_height = pending_named_page
                        .map(|geom| geom.content_height)
                        .unwrap_or(default_content_height);
                    for bg in &absolute_backgrounds {
                        current_elements.push(bg.clone());
                    }
                    current_elements.push((0.0, moved_element.clone()));
                    y = moved_flow_height;
                    on_first_page = false;
                    left_floats.clear();
                    right_floats.clear();
                    advance_positioned_ancestors_after_page_break(
                        &mut positioned_y_by_depth,
                        consumed_height,
                    );
                    page_broke_mid_loop = false;
                }
            }
        }
        if page_broke_mid_loop {
            let painted_fixed_text = matches!(
                &element,
                LayoutElement::TextBlock {
                    background_color: Some(_),
                    block_height: Some(_),
                    ..
                }
            );
            if painted_fixed_text
                && elem_position == Position::Static
                && elem_float == Float::None
                && !in_table_body
            {
                let avail_below_box_top = effective_content_height - (y + margin_top_val);
                if let Some((first, rest)) =
                    split_fixed_height_text_block(&element, avail_below_box_top)
                {
                    y += margin_top_val;
                    collect_footnotes_from_element(&first, &mut current_footnotes);
                    current_elements.push((y, first));
                    let consumed_height = content_height;
                    let margin_override = pending_named_page
                        .map(|geom| geom.margin)
                        .or_else(|| page_margin_override(pages.len()));
                    let page_size_override = pending_named_page.map(|geom| geom.page_size);
                    pages.push(Page {
                        elements: std::mem::take(&mut current_elements),
                        running_elements: current_running_elements.clone(),
                        footnotes: std::mem::take(&mut current_footnotes),
                        margin_override,
                        page_size_override,
                    });
                    content_height = pending_named_page
                        .map(|geom| geom.content_height)
                        .unwrap_or(default_content_height);
                    for bg in &absolute_backgrounds {
                        current_elements.push(bg.clone());
                    }
                    y = 0.0;
                    prev_margin_bottom = 0.0;
                    first_on_page = true;
                    on_first_page = false;
                    left_floats.clear();
                    right_floats.clear();
                    advance_positioned_ancestors_after_page_break(
                        &mut positioned_y_by_depth,
                        consumed_height,
                    );
                    work.push_front(rest);
                    continue;
                }
            }
            // Repeat the running footer at the bottom of the page being closed,
            // directly after the last body row (matching Chrome: the footer is
            // NOT flushed to the page edge — any reserved slack stays as
            // whitespace below it).
            if in_table_body && !pending_table_footers.is_empty() {
                for footer in pending_table_footers.clone() {
                    let footer_h = row_content_height(&footer);
                    collect_footnotes_from_element(&footer, &mut current_footnotes);
                    current_elements.push((y, footer));
                    y += footer_h;
                }
            }
            let consumed_height = y;
            // A natural (overflow) break inside named content keeps the active
            // named margin on the continuation page.
            let margin_override = pending_named_page
                .map(|geom| geom.margin)
                .or_else(|| page_margin_override(pages.len()));
            let page_size_override = pending_named_page.map(|geom| geom.page_size);
            pages.push(Page {
                elements: std::mem::take(&mut current_elements),
                running_elements: current_running_elements.clone(),
                footnotes: std::mem::take(&mut current_footnotes),
                margin_override,
                page_size_override,
            });
            // Continuations inside named content keep that named fragmentainer.
            content_height = pending_named_page
                .map(|geom| geom.content_height)
                .unwrap_or(default_content_height);
            // Duplicate root background onto the new page.
            for bg in &absolute_backgrounds {
                current_elements.push(bg.clone());
            }
            y = 0.0;
            on_first_page = false;
            // prev_margin_bottom and first_on_page are reset at the bottom of
            // this iteration (float or normal-flow branch overwrites both).
            left_floats.clear();
            right_floats.clear();
            advance_positioned_ancestors_after_page_break(
                &mut positioned_y_by_depth,
                consumed_height,
            );
            // Re-emit <thead> rows at the top of the new page if we're in the
            // middle of a table body (Chrome parity for long tables).
            if in_table_body
                && !suppress_repeated_headers_after_break
                && !pending_table_headers.is_empty()
            {
                for header in pending_table_headers.clone() {
                    let header_h = match &header {
                        LayoutElement::TableRow { cells, .. } => cells
                            .iter()
                            .map(table_cell_content_height)
                            .fold(0.0f32, f32::max),
                        _ => 0.0,
                    };
                    collect_footnotes_from_element(&header, &mut current_footnotes);
                    current_elements.push((y, header));
                    y += header_h;
                }
            }
        }

        // After a mid-loop page break, the current element is now the first
        // in-flow block on a continuation page. Its margin-top applies as-is
        // (no collapse with root — body is mid-flow across the page break).
        let effective_margin_top = margin_top_val;
        let _ = page_broke_mid_loop;

        // Handle floated elements (floats don't participate in margin collapsing)
        if elem_float != Float::None {
            y += effective_margin_top;
            let float_y_end = y + content_h_val;
            let region = FloatRegion {
                y_start: y,
                y_end: float_y_end,
                side: elem_float,
            };
            if elem_float == Float::Left {
                left_floats.push(region);
            } else {
                right_floats.push(region);
            }
            collect_footnotes_from_element(&element, &mut current_footnotes);
            current_elements.push((y, element));
            prev_margin_bottom = 0.0;
            first_on_page = false;
            continue;
        }

        // CSS Fragmentation 3 §3: if this in-flow box STILL overflows the page
        // after the break-between handling above, it is genuinely taller than a
        // full fragmentainer and would otherwise be clipped (data loss). Split it
        // at an internal break point, place the first fragment to fill the rest
        // of this page, and resume the continuation at the top of the next one.
        //
        // The guard `y + element_height > content_height` is true ONLY for a box
        // taller than the remaining space that the break-between logic could not
        // resolve (i.e. taller than a full empty page, or a too-tall box already
        // at the page top). Every box that fits — the entire existing corpus —
        // skips this block and takes the unchanged whole-placement path below.
        // The small epsilon absorbs sub-point text-measurement rounding so a box
        // that merely grazes the page bottom is not spuriously fragmented.
        const FRAG_EPSILON: f32 = 0.5;
        if elem_position == Position::Static
            && y + element_height > effective_content_height + FRAG_EPSILON
        {
            let avail_below_box_top = effective_content_height - (y + effective_margin_top);
            // A too-tall text block splits at a line boundary; a too-tall raster
            // image slices at the page edge (each page embeds only its slice); a
            // too-tall container splits between its children, re-enqueuing the
            // continuation so it resumes on the next page.
            let split = split_element(&element, avail_below_box_top);
            if let Some((first, rest)) = split {
                // Place the first fragment at the (margin-adjusted) cursor; it
                // fills the remainder of this page.
                y += effective_margin_top;
                collect_footnotes_from_element(&first, &mut current_footnotes);
                current_elements.push((y, first));
                // Close the page (the fragmentainer is full) and reset flow state
                // for the continuation, mirroring a normal mid-loop page break.
                let consumed_height = content_height;
                let margin_override = pending_named_page
                    .map(|geom| geom.margin)
                    .or_else(|| page_margin_override(pages.len()));
                let page_size_override = pending_named_page.map(|geom| geom.page_size);
                pages.push(Page {
                    elements: std::mem::take(&mut current_elements),
                    running_elements: current_running_elements.clone(),
                    footnotes: std::mem::take(&mut current_footnotes),
                    margin_override,
                    page_size_override,
                });
                // Continuations inside named content keep that named fragmentainer.
                content_height = pending_named_page
                    .map(|geom| geom.content_height)
                    .unwrap_or(default_content_height);
                for bg in &absolute_backgrounds {
                    current_elements.push(bg.clone());
                }
                y = 0.0;
                prev_margin_bottom = 0.0;
                first_on_page = true;
                on_first_page = false;
                left_floats.clear();
                right_floats.clear();
                advance_positioned_ancestors_after_page_break(
                    &mut positioned_y_by_depth,
                    consumed_height,
                );
                // Resume with the continuation on the next page.
                work.push_front(rest);
                continue;
            }
        }

        y += effective_margin_top;

        // Handle position: relative -- offset from normal position
        let effective_y = if elem_position == Position::Relative {
            y + elem_offset_top
        } else {
            y
        };

        // Track positioned ancestor y for absolute children.
        if elem_positioned_depth > 0
            && (elem_position == Position::Relative || elem_position == Position::Absolute)
        {
            positioned_y_by_depth.insert(elem_positioned_depth, effective_y);
        }
        // A flex container records its PADDING-box top (border-box top + top
        // border) under its own depth so absolute children — whose `top`/resolved
        // `bottom` offsets are measured from the padding box — anchor correctly.
        if let Some((depth, border_top)) = flex_cb_depth {
            positioned_y_by_depth.insert(depth, effective_y + border_top);
        }

        collect_footnotes_from_element(&element, &mut current_footnotes);
        current_elements.push((effective_y, element));
        y += content_h_val;
        prev_margin_bottom = margin_bottom_val;
        first_on_page = false;
    }

    // Finalize the pending page — but suppress a TRAILING BLANK page. A forced
    // break (`break-after: always` / `page-break-after: always`) on the last
    // in-flow box seeds a fresh page that ends up holding ONLY the duplicated
    // repeat-on-each-page backgrounds and no real content. Browsers drop such a
    // trailing empty page (Chrome emits one page for `…<div break-after:always>`,
    // not two), so only push the pending page if it carries real content — unless
    // it is the only page, so an otherwise-empty single-page document (e.g. an
    // empty body with a page background) still renders its one page.
    let has_real_content = current_elements.iter().any(|(_, el)| {
        !matches!(
            el,
            LayoutElement::TextBlock {
                repeat_on_each_page: true,
                ..
            }
        )
    });
    if !current_elements.is_empty() && (has_real_content || pages.is_empty()) {
        // The last page keeps the active named margin (a `page: <name>` block at
        // the document end, the common cover-page case).
        let margin_override = pending_named_page
            .map(|geom| geom.margin)
            .or_else(|| page_margin_override(pages.len()));
        let page_size_override = pending_named_page.map(|geom| geom.page_size);
        pages.push(Page {
            elements: current_elements,
            running_elements: current_running_elements.clone(),
            footnotes: std::mem::take(&mut current_footnotes),
            margin_override,
            page_size_override,
        });
    }

    if pages.is_empty() {
        pages.push(Page {
            elements: Vec::new(),
            running_elements: current_running_elements,
            footnotes: current_footnotes,
            margin_override: page_margin_override(0),
            page_size_override: None,
        });
    }

    // Sort elements within each page by z_index for correct rendering order.
    // Static elements (z_index 0) stay in document order; positioned elements
    // with higher z_index are moved later so they render on top.
    for page in &mut pages {
        page.elements
            .sort_by_key(|(_, element)| layout_element_paint_order(element));
    }

    pages
}

#[cfg(test)]
mod break_tests {
    use super::*;

    /// A fixed-height, in-flow content block (counts as "real content" for the
    /// leading-blank-page suppression).
    fn block(h: f32) -> LayoutElement {
        let mut e = LayoutElement::empty_spacer();
        if let LayoutElement::TextBlock { block_height, .. } = &mut e {
            *block_height = Some(h);
        }
        e
    }

    fn brk(side: PageBreakSide) -> LayoutElement {
        LayoutElement::PageBreak(side, None)
    }

    #[test]
    fn forced_break_page_paginates() {
        // Two blocks split by a plain forced break => two pages, one block each.
        let pages = paginate(
            vec![block(100.0), brk(PageBreakSide::Any), block(100.0)],
            1000.0,
            0.0,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 1);
        assert_eq!(pages[1].elements.len(), 1);
    }

    #[test]
    fn leading_forced_break_emits_no_blank_page() {
        // A forced break before any real content is ignored (no leading blank).
        let pages = paginate(vec![brk(PageBreakSide::Any), block(100.0)], 1000.0, 0.0);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].elements.len(), 1);
    }

    #[test]
    fn named_page_break_applies_named_margin_to_started_page() {
        // CSS Paged Media 3 §3.4: a `page: <name>` break starts a page that
        // adopts the matching `@page <name>` margin; the page before it keeps the
        // default geometry.
        let named_margin = crate::types::Margin::uniform(5.0);
        let mut named = HashMap::new();
        named.insert(
            "wide".to_string(),
            NamedPageGeom {
                content_height: 990.0,
                margin: named_margin,
                page_size: PageSize::A4,
            },
        );
        let pages = paginate_with_first_page(
            vec![
                block(100.0),
                LayoutElement::PageBreak(PageBreakSide::Any, Some("wide".to_string())),
                block(100.0),
            ],
            1000.0,
            0.0,
            None,
            SpreadMargins::default(),
            named,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages[0].margin_override, None,
            "page 1 keeps default margin"
        );
        assert_eq!(
            pages[1].margin_override,
            Some(named_margin),
            "page 2 adopts the @page wide margin"
        );
    }

    #[test]
    fn named_page_break_to_unknown_name_keeps_default_margin() {
        // A `page: <name>` with no matching `@page <name>` rule still forces the
        // break, but the started page keeps the default geometry (no override).
        let pages = paginate_with_first_page(
            vec![
                block(100.0),
                LayoutElement::PageBreak(PageBreakSide::Any, Some("ghost".to_string())),
                block(100.0),
            ],
            1000.0,
            0.0,
            None,
            SpreadMargins::default(),
            HashMap::new(),
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].margin_override, None);
    }

    #[test]
    fn consecutive_forced_breaks_collapse() {
        // Two adjacent breaks between two blocks still yield exactly two pages.
        let pages = paginate(
            vec![
                block(100.0),
                brk(PageBreakSide::Any),
                brk(PageBreakSide::Any),
                block(100.0),
            ],
            1000.0,
            0.0,
        );
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn sided_break_right_inserts_blank_parity_page() {
        // Content on page 1 (a right/recto page). `break-*: right` then forces the
        // next content onto the next right page — page 2 would be a LEFT page, so a
        // blank page is inserted and the content lands on page 3.
        let pages = paginate(
            vec![block(100.0), brk(PageBreakSide::Right), block(100.0)],
            1000.0,
            0.0,
        );
        assert_eq!(pages.len(), 3, "expected blank parity page");
        assert!(
            pages[1].elements.is_empty(),
            "middle page should be the inserted blank"
        );
        assert_eq!(pages[2].elements.len(), 1);
    }

    #[test]
    fn sided_break_left_needs_no_blank_when_next_is_left() {
        // Content on page 1 (right). `break-*: left` wants a LEFT page; page 2 is
        // already a left page, so no blank is inserted (2 pages total).
        let pages = paginate(
            vec![block(100.0), brk(PageBreakSide::Left), block(100.0)],
            1000.0,
            0.0,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].elements.len(), 1);
    }
}
