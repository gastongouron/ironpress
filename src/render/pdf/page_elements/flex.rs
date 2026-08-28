use super::*;
use crate::layout::elements::FlexRow;

pub(in crate::render::pdf) fn render_flex_row(
    content: &mut String,
    element: &FlexRow,
    frame: PageElementFrame<'_>,
    phase: ElementPaintPhase,
    ctx: &mut PageRenderContext<'_>,
) {
    let page_size = frame.page_size;
    let margin = frame.margin;
    let y_pos = &frame.y_pos;
    let cells = &element.content.cells;
    let row_height = &element.content.row_height;
    let flex_offset_left = element.inline_offset.value();
    let background_color = &element.paint.background.color;
    let container_width = element.box_model.size.resolve_width(frame.available_width);
    let padding = &element.box_model.padding;
    let border = &element.box_model.border;
    let flex_radii = &element.paint.border_radii;
    let box_shadow = &element.paint.shadows;
    let background_gradient = &element.paint.background.layers.gradient;
    let background_radial_gradient = &element.paint.background.layers.radial_gradient;
    let background_conic_gradient = &element.paint.background.layers.conic_gradient;
    let background_svg = ctx
        .text
        .pdf_writer
        .resolve_background_svg(&element.paint.background.layers);
    let background_svg = &background_svg;
    let background_blur_radius = &element.paint.background.layers.blur_radius;
    let flex_bg_size = &element.paint.background.layers.size;
    let flex_bg_pos = &element.paint.background.layers.position;
    let flex_bg_repeat = &element.paint.background.layers.repeat;
    let flex_bg_origin = &element.paint.background.layers.origin;
    let flex_bg_clip = &element.paint.background.layers.clip;
    let align_items = &element.content.alignment;
    let row_y = page_size.height - margin.top - y_pos;
    let flow_height = element.box_model.size.height.resolve(*row_height);
    let full_height = padding.vertical() + flow_height + border.vertical_width();
    // Inline-axis origin of the flex container's border box: the
    // page content-left plus the container's own resolved
    // horizontal margin / auto-centering (see `FlexRow.offset_left`).
    // Flex establishes a formatting context, not a separate positioning model:
    // its authored inline inset must survive exactly like a block container's.
    // Pagination already resolves the block-axis inset into `frame.y_pos`.
    let flex_left = margin.left + flex_offset_left + element.positioning.insets.left;
    // Inline-axis origin of the flex *content* box: in-flow cells
    // begin inside the container's left border (CSS box model — a
    // cell's `x_offset` is measured from the content box, so the
    // border-left width must be added, mirroring the cross-axis
    // `text_area_top` which already subtracts `border.top.width`).
    let cells_left = flex_left + border.left.width;
    let flex_geometry = LayoutBoxGeometry::from_layout(
        PdfRect::new(flex_left, row_y - full_height, container_width, full_height),
        border,
        *padding,
        element.paint.border_image.as_ref(),
    );
    let flex_box_geometry = ctx.text.pdf_writer.resolve_box_geometry(flex_geometry);
    let flex_paint_geometry = flex_box_geometry.painting();
    let flex_fragment_geometry = flex_box_geometry.fragment(Default::default());
    let flex_paint_box = flex_paint_geometry.border_box;
    let flex_background_geometry =
        flex_box_geometry.background(*flex_bg_origin, *flex_bg_clip, *flex_radii);
    let flex_gradient_reference = flex_background_geometry
        .positioning_area
        .generated_image_box();
    let flex_image_reference = flex_background_geometry
        .positioning_area
        .intrinsic_image_box();
    let flex_background_clip = flex_background_geometry.painting_box;
    let flex_gradient_area = LayerPaintArea::new(
        flex_gradient_reference,
        flex_background_geometry.image_destination_box,
    );
    let flex_group = PaintGroupScope::begin(content, element, flex_fragment_geometry, ctx);

    if phase.paints_decoration() {
        // Draw box shadow with blur
        render_box_shadows(
            content,
            box_shadow,
            flex_fragment_geometry,
            *flex_radii,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
            ctx.text.pdf_writer,
        );

        // Draw container background
        if let Some(background) = background_color {
            paint_solid_background(
                content,
                *background,
                flex_background_clip,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
            );
        }

        // Draw container linear gradient
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
                    element.paint.background.blend_mode.background_layer(0),
                ),
                flex_gradient_area,
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            content.push_str("Q\n");
        }

        // Draw container radial gradient
        if let Some(gradient) = background_radial_gradient {
            let gradient = radial_with_background_layer(
                gradient,
                background_layer_box(*flex_bg_size, *flex_bg_pos, *flex_bg_repeat),
            );
            let clipped = flex_background_clip.push_rounded_clip(content);
            render_radial_gradient(
                content,
                &gradient,
                flex_gradient_area,
                ctx.shadings,
                ctx.shading_counter,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if clipped {
                content.push_str("Q\n");
            }
        }

        // Draw container conic gradient
        if let Some(gradient) = background_conic_gradient {
            let gradient = conic_with_background_layer(
                gradient,
                background_layer_box(*flex_bg_size, *flex_bg_pos, *flex_bg_repeat),
            );
            let clipped = flex_background_clip.push_rounded_clip(content);
            render_conic_gradient(
                content,
                &gradient,
                flex_gradient_area,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );
            if clipped {
                content.push_str("Q\n");
            }
        }

        // Draw inset box-shadow for flex container (after backgrounds).
        render_box_shadows_inset(
            content,
            box_shadow,
            flex_fragment_geometry,
            *flex_radii,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
            ctx.text.pdf_writer,
        );

        // Draw SVG background image for flex container
        if let Some(svg_tree) = background_svg {
            render_svg_background(
                content,
                svg_tree,
                PdfBackgroundResources::new(
                    ctx.text.pdf_writer,
                    ctx.text.page_images,
                    ctx.shadings,
                    ctx.shading_counter,
                    Some(ctx.page_ext_gstates),
                )
                .with_custom_fonts(ctx.text.custom_fonts, ctx.text.prepared_custom_fonts),
                PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
                    flex_image_reference.into(),
                    flex_background_geometry.image_destination_box.into(),
                    flex_background_clip.radii,
                    *background_blur_radius,
                    *flex_bg_size,
                    *flex_bg_pos,
                    *flex_bg_repeat,
                )),
            );
        }

        // Draw border
        if border.has_any() || element.paint.border_image.is_some() {
            paint_box_decoration(
                content,
                flex_fragment_geometry,
                border,
                *flex_radii,
                element.paint.border_image.as_ref(),
                BorderPaintResources::from_page(ctx),
            );
        }
    }

    if !phase.paints_contents() {
        flex_group.finish(content, ctx);
        return;
    }

    // Overflow clips the descendants of a flex formatting context to its
    // padding box, exactly as for an ordinary container. Register the clip with
    // the stacking scheduler too so positioned descendants cannot escape when
    // their paint is deferred to an ancestor context.
    let needs_clip = element.overflow.combined.clips();
    if needs_clip {
        let mut command = String::from("q\n");
        command.push_str(&overflow_clip_path(
            flex_paint_box.left,
            flex_paint_box.bottom,
            flex_paint_box.width,
            flex_paint_box.height,
            flex_paint_geometry.border,
            *flex_radii,
        ));
        command.push_str("W n\n");
        content.push_str(&command);
        ctx.stacking.push_clip(command);
    }

    // Render each flex cell at its computed x-offset
    let text_area_top = row_y - border.top.width - padding.top;

    // Flex order is already resolved by layout. Traverse in that order for
    // geometry, then schedule each item in the flex container's nearest CSS
    // stacking context. Non-context items allow their positioned descendants
    // to escape just like ordinary block ancestors.
    let stacking_scope = StackingScope::for_element(element);
    let mut stacking_plan = StackingPaintPlan::default();
    for cell in cells {
        let mut prior_box_paint_grid = None;
        let marker = ctx.stacking.marker();
        let mut cell_content = String::new();
        'paint_cell: {
            let content = &mut cell_content;
            let cell_x = cells_left + padding.left + cell.x_offset;
            // For single-line rows `line_cross_size == row_height`.
            // For multi-line wrap, each cell's line_cross_size is its
            // own flex line height, so alignment is per-line.
            // Compute per-cell height and vertical offset based on the
            // effective cross-axis alignment. Pagination uses the same domain
            // helper when it propagates fragmentainer space into descendants.
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
            let cross_geometry = cell.cross_geometry(*row_height, *align_items, baseline_shift);
            let cell_render_h = cell
                .fragmentation
                .fragment_block_extent
                .unwrap_or(cross_geometry.size);
            let cell_y_shift = cross_geometry.offset;
            let cell_geometry = LayoutBoxGeometry::from_layout(
                PdfRect::new(
                    cell_x,
                    text_area_top - cell_y_shift - cell_render_h,
                    cell.width,
                    cell_render_h,
                ),
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
            let cell_paint_box = cell_paint_geometry.border_box;
            let cell_background = cell_box_geometry.background(
                cell.paint.background.layers.origin,
                cell.paint.background.layers.clip,
                cell.paint.border_radii,
            );
            let cell_background_clip = cell_background.painting_box;
            let cell_background_svg = ctx
                .text
                .pdf_writer
                .resolve_background_svg(&cell.paint.background.layers);
            let cell_shadows = FlexCellShadows::new(cell, cell_fragment_geometry);
            let cell_inner_w = cell_geometry.content_box().width;
            let cell_group = PaintGroupScope::begin(content, cell, cell_fragment_geometry, ctx);
            if paint_cell_filter_output(content, &cell.paint, cell_paint_geometry, ctx) {
                cell_group.finish(content, ctx);
                break 'paint_cell;
            }
            cell_shadows.paint_outset(content, ctx);

            if cell.paint.background.layers.blur_radius > 0.0
                && cell.lines.is_empty()
                && cell.nested_elements.is_empty()
                && cell.paint.background.layers.gradient.is_none()
                && cell.paint.background.layers.radial_gradient.is_none()
                && cell.paint.background.layers.conic_gradient.is_none()
                && cell_background_svg.is_none()
                && cell.paint.border_radii.is_zero()
                && let Some(blurred) = crate::render::blur::blur_box(
                    cell_paint_box.width,
                    cell_paint_box.height,
                    cell.paint.background.color,
                    &cell.border,
                    cell.paint.background.layers.blur_radius,
                    ctx.text.pdf_writer.opts.raster_quality.filter_dpi,
                )
            {
                let img_obj_id = ctx.text.pdf_writer.add_image_object(
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
                    w = cell_paint_box.width + 2.0 * ov,
                    h = cell_paint_box.height + 2.0 * ov,
                    ix = cell_paint_box.left - ov,
                    iy = cell_paint_box.bottom - ov,
                    name = img_name,
                ));
                ctx.text.page_images.push(ImageRef {
                    name: img_name,
                    obj_id: img_obj_id,
                });
                cell_group.finish(content, ctx);
                break 'paint_cell;
            }

            // Draw cell background
            if let Some(background) = cell.paint.background.color {
                paint_solid_background(
                    content,
                    background,
                    cell_background_clip,
                    ctx.page_ext_gstates,
                    ctx.bg_alpha_counter,
                );
            }

            paint_box_gradient_backgrounds(content, &cell.paint, cell_box_geometry, ctx);

            cell_shadows.paint_inset(content, ctx);

            // Draw cell borders through the same geometry used by every other box.
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

            if let Some(svg_tree) = &cell_background_svg {
                render_svg_background(
                    content,
                    svg_tree,
                    PdfBackgroundResources::new(
                        ctx.text.pdf_writer,
                        ctx.text.page_images,
                        ctx.shadings,
                        ctx.shading_counter,
                        Some(ctx.page_ext_gstates),
                    )
                    .with_custom_fonts(ctx.text.custom_fonts, ctx.text.prepared_custom_fonts),
                    PdfBackgroundPaintContext::local(BackgroundPaintContext::new(
                        cell_background
                            .positioning_area
                            .intrinsic_image_box()
                            .into(),
                        cell_background.image_destination_box.into(),
                        cell_background_clip.radii,
                        cell.paint.background.layers.blur_radius,
                        cell.paint.background.layers.size,
                        cell.paint.background.layers.position,
                        cell.paint.background.layers.repeat,
                    )),
                );
            }

            // Render cell text
            let mut baseline_cursor = TextBaselineCursor::new(
                text_area_top - cell_y_shift - cell.border.top.width - cell.padding.top,
                ctx.text.pdf_writer.page_content_transform,
            );
            for line in &cell.lines {
                let metrics = line_box_metrics(line, ctx.text.custom_fonts);
                let text_y = baseline_cursor.next_horizontal(metrics);
                let line_annotation_top = text_y + metrics.ascender + metrics.half_leading;
                let line_annotation_bottom = text_y - metrics.descender - metrics.half_leading;
                let text_content: String = line.runs.iter().map(|r| r.text.as_str()).collect();
                if text_content.is_empty() {
                    continue;
                }
                let merged = crate::text::coalesce_text_runs(&line.runs);
                // Calculate line width for text-align
                let line_width: f32 = merged
                    .iter()
                    .map(|run| estimate_run_width_with_fonts(run, ctx.text.custom_fonts))
                    .sum();
                let text_x = match cell.text_align {
                    TextAlign::Right => {
                        cell_x
                            + cell.border.left.width
                            + cell.padding.left
                            + (cell_inner_w - line_width).max(0.0)
                    }
                    TextAlign::Center => {
                        cell_x
                            + cell.border.left.width
                            + cell.padding.left
                            + ((cell_inner_w - line_width) / 2.0).max(0.0)
                    }
                    _ => cell_x + cell.border.left.width + cell.padding.left,
                };
                let mut x = text_x;
                for (run_index, run) in merged.iter().enumerate() {
                    if let Some(advance) = run.atomic_inline_advance() {
                        x += advance;
                        continue;
                    }
                    if run.text.is_empty() {
                        continue;
                    }
                    let rw = estimate_run_width_with_fonts(run, ctx.text.custom_fonts);
                    let previous = merged[..run_index].iter().rev().find(|previous| {
                        previous.inline_box.is_none() && !previous.text.is_empty()
                    });
                    let decoration =
                        HorizontalRunDecorations::new(run, x, rw, text_y, ctx.text.custom_fonts)
                            .continuing_after(previous);

                    // Draw background rectangle for inline spans
                    if let Some(background) = run.background_color {
                        let (br, bgc, bb, ba) = background.to_f32_rgba();
                        let needs_inline_bg_alpha = ba < 1.0;
                        if needs_inline_bg_alpha {
                            let gs_name = format!("GSfiba{}", ctx.bg_alpha_counter);
                            *ctx.bg_alpha_counter += 1;
                            ctx.page_ext_gstates.push((gs_name.clone(), ba));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        let rx = x - run.padding.left;
                        let rw2 = rw + run.padding.horizontal();
                        let (ry, rh) = inline_background_y_and_height(
                            run,
                            text_y,
                            run.padding,
                            ctx.text.custom_fonts,
                        );
                        content.push_str(&format!("{br} {bgc} {bb} rg\n"));
                        content.push_str(
                            &PdfRect::new(rx, ry, rw2, rh)
                                .rounded(run.border_radii)
                                .path_or_rect(),
                        );
                        content.push_str("f\n");
                        if needs_inline_bg_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    decoration.paint_text(
                        content,
                        crate::layout::text::line_primary_font_size(&merged),
                        ctx.text.prepared_custom_fonts,
                        0.0,
                        ctx.text.pdf_writer,
                        ctx.text.page_images,
                    );

                    if decoration_is_emphasis(run) {
                        render_text_emphasis_marks(
                            content,
                            run,
                            TextEmphasisPlacement {
                                origin: PdfPoint::new(x, text_y),
                                color: run.metadata.emphasis.color,
                            },
                            ctx.text.custom_fonts,
                            ctx.text.prepared_custom_fonts,
                            ctx.text.pdf_writer,
                            ctx.text.page_images,
                        );
                    }

                    if let Some(annotation) = text_run_link_annotation(
                        run,
                        PdfRect::new(
                            x,
                            line_annotation_bottom,
                            rw,
                            line_annotation_top - line_annotation_bottom,
                        ),
                    ) {
                        ctx.text.annotations.push(annotation);
                    }

                    x += rw;
                }
            }

            // Render nested elements (tables, images, SVGs, blocks,
            // etc. inside flex/inline-block items) through the shared
            // block child renderer so variant support matches normal
            // container children.
            if !cell.nested_elements.is_empty() {
                let text_h: f32 = cell.lines.iter().map(|l| l.height).sum();
                let (nested_x, nested_y, nested_w, padding_origin) = match cell.nested_origin {
                    FlexNestedOrigin::ContentBox => {
                        let x = cell_x + cell.border.left.width + cell.padding.left;
                        let y = text_area_top
                            - cell_y_shift
                            - cell.border.top.width
                            - cell.padding.top
                            - text_h;
                        (
                            x,
                            y,
                            (cell.width
                                - cell.border.horizontal_width()
                                - cell.padding.horizontal())
                            .max(0.0),
                            PdfPoint::new(x - cell.padding.left, y + cell.padding.top + text_h),
                        )
                    }
                    FlexNestedOrigin::TableBorderBox => {
                        let y = text_area_top - cell_y_shift - text_h;
                        (cell_x, y, cell.width, PdfPoint::new(cell_x, y + text_h))
                    }
                };
                let mut nested_abs_origins: HashMap<usize, PdfPoint> = HashMap::new();
                if element.positioning.containing_block_depth > 0 {
                    let padding_box = flex_geometry.padding_box();
                    nested_abs_origins.insert(
                        element.positioning.containing_block_depth,
                        PdfPoint::new(padding_box.left, padding_box.top()),
                    );
                }
                render_container_children(
                    content,
                    &cell.nested_elements,
                    ContainerFrame::new(
                        PdfPoint::new(nested_x, nested_y),
                        crate::types::Size::new(
                            nested_w,
                            (cell_geometry.content_box().height - text_h).max(0.0),
                        ),
                        padding_origin,
                    ),
                    &mut nested_abs_origins,
                    ctx,
                    ContainerRenderOptions {
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
    if needs_clip {
        ctx.stacking.pop_clip();
        content.push_str("Q\n");
    }
    flex_group.finish(content, ctx);
}
