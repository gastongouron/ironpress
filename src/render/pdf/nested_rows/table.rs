use super::*;
use crate::layout::cells::TableRowCells;
use crate::layout::elements::TableRow;

impl NestedRowsRenderer<'_, '_> {
    pub(super) fn render_table_row(&mut self, element: &TableRow) {
        self.handled = true;
        let content = &mut *self.content;
        let origin_x = self.origin_x;
        let mut cursor_y = self.cursor_y;
        let page_ext_gstates = &mut *self.page_ext_gstates;
        let bg_alpha_counter = &mut *self.bg_alpha_counter;
        let custom_fonts = self.custom_fonts;
        let prepared_custom_fonts = self.prepared_custom_fonts;
        let page_shadings = &mut *self.page_shadings;
        let shading_counter = &mut *self.shading_counter;
        let pdf_writer = &mut *self.pdf_writer;
        let page_images = &mut *self.page_images;
        let annotations = &mut *self.annotations;
        let page_paint_box = self.page_paint_box;
        let initial_fixed_origin = self.initial_fixed_origin;
        let page_height = self.page_height;
        let cells = &element.content.cells;
        let col_widths = &element.content.column_widths;
        let border_collapse = element.formatting.border_collapse;
        let outer_margins = element.flow.margins;
        let internal_spacing = element.flow.internal;
        let flow_extra_bottom = element.flow.extra_end;
        let cell_frames = element.cell_inline_frames();
        // A header or footer row that pagination repeated is page furniture
        // in tagged output: the structure tree keeps the authored row, and
        // readers hear each header once (Matterhorn 18-001). Its cells paint
        // as artifacts and its links get no annotations. Untagged output is
        // unchanged.
        let repeated_copy = pdf_writer.structure.is_enabled()
            && element.fragmentation.instance == crate::layout::elements::RowInstance::RepeatedCopy;
        let annotation_marker = annotations.len();
        if self.first_margin == FirstMarginState::Pending {
            cursor_y -=
                collapsed_margin_top_extra(outer_margins.start, self.previous_margin_bottom);
        }
        self.first_margin = FirstMarginState::Pending;
        cursor_y -= internal_spacing.start;
        let row_y = cursor_y;
        let row_height = cells.row_block_extent();
        if !self.paint {
            cursor_y -= row_height + internal_spacing.end + flow_extra_bottom + outer_margins.end;
            self.cursor_y = cursor_y;
            self.previous_margin_bottom = outer_margins.end;
            return;
        }
        let collapsed = border_collapse == BorderCollapse::Collapse;
        if collapsed && self.table_cell_phase.paints_borders() {
            paint_resolved_collapsed_row_borders(
                content,
                &element.collapsed_borders,
                CollapsedRowBorderGeometry::new(
                    col_widths,
                    origin_x + element.grid_inline_offset(),
                    row_y,
                    row_height,
                    pdf_writer.page_content_transform,
                ),
                page_ext_gstates,
                bg_alpha_counter,
            );
        }
        if collapsed && self.table_cell_phase == TableCellPaintPhase::Borders {
            cursor_y -= row_height + internal_spacing.end + flow_extra_bottom + outer_margins.end;
            self.cursor_y = cursor_y;
            self.previous_margin_bottom = outer_margins.end;
            return;
        }
        let baseline_shifts = row_baseline_shifts(cells, custom_fonts);
        let stacking_scope = StackingScope::for_element(element);
        let mut stacking_plan = StackingPaintPlan::default();
        for (cell_idx, cell) in cells.iter().enumerate() {
            let Some(cell_frame) = cell_frames.get(cell_idx).copied().flatten() else {
                continue;
            };
            let phaseable = cell.layout.stacking_level().is_in_flow()
                && crate::layout::elements::BoxPaintOwner::supports_phased_paint(&cell.layout);
            let cell_phase = match (self.table_cell_phase, phaseable) {
                (TableCellPaintPhase::All, _) => TableCellPaintPhase::All,
                (phase, true) => phase,
                (TableCellPaintPhase::Contents, false) => TableCellPaintPhase::All,
                _ => continue,
            };
            let marker = self.stacking.marker();
            let mut cell_content = String::new();
            // The region lives in the cell's own content, so it stays around
            // the cell when the stacking plan moves that content.
            let cell_region = if repeated_copy {
                pdf_writer
                    .structure
                    .begin_artifact(&mut cell_content, structure::ArtifactKind::Pagination)
            } else {
                structure::OpenMark::Skipped
            };
            'paint_cell: {
                let content = &mut cell_content;
                let cell_x = origin_x + cell_frame.offset();
                let cell_w = cell_frame.extent();
                let cell_height = row_height
                    + self
                        .row_heights
                        .iter()
                        .skip(self.element_index + 1)
                        .take(cell.span.rows.saturating_sub(1))
                        .flatten()
                        .sum::<f32>();
                let cell_geometry = LayoutBoxGeometry::from_layout(
                    PdfRect::new(cell_x, row_y - cell_height, cell_w, cell_height),
                    &cell.layout.box_model.border,
                    cell.layout.box_model.padding(),
                    cell.layout.paint.border_image.as_ref(),
                );
                let cell_box_geometry = pdf_writer.resolve_box_geometry(cell_geometry);
                let cell_paint_geometry = cell_box_geometry.painting();
                let cell_fragment_geometry = cell_box_geometry.fragment(Default::default());
                let cell_background = cell_box_geometry.background(
                    cell.layout.paint.background.layers.origin,
                    cell.layout.paint.background.layers.clip,
                    cell.layout.paint.border_radii,
                );
                let cell_group = {
                    let mut cell_ctx = PageRenderContext::new(
                        pdf_writer,
                        page_images,
                        custom_fonts,
                        prepared_custom_fonts,
                        page_shadings,
                        shading_counter,
                        page_ext_gstates,
                        bg_alpha_counter,
                        annotations,
                        page_paint_box,
                        page_height,
                    )
                    .with_initial_fixed_origin(initial_fixed_origin);
                    PaintGroupScope::begin(
                        content,
                        &cell.layout,
                        cell_fragment_geometry,
                        &mut cell_ctx,
                    )
                };
                let filtered = cell_phase == TableCellPaintPhase::All && {
                    let mut cell_ctx = PageRenderContext::new(
                        pdf_writer,
                        page_images,
                        custom_fonts,
                        prepared_custom_fonts,
                        page_shadings,
                        shading_counter,
                        page_ext_gstates,
                        bg_alpha_counter,
                        annotations,
                        page_paint_box,
                        page_height,
                    )
                    .with_initial_fixed_origin(initial_fixed_origin);
                    paint_box_filter_output(
                        content,
                        &cell.layout,
                        cell_paint_geometry,
                        &mut cell_ctx,
                    )
                };
                if filtered {
                    let mut cell_ctx = PageRenderContext::new(
                        pdf_writer,
                        page_images,
                        custom_fonts,
                        prepared_custom_fonts,
                        page_shadings,
                        shading_counter,
                        page_ext_gstates,
                        bg_alpha_counter,
                        annotations,
                        page_paint_box,
                        page_height,
                    )
                    .with_initial_fixed_origin(initial_fixed_origin);
                    cell_group.finish(content, &mut cell_ctx);
                    break 'paint_cell;
                }
                if cell_phase.paints_backgrounds() && !cell.table.hide_if_empty {
                    render_box_shadows(
                        content,
                        &cell.layout.paint.shadows,
                        cell_fragment_geometry,
                        cell.layout.paint.border_radii,
                        page_ext_gstates,
                        bg_alpha_counter,
                        pdf_writer,
                    );
                }
                if cell_phase.paints_backgrounds() {
                    let background_boundary =
                        CollapsedCellBackgroundBoundary::for_late_cell(cell, border_collapse);
                    let background_clipped =
                        background_boundary.begin(content, cell_paint_geometry.border_box);
                    if let Some(background) = cell
                        .layout
                        .paint
                        .background
                        .color
                        .filter(|_| !cell.table.hide_if_empty)
                    {
                        paint_solid_background(
                            content,
                            background,
                            cell_background.painting_box,
                            page_ext_gstates,
                            bg_alpha_counter,
                        );
                    }
                    if !cell.table.hide_if_empty {
                        let mut cell_ctx = PageRenderContext::new(
                            pdf_writer,
                            page_images,
                            custom_fonts,
                            prepared_custom_fonts,
                            page_shadings,
                            shading_counter,
                            page_ext_gstates,
                            bg_alpha_counter,
                            annotations,
                            page_paint_box,
                            page_height,
                        )
                        .with_initial_fixed_origin(initial_fixed_origin);
                        paint_box_gradient_backgrounds(
                            content,
                            &cell.layout.paint,
                            cell_box_geometry,
                            &mut cell_ctx,
                        );
                        render_box_shadows_inset(
                            content,
                            &cell.layout.paint.shadows,
                            cell_fragment_geometry,
                            cell.layout.paint.border_radii,
                            cell_ctx.page_ext_gstates,
                            cell_ctx.bg_alpha_counter,
                            cell_ctx.text.pdf_writer,
                        );
                    }
                    CollapsedCellBackgroundBoundary::finish(content, background_clipped);
                }

                let border = &cell.layout.box_model.border;
                if cell_phase.paints_borders()
                    && !collapsed
                    && (border.has_any() || cell.layout.paint.border_image.is_some())
                    && !cell.table.hide_if_empty
                {
                    paint_box_decoration(
                        content,
                        cell_fragment_geometry,
                        border,
                        cell.layout.paint.border_radii,
                        cell.layout.paint.border_image.as_ref(),
                        BorderPaintResources {
                            shadings: page_shadings,
                            shading_counter,
                            page_ext_gstates,
                            alpha_counter: bg_alpha_counter,
                            pdf_writer,
                            page_images,
                        },
                    );
                }

                if cell_phase.paints_contents() {
                    let content_clip = cell.table.clips.then(|| {
                        ContentClip::rounded_padding_box(
                            cell_paint_geometry,
                            cell.layout.paint.border_radii,
                        )
                    });
                    if let Some(clip) = &content_clip {
                        clip.begin(content, self.stacking);
                    }
                    let mut page_context = PageRenderContext::new(
                        pdf_writer,
                        page_images,
                        custom_fonts,
                        prepared_custom_fonts,
                        page_shadings,
                        shading_counter,
                        page_ext_gstates,
                        bg_alpha_counter,
                        annotations,
                        page_paint_box,
                        page_height,
                    )
                    .with_initial_fixed_origin(initial_fixed_origin);
                    page_context.stacking = self.stacking.fork();
                    render_cell_content(
                        content,
                        &cell.layout,
                        CellRenderBox::new(PdfPoint::new(cell_x, row_y), cell_w, row_height)
                            .with_baseline_shift(
                                baseline_shifts.get(cell_idx).copied().unwrap_or(0.0),
                            ),
                        self.abs_origins,
                        &mut page_context,
                    );
                    cell_group.finish(content, &mut page_context);
                    self.stacking.restore(page_context.stacking.take_since(0));
                    if let Some(clip) = &content_clip {
                        clip.finish(content, self.stacking);
                    }
                } else {
                    let mut page_context = PageRenderContext::new(
                        pdf_writer,
                        page_images,
                        custom_fonts,
                        prepared_custom_fonts,
                        page_shadings,
                        shading_counter,
                        page_ext_gstates,
                        bg_alpha_counter,
                        annotations,
                        page_paint_box,
                        page_height,
                    )
                    .with_initial_fixed_origin(initial_fixed_origin);
                    cell_group.finish(content, &mut page_context);
                }
            }
            pdf_writer.structure.end(&mut cell_content, cell_region);
            let descendants = self.stacking.take_since(marker);
            self.stacking.commit(
                stacking_scope,
                content,
                &mut stacking_plan,
                cell.layout.stacking_level(),
                cell_content,
                descendants,
            );
        }
        if stacking_scope.is_local() {
            self.stacking.paint_plan(stacking_plan, content);
        }
        if repeated_copy {
            annotations.truncate(annotation_marker);
        }
        cursor_y -= row_height + internal_spacing.end + flow_extra_bottom + outer_margins.end;
        self.cursor_y = cursor_y;
        self.previous_margin_bottom = outer_margins.end;
    }
}
