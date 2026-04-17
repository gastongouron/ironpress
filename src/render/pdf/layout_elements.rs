use super::*;

pub(super) struct TextRenderContext<'a> {
    custom_fonts: &'a HashMap<String, TtfFont>,
    prepared_custom_fonts: &'a PreparedCustomFonts,
    annotations: &'a mut Vec<LinkAnnotation>,
}

impl<'a> TextRenderContext<'a> {
    pub(super) fn new(
        custom_fonts: &'a HashMap<String, TtfFont>,
        prepared_custom_fonts: &'a PreparedCustomFonts,
        annotations: &'a mut Vec<LinkAnnotation>,
    ) -> Self {
        Self {
            custom_fonts,
            prepared_custom_fonts,
            annotations,
        }
    }
}

pub(super) struct PageRenderContext<'a> {
    pdf_writer: &'a mut PdfWriter,
    page_images: &'a mut Vec<ImageRef>,
    shadings: &'a mut Vec<ShadingEntry>,
    shading_counter: &'a mut usize,
    pub(super) page_ext_gstates: &'a mut Vec<(String, f32)>,
    pub(super) bg_alpha_counter: &'a mut usize,
    text: TextRenderContext<'a>,
}

impl<'a> PageRenderContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        pdf_writer: &'a mut PdfWriter,
        page_images: &'a mut Vec<ImageRef>,
        custom_fonts: &'a HashMap<String, TtfFont>,
        prepared_custom_fonts: &'a PreparedCustomFonts,
        shadings: &'a mut Vec<ShadingEntry>,
        shading_counter: &'a mut usize,
        page_ext_gstates: &'a mut Vec<(String, f32)>,
        bg_alpha_counter: &'a mut usize,
        annotations: &'a mut Vec<LinkAnnotation>,
    ) -> Self {
        Self {
            pdf_writer,
            page_images,
            shadings,
            shading_counter,
            page_ext_gstates,
            bg_alpha_counter,
            text: TextRenderContext::new(custom_fonts, prepared_custom_fonts, annotations),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct NestedLayoutFrame {
    origin_x: f32,
    top_y: f32,
    initial_origin_x: f32,
    initial_top_y: f32,
    available_width: f32,
}

impl NestedLayoutFrame {
    pub(super) const fn new(
        origin_x: f32,
        top_y: f32,
        initial_origin_x: f32,
        initial_top_y: f32,
        available_width: f32,
    ) -> Self {
        Self {
            origin_x,
            top_y,
            initial_origin_x,
            initial_top_y,
            available_width,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct CellTextPlacement {
    cell_x: f32,
    content_top: f32,
    col_width: f32,
}

impl CellTextPlacement {
    pub(super) const fn new(cell_x: f32, content_top: f32, col_width: f32) -> Self {
        Self {
            cell_x,
            content_top,
            col_width,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct TableCellRenderBox {
    cell_x: f32,
    row_y: f32,
    col_width: f32,
    row_height: f32,
    nested_frame: NestedLayoutFrame,
}

impl TableCellRenderBox {
    pub(super) const fn new(
        cell_x: f32,
        row_y: f32,
        col_width: f32,
        row_height: f32,
        nested_frame: NestedLayoutFrame,
    ) -> Self {
        Self {
            cell_x,
            row_y,
            col_width,
            row_height,
            nested_frame,
        }
    }
}

pub(super) struct NestedTextBlock<'a> {
    pub(super) lines: &'a [TextLine],
    pub(super) text_align: TextAlign,
    pub(super) padding_top: f32,
    pub(super) padding_bottom: f32,
    pub(super) padding_left: f32,
    pub(super) padding_right: f32,
    pub(super) border: crate::layout::engine::LayoutBorder,
    pub(super) block_width: Option<f32>,
    pub(super) block_height: Option<f32>,
    pub(super) background_color: Option<(f32, f32, f32, f32)>,
    pub(super) background_svg: Option<&'a crate::parser::svg::SvgTree>,
    pub(super) background_blur_radius: f32,
    pub(super) background_size: BackgroundSize,
    pub(super) background_position: BackgroundPosition,
    pub(super) background_repeat: BackgroundRepeat,
    pub(super) background_origin: BackgroundOrigin,
    pub(super) background_blur_canvas_box: Option<SvgViewportBox>,
    pub(super) border_radius: f32,
}

/// Compute the height of a table row from its cells.
pub(super) fn compute_row_height(cells: &[TableCell]) -> f32 {
    cells
        .iter()
        .map(table_cell_content_height)
        .fold(0.0f32, f32::max)
}

pub(super) fn table_cell_geometry(
    col_widths: &[f32],
    col_pos: usize,
    colspan: usize,
    spacing: f32,
    origin_x: f32,
) -> (f32, f32) {
    let cell_x = origin_x + col_widths.iter().take(col_pos).sum::<f32>() + spacing * col_pos as f32;
    let cell_w = col_widths.iter().skip(col_pos).take(colspan).sum::<f32>()
        + spacing * colspan.saturating_sub(1) as f32;
    (cell_x, cell_w)
}

pub(super) fn render_cell_content(
    content: &mut String,
    cell: &TableCell,
    placement: TableCellRenderBox,
    ctx: &mut PageRenderContext<'_>,
) {
    let content_top = table_cell_content_top(cell, placement.row_y, placement.row_height);
    if !cell.nested_rows.is_empty() {
        let text_h: f32 = cell.lines.iter().map(|line| line.height).sum();
        render_cell_text(
            content,
            cell,
            CellTextPlacement::new(placement.cell_x, content_top, placement.col_width),
            &mut ctx.text,
        );
        render_nested_layout_elements(
            content,
            &cell.nested_rows,
            NestedLayoutFrame::new(
                placement.cell_x + cell.padding_left,
                content_top - text_h - cell.padding_bottom,
                placement.nested_frame.initial_origin_x,
                placement.nested_frame.initial_top_y,
                (placement.col_width - cell.padding_left - cell.padding_right).max(0.0),
            ),
            ctx,
        );
        return;
    }

    render_cell_text(
        content,
        cell,
        CellTextPlacement::new(placement.cell_x, content_top, placement.col_width),
        &mut ctx.text,
    );
}

pub(super) fn render_cell_text(
    content: &mut String,
    cell: &TableCell,
    placement: CellTextPlacement,
    ctx: &mut TextRenderContext<'_>,
) {
    let cell_inner_w = placement.col_width - cell.padding_left - cell.padding_right;
    let mut text_y = placement.content_top;
    for line in &cell.lines {
        let metrics = line_box_metrics(line, ctx.custom_fonts);
        text_y -= metrics.half_leading + metrics.ascender;
        let line_annotation_box = TextLineAnnotationBox {
            top: text_y + metrics.ascender + metrics.half_leading,
            bottom: text_y - metrics.descender - metrics.half_leading,
        };
        let text_content: String = line.runs.iter().map(|run| run.text.as_str()).collect();
        if text_content.is_empty() {
            continue;
        }
        let merged = merge_runs(&line.runs);
        let line_width: f32 = merged
            .iter()
            .map(|run| estimate_run_width_with_fonts(run, ctx.custom_fonts))
            .sum();
        let text_x = match cell.text_align {
            TextAlign::Right => {
                placement.cell_x + cell.padding_left + (cell_inner_w - line_width).max(0.0)
            }
            TextAlign::Center => {
                placement.cell_x + cell.padding_left + ((cell_inner_w - line_width) / 2.0).max(0.0)
            }
            _ => placement.cell_x + cell.padding_left,
        };
        let mut x = text_x;
        for run in &merged {
            if run.text.is_empty() {
                continue;
            }
            let (r, g, b) = run.color;
            let run_width = estimate_run_width_with_fonts(run, ctx.custom_fonts);

            if let Some((background_r, background_g, background_b, _background_a)) =
                run.background_color
            {
                let (pad_h, pad_v) = run.padding;
                let rx = x - pad_h;
                let ry = text_y - 2.0 - pad_v;
                let rw2 = run_width + pad_h * 2.0;
                let rh = run.font_size + 2.0 + pad_v * 2.0;
                content.push_str(&format!(
                    "{background_r} {background_g} {background_b} rg\n"
                ));
                if run.border_radius > 0.0 {
                    content.push_str(&rounded_rect_path(rx, ry, rw2, rh, run.border_radius));
                    content.push_str("\nf\n");
                } else {
                    content.push_str(&format!("{rx} {ry} {rw2} {rh} re\nf\n"));
                }
            }

            render_run_text(
                content,
                run,
                x,
                text_y,
                ctx.custom_fonts,
                ctx.prepared_custom_fonts,
            );

            if run.underline {
                let (_, descender_ratio) = crate::fonts::font_metrics_ratios(
                    &run.font_family,
                    run.bold,
                    run.italic,
                    ctx.custom_fonts,
                );
                let desc = descender_ratio * run.font_size;
                let underline_y = text_y - desc * 0.6;
                let thickness = (run.font_size * 0.07).max(0.5);
                content.push_str(&format!(
                    "{r} {g} {b} RG\n{thickness} w\n{x} {underline_y} m {x2} {underline_y} l\nS\n",
                    x2 = x + run_width,
                ));
            }

            if run.line_through {
                let strike_y = text_y + run.font_size * 0.3;
                let thickness = (run.font_size * 0.07).max(0.5);
                content.push_str(&format!(
                    "{r} {g} {b} RG\n{thickness} w\n{x} {strike_y} m {x2} {strike_y} l\nS\n",
                    x2 = x + run_width,
                ));
            }

            if let Some(annotation) =
                text_run_link_annotation(run, x, run_width, line_annotation_box)
            {
                ctx.annotations.push(annotation);
            }

            x += run_width;
        }
        text_y -= metrics.descender + metrics.half_leading;
    }
}

fn table_cell_content_top(cell: &TableCell, row_y: f32, row_height: f32) -> f32 {
    let content_height = table_cell_content_height(cell);
    let offset = match cell.vertical_align {
        VerticalAlign::Middle => ((row_height - content_height) / 2.0).max(0.0),
        VerticalAlign::Bottom => (row_height - content_height).max(0.0),
        VerticalAlign::Top
        | VerticalAlign::Baseline
        | VerticalAlign::Super
        | VerticalAlign::Sub => 0.0,
    };
    row_y - offset - cell.padding_top
}

pub(super) fn table_row_total_height(row: &LayoutElement) -> f32 {
    match row {
        LayoutElement::TableRow {
            cells,
            margin_top,
            margin_bottom,
            ..
        } => margin_top + compute_row_height(cells) + margin_bottom,
        _ => 0.0,
    }
}

pub(super) fn render_nested_text_block(
    content: &mut String,
    block: NestedTextBlock<'_>,
    frame: NestedLayoutFrame,
    ctx: &mut PageRenderContext<'_>,
) {
    let render_width = block.block_width.unwrap_or(frame.available_width).max(0.0);
    let total_height = text_block_total_height(
        block.lines,
        block.padding_top,
        block.padding_bottom,
        block.block_height,
    );
    let block_bottom = frame.top_y - total_height;

    if let Some((r, g, b, a)) = block.background_color {
        let needs_bg_alpha = a < 1.0;
        if needs_bg_alpha {
            let gs_name = format!("GSba{}", ctx.bg_alpha_counter);
            *ctx.bg_alpha_counter += 1;
            ctx.page_ext_gstates.push((gs_name.clone(), a));
            content.push_str(&format!("/{gs_name} gs\n"));
        }
        content.push_str(&format!("{r} {g} {b} rg\n"));
        if block.border_radius > 0.0 {
            content.push_str(&rounded_rect_path(
                frame.origin_x,
                block_bottom,
                render_width,
                total_height,
                block.border_radius,
            ));
        } else {
            content.push_str(&format!(
                "{x} {y} {w} {h} re\n",
                x = frame.origin_x,
                y = block_bottom,
                w = render_width,
                h = total_height,
            ));
        }
        content.push_str("f\n");
        if needs_bg_alpha {
            content.push_str("/GSDefault gs\n");
        }
    }

    if let Some(svg_tree) = block.background_svg {
        let (ref_x, ref_y, ref_w, ref_h) = match block.background_origin {
            BackgroundOrigin::Border => (
                frame.origin_x - block.border.left.width,
                block_bottom - block.border.bottom.width,
                render_width + block.border.left.width + block.border.right.width,
                total_height + block.border.top.width + block.border.bottom.width,
            ),
            BackgroundOrigin::Content => (
                frame.origin_x + block.padding_left,
                block_bottom + block.padding_bottom,
                (render_width - block.padding_left - block.padding_right).max(0.0),
                (total_height - block.padding_top - block.padding_bottom).max(0.0),
            ),
            BackgroundOrigin::Padding => (frame.origin_x, block_bottom, render_width, total_height),
        };
        render_svg_background(
            content,
            svg_tree,
            ctx.pdf_writer,
            ctx.page_images,
            ctx.shadings,
            ctx.shading_counter,
            Some(ctx.page_ext_gstates),
            BackgroundPaintContext::new(
                SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                SvgViewportBox::new(
                    frame.origin_x - block.border.left.width,
                    block_bottom - block.border.bottom.width,
                    render_width + block.border.left.width + block.border.right.width,
                    total_height + block.border.top.width + block.border.bottom.width,
                ),
                block.border_radius,
                block.background_blur_radius,
                block.background_size,
                block.background_position,
                block.background_repeat,
            )
            .with_blur_canvas_box(block.background_blur_canvas_box),
        );
    }

    if block.border.has_any() {
        let x1 = frame.origin_x;
        let x2 = frame.origin_x + render_width;
        let y_top = frame.top_y;
        let y_bottom = block_bottom;
        if block.border.top.width > 0.0 {
            let (r, g, b) = block.border.top.color;
            content.push_str(dash_pattern_for_style(block.border.top.style));
            content.push_str(&format!(
                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                block.border.top.width
            ));
            content.push_str(reset_dash_pattern(block.border.top.style));
        }
        if block.border.right.width > 0.0 {
            let (r, g, b) = block.border.right.color;
            content.push_str(dash_pattern_for_style(block.border.right.style));
            content.push_str(&format!(
                "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                block.border.right.width
            ));
            content.push_str(reset_dash_pattern(block.border.right.style));
        }
        if block.border.bottom.width > 0.0 {
            let (r, g, b) = block.border.bottom.color;
            content.push_str(dash_pattern_for_style(block.border.bottom.style));
            content.push_str(&format!(
                "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                block.border.bottom.width
            ));
            content.push_str(reset_dash_pattern(block.border.bottom.style));
        }
        if block.border.left.width > 0.0 {
            let (r, g, b) = block.border.left.color;
            content.push_str(dash_pattern_for_style(block.border.left.style));
            content.push_str(&format!(
                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                block.border.left.width
            ));
            content.push_str(reset_dash_pattern(block.border.left.style));
        }
    }

    if !block.lines.is_empty() {
        let proxy_cell = TableCell {
            lines: block.lines.to_vec(),
            nested_rows: Vec::new(),
            bold: false,
            background_color: None,
            padding_top: block.padding_top,
            padding_right: block.padding_right,
            padding_bottom: block.padding_bottom,
            padding_left: block.padding_left,
            colspan: 1,
            rowspan: 1,
            border: crate::layout::engine::LayoutBorder::default(),
            text_align: block.text_align,
            vertical_align: VerticalAlign::Baseline,
        };
        render_cell_text(
            content,
            &proxy_cell,
            CellTextPlacement::new(
                frame.origin_x,
                frame.top_y - block.padding_top,
                render_width,
            ),
            &mut ctx.text,
        );
    }
}

pub(super) fn render_nested_layout_elements(
    content: &mut String,
    elements: &[LayoutElement],
    frame: NestedLayoutFrame,
    ctx: &mut PageRenderContext<'_>,
) {
    let mut planned = plan_nested_layout_elements(elements, frame);
    planned.sort_by_key(|planned_element| layout_element_paint_order(planned_element.element));

    for planned_element in planned {
        match planned_element.element {
            LayoutElement::TableRow {
                cells,
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
                let row_y = planned_element.top_y;
                let row_height = compute_row_height(cells);

                let mut col_pos: usize = 0;
                for cell in cells {
                    if cell.rowspan == 0 {
                        col_pos += cell.colspan;
                        continue;
                    }

                    let (cell_x, cell_w) = table_cell_geometry(
                        col_widths,
                        col_pos,
                        cell.colspan,
                        spacing,
                        planned_element.origin_x,
                    );

                    let cell_height = if cell.rowspan > 1 {
                        let mut total_height = row_height;
                        for offset in 1..cell.rowspan {
                            let future_idx = planned_element.source_index + offset;
                            if let Some(future_row) = elements.get(future_idx) {
                                total_height += table_row_total_height(future_row);
                            }
                        }
                        total_height
                    } else {
                        row_height
                    };

                    if let Some((r, g, b, a)) = cell.background_color {
                        let needs_cell_bg_alpha = a < 1.0;
                        if needs_cell_bg_alpha {
                            let gs_name = format!("GSba{}", ctx.bg_alpha_counter);
                            *ctx.bg_alpha_counter += 1;
                            ctx.page_ext_gstates.push((gs_name.clone(), a));
                            content.push_str(&format!("/{gs_name} gs\n"));
                        }
                        content.push_str(&format!(
                            "{r} {g} {b} rg\n{x} {y} {w} {h} re\nf\n",
                            x = cell_x,
                            y = row_y - cell_height,
                            w = cell_w,
                            h = cell_height,
                        ));
                        if needs_cell_bg_alpha {
                            content.push_str("/GSDefault gs\n");
                        }
                    }

                    if cell.border.has_any() {
                        let x1 = cell_x;
                        let x2 = cell_x + cell_w;
                        let y_top = row_y;
                        let y_bottom = row_y - cell_height;
                        if cell.border.top.width > 0.0 {
                            let (r, g, b) = cell.border.top.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                cell.border.top.width
                            ));
                        }
                        if cell.border.right.width > 0.0 {
                            let (r, g, b) = cell.border.right.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                cell.border.right.width
                            ));
                        }
                        if cell.border.bottom.width > 0.0 {
                            let (r, g, b) = cell.border.bottom.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                cell.border.bottom.width
                            ));
                        }
                        if cell.border.left.width > 0.0 {
                            let (r, g, b) = cell.border.left.color;
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                cell.border.left.width
                            ));
                        }
                    }

                    render_cell_content(
                        content,
                        cell,
                        TableCellRenderBox::new(cell_x, row_y, cell_w, row_height, frame),
                        ctx,
                    );

                    col_pos += cell.colspan;
                }
            }
            LayoutElement::TextBlock {
                lines,
                text_align,
                background_color,
                padding_top,
                padding_bottom,
                padding_left,
                padding_right,
                border,
                block_width,
                block_height,
                border_radius,
                background_gradient: _,
                background_radial_gradient: _,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                ..
            } => {
                render_nested_text_block(
                    content,
                    NestedTextBlock {
                        lines,
                        text_align: *text_align,
                        padding_top: *padding_top,
                        padding_bottom: *padding_bottom,
                        padding_left: *padding_left,
                        padding_right: *padding_right,
                        border: *border,
                        block_width: *block_width,
                        block_height: *block_height,
                        background_color: *background_color,
                        background_svg: background_svg.as_ref(),
                        background_blur_radius: *background_blur_radius,
                        background_size: *background_size,
                        background_position: *background_position,
                        background_repeat: *background_repeat,
                        background_origin: *background_origin,
                        background_blur_canvas_box: planned_element.blur_canvas_box,
                        border_radius: *border_radius,
                    },
                    NestedLayoutFrame::new(
                        planned_element.origin_x,
                        planned_element.top_y,
                        frame.initial_origin_x,
                        frame.initial_top_y,
                        planned_element.available_width,
                    ),
                    ctx,
                );
            }
            _ => {}
        }
    }
}

pub(super) struct PlannedNestedElement<'a> {
    pub(super) element: &'a LayoutElement,
    pub(super) source_index: usize,
    pub(super) origin_x: f32,
    pub(super) top_y: f32,
    pub(super) available_width: f32,
    pub(super) blur_canvas_box: Option<SvgViewportBox>,
}

pub(super) fn plan_nested_layout_elements(
    elements: &[LayoutElement],
    frame: NestedLayoutFrame,
) -> Vec<PlannedNestedElement<'_>> {
    let mut cursor_y = frame.top_y;
    let mut positioned_origins: HashMap<usize, (f32, f32)> = HashMap::new();
    let mut planned = Vec::with_capacity(elements.len());

    for (element_idx, element) in elements.iter().enumerate() {
        match element {
            LayoutElement::TableRow {
                cells,
                margin_top,
                margin_bottom,
                ..
            } => {
                cursor_y -= *margin_top;
                let row_y = cursor_y;
                planned.push(PlannedNestedElement {
                    element,
                    source_index: element_idx,
                    origin_x: frame.origin_x,
                    top_y: row_y,
                    available_width: frame.available_width,
                    blur_canvas_box: None,
                });
                cursor_y -= compute_row_height(cells) + *margin_bottom;
            }
            LayoutElement::TextBlock {
                margin_top,
                margin_bottom,
                containing_block,
                positioned_depth,
                position,
                offset_top,
                offset_left,
                lines,
                padding_top,
                padding_bottom,
                block_height,
                ..
            } => {
                let containing_origin =
                    containing_block.and_then(|cb| positioned_origins.get(&cb.depth).copied());
                let base_origin_x = match position {
                    Position::Absolute => {
                        containing_origin.map_or(frame.initial_origin_x, |(x, _)| x)
                    }
                    _ => containing_origin.map_or(frame.origin_x, |(x, _)| x),
                };
                let base_top_y = match position {
                    Position::Absolute => {
                        containing_origin.map_or(frame.initial_top_y, |(_, y)| y) - *margin_top
                    }
                    _ => cursor_y - *margin_top,
                };
                let element_top_y = match position {
                    Position::Absolute | Position::Relative => base_top_y - *offset_top,
                    Position::Static => base_top_y,
                };
                let element_origin_x = base_origin_x + offset_left;
                let blur_canvas_box = containing_block.and_then(|cb| {
                    containing_origin
                        .map(|(x, y)| SvgViewportBox::new(x, y - cb.height, cb.width, cb.height))
                });
                planned.push(PlannedNestedElement {
                    element,
                    source_index: element_idx,
                    origin_x: element_origin_x,
                    top_y: element_top_y,
                    available_width: frame.available_width,
                    blur_canvas_box,
                });
                if *positioned_depth > 0
                    && (*position == Position::Relative || *position == Position::Absolute)
                {
                    positioned_origins.insert(*positioned_depth, (element_origin_x, element_top_y));
                }
                if *position != Position::Absolute {
                    cursor_y = base_top_y
                        - text_block_total_height(
                            lines,
                            *padding_top,
                            *padding_bottom,
                            *block_height,
                        )
                        - *margin_bottom;
                }
            }
            _ => {}
        }
    }

    planned
}
