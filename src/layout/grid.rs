use crate::parser::css::{AncestorInfo, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode};
use crate::style::computed::{
    ComputedStyle, FontWeight, GridTrack, TextAlign, VerticalAlign, compute_style_with_context,
};

use super::context::{LayoutContext, LayoutEnv};
use super::engine::{BackgroundFields, LayoutBorder, LayoutElement};
use super::table::TableCell;
use super::text::{
    FlexTextRunCollector, TextWrapOptions, resolved_line_height_factor, wrap_text_runs,
};

/// Resolve grid column widths from track definitions.
fn resolve_grid_columns(tracks: &[GridTrack], available_width: f32, gap: f32) -> Vec<f32> {
    if tracks.is_empty() {
        return vec![available_width];
    }

    let num_gaps = if tracks.len() > 1 {
        (tracks.len() - 1) as f32 * gap
    } else {
        0.0
    };
    let space = available_width - num_gaps;

    // First pass: consume fixed-width columns
    let mut fixed_total: f32 = 0.0;
    let mut fr_total: f32 = 0.0;
    let mut auto_count: usize = 0;
    let mut minmax_count: usize = 0;

    for track in tracks {
        match track {
            GridTrack::Fixed(v) => fixed_total += *v,
            GridTrack::Fr(v) => fr_total += *v,
            GridTrack::Auto => auto_count += 1,
            GridTrack::Minmax(min, _) => {
                fixed_total += min;
                minmax_count += 1;
            }
        }
    }

    let remaining = (space - fixed_total).max(0.0);

    // Auto columns are treated like 1fr each for distribution purposes
    let effective_fr_total = fr_total + auto_count as f32 + minmax_count as f32;
    let per_fr = if effective_fr_total > 0.0 {
        remaining / effective_fr_total
    } else {
        0.0
    };

    tracks
        .iter()
        .map(|track| match track {
            GridTrack::Fixed(v) => *v,
            GridTrack::Fr(v) => per_fr * *v,
            GridTrack::Auto => per_fr,
            GridTrack::Minmax(min, max) => {
                let desired = min + per_fr;
                if *max < f32::MAX {
                    desired.clamp(*min, *max)
                } else {
                    desired
                }
            }
        })
        .collect()
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
    let inner_width = available_width - style.padding.left - style.padding.right;
    let gap = style.grid_gap;

    let col_widths = resolve_grid_columns(&style.grid_template_columns, inner_width, gap);
    let num_cols = col_widths.len();

    // Build ancestors list for children of this element
    let mut child_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        element: el,
        child_index: 0,
        sibling_count: 0,
        preceding_siblings: Vec::new(),
    });

    // Collect element children (skip text nodes)
    let children: Vec<&ElementNode> = el
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

    let child_count = children.len();

    // Lay out children into grid cells, row by row
    let mut child_idx = 0;
    let mut is_first_row = true;
    let mut grid_children: Vec<LayoutElement> = Vec::new();

    while child_idx < children.len() {
        let row_end = (child_idx + num_cols).min(children.len());
        let mut cells = Vec::new();

        for (col, child_el) in children[child_idx..row_end].iter().enumerate() {
            let classes = child_el.class_list();
            let selector_ctx = SelectorContext {
                ancestors: child_ancestors.clone(),
                child_index: child_idx + col,
                sibling_count: child_count,
                preceding_siblings: Vec::new(),
            };
            let child_style = compute_style_with_context(
                child_el.tag,
                child_el.style_attr(),
                style,
                env.rules,
                child_el.tag_name(),
                &classes,
                child_el.id(),
                &child_el.attributes,
                &selector_ctx,
            );

            let cell_width = col_widths[col];
            let cell_inner =
                (cell_width - child_style.padding.left - child_style.padding.right).max(1.0);

            let mut runs = Vec::new();
            FlexTextRunCollector {
                runs: &mut runs,
                rules: env.rules,
                fonts: env.fonts,
            }
            .collect(
                &child_el.children,
                &child_style,
                None,
                (0.0, 0.0),
                &child_ancestors,
            );
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    cell_inner,
                    child_style.font_size,
                    resolved_line_height_factor(&child_style, env.fonts),
                    child_style.overflow_wrap,
                ),
                env.fonts,
            );

            let bg = child_style
                .background_color
                .map(|c: crate::types::Color| c.to_f32_rgba());

            cells.push(TableCell {
                lines,
                nested_rows: Vec::new(),
                bold: child_style.font_weight == FontWeight::Bold,
                background_color: bg,
                padding_top: child_style.padding.top,
                padding_right: child_style.padding.right,
                padding_bottom: child_style.padding.bottom,
                padding_left: child_style.padding.left,
                colspan: 1,
                rowspan: 1,
                border: LayoutBorder::from_computed(&child_style.border),
                text_align: child_style.text_align,
                vertical_align: child_style.vertical_align,
            });
        }

        // Fill remaining columns with empty cells if the row is incomplete
        while cells.len() < num_cols {
            cells.push(TableCell {
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
            });
        }

        let margin_top = if is_first_row { 0.0 } else { gap };

        grid_children.push(LayoutElement::GridRow {
            cells,
            col_widths: col_widths.clone(),
            gap,
            margin_top,
            margin_bottom: 0.0,
            border: LayoutBorder::default(),
            padding_left: 0.0,
            padding_right: 0.0,
            padding_top: 0.0,
            padding_bottom: 0.0,
        });

        is_first_row = false;
        child_idx = row_end;
    }

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
        block_width: Some(inner_width + style.padding.left + style.padding.right),
        block_height: None,
        opacity: style.opacity,
        float: style.float,
        position: style.position,
        offset_top: 0.0,
        offset_left: 0.0,
        overflow: style.overflow,
        transform: style.transform,
        box_shadow: style.box_shadow,
        background_gradient,
        background_radial_gradient,
        background_svg,
        background_blur_radius,
        background_size,
        background_position,
        background_repeat,
        background_origin,
        z_index: style.z_index,
    });
}
