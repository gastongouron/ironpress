use super::engine::{LayoutElement, Page, layout_element_paint_order, table_cell_content_height};
use crate::style::computed::{BorderCollapse, Clear, Float, Overflow, Position};
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
            let row_h = cells
                .iter()
                .map(table_cell_content_height)
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
            ..
        } => {
            let children_h: f32 = children
                .iter()
                .map(|c| estimate_element_height_bounded(c, depth - 1))
                .sum();
            let content_h = padding_top + children_h + padding_bottom + border.vertical_width();
            let effective_h = block_height.map_or(content_h, |h| content_h.max(h));
            margin_top + effective_h + margin_bottom
        }
        _ => 0.0,
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
    #[allow(unused_assignments)]
    let mut in_table_body = false;

    for element in elements {
        // Track <thead> header rows so we can repeat them across page breaks
        // that occur mid-table. Reset when leaving the table.
        match &element {
            LayoutElement::TableRow { is_header, .. } => {
                if *is_header {
                    pending_table_headers.push(element.clone());
                    in_table_body = false;
                } else {
                    in_table_body = true;
                }
            }
            _ => {
                pending_table_headers.clear();
                in_table_body = false;
            }
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
                let row_height = cells
                    .iter()
                    .map(table_cell_content_height)
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
                let effective_h = if *overflow == Overflow::Hidden {
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

        let page_broke_mid_loop = y + element_height > content_height && y > 0.0;
        if page_broke_mid_loop {
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

        current_elements.push((effective_y, element));
        y += content_h_val;
        prev_margin_bottom = margin_bottom_val;
        first_on_page = false;
    }

    if !current_elements.is_empty() {
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
