use super::*;
use crate::layout::elements::FlexRow;

pub(super) fn render_flex_child(
    content: &mut String,
    child: &FlexRow,
    child_index: usize,
    flow: &ContainerFlowContext<'_>,
    position: FlowPosition,
    abs_origins: &mut HashMap<usize, PdfPoint>,
    ctx: &mut PageRenderContext<'_>,
) -> FlowPosition {
    let phase = flow.paint_phase;
    let flow_top_by_index = flow.flow_top_by_index;
    let FlowPosition {
        y: _,
        mut cursor_y,
        previous_margin_bottom: mut prev_margin_bottom,
    } = position;
    let mut y;
    let cells = &child.content.cells;
    let flex_mt = &child.box_model.margins.start;
    let flex_mb = &child.box_model.margins.end;
    let background_color = &child.paint.background.color;
    let border = &child.box_model.border;
    let flex_border_radii = &child.paint.border_radii;
    let box_shadow = &child.paint.shadows;
    let background_gradient = &child.paint.background.layers.gradient;
    let background_radial_gradient = &child.paint.background.layers.radial_gradient;
    let background_conic_gradient = &child.paint.background.layers.conic_gradient;
    let background_svg = ctx
        .text
        .pdf_writer
        .resolve_background_svg(&child.paint.background.layers);
    let background_svg = &background_svg;
    let background_blur_radius = &child.paint.background.layers.blur_radius;
    let flex_bg_size = &child.paint.background.layers.size;
    let flex_bg_pos = &child.paint.background.layers.position;
    let flex_bg_repeat = &child.paint.background.layers.repeat;
    let flex_bg_origin = &child.paint.background.layers.origin;
    let flex_bg_clip = &child.paint.background.layers.clip;
    let flex_padding = &child.box_model.padding;
    let flex_row_h = &child.content.row_height;
    let align_items = &child.content.alignment;
    let flex_positioned_depth = &child.positioning.containing_block_depth;
    let planned_flow_top = flow_top_by_index.get(&child_index).copied();
    if let Some(top) = planned_flow_top {
        y = top;
    } else {
        cursor_y -= collapsed_margin_top_extra(*flex_mt, prev_margin_bottom);
        y = cursor_y;
    }
    let row_h = crate::layout::engine::estimate_element_height(child) - flex_mt - flex_mb;

    // The flex container honors its explicit width: paint its
    // background at `container_width` (already clamped to the
    // layout-time available width), not the full available width.
    // Mirrors the top-level FlexRow arm; without this a `width:Npx`
    // flex box painted its background across the whole content width.
    let flex_w = child.box_model.size.resolve_width(flow.frame.width());
    let used_origin = child.positioning.resolve_in_flow_origin(
        crate::types::Point::new(child.inline_offset.value(), flow.container_top_y - y),
        crate::types::Size::new(flex_w, row_h),
        flow.frame.size,
    );
    let x = flow.frame.content_origin.x + used_origin.x;
    let paint_y = flow.container_top_y - used_origin.y;
    let flex_geometry = LayoutBoxGeometry::from_layout(
        PdfRect::from_top(x, paint_y, flex_w, row_h),
        border,
        *flex_padding,
        child.paint.border_image.as_ref(),
    );
    let flex_box_geometry = ctx.text.pdf_writer.resolve_box_geometry(flex_geometry);
    let flex_fragment_geometry = flex_box_geometry.fragment(Default::default());
    let flex_background =
        flex_box_geometry.background(*flex_bg_origin, *flex_bg_clip, *flex_border_radii);
    let flex_gradient_reference = flex_background.positioning_area.generated_image_box();
    let flex_image_reference = flex_background.positioning_area.intrinsic_image_box();
    let flex_background_clip = flex_background.painting_box;
    let flex_gradient_area = LayerPaintArea::new(
        flex_gradient_reference,
        flex_background.image_destination_box,
    );
    let flex_group = PaintGroupScope::begin(content, child, flex_fragment_geometry, ctx);

    // A flex container that establishes a containing block records its
    // padding-box origin under its `positioned_depth`.
    if *flex_positioned_depth > 0 {
        let padding_box = flex_geometry.padding_box();
        abs_origins.insert(
            *flex_positioned_depth,
            PdfPoint::new(padding_box.left, padding_box.top()),
        );
    }

    if phase.paints_decoration() {
        render_box_shadows(
            content,
            box_shadow,
            flex_fragment_geometry,
            *flex_border_radii,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
            ctx.text.pdf_writer,
        );

        // Draw flex row background
        if let Some(color) = background_color {
            paint_solid_background(
                content,
                *color,
                flex_background_clip,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
            );
        }

        if let Some(gradient) = background_gradient {
            let gradient = linear_with_background_layer(
                gradient,
                background_layer_box(*flex_bg_size, *flex_bg_pos, *flex_bg_repeat),
            );
            flex_background_clip.push_clip(content);
            render_linear_gradient(
                content,
                &gradient,
                GradientBackdrop::isolated_linear_layer(
                    *background_color,
                    background_radial_gradient.is_some()
                        || background_conic_gradient.is_some()
                        || background_svg.is_some(),
                    child.paint.background.blend_mode.background_layer(0),
                ),
                flex_gradient_area,
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            content.push_str("Q\n");
        }

        if let Some(gradient) = background_radial_gradient {
            let gradient = radial_with_background_layer(
                gradient,
                background_layer_box(*flex_bg_size, *flex_bg_pos, *flex_bg_repeat),
            );
            let rounded_clip = flex_background_clip.push_rounded_clip(content);
            render_radial_gradient(
                content,
                &gradient,
                flex_gradient_area,
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if rounded_clip {
                content.push_str("Q\n");
            }
        }

        if let Some(gradient) = background_conic_gradient {
            let gradient = conic_with_background_layer(
                gradient,
                background_layer_box(*flex_bg_size, *flex_bg_pos, *flex_bg_repeat),
            );
            let rounded_clip = flex_background_clip.push_rounded_clip(content);
            render_conic_gradient(
                content,
                &gradient,
                flex_gradient_area,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if rounded_clip {
                content.push_str("Q\n");
            }
        }

        render_box_shadows_inset(
            content,
            box_shadow,
            flex_fragment_geometry,
            *flex_border_radii,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
            ctx.text.pdf_writer,
        );

        if let Some(svg_tree) = background_svg {
            render_svg_background(
                content,
                svg_tree,
                PdfBackgroundResources::new(
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                    ctx.shadings,
                    ctx.shading_counter,
                    Some(&mut *ctx.page_ext_gstates),
                )
                .with_custom_fonts(ctx.text.custom_fonts, ctx.text.prepared_custom_fonts),
                PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
                    flex_image_reference.into(),
                    flex_background.image_destination_box.into(),
                    flex_background_clip.radii,
                    *background_blur_radius,
                    *flex_bg_size,
                    *flex_bg_pos,
                    *flex_bg_repeat,
                )),
            );
        }

        // Draw the flex container's own border. Mirrors the top-level
        // FlexRow arm; the nested arm previously painted the background
        // but never the container border, so a bordered flex box nested
        // inside a block lost its frame entirely.
        if border.has_any() || child.paint.border_image.is_some() {
            paint_box_decoration(
                content,
                flex_fragment_geometry,
                border,
                *flex_border_radii,
                child.paint.border_image.as_ref(),
                BorderPaintResources::from_page(ctx),
            );
        }
    }

    // Render flex cells. Anchor each cell to its layout-computed
    // main-axis offset (which folds in justify-content spacing and
    // `gap`) instead of accumulating widths — mirrors the top-level
    // FlexRow arm. Without this, nested flex rows packed left and
    // ignored justify-content/gap entirely.
    let cell_base_x = flex_geometry.content_box().left;
    let content_y = flex_geometry.content_box().top();

    let stacking_scope = StackingScope::for_element(child);
    let mut stacking_plan = StackingPaintPlan::default();
    for cell in cells {
        let mut prior_box_paint_grid = None;
        let marker = ctx.stacking.marker();
        let mut cell_content = String::new();
        'paint_cell: {
            let content = &mut cell_content;
            let cell_supports_phases =
                crate::layout::elements::BoxPaintOwner::supports_phased_paint(cell);
            let cell_phase = if phase == ElementPaintPhase::All || cell_supports_phases {
                phase
            } else if phase == ElementPaintPhase::Contents {
                ElementPaintPhase::All
            } else {
                break 'paint_cell;
            };
            let cell_w = cell.width;
            let cell_x = cell_base_x + cell.x_offset;
            // Cross-axis (vertical) placement per align-items/align-self,
            // mirroring the top-level FlexRow arm. Stretch fills the line
            // cross size; otherwise the cell keeps its natural height and
            // is anchored at start/end/center. Without this the nested arm
            // force-stretched every cell to the full row height.
            // `align-self` on the item overrides the container's
            // `align-items` unless it is `auto`.
            let effective_align = flex_cell_align(cell, *align_items);
            let baseline_shift = if effective_align == AlignItems::Baseline {
                match (
                    flex_cell_baseline(cell, ctx.text.custom_fonts),
                    flex_line_max_baseline(
                        cells,
                        cell.line_id,
                        *align_items,
                        ctx.text.custom_fonts,
                    ),
                ) {
                    (Some(own), Some(max)) => (max - own).max(0.0),
                    _ => 0.0,
                }
            } else {
                0.0
            };
            let cross_geometry = cell.cross_geometry(*flex_row_h, *align_items, baseline_shift);
            let cell_h = cross_geometry.size;
            let cell_y_shift = cross_geometry.offset;
            let cell_top = content_y - cell_y_shift;
            let cell_bottom = cell_top - cell_h;
            let cell_geometry = LayoutBoxGeometry::from_layout(
                PdfRect::new(cell_x, cell_bottom, cell_w, cell_h),
                &cell.border,
                cell.padding,
                cell.paint.border_image.as_ref(),
            );
            if cell.role.is_atomic_inline() {
                prior_box_paint_grid = ctx
                    .text
                    .pdf_writer
                    .enter_atomic_inline_paint_grid(cell_geometry.border_box.top_left());
            }
            let cell_box_geometry = ctx.text.pdf_writer.resolve_box_geometry(cell_geometry);
            let cell_paint_geometry = cell_box_geometry.painting();
            let cell_fragment_geometry =
                cell_box_geometry.fragment(cell.fragmentation.box_fragmentation);
            let cell_background = cell_box_geometry.background(
                cell.paint.background.layers.origin,
                cell.paint.background.layers.clip,
                cell.paint.border_radii,
            );
            let cell_shadows = FlexCellShadows::new(cell, cell_fragment_geometry);
            let cell_group = PaintGroupScope::begin(content, cell, cell_fragment_geometry, ctx);
            if cell_phase == ElementPaintPhase::All
                && paint_cell_filter_output(content, &cell.paint, cell_paint_geometry, ctx)
            {
                cell_group.finish(content, ctx);
                break 'paint_cell;
            }
            if cell_phase.paints_decoration() {
                cell_shadows.paint_outset(content, ctx);
                // Draw cell background
                if let Some(color) = cell.paint.background.color {
                    paint_solid_background(
                        content,
                        color,
                        cell_background.painting_box,
                        ctx.page_ext_gstates,
                        ctx.bg_alpha_counter,
                    );
                }
                paint_box_gradient_backgrounds(content, &cell.paint, cell_box_geometry, ctx);
                cell_shadows.paint_inset(content, ctx);
                // Draw cell border through the shared rounded-ring painter.
                if cell.border.has_any() || cell.paint.border_image.is_some() {
                    paint_box_decoration(
                        content,
                        cell_fragment_geometry,
                        &cell.border,
                        cell.paint.border_radii,
                        cell.paint.border_image.as_ref(),
                        BorderPaintResources::from_page(ctx),
                    );
                }
            }
            // Draw cell text. Seat it relative to the cell's *content
            // box*, not its border box: the content origin is the
            // border-box top-left (`cell_top`, `cell_x`) inset by the
            // cell's top/left border and padding. This mirrors the
            // `flex_cell_baseline` model (`border-top + padding-top
            // + ...`) and the top-level FlexRow arm; without the inset the
            // text sat at the border-box top-left, painting it too high
            // and too far left.
            let content_box = cell_geometry.content_box();
            let content_left = content_box.left;
            let content_w = content_box.width;
            let mut baseline_cursor = TextBaselineCursor::new(
                content_box.top(),
                ctx.text.pdf_writer.page_content_transform,
            );
            for line in cell.lines.iter().filter(|_| cell_phase.paints_contents()) {
                let metrics = line_box_metrics(line, ctx.text.custom_fonts);
                let text_y = baseline_cursor.next_horizontal(metrics);
                let merged = crate::text::coalesce_text_runs(&line.runs);
                let line_width: f32 = merged
                    .iter()
                    .map(|r| estimate_run_width_with_fonts(r, ctx.text.custom_fonts))
                    .sum();
                let text_x = match cell.text_align {
                    TextAlign::Right => content_left + (content_w - line_width).max(0.0),
                    TextAlign::Center => content_left + (content_w - line_width).max(0.0) / 2.0,
                    _ => content_left,
                };
                let mut lx = text_x;
                for (run_index, run) in merged.iter().enumerate() {
                    let run_width = estimate_run_width_with_fonts(run, ctx.text.custom_fonts);
                    let previous = merged[..run_index].iter().rev().find(|previous| {
                        previous.inline_box.is_none() && !previous.text.is_empty()
                    });
                    let decoration = HorizontalRunDecorations::new(
                        run,
                        lx,
                        run_width,
                        text_y,
                        ctx.text.custom_fonts,
                    )
                    .continuing_after(previous);
                    let rw = decoration.paint_text(
                        content,
                        crate::layout::text::line_primary_font_size(&merged),
                        ctx.text.prepared_custom_fonts,
                        0.0,
                        ctx.text.pdf_writer,
                        ctx.text.page_images,
                    );
                    lx += rw;
                }
            }
            // Render nested elements in flex cells (tables, containers)
            if !cell.nested_elements.is_empty() {
                let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
                let nested_y = content_box.top() - text_h;
                render_container_children(
                    content,
                    &cell.nested_elements,
                    ContainerFrame::new(
                        PdfPoint::new(content_box.left, nested_y),
                        crate::types::Size::new(
                            content_box.width,
                            (content_box.height - text_h).max(0.0),
                        ),
                        PdfPoint::new(
                            content_box.left - cell.padding.left,
                            nested_y + cell.padding.top,
                        ),
                    ),
                    abs_origins,
                    ctx,
                    ContainerRenderOptions {
                        paint_phase: cell_phase,
                        stacking_scope: if cell.establishes_stacking_context() {
                            StackingScope::Local
                        } else {
                            StackingScope::Ancestor
                        },
                        ..Default::default()
                    },
                );
            }
            cell_group.finish(content, ctx);
        }
        let descendants = ctx.stacking.take_since(marker);
        if let Some(prior) = prior_box_paint_grid {
            ctx.text.pdf_writer.restore_box_paint_grid(prior);
        }
        ctx.stacking.commit(
            stacking_scope,
            content,
            &mut stacking_plan,
            cell.stacking_level(),
            cell_content,
            descendants,
        );
    }
    if stacking_scope.is_local() {
        ctx.stacking.paint_plan(stacking_plan, content);
    }
    flex_group.finish(content, ctx);
    if planned_flow_top.is_none() {
        cursor_y -= row_h + flex_mb;
        y = cursor_y;
    }
    prev_margin_bottom = *flex_mb;

    FlowPosition::new(y, cursor_y, prev_margin_bottom)
}
