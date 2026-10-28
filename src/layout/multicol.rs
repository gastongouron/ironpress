//! CSS multi-column layout (`column-count` / `column-width` / `columns`).
//!
//! Implements a column-major *balanced* flow: the container's block-level
//! children are laid out top-to-bottom filling column 1, then column 2, etc.,
//! with the content distributed so the columns end up roughly equal height.
//! Each column retains fragmentainer-local physical placement independently
//! from authored CSS positioning, so it sits beside its peers without changing
//! containing-block, stacking, or fragmentation semantics.
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
//! - Balanced columns are filled sequentially after resolving their shortest
//!   legal block size. Class-A descendant boundaries are preferred over line
//!   splits, and `break-inside: avoid` keeps a box whole whenever it can fit.

mod distribution;
mod fill;
mod flow;
mod fragments;
mod geometry;
mod items;
mod page_rows;

use distribution::max_vertical_rl_item_height;
use fill::MulticolBlockFlow;
use flow::BoxFragmentPlacement;
use fragments::{
    empty_flow_anchor, item_is_splittable, make_band_container, make_column_container,
    make_fragment_box, make_rule_container,
};
use geometry::{column_rule_x, column_x, resolve_columns};
use items::{ChildMulticolInfo, ColumnBreakInfo, MultiColItem, child_multicol_info};
use page_rows::{
    build_balanced_paginated_column_rows, build_paginated_column_rows,
    build_paginated_column_rows_with_spans,
};

use crate::layout::elements::{
    BlockSize, BoxFragmentation, BoxModel, BoxPaint, Container, InlineOffset, IntoLayoutNode,
    LayoutNode, LayoutSize, MulticolContainer, OverflowBehavior, PageBreak, Positioning,
};
use crate::layout::flow_metrics::BlockMargins;
use crate::parser::css::{AncestorInfo, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::ComputedStyle;
use crate::types::{Point, Size};

use super::context::{LayoutContext, LayoutEnv};
use super::engine::{
    ElementSiblingContext, LayoutBorder, LayoutTreeContext, PageBreakSide, flatten_element,
};
use super::helpers::{PseudoBoxContext, build_pseudo_block};
use super::inline_formatting::GeneratedContentStyles;
use super::roundoff::is_positive_with_roundoff;

/// Geometry of one principal multicol box fragment.
///
/// Keeping the semantic [`BlockSize`] beside the physical inline size prevents
/// measured auto/minimum heights from becoming fixed-height overflow boxes at
/// the pagination boundary.
struct MulticolFragmentGeometry {
    size: LayoutSize,
    margins: BlockMargins,
    inline_offset: InlineOffset,
}

impl MulticolFragmentGeometry {
    const fn new(
        inline_size: f32,
        block_size: BlockSize,
        inline_offset: InlineOffset,
        margins: BlockMargins,
    ) -> Self {
        Self {
            size: LayoutSize::fixed_inline(inline_size, block_size),
            margins,
            inline_offset,
        }
    }
}

#[derive(Clone, Copy)]
struct ColumnRuleSpan {
    gap_after: usize,
    inline_offset: f32,
    block_offset: f32,
    block_size: f32,
}

pub(crate) fn layout_multicol_container(
    el: &ElementNode,
    style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutNode>,
    ancestors: &[AncestorInfo],
    positioned_depth: usize,
    env: &mut LayoutEnv,
) {
    let available_width = ctx.available_width();
    let border_pad_w = style.border.horizontal_width() + style.padding.horizontal();

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
        None => (available_width - style.padding.horizontal()).max(0.0),
    };
    let border_box_w = inner_width + border_pad_w;

    let inline_offset = InlineOffset::resolve_block_start(style, available_width, border_box_w);

    // Column gap: `normal` resolves to 1em (the element's font-size, in pt).
    let gap = if style.column_gap_is_normal {
        style.font_size
    } else if let Some(frac) = style.column_gap_pct {
        inner_width * frac
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
        preceding_siblings: Default::default(),
        following_siblings: Default::default(),
        is_empty: false,
    });

    let element_count = el
        .children
        .iter()
        .filter(|n| matches!(n, DomNode::Element(_)))
        .count();

    let mut items: Vec<MultiColItem> = Vec::new();
    let container_selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        ..Default::default()
    };
    let generated_styles =
        GeneratedContentStyles::resolve(el, style, env.rules, &container_selector_ctx, env.fonts);
    if let Some(pseudo_style) = generated_styles.before() {
        let pseudo = build_pseudo_block(
            pseudo_style,
            el,
            PseudoBoxContext::new(col_width, env.fonts, env.filter_defs, &mut *env.resources)
                .with_positioned_ancestor_depth(positioned_depth),
            env.counter_state,
            false,
        );
        items.push(MultiColItem::from_layout(
            vec![pseudo],
            ChildMulticolInfo::from_style(pseudo_style, ColumnBreakInfo::default()),
        ));
    }
    let mut element_index = 0usize;
    let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
    for node in &el.children {
        let DomNode::Element(child_el) = node else {
            continue;
        };
        // Decide span-all by computing the child's style cheaply.
        let child_info = child_multicol_info(
            child_el,
            style,
            env,
            &child_ancestors,
            element_index,
            element_count,
            &preceding_siblings,
        );
        let item_ctx = if child_info.span_all {
            &full_ctx
        } else {
            &col_ctx
        };

        let mut buf: Vec<LayoutNode> = Vec::new();
        flatten_element(
            child_el,
            LayoutTreeContext::new(style, item_ctx, &child_ancestors)
                .with_positioned_ancestor_depth(positioned_depth)
                .for_element(
                    ElementSiblingContext::new(element_index, element_count)
                        .with_neighbors(&preceding_siblings, &[]),
                ),
            &mut buf,
            env,
        );
        items.push(MultiColItem::from_layout(buf, child_info));

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
    if let Some(pseudo_style) = generated_styles.after() {
        let pseudo = build_pseudo_block(
            pseudo_style,
            el,
            PseudoBoxContext::new(col_width, env.fonts, env.filter_defs, &mut *env.resources)
                .with_positioned_ancestor_depth(positioned_depth),
            env.counter_state,
            false,
        );
        items.push(MultiColItem::from_layout(
            vec![pseudo],
            ChildMulticolInfo::from_style(pseudo_style, ColumnBreakInfo::default()),
        ));
    }

    // ---- Distribute items into columns (balanced, span-all as a band) -------
    // The output is a sequence of "segments": each segment is either a
    // full-width band (one span-all item) or a balanced multicol run. We track
    // the running vertical cursor so successive segments stack.
    let pad_left = style.border.left.used_width() + style.padding.left;
    let pad_top = style.border.top.used_width() + style.padding.top;
    // Anonymous columns, bands, and rules retain padding-box-relative physical
    // placement independently of authored CSS positioning. The height
    // accounting (`cursor_y`/`max_bottom`) stays in border-box coordinates.

    let block_flow = MulticolBlockFlow::from_style(style);
    let preferred_border_box_h = block_flow.preferred_border_box();

    if style.writing_mode.is_vertical() {
        let natural_block_size = preferred_border_box_h.unwrap_or_else(|| {
            pad_top
                + max_vertical_rl_item_height(&items)
                + style.padding.bottom
                + style.border.bottom.used_width()
        });
        let block_size = BlockSize::from_style(style, natural_block_size);
        let mut x_cursor = pad_left + inner_width;
        let mut column_children: Vec<LayoutNode> = Vec::new();
        for item in &items {
            if item.elements.len() != 1 {
                continue;
            }
            let item_w = if item.width > 0.0 {
                item.width
            } else {
                col_width
            };
            x_cursor -= item_w;
            column_children.push(make_fragment_box(
                &item.elements[0],
                BoxFragmentPlacement::whole(
                    Point::new(x_cursor - pad_left, 0.0),
                    Size::new(item_w, item.height),
                ),
            ));
        }
        output.push(emit_multicol_wrapper(
            style,
            column_children,
            MulticolFragmentGeometry::new(
                border_box_w,
                block_size,
                inline_offset,
                BlockMargins::new(style.margin.top, style.margin.bottom),
            ),
            positioned_depth,
        ));
        output.push(
            PageBreak {
                side: PageBreakSide::Any,
                page_name: None,
            }
            .boxed(),
        );
        output.push(empty_flow_anchor());
        return;
    }

    // ---- Page-aware column fragmentation (CSS Multicol §2 + Fragmentation 3) -
    // An auto-height, in-flow multicol whose column content is taller than the
    // page's content box is a *nested* fragmentation context: its columns fill to
    // the page height, then continue as a fresh row of column boxes on the next
    // page (CSS Multicol §2 — "a column box never splits across pages"). Without
    // this the whole multicol would be one atomic box that overflows off the page
    // bottom and is clipped (data loss). Only the genuinely-overflowing case takes
    // this path; everything that fits on one page (the entire existing corpus)
    // produces a single page-row and falls through to the byte-for-byte
    // single-page layout below.
    let page_content_h = ctx.available_height();
    let wrapper_v_extra = style.border.vertical_width() + style.padding.vertical();
    let col_fill_h = (page_content_h - wrapper_v_extra).max(0.0);
    let in_flow = style.position.is_in_flow() && style.float == crate::style::computed::Float::None;
    let all_splittable = !items.is_empty() && items.iter().all(item_is_splittable);
    let no_span = items.iter().all(|it| !it.span_all);
    if preferred_border_box_h.is_none()
        && in_flow
        && no_span
        && all_splittable
        && num_cols >= 1
        && is_positive_with_roundoff(col_fill_h)
    {
        let rows = if style.column_fill_auto {
            build_paginated_column_rows(
                &items, num_cols, col_width, gap, pad_left, col_fill_h, style,
            )
        } else {
            build_balanced_paginated_column_rows(
                &items, num_cols, col_width, gap, pad_left, col_fill_h, style,
            )
        };
        // Only fragment when the content genuinely spills past one page (more than
        // one page-row). A single row means it fits — fall through unchanged.
        if rows.len() > 1 {
            let last = rows.len() - 1;
            for (i, (row_children, row_max)) in rows.into_iter().enumerate() {
                let is_last = i == last;
                // Non-final page-rows fill the whole page (so the next row breaks
                // onto a fresh page); the final row shrink-wraps its content.
                let block_h = if is_last {
                    pad_top + row_max + style.padding.bottom + style.border.bottom.used_width()
                } else {
                    page_content_h
                };
                // Only the first fragment carries the top margin and only the last
                // carries the bottom margin (the box is a single flow element).
                let mt = if i == 0 { style.margin.top } else { 0.0 };
                let mb = if is_last { style.margin.bottom } else { 0.0 };
                output.push(emit_multicol_wrapper(
                    style,
                    row_children,
                    MulticolFragmentGeometry::new(
                        border_box_w,
                        BlockSize::fragment(block_h),
                        inline_offset,
                        BlockMargins::new(mt, mb),
                    ),
                    positioned_depth,
                ));
            }
            return;
        }
    }
    if preferred_border_box_h.is_none()
        && in_flow
        && !no_span
        && num_cols >= 1
        && is_positive_with_roundoff(col_fill_h)
    {
        let rows = build_paginated_column_rows_with_spans(
            &items,
            num_cols,
            col_width,
            gap,
            pad_left,
            col_fill_h,
            inner_width,
            style,
        );
        if rows.len() > 1 {
            let last = rows.len() - 1;
            for (i, (row_children, row_max)) in rows.into_iter().enumerate() {
                let is_last = i == last;
                let block_h = if is_last {
                    pad_top + row_max + style.padding.bottom + style.border.bottom.used_width()
                } else {
                    page_content_h
                };
                let mt = if i == 0 { style.margin.top } else { 0.0 };
                let mb = if is_last { style.margin.bottom } else { 0.0 };
                output.push(emit_multicol_wrapper(
                    style,
                    row_children,
                    MulticolFragmentGeometry::new(
                        border_box_w,
                        BlockSize::fragment(block_h),
                        inline_offset,
                        BlockMargins::new(mt, mb),
                    ),
                    positioned_depth,
                ));
            }
            return;
        }
    }

    let mut column_children: Vec<LayoutNode> = Vec::new();
    // A pending rule span recorded per balanced run.
    let mut rule_spans: Vec<ColumnRuleSpan> = Vec::new();
    let mut cursor_y = pad_top; // distance from border-box top to current band top
    let mut max_bottom = pad_top;

    let mut i = 0usize;
    while i < items.len() {
        if items[i].span_all {
            // Full-width band spanning all columns.
            let band_h = items[i].height;
            let band = make_band_container(
                std::mem::take(&mut items[i].elements),
                style.padding.left,
                style.padding.top + cursor_y - pad_top,
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
        let use_fragmentation = run.iter().all(item_is_splittable) && !run.is_empty();

        let mut run_max_h = 0.0f32;
        let mut run_nonempty_cols = vec![false; num_cols];
        if use_fragmentation {
            // ---- Fragmenting fill path (auto or balance) -------------------
            let run_indices: Vec<usize> = (0..run.len()).collect();
            let fragmented = block_flow.fragment(run, &run_indices, num_cols);
            if fragmented.columns.len() > run_nonempty_cols.len() {
                run_nonempty_cols.resize(fragmented.columns.len(), false);
            }
            for (c, frags) in fragmented.columns.iter().enumerate() {
                if frags.is_empty() {
                    continue;
                }
                run_nonempty_cols[c] = true;
                run_max_h = run_max_h.max(fragmented.used_block_sizes[c]);
                let col_x = column_x(style, pad_left, col_width, gap, num_cols, c);
                let mut col_kids: Vec<LayoutNode> = Vec::new();
                for f in frags {
                    col_kids.push(make_fragment_box(
                        &run[f.item].elements[0],
                        f.placement(0.0, col_width),
                    ));
                }
                column_children.push(make_column_container(
                    col_kids,
                    c,
                    col_x - pad_left + style.padding.left,
                    style.padding.top + cursor_y - pad_top,
                    col_width,
                    fragmented.used_block_sizes[c],
                ));
            }
        } else {
            // ---- Atomic bucket path (balance, or non-slice-able auto) ------
            let run_indices = (0..run.len()).collect::<Vec<_>>();
            let buckets = block_flow.distribute_atomic(run, &run_indices, num_cols);

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
                run_nonempty_cols[c] = true;
                let col_x = column_x(style, pad_left, col_width, gap, num_cols, c);
                let mut col_kids: Vec<LayoutNode> = Vec::new();
                let mut col_height = 0.0f32;
                for &idx in bucket {
                    col_height += run[idx].height;
                    col_kids.append(&mut run[idx].elements);
                }
                // Used column height for sizing: drop the trailing margin at a break.
                let used_col_height = if c != last_nonempty_col {
                    let trailing_margin =
                        bucket.last().map_or(0.0, |&index| run[index].margin_bottom);
                    (col_height - trailing_margin).max(0.0)
                } else {
                    col_height
                };
                run_max_h = run_max_h.max(used_col_height);
                column_children.push(make_column_container(
                    col_kids,
                    c,
                    col_x - pad_left + style.padding.left,
                    style.padding.top + cursor_y - pad_top,
                    col_width,
                    col_height,
                ));
            }
        }

        // Record one rule span per gap for this run. The resolved block flow
        // distinguishes an authored height from a maximum-height cap.
        if style.column_rule.used_width() > 0.0 && run_nonempty_cols.len() > 1 {
            let rule_w = style.column_rule.used_width();
            for c in 0..run_nonempty_cols.len().saturating_sub(1) {
                let has_left = run_nonempty_cols.get(c).copied().unwrap_or(false);
                let has_right = run_nonempty_cols.get(c + 1).copied().unwrap_or(false);
                if !has_left || !has_right {
                    continue;
                }
                let gap_center = column_rule_x(style, pad_left, col_width, gap, num_cols, c);
                let rule_x = gap_center - rule_w / 2.0;
                rule_spans.push(ColumnRuleSpan {
                    gap_after: c,
                    inline_offset: rule_x,
                    block_offset: cursor_y,
                    block_size: block_flow.rule_block_size(run_max_h),
                });
            }
        }

        cursor_y += run_max_h;
        max_bottom = max_bottom.max(cursor_y);
    }

    // Emit the recorded column rules. CSS Multicol section 4 makes each rule as
    // tall as its anonymous column box.
    if !rule_spans.is_empty() {
        let rule_w = style.column_rule.used_width();
        let rule_color = style.column_rule.color.resolve(style.color);
        let mut rule_children: Vec<LayoutNode> = Vec::new();
        for span in rule_spans {
            rule_children.push(make_rule_container(
                span.gap_after,
                span.inline_offset - pad_left + style.padding.left,
                span.block_offset - pad_top + style.padding.top,
                rule_w,
                span.block_size,
                rule_color,
                style.column_rule.style,
            ));
        }
        rule_children.extend(column_children);
        column_children = rule_children;
    }

    // ---- Outer container height --------------------------------------------
    // An explicit height wins; otherwise size to the tallest column run plus
    // the bottom padding.
    let content_box_h = max_bottom + style.padding.bottom + style.border.bottom.used_width();
    let block_size = BlockSize::from_style(style, content_box_h);

    // ---- Emit the wrapping container ---------------------------------------
    output.push(emit_multicol_wrapper(
        style,
        column_children,
        MulticolFragmentGeometry::new(
            border_box_w,
            block_size,
            inline_offset,
            BlockMargins::new(style.margin.top, style.margin.bottom),
        ),
        positioned_depth,
    ));
}

/// Build one multicol wrapper [`Container`] holding `column_children`.
/// The retained columns, bands, and rules carry physical fragment placements.
/// Shared by the single-page
/// layout and by each page-row of the paginated path, which override the wrapper's
/// `block_height` (full page for a continuing fragment, shrink-wrapped for the
/// last) and its `margin_top`/`margin_bottom` (the box is one flow element, so only
/// the first fragment keeps the top margin and only the last keeps the bottom).
fn emit_multicol_wrapper(
    style: &ComputedStyle,
    mut column_children: Vec<LayoutNode>,
    geometry: MulticolFragmentGeometry,
    positioned_depth: usize,
) -> LayoutNode {
    let border = LayoutBorder::from_computed(&style.border, style.color);
    if crate::layout::helpers::establishes_containing_block(style) {
        let border_box = Size::new(
            geometry.size.width.fixed_value().unwrap_or_default(),
            geometry.size.height.used().unwrap_or_default(),
        );
        crate::layout::helpers::resolve_absolute_descendants_containing_block(
            &mut column_children,
            crate::layout::engine::ContainingBlock {
                x: 0.0,
                width: (border_box.width - border.horizontal_width()).max(0.0),
                height: (border_box.height - border.vertical_width()).max(0.0),
                depth: positioned_depth,
            },
        );
    }
    MulticolContainer::new(Container {
        children: column_children,
        box_model: BoxModel {
            size: geometry.size,
            margins: geometry.margins,
            padding: style.padding,
            border,
        },
        paint: BoxPaint::from_style(style, geometry.size),
        flow: crate::layout::elements::BlockFlow {
            float: style.float,
            clear: style.clear,
        },
        positioning: Positioning::from_style(style)
            .with_resolved_insets(crate::types::EdgeSizes {
                left: geometry.inline_offset.value(),
                ..Default::default()
            })
            .with_containing_block_depth(positioned_depth),
        fragmentation: BoxFragmentation::from_style(style)
            .with_decoration(crate::style::computed::BoxDecorationBreak::Slice),
        overflow: OverflowBehavior {
            combined: style.overflow,
            x: style.overflow_x,
            y: style.overflow_y,
        },
    })
    .boxed()
}

#[cfg(test)]
mod tests;
