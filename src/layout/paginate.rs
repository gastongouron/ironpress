use super::engine::{LayoutElement, Page, layout_element_paint_order, table_cell_content_height};
use crate::style::computed::{BorderCollapse, Clear, Float, ObjectFit, Position};
use std::collections::HashMap;

fn advance_positioned_ancestors_after_page_break(
    positioned_y_by_depth: &mut HashMap<usize, f32>,
    consumed_height: f32,
) {
    for y in positioned_y_by_depth.values_mut() {
        *y -= consumed_height;
    }
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

    // Content-box height available for text lines on this page: the space below
    // the box's border-box top, minus the top border + top padding (the first
    // fragment carries no bottom border/padding under `slice`, so its lines may
    // extend to the page bottom).
    let avail_lines = avail_below_box_top - border.top.width - padding_top;

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

    // First fragment: the lines that fit, with the box's top decoration but NO
    // bottom border/padding/margin (slice).
    let mut first = element.clone();
    if let LayoutElement::TextBlock {
        lines: f_lines,
        margin_bottom: f_mb,
        padding_bottom: f_pb,
        border: f_border,
        ..
    } = &mut first
    {
        *f_lines = lines[..idx].to_vec();
        *f_mb = 0.0;
        *f_pb = 0.0;
        f_border.bottom.width = 0.0;
    }

    // Continuation: the remaining lines with NO top margin/border/padding,
    // keeping the original bottom decoration so the LAST fragment closes the box.
    let mut rest = element.clone();
    if let LayoutElement::TextBlock {
        lines: r_lines,
        margin_top: r_mt,
        padding_top: r_pt,
        border: r_border,
        ..
    } = &mut rest
    {
        *r_lines = lines[idx..].to_vec();
        *r_mt = 0.0;
        *r_pt = 0.0;
        r_border.top.width = 0.0;
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

pub(crate) fn paginate(
    elements: Vec<LayoutElement>,
    content_height: f32,
    root_margin_top: f32,
) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut current_elements: Vec<(f32, LayoutElement)> = Vec::new();
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
        // Track <thead>/<tfoot> rows so we can repeat them across page breaks
        // that occur mid-table: the header at each page top, the footer at each
        // page bottom. Reset when leaving the table.
        match &element {
            LayoutElement::TableRow {
                is_header,
                is_footer,
                ..
            } => {
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
                }
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
            }
        }

        // A `<tfoot>` row reaching the normal flow is the FINAL-page footer (the
        // reorder put it after every body row): place it directly after the last
        // body row on the current page. Its height was reserved while the body
        // rows were placed, so it always fits — skip the generic fit/break path.
        if matches!(&element, LayoutElement::TableRow { is_footer: true, .. }) {
            let fh = row_content_height(&element);
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
            LayoutElement::PageBreak => {
                let consumed_height = y;
                pages.push(Page {
                    elements: std::mem::take(&mut current_elements),
                });
                // Duplicate root background onto the new page.
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
        let page_broke_mid_loop =
            y + element_height + footer_reserve > content_height && y > 0.0;
        if page_broke_mid_loop {
            // Repeat the running footer at the bottom of the page being closed,
            // directly after the last body row (matching Chrome: the footer is
            // NOT flushed to the page edge — any reserved slack stays as
            // whitespace below it).
            if in_table_body && !pending_table_footers.is_empty() {
                for footer in pending_table_footers.clone() {
                    let footer_h = row_content_height(&footer);
                    current_elements.push((y, footer));
                    y += footer_h;
                }
            }
            let consumed_height = y;
            pages.push(Page {
                elements: std::mem::take(&mut current_elements),
            });
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
            if in_table_body && !pending_table_headers.is_empty() {
                for header in pending_table_headers.clone() {
                    let header_h = match &header {
                        LayoutElement::TableRow { cells, .. } => cells
                            .iter()
                            .map(table_cell_content_height)
                            .fold(0.0f32, f32::max),
                        _ => 0.0,
                    };
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
            && y + element_height > content_height + FRAG_EPSILON
        {
            let avail_below_box_top = content_height - (y + effective_margin_top);
            // A too-tall text block splits at a line boundary; a too-tall raster
            // image slices at the page edge (each page embeds only its slice).
            let split = split_text_block(&element, avail_below_box_top)
                .or_else(|| split_image_block(&element, avail_below_box_top));
            if let Some((first, rest)) = split {
                // Place the first fragment at the (margin-adjusted) cursor; it
                // fills the remainder of this page.
                y += effective_margin_top;
                current_elements.push((y, first));
                // Close the page (the fragmentainer is full) and reset flow state
                // for the continuation, mirroring a normal mid-loop page break.
                let consumed_height = content_height;
                pages.push(Page {
                    elements: std::mem::take(&mut current_elements),
                });
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
        pages.push(Page {
            elements: current_elements,
        });
    }

    if pages.is_empty() {
        pages.push(Page {
            elements: Vec::new(),
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
