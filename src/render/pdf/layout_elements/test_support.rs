use super::*;
use crate::layout::cells::TableRowCells;
use crate::layout::elements::{LayoutElementTestExt, LayoutNode};

#[cfg(test)]
pub(in crate::render::pdf) fn table_row_total_height(row: &dyn LayoutElement) -> f32 {
    row.inspect_table(|row| row.flow.outer_extent(row.content.cells.row_block_extent()))
        .unwrap_or_default()
}

#[cfg(test)]
pub(in crate::render::pdf) fn render_nested_text_block(
    content: &mut String,
    block: NestedTextBlock<'_>,
    frame: NestedLayoutFrame,
    ctx: &mut PageRenderContext<'_>,
) {
    let render_width = block.block_width.unwrap_or(frame.available_width).max(0.0);
    let padding_box_height =
        text_block_total_height(block.lines, block.padding, block.block_height, block.clips);
    let border_box_height = padding_box_height + block.border.vertical_width();
    let geometry = LayoutBoxGeometry::from_layout(
        PdfRect::from_top(
            frame.origin.x,
            frame.origin.y,
            render_width,
            border_box_height,
        ),
        &block.border,
        block.padding,
        None,
    );
    let box_geometry = ctx.text.pdf_writer.resolve_box_geometry(geometry);
    let paint_geometry = box_geometry.painting();
    let fragment_geometry = box_geometry.fragment(Default::default());
    // CSS `filter: blur()` on a solid box (css-filter-effects-1 §4.1): rasterize
    // the box's painted output (bg fill + border), gaussian-blur it, and embed it
    // overflowing the border box. Restricted to a plain solid box (no SVG/raster
    // bg, no text, no clip, square corners) so the vector paint path below is
    // byte-unchanged for everything else. `background_blur_radius` carries the
    // element's `style.filter.blur_radius` here.
    if block.background_blur_radius > 0.0
        && block.lines.is_empty()
        && block.background_svg.is_none()
        && !block.clips
        && block.border_radii.is_zero()
        && let Some(blurred) = crate::render::blur::blur_box(
            paint_geometry.border_box.width,
            paint_geometry.border_box.height,
            block.background_color,
            &block.border,
            block.background_blur_radius,
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
            w = paint_geometry.border_box.width + 2.0 * ov,
            h = paint_geometry.border_box.height + 2.0 * ov,
            ix = paint_geometry.border_box.left - ov,
            iy = paint_geometry.border_box.bottom - ov,
            name = img_name,
        ));
        ctx.text.page_images.push(ImageRef {
            name: img_name,
            obj_id: img_obj_id,
        });
        return;
    }

    let background_geometry = box_geometry.background(
        block.background_origin,
        block.background_clip,
        block.border_radii,
    );
    let background_clip = background_geometry.painting_box;

    if let Some(color) = block.background_color {
        paint_solid_background(
            content,
            color,
            background_clip,
            ctx.page_ext_gstates,
            ctx.bg_alpha_counter,
        );
    }

    if let Some(svg_tree) = block.background_svg {
        render_svg_background(
            content,
            svg_tree,
            PdfBackgroundResources::new(
                ctx.text.pdf_writer,
                ctx.text.page_images,
                ctx.shadings,
                ctx.shading_counter,
                Some(ctx.page_ext_gstates),
            ),
            PdfBackgroundPaintContext::local(
                BackgroundPaintContext::new(
                    background_geometry
                        .positioning_area
                        .intrinsic_image_box()
                        .into(),
                    background_geometry.image_destination_box.into(),
                    background_clip.radii,
                    block.background_blur_radius,
                    block.background_size,
                    block.background_position,
                    block.background_repeat,
                )
                .with_blur_canvas_box(block.background_blur_canvas_box),
            ),
        );
    }

    paint_box_decoration(
        content,
        fragment_geometry,
        &block.border,
        block.border_radii,
        None,
        BorderPaintResources::from_page(ctx),
    );

    if !block.lines.is_empty() {
        let proxy_cell = CellBox {
            content: crate::layout::cells::CellContent {
                lines: block.lines.to_vec(),
                ..Default::default()
            },
            box_model: crate::layout::cells::CellBoxModel {
                content_insets: block.padding,
                ..Default::default()
            },
            alignment: crate::layout::cells::CellAlignment {
                inline: block.text_align,
                ..Default::default()
            },
            ..Default::default()
        };
        render_cell_text(
            content,
            &proxy_cell,
            CellTextPlacement::new(
                PdfPoint::new(
                    geometry.padding_box().left,
                    geometry.padding_box().top() - block.padding.top,
                ),
                geometry.padding_box().width,
            )
            .with_first_line_indent(block.text_indent),
            ctx,
        );
    }
}

#[cfg(test)]
pub(in crate::render::pdf) fn render_nested_layout_elements(
    content: &mut String,
    elements: &[LayoutNode],
    frame: NestedLayoutFrame,
    ctx: &mut PageRenderContext<'_>,
) {
    let mut planned = plan_nested_layout_elements(elements, frame);
    planned.sort_by_key(|planned_element| layout_element_paint_order(planned_element.element));

    for planned_element in planned {
        if planned_element
            .element
            .inspect_table(|row| {
                let cells = &row.content.cells;
                let cell_frames = row.cell_inline_frames();
                let row_y = planned_element.origin.y;
                let row_height = cells.row_block_extent();
                let baseline_shifts = row_baseline_shifts(cells, ctx.text.custom_fonts);

                for (cell_idx, cell) in cells.iter().enumerate() {
                    let Some(cell_frame) = cell_frames.get(cell_idx).copied().flatten() else {
                        continue;
                    };
                    let cell_x = planned_element.origin.x + cell_frame.offset();
                    let cell_w = cell_frame.extent();

                    let cell_height = if cell.span.rows > 1 {
                        let mut total_height = row_height;
                        for offset in 1..cell.span.rows {
                            let future_idx = planned_element.source_index + offset;
                            if let Some(future_row) = elements.get(future_idx) {
                                total_height += table_row_total_height(future_row);
                            }
                        }
                        total_height
                    } else {
                        row_height
                    };
                    let cell_geometry = LayoutBoxGeometry::from_layout(
                        PdfRect::new(cell_x, row_y - cell_height, cell_w, cell_height),
                        &cell.layout.box_model.border,
                        cell.layout.box_model.padding(),
                        cell.layout.paint.border_image.as_ref(),
                    );
                    let cell_box_geometry = ctx.text.pdf_writer.resolve_box_geometry(cell_geometry);
                    let cell_paint_geometry = cell_box_geometry.painting();
                    let cell_fragment_geometry = cell_box_geometry.fragment(Default::default());

                    if let Some(color) = cell
                        .layout
                        .paint
                        .background
                        .color
                        .filter(|_| !cell.table.hide_if_empty)
                    {
                        let (r, g, b) = color.to_f32_rgb();
                        let a = color.alpha();
                        let needs_cell_bg_alpha = a < 1.0;
                        if needs_cell_bg_alpha {
                            let gs_name = format!("GSba{}", ctx.bg_alpha_counter);
                            *ctx.bg_alpha_counter += 1;
                            ctx.page_ext_gstates.push((gs_name.clone(), a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            x = cell_paint_geometry.border_box.left,
                            y = cell_paint_geometry.border_box.bottom,
                            w = cell_paint_geometry.border_box.width,
                            h = cell_paint_geometry.border_box.height,
                        ));
                        if needs_cell_bg_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    if !cell.table.hide_if_empty {
                        paint_box_decoration(
                            content,
                            cell_fragment_geometry,
                            &cell.layout.box_model.border,
                            cell.layout.paint.border_radii,
                            cell.layout.paint.border_image.as_ref(),
                            BorderPaintResources::from_page(ctx),
                        );
                    }

                    let abs_origins = HashMap::new();
                    render_cell_content(
                        content,
                        &cell.layout,
                        CellRenderBox::new(PdfPoint::new(cell_x, row_y), cell_w, row_height)
                            .with_baseline_shift(
                                baseline_shifts.get(cell_idx).copied().unwrap_or(0.0),
                            ),
                        &abs_origins,
                        ctx,
                    );
                }
            })
            .is_some()
        {
            continue;
        }

        if planned_element
            .element
            .inspect_text(|element| {
                let lines = &element.lines;
                let text_align = &element.text.alignment;
                let background_color = &element.paint.background.color;
                let padding = &element.box_model.padding;
                let border = &element.box_model.border;
                let block_width = &element.box_model.size.width;
                let block_height = &element.box_model.size.height;
                let border_radii = &element.paint.border_radii;
                let clip_rect = &element.clipping.rect;
                let background = &element.paint.background.layers;
                let text_indent = &element.text.indent;
                render_nested_text_block(
                    content,
                    NestedTextBlock {
                        lines,
                        text_align: *text_align,
                        padding: *padding,
                        border: *border,
                        block_width: block_width.fixed_value(),
                        block_height: block_height.used(),
                        clips: clip_rect.is_some(),
                        background_color: *background_color,
                        background_svg: background.svg.as_ref(),
                        background_blur_radius: background.blur_radius,
                        background_size: background.size,
                        background_position: background.position,
                        background_repeat: background.repeat,
                        background_origin: background.origin,
                        background_clip: background.clip,
                        background_blur_canvas_box: planned_element.blur_canvas_box,
                        border_radii: *border_radii,
                        text_indent: *text_indent,
                    },
                    NestedLayoutFrame::new(
                        planned_element.origin,
                        frame.initial_origin,
                        planned_element.available_width,
                    ),
                    ctx,
                );
            })
            .is_some()
        {
            continue;
        }

        planned_element.element.inspect_container(|element| {
            let children = &element.children;
            let background_color = &element.paint.background.color;
            let border = &element.box_model.border;
            let border_radii = &element.paint.border_radii;
            let padding = &element.box_model.padding;
            let block_width = &element.box_model.size.width;
            let block_height = &element.box_model.size.height;
            let visible = &element.paint.visible;
            let background = &element.paint.background.layers;
            let render_width = block_width
                .resolve(planned_element.available_width)
                .max(0.0);
            // `NestedTextBlock` accepts a padding-box height, while a
            // Container carries a border-box height. Normalize that contract
            // once before handing the box to the shared painter.
            let children_h: f32 = children
                .iter()
                .map(|child| crate::layout::engine::estimate_element_height(child.as_ref()))
                .sum();
            let padding_box_h = block_height
                .used()
                .map_or(padding.vertical() + children_h, |height| {
                    (height - border.vertical_width()).max(0.0)
                });
            let geometry = LayoutBoxGeometry::from_layout(
                PdfRect::from_top(
                    planned_element.origin.x,
                    planned_element.origin.y,
                    render_width,
                    padding_box_h + border.vertical_width(),
                ),
                border,
                *padding,
                element.paint.border_image.as_ref(),
            );
            // Paint the container's own background + border box (no text).
            // CSS2 §11.2: `visibility: hidden` suppresses only this box's own
            // decoration; a `visibility: visible` descendant still paints, so
            // the children below are recursed regardless of `visible`.
            if *visible {
                render_nested_text_block(
                    content,
                    NestedTextBlock {
                        lines: &[],
                        text_align: TextAlign::Left,
                        padding: *padding,
                        border: *border,
                        block_width: Some(render_width),
                        block_height: Some(padding_box_h),
                        // `padding_box_h` already resolves the definite/auto height, and
                        // there are no lines to grow it, so clipping is moot here.
                        clips: false,
                        background_color: *background_color,
                        background_svg: background.svg.as_ref(),
                        background_blur_radius: background.blur_radius,
                        background_size: background.size,
                        background_position: background.position,
                        background_repeat: background.repeat,
                        background_origin: background.origin,
                        background_clip: background.clip,
                        background_blur_canvas_box: planned_element.blur_canvas_box,
                        border_radii: *border_radii,
                        text_indent: 0.0,
                    },
                    NestedLayoutFrame::new(
                        planned_element.origin,
                        frame.initial_origin,
                        render_width,
                    ),
                    ctx,
                );
            }
            // Recurse into the container's children at its content origin.
            if !children.is_empty() {
                let content_box = geometry.content_box();
                render_nested_layout_elements(
                    content,
                    children,
                    NestedLayoutFrame::new(
                        PdfPoint::new(content_box.left, content_box.top()),
                        frame.initial_origin,
                        content_box.width,
                    ),
                    ctx,
                );
            }
        });
    }
}

#[cfg(test)]
pub(in crate::render::pdf) struct PlannedNestedElement<'a> {
    pub(in crate::render::pdf) element: &'a dyn LayoutElement,
    pub(in crate::render::pdf) source_index: usize,
    pub(in crate::render::pdf) origin: PdfPoint,
    pub(in crate::render::pdf) available_width: f32,
    pub(in crate::render::pdf) blur_canvas_box: Option<SvgViewportBox>,
}

#[cfg(test)]
pub(in crate::render::pdf) fn plan_nested_layout_elements(
    elements: &[LayoutNode],
    frame: NestedLayoutFrame,
) -> Vec<PlannedNestedElement<'_>> {
    let mut cursor_y = frame.origin.y;
    let mut positioned_origins: HashMap<usize, PdfPoint> = HashMap::new();
    let mut planned = Vec::with_capacity(elements.len());

    for (element_idx, element) in elements.iter().enumerate() {
        if element
            .inspect_table(|row| {
                let cells = &row.content.cells;
                cursor_y -= row.flow.margins.start + row.flow.internal.start;
                let row_y = cursor_y;
                planned.push(PlannedNestedElement {
                    element,
                    source_index: element_idx,
                    origin: PdfPoint::new(frame.origin.x, row_y),
                    available_width: frame.available_width,
                    blur_canvas_box: None,
                });
                cursor_y -= cells.row_block_extent()
                    + row.flow.internal.end
                    + row.flow.extra_end
                    + row.flow.margins.end;
            })
            .is_some()
        {
            continue;
        }

        if element
            .inspect_text(|text| {
                let margin_top = &text.box_model.margins.start;
                let margin_bottom = &text.box_model.margins.end;
                let containing_block = &text.positioning.containing_block;
                let positioned_depth = &text.positioning.containing_block_depth;
                let position = &text.positioning.scheme;
                let offset_top = &text.positioning.insets.top;
                let offset_left = &text.positioning.insets.left;
                let lines = &text.lines;
                let padding = &text.box_model.padding;
                let block_height = &text.box_model.size.height;
                let clip_rect = &text.clipping.rect;
                let containing_origin =
                    containing_block.and_then(|cb| positioned_origins.get(&cb.depth).copied());
                let base_origin_x = match position {
                    Position::Absolute | Position::Fixed => {
                        containing_origin.map_or(frame.initial_origin.x, |origin| origin.x)
                    }
                    _ => containing_origin.map_or(frame.origin.x, |origin| origin.x),
                };
                let base_top_y = match position {
                    Position::Absolute | Position::Fixed => {
                        containing_origin.map_or(frame.initial_origin.y, |origin| origin.y)
                            - *margin_top
                    }
                    _ => cursor_y - *margin_top,
                };
                let element_top_y = match position {
                    Position::Absolute
                    | Position::Fixed
                    | Position::Relative
                    | Position::Sticky => base_top_y - *offset_top,
                    Position::Static => base_top_y,
                };
                let element_origin_x = base_origin_x + offset_left;
                let blur_canvas_box = containing_block.and_then(|cb| {
                    containing_origin.map(|origin| {
                        PdfRect::from_top(origin.x, origin.y, cb.width, cb.height).into()
                    })
                });
                planned.push(PlannedNestedElement {
                    element,
                    source_index: element_idx,
                    origin: PdfPoint::new(element_origin_x, element_top_y),
                    available_width: frame.available_width,
                    blur_canvas_box,
                });
                if *positioned_depth > 0 && position.is_positioned() {
                    positioned_origins.insert(
                        *positioned_depth,
                        PdfPoint::new(element_origin_x, element_top_y),
                    );
                }
                if !position.is_absolute() {
                    cursor_y = base_top_y
                        - text_block_total_height(
                            lines,
                            *padding,
                            block_height.used(),
                            clip_rect.is_some(),
                        )
                        - *margin_bottom;
                }
            })
            .is_some()
        {
            continue;
        }

        if element
            .inspect_container(|container| {
                let margin_top = &container.box_model.margins.start;
                let margin_bottom = &container.box_model.margins.end;
                let position = &container.positioning.scheme;
                let offset_top = &container.positioning.insets.top;
                let offset_left = &container.positioning.insets.left;
                // A block child of a cell (e.g. a `<div>` with a background)
                // flattens to a Container. Position it in the cell's flow like a
                // TextBlock so its background/border/children paint instead of
                // being silently dropped.
                let base_top_y = cursor_y - *margin_top;
                let element_top_y = match position {
                    Position::Absolute
                    | Position::Fixed
                    | Position::Relative
                    | Position::Sticky => base_top_y - *offset_top,
                    Position::Static => base_top_y,
                };
                let element_origin_x = frame.origin.x + offset_left;
                planned.push(PlannedNestedElement {
                    element,
                    source_index: element_idx,
                    origin: PdfPoint::new(element_origin_x, element_top_y),
                    available_width: frame.available_width,
                    blur_canvas_box: None,
                });
                if !position.is_absolute() {
                    let box_h = crate::layout::engine::estimate_element_height(element)
                        - *margin_top
                        - *margin_bottom;
                    cursor_y = base_top_y - box_h - *margin_bottom;
                }
            })
            .is_some()
        {
            continue;
        }

        if element
            .inspect_image(|image| {
                let height = &image.geometry.size.height;
                let flow_extra_bottom = &image.geometry.flow.extra_end;
                let margin_top = &image.geometry.flow.margins.start;
                let margin_bottom = &image.geometry.flow.margins.end;
                let offset_top = &image.positioning.insets.top;
                let offset_left = &image.positioning.insets.left;
                let top_y = cursor_y - *margin_top;
                planned.push(PlannedNestedElement {
                    element,
                    source_index: element_idx,
                    origin: PdfPoint::new(frame.origin.x + *offset_left, top_y - *offset_top),
                    available_width: frame.available_width,
                    blur_canvas_box: None,
                });
                cursor_y = top_y - *height - *flow_extra_bottom - *margin_bottom;
            })
            .is_some()
        {
            continue;
        }

        element.inspect_svg(|svg| {
            let height = &svg.geometry.size.height;
            let flow_extra_bottom = &svg.geometry.flow.extra_end;
            let margin_top = &svg.geometry.flow.margins.start;
            let margin_bottom = &svg.geometry.flow.margins.end;
            let offset_top = &svg.positioning.insets.top;
            let offset_left = &svg.positioning.insets.left;
            let top_y = cursor_y - *margin_top;
            planned.push(PlannedNestedElement {
                element,
                source_index: element_idx,
                origin: PdfPoint::new(frame.origin.x + *offset_left, top_y - *offset_top),
                available_width: frame.available_width,
                blur_canvas_box: None,
            });
            cursor_y = top_y - *height - *flow_extra_bottom - *margin_bottom;
        });
    }

    planned
}
