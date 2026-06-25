use super::*;

pub(super) struct TextRenderContext<'a> {
    custom_fonts: &'a HashMap<String, TtfFont>,
    prepared_custom_fonts: &'a PreparedCustomFonts,
    annotations: &'a mut Vec<LinkAnnotation>,
    // Threaded so `render_cell_text` can embed blurred `text-shadow` image
    // XObjects (it rasterizes + blurs the shadow glyphs, like the page path).
    pdf_writer: &'a mut PdfWriter,
    page_images: &'a mut Vec<ImageRef>,
}

impl<'a> TextRenderContext<'a> {
    pub(super) fn new(
        custom_fonts: &'a HashMap<String, TtfFont>,
        prepared_custom_fonts: &'a PreparedCustomFonts,
        annotations: &'a mut Vec<LinkAnnotation>,
        pdf_writer: &'a mut PdfWriter,
        page_images: &'a mut Vec<ImageRef>,
    ) -> Self {
        Self {
            custom_fonts,
            prepared_custom_fonts,
            annotations,
            pdf_writer,
            page_images,
        }
    }
}

pub(super) struct PageRenderContext<'a> {
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
            shadings,
            shading_counter,
            page_ext_gstates,
            bg_alpha_counter,
            text: TextRenderContext::new(
                custom_fonts,
                prepared_custom_fonts,
                annotations,
                pdf_writer,
                page_images,
            ),
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
    /// Extra horizontal offset applied to the FIRST rendered line only (CSS
    /// `text-indent`). Negative values pull the first line left, used to hang a
    /// list marker into the surrounding padding.
    first_line_indent: f32,
}

impl CellTextPlacement {
    pub(super) const fn new(cell_x: f32, content_top: f32, col_width: f32) -> Self {
        Self {
            cell_x,
            content_top,
            col_width,
            first_line_indent: 0.0,
        }
    }

    pub(super) const fn with_first_line_indent(mut self, first_line_indent: f32) -> Self {
        self.first_line_indent = first_line_indent;
        self
    }
}

#[derive(Clone, Copy)]
pub(super) struct TableCellRenderBox {
    cell_x: f32,
    row_y: f32,
    col_width: f32,
    row_height: f32,
    nested_frame: NestedLayoutFrame,
    /// Extra downward offset applied to this cell's content so a
    /// `vertical-align: baseline` cell's first text baseline lines up with the
    /// common baseline of the other baseline-aligned cells in the same row. 0.0
    /// when the cell is not baseline-aligned or shares the row's tallest
    /// baseline (the common case, so existing single-font rows are unaffected).
    baseline_shift: f32,
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
            baseline_shift: 0.0,
        }
    }

    pub(super) const fn with_baseline_shift(mut self, shift: f32) -> Self {
        self.baseline_shift = shift;
        self
    }
}

/// First text baseline distance from a cell's content-box top: the leading above
/// the first line plus its ascent. Returns `None` for cells with no rendered
/// text line (nothing to baseline-align).
pub(super) fn table_cell_first_baseline(
    cell: &TableCell,
    custom_fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    let line = cell
        .lines
        .iter()
        .find(|line| line.runs.iter().any(|run| !run.text.is_empty()))?;
    let metrics = line_box_metrics(line, custom_fonts);
    Some(cell.padding_top + metrics.half_leading + metrics.ascender)
}

/// Per-cell baseline shifts for one row: each `vertical-align: baseline` cell
/// with text is offset down so its first baseline matches the row's deepest
/// baseline. Index i corresponds to `cells[i]`; non-baseline / text-less cells
/// get 0.0. All-equal rows (same font + line-height) yield all-zero shifts, so
/// uniform tables render exactly as before.
pub(super) fn row_baseline_shifts(
    cells: &[TableCell],
    custom_fonts: &HashMap<String, TtfFont>,
) -> Vec<f32> {
    let baselines: Vec<Option<f32>> = cells
        .iter()
        .map(|cell| {
            if cell.vertical_align == VerticalAlign::Baseline {
                table_cell_first_baseline(cell, custom_fonts)
            } else {
                None
            }
        })
        .collect();
    let common = baselines
        .iter()
        .filter_map(|b| *b)
        .fold(f32::NEG_INFINITY, f32::max);
    if !common.is_finite() {
        return vec![0.0; cells.len()];
    }
    baselines
        .iter()
        .map(|b| b.map_or(0.0, |own| (common - own).max(0.0)))
        .collect()
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
    /// Whether the box clips overflow (`overflow: hidden`/`scroll`). When true a
    /// definite `block_height` is a hard size and content is clipped to it rather
    /// than growing the box.
    pub(super) clips: bool,
    pub(super) background_color: Option<(f32, f32, f32, f32)>,
    pub(super) background_svg: Option<&'a crate::parser::svg::SvgTree>,
    pub(super) background_blur_radius: f32,
    pub(super) background_size: BackgroundSize,
    pub(super) background_position: BackgroundPosition,
    pub(super) background_repeat: BackgroundRepeat,
    pub(super) background_origin: BackgroundOrigin,
    pub(super) background_clip: BackgroundClip,
    pub(super) background_blur_canvas_box: Option<SvgViewportBox>,
    pub(super) border_radius: f32,
    /// CSS `text-indent` applied to the first line only. List items use a
    /// negative value here to hang an `outside` marker into the padding band.
    pub(super) text_indent: f32,
}

/// Compute the height of a table row from its cells.
pub(super) fn compute_row_height(cells: &[TableCell]) -> f32 {
    cells
        .iter()
        .map(table_cell_content_height)
        .fold(0.0f32, f32::max)
}

/// Compute a grid row's painted height. Unlike a table row, a grid track size is
/// resolved during layout (css-grid-1 §11): the row track already accounts for
/// each item's definite/auto height, and a grid item with a definite height does
/// NOT grow its track when its content is taller — the content overflows the box
/// instead. So the painted row height is the track height carried on each cell as
/// `min_content_height`, never grown by the cells' intrinsic content height.
pub(super) fn compute_grid_row_height(cells: &[TableCell]) -> f32 {
    cells
        .iter()
        .map(|cell| cell.min_content_height)
        .fold(0.0f32, f32::max)
}

/// Paint-origin shift for a `border-collapse: collapse` table (CSS2 §17.6.2).
/// ironpress strokes each cell border CENTERED on its box edge, so the table's
/// outer collapsed border extends half its width OUTSIDE the table's border box.
/// Chrome instead keeps the whole collapsed border inside the table box (the
/// border-box left edge IS the outer pixel of the border). Shifting the painted
/// table right/down by half the outer border makes the outer edge land on the
/// box edge — aligning collapsed tables with Chrome (separate tables, which inset
/// their borders, already align). Returns `(dx, dy)` to add to the paint origin.
pub(super) fn collapse_paint_offset(
    cells: &[TableCell],
    border_collapse: BorderCollapse,
) -> (f32, f32) {
    if border_collapse != BorderCollapse::Collapse {
        return (0.0, 0.0);
    }
    // Outer-left border = left border of the first real (non-phantom) cell;
    // outer-top border = its top border. These are the table's leading edges.
    let lead = cells.iter().find(|c| c.rowspan != 0);
    match lead {
        Some(cell) => (cell.border.left.width / 2.0, cell.border.top.width / 2.0),
        None => (0.0, 0.0),
    }
}

pub(super) fn table_cell_geometry(
    col_widths: &[f32],
    col_pos: usize,
    colspan: usize,
    spacing: f32,
    origin_x: f32,
) -> (f32, f32) {
    // `border-spacing` is drawn before the first column and between every pair of
    // columns (and after the last), so the first cell is inset by one `spacing`
    // and each subsequent column is preceded by another. For `border-collapse`
    // (spacing == 0) this leading inset vanishes.
    let cell_x = origin_x
        + spacing
        + col_widths.iter().take(col_pos).sum::<f32>()
        + spacing * col_pos as f32;
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
    let content_top = table_cell_content_top(cell, placement.row_y, placement.row_height)
        - placement.baseline_shift;
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
                // `content_top` is already the content-box top (row top minus the
                // cell's top padding). Nested block content starts just below any
                // cell text; it must NOT be shifted down by the bottom padding.
                content_top - text_h,
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
    let mut first_drawn_line = true;
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
        // CSS `text-indent` shifts the start of the first rendered line. List
        // items pass a negative value so an `outside` marker (the first run)
        // hangs left into the padding while the following text lands at the
        // content edge.
        let first_line_indent = if first_drawn_line {
            placement.first_line_indent
        } else {
            0.0
        };
        first_drawn_line = false;
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
            _ => placement.cell_x + cell.padding_left + first_line_indent,
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
                0.0,
                ctx.pdf_writer,
                ctx.page_images,
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
    // `vertical-align` positions the cell's *actual* content within the (taller)
    // cell box, so use the intrinsic content height — not the value clamped to
    // the cell's own `min_content_height`, which would leave no room to offset.
    let content_height = table_cell_intrinsic_content_height(cell);
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
        block.clips,
    );
    let block_bottom = frame.top_y - total_height;

    // CSS `filter: blur()` on a solid box (css-filter-effects-1 §4.1): rasterize
    // the box's painted output (bg fill + border), gaussian-blur it, and embed it
    // overflowing the border box. Restricted to a plain solid box (no SVG/raster
    // bg, no text, no clip, square corners) so the vector paint path below is
    // byte-unchanged for everything else. `background_blur_radius` carries the
    // element's `style.blur_radius` here.
    if block.background_blur_radius > 0.0
        && block.lines.is_empty()
        && block.background_svg.is_none()
        && !block.clips
        && block.border_radius == 0.0
        && let Some(blurred) = crate::render::blur::blur_box(
            render_width,
            total_height,
            block.background_color,
            &block.border,
            block.background_blur_radius,
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
            w = render_width + 2.0 * ov,
            h = total_height + 2.0 * ov,
            ix = frame.origin_x - ov,
            iy = block_bottom - ov,
            name = img_name,
        ));
        ctx.text.page_images.push(ImageRef {
            name: img_name,
            obj_id: img_obj_id,
        });
        return;
    }

    // The box `background-clip` confines the painted fill to. In this nested
    // model `frame.origin_x` × `render_width` is the box the border centerline
    // runs along; padding-box stops the fill at the inner border edge and
    // content-box additionally insets by the padding (css-backgrounds-3 §3.4).
    let nested_background_clip_rect = |block: &NestedTextBlock<'_>, frame: &NestedLayoutFrame| {
        background_clip_rect(
            block.background_clip,
            frame.origin_x,
            block_bottom,
            render_width,
            total_height,
            block.border.left.width,
            block.border.right.width,
            block.border.top.width,
            block.border.bottom.width,
            block.padding_left,
            block.padding_right,
            block.padding_top,
            block.padding_bottom,
        )
    };

    if let Some((r, g, b, a)) = block.background_color {
        let needs_bg_alpha = a < 1.0;
        if needs_bg_alpha {
            let gs_name = format!("GSba{}", ctx.bg_alpha_counter);
            *ctx.bg_alpha_counter += 1;
            ctx.page_ext_gstates.push((gs_name.clone(), a));
            content.push_str(&format!("/{gs_name} gs\n"));
        }
        content.push_str(&format!("{r} {g} {b} rg\n"));
        let (n_clip_x, n_clip_y, n_clip_w, n_clip_h) = nested_background_clip_rect(&block, &frame);
        let nested_needs_clip = block.background_clip != BackgroundClip::Border;
        if nested_needs_clip {
            push_background_clip(
                content,
                n_clip_x,
                n_clip_y,
                n_clip_w,
                n_clip_h,
                block.border_radius,
            );
            content.push_str(&format!("{n_clip_x} {n_clip_y} {n_clip_w} {n_clip_h} re\n"));
            content.push_str("f\n");
            content.push_str("Q\n");
        } else if block.border_radius > 0.0 {
            content.push_str(&rounded_rect_path(
                frame.origin_x,
                block_bottom,
                render_width,
                total_height,
                block.border_radius,
            ));
            content.push_str("f\n");
        } else {
            content.push_str(&format!(
                "{x} {y} {w} {h} re\n",
                x = frame.origin_x,
                y = block_bottom,
                w = render_width,
                h = total_height,
            ));
            content.push_str("f\n");
        }
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
            ctx.text.pdf_writer,
            ctx.text.page_images,
            ctx.shadings,
            ctx.shading_counter,
            Some(ctx.page_ext_gstates),
            BackgroundPaintContext::new(
                SvgViewportBox::new(ref_x, ref_y, ref_w, ref_h),
                if block.background_clip == BackgroundClip::Border {
                    // Default: clip to the (outward-expanded) border box, as
                    // before — keeps existing raster-background behaviour stable.
                    SvgViewportBox::new(
                        frame.origin_x - block.border.left.width,
                        block_bottom - block.border.bottom.width,
                        render_width + block.border.left.width + block.border.right.width,
                        total_height + block.border.top.width + block.border.bottom.width,
                    )
                } else {
                    let (cx, cy, cw, ch) = nested_background_clip_rect(&block, &frame);
                    SvgViewportBox::new(cx, cy, cw, ch)
                },
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
            let a = begin_border_alpha(
                content,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                block.border.top.alpha,
            );
            content.push_str(&dash_pattern_for_style(
                block.border.top.style,
                block.border.top.width,
            ));
            content.push_str(&format!(
                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                block.border.top.width
            ));
            content.push_str(reset_dash_pattern(block.border.top.style));
            end_border_alpha(content, a);
        }
        if block.border.right.width > 0.0 {
            let (r, g, b) = block.border.right.color;
            let a = begin_border_alpha(
                content,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                block.border.right.alpha,
            );
            content.push_str(&dash_pattern_for_style(
                block.border.right.style,
                block.border.right.width,
            ));
            content.push_str(&format!(
                "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                block.border.right.width
            ));
            content.push_str(reset_dash_pattern(block.border.right.style));
            end_border_alpha(content, a);
        }
        if block.border.bottom.width > 0.0 {
            let (r, g, b) = block.border.bottom.color;
            let a = begin_border_alpha(
                content,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                block.border.bottom.alpha,
            );
            content.push_str(&dash_pattern_for_style(
                block.border.bottom.style,
                block.border.bottom.width,
            ));
            content.push_str(&format!(
                "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                block.border.bottom.width
            ));
            content.push_str(reset_dash_pattern(block.border.bottom.style));
            end_border_alpha(content, a);
        }
        if block.border.left.width > 0.0 {
            let (r, g, b) = block.border.left.color;
            let a = begin_border_alpha(
                content,
                ctx.page_ext_gstates,
                ctx.bg_alpha_counter,
                block.border.left.alpha,
            );
            content.push_str(&dash_pattern_for_style(
                block.border.left.style,
                block.border.left.width,
            ));
            content.push_str(&format!(
                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                block.border.left.width
            ));
            content.push_str(reset_dash_pattern(block.border.left.style));
            end_border_alpha(content, a);
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
            min_content_height: 0.0,
            hide_if_empty: false,
            grid_inset: None,
            clips: false,
            background_gradient: None,
            background_radial_gradient: None,
            background_conic_gradient: None,
        };
        render_cell_text(
            content,
            &proxy_cell,
            CellTextPlacement::new(
                frame.origin_x,
                frame.top_y - block.padding_top,
                render_width,
            )
            .with_first_line_indent(block.text_indent),
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
                let (collapse_dx, collapse_dy) = collapse_paint_offset(cells, *border_collapse);
                let row_y = planned_element.top_y - collapse_dy;
                let row_height = compute_row_height(cells);
                let baseline_shifts = row_baseline_shifts(cells, ctx.text.custom_fonts);

                let mut col_pos: usize = 0;
                for (cell_idx, cell) in cells.iter().enumerate() {
                    if cell.rowspan == 0 {
                        col_pos += cell.colspan;
                        continue;
                    }

                    let (cell_x, cell_w) = table_cell_geometry(
                        col_widths,
                        col_pos,
                        cell.colspan,
                        spacing,
                        planned_element.origin_x + collapse_dx,
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

                    if let Some((r, g, b, a)) =
                        cell.background_color.filter(|_| !cell.hide_if_empty)
                    {
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

                    if cell.border.has_any() && !cell.hide_if_empty {
                        let x1 = cell_x;
                        let x2 = cell_x + cell_w;
                        let y_top = row_y;
                        let y_bottom = row_y - cell_height;
                        if cell.border.top.width > 0.0 {
                            let (r, g, b) = cell.border.top.color;
                            let a = begin_border_alpha(
                                content,
                                ctx.page_ext_gstates,
                                ctx.bg_alpha_counter,
                                cell.border.top.alpha,
                            );
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x2} {y_top} l S\n",
                                cell.border.top.width
                            ));
                            end_border_alpha(content, a);
                        }
                        if cell.border.right.width > 0.0 {
                            let (r, g, b) = cell.border.right.color;
                            let a = begin_border_alpha(
                                content,
                                ctx.page_ext_gstates,
                                ctx.bg_alpha_counter,
                                cell.border.right.alpha,
                            );
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x2} {y_top} m {x2} {y_bottom} l S\n",
                                cell.border.right.width
                            ));
                            end_border_alpha(content, a);
                        }
                        if cell.border.bottom.width > 0.0 {
                            let (r, g, b) = cell.border.bottom.color;
                            let a = begin_border_alpha(
                                content,
                                ctx.page_ext_gstates,
                                ctx.bg_alpha_counter,
                                cell.border.bottom.alpha,
                            );
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_bottom} m {x2} {y_bottom} l S\n",
                                cell.border.bottom.width
                            ));
                            end_border_alpha(content, a);
                        }
                        if cell.border.left.width > 0.0 {
                            let (r, g, b) = cell.border.left.color;
                            let a = begin_border_alpha(
                                content,
                                ctx.page_ext_gstates,
                                ctx.bg_alpha_counter,
                                cell.border.left.alpha,
                            );
                            content.push_str(&format!(
                                "{r} {g} {b} RG\n{} w\n{x1} {y_top} m {x1} {y_bottom} l S\n",
                                cell.border.left.width
                            ));
                            end_border_alpha(content, a);
                        }
                    }

                    render_cell_content(
                        content,
                        cell,
                        TableCellRenderBox::new(cell_x, row_y, cell_w, row_height, frame)
                            .with_baseline_shift(
                                baseline_shifts.get(cell_idx).copied().unwrap_or(0.0),
                            ),
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
                clip_rect,
                background_gradient: _,
                background_radial_gradient: _,
                background_conic_gradient: _,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                background_clip,
                text_indent,
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
                        clips: clip_rect.is_some(),
                        background_color: *background_color,
                        background_svg: background_svg.as_ref(),
                        background_blur_radius: *background_blur_radius,
                        background_size: *background_size,
                        background_position: *background_position,
                        background_repeat: *background_repeat,
                        background_origin: *background_origin,
                        background_clip: *background_clip,
                        background_blur_canvas_box: planned_element.blur_canvas_box,
                        border_radius: *border_radius,
                        text_indent: *text_indent,
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
            LayoutElement::Container {
                children,
                background_color,
                border,
                border_radius,
                padding_top,
                padding_bottom,
                padding_left,
                padding_right,
                block_width,
                block_height,
                visible,
                background_svg,
                background_blur_radius,
                background_size,
                background_position,
                background_repeat,
                background_origin,
                background_clip,
                ..
            } => {
                let render_width = block_width
                    .unwrap_or(planned_element.available_width)
                    .max(0.0);
                // Border-box height of the container: an explicit `block_height`
                // (definite height) wins; otherwise derive from the children.
                let children_h: f32 = children
                    .iter()
                    .map(crate::layout::engine::estimate_element_height)
                    .sum();
                let box_h = block_height.unwrap_or(
                    *padding_top + children_h + *padding_bottom + border.vertical_width(),
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
                            padding_top: *padding_top,
                            padding_bottom: *padding_bottom,
                            padding_left: *padding_left,
                            padding_right: *padding_right,
                            border: *border,
                            block_width: Some(render_width),
                            block_height: Some(box_h),
                            // `box_h` already resolves the definite/auto height, and
                            // there are no lines to grow it, so clipping is moot here.
                            clips: false,
                            background_color: *background_color,
                            background_svg: background_svg.as_ref(),
                            background_blur_radius: *background_blur_radius,
                            background_size: *background_size,
                            background_position: *background_position,
                            background_repeat: *background_repeat,
                            background_origin: *background_origin,
                            background_clip: *background_clip,
                            background_blur_canvas_box: planned_element.blur_canvas_box,
                            border_radius: *border_radius,
                            text_indent: 0.0,
                        },
                        NestedLayoutFrame::new(
                            planned_element.origin_x,
                            planned_element.top_y,
                            frame.initial_origin_x,
                            frame.initial_top_y,
                            render_width,
                        ),
                        ctx,
                    );
                }
                // Recurse into the container's children at its content origin.
                if !children.is_empty() {
                    render_nested_layout_elements(
                        content,
                        children,
                        NestedLayoutFrame::new(
                            planned_element.origin_x + *padding_left + border.left.width,
                            planned_element.top_y - *padding_top - border.top.width,
                            frame.initial_origin_x,
                            frame.initial_top_y,
                            (render_width
                                - *padding_left
                                - *padding_right
                                - border.horizontal_width())
                            .max(0.0),
                        ),
                        ctx,
                    );
                }
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
                clip_rect,
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
                            clip_rect.is_some(),
                        )
                        - *margin_bottom;
                }
            }
            LayoutElement::Container {
                margin_top,
                margin_bottom,
                position,
                offset_top,
                offset_left,
                ..
            } => {
                // A block child of a cell (e.g. a `<div>` with a background)
                // flattens to a Container. Position it in the cell's flow like a
                // TextBlock so its background/border/children paint instead of
                // being silently dropped.
                let base_top_y = cursor_y - *margin_top;
                let element_top_y = match position {
                    Position::Absolute | Position::Relative => base_top_y - *offset_top,
                    Position::Static => base_top_y,
                };
                let element_origin_x = frame.origin_x + offset_left;
                planned.push(PlannedNestedElement {
                    element,
                    source_index: element_idx,
                    origin_x: element_origin_x,
                    top_y: element_top_y,
                    available_width: frame.available_width,
                    blur_canvas_box: None,
                });
                if *position != Position::Absolute {
                    let box_h = crate::layout::engine::estimate_element_height(element)
                        - *margin_top
                        - *margin_bottom;
                    cursor_y = base_top_y - box_h - *margin_bottom;
                }
            }
            _ => {}
        }
    }

    planned
}
