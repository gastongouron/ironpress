#[cfg(test)]
use super::geometry::LayoutBoxGeometry;
use super::geometry::{PdfPoint, PdfRect};
use super::*;
use crate::layout::elements::LayoutNode;
#[cfg(test)]
use crate::types::CornerRadii;

pub(super) struct TextRenderContext<'a> {
    pub(super) page_height: f32,
    pub(super) custom_fonts: &'a HashMap<String, TtfFont>,
    pub(super) prepared_custom_fonts: &'a PreparedCustomFonts,
    pub(super) annotations: &'a mut Vec<LinkAnnotation>,
    // Threaded so `render_cell_text` can embed blurred `text-shadow` image
    // XObjects (it rasterizes + blurs the shadow glyphs, like the page path).
    pub(super) pdf_writer: &'a mut PdfWriter,
    pub(super) page_images: &'a mut Vec<ImageRef>,
}

impl<'a> TextRenderContext<'a> {
    pub(super) fn new(
        page_height: f32,
        custom_fonts: &'a HashMap<String, TtfFont>,
        prepared_custom_fonts: &'a PreparedCustomFonts,
        annotations: &'a mut Vec<LinkAnnotation>,
        pdf_writer: &'a mut PdfWriter,
        page_images: &'a mut Vec<ImageRef>,
    ) -> Self {
        Self {
            page_height,
            custom_fonts,
            prepared_custom_fonts,
            annotations,
            pdf_writer,
            page_images,
        }
    }

    pub(super) fn annotation_marker(&self) -> usize {
        self.annotations.len()
    }

    pub(super) fn discard_annotations_since(&mut self, marker: usize) {
        self.annotations.truncate(marker);
    }
}

pub(super) struct PageRenderContext<'a> {
    pub(super) paint_box: PdfRect,
    /// Top-left of the initial fixed containing block (the page area in paged
    /// media), in PDF page coordinates.
    pub(super) initial_fixed_origin: PdfPoint,
    pub(super) shadings: &'a mut Vec<ShadingEntry>,
    pub(super) shading_counter: &'a mut usize,
    pub(super) page_ext_gstates: &'a mut Vec<(String, f32)>,
    pub(super) bg_alpha_counter: &'a mut usize,
    pub(super) stacking: StackingTraversal,
    pub(super) text: TextRenderContext<'a>,
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
        paint_box: PdfRect,
        page_height: f32,
    ) -> Self {
        Self {
            paint_box,
            initial_fixed_origin: PdfPoint::new(paint_box.left, paint_box.top()),
            shadings,
            shading_counter,
            page_ext_gstates,
            bg_alpha_counter,
            stacking: StackingTraversal::default(),
            text: TextRenderContext::new(
                page_height,
                custom_fonts,
                prepared_custom_fonts,
                annotations,
                pdf_writer,
                page_images,
            ),
        }
    }

    pub(super) const fn with_initial_fixed_origin(mut self, origin: PdfPoint) -> Self {
        self.initial_fixed_origin = origin;
        self
    }
}

#[derive(Clone, Copy)]
pub(super) struct NestedLayoutFrame {
    origin: PdfPoint,
    initial_origin: PdfPoint,
    available_width: f32,
}

impl NestedLayoutFrame {
    pub(super) const fn new(
        origin: PdfPoint,
        initial_origin: PdfPoint,
        available_width: f32,
    ) -> Self {
        Self {
            origin,
            initial_origin,
            available_width,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct CellTextPlacement {
    origin: PdfPoint,
    col_width: f32,
    /// Extra horizontal offset applied to the FIRST rendered line only (CSS
    /// `text-indent`). Negative values pull the first line left, used to hang a
    /// list marker into the surrounding padding.
    first_line_indent: f32,
}

impl CellTextPlacement {
    pub(super) const fn new(origin: PdfPoint, col_width: f32) -> Self {
        Self {
            origin,
            col_width,
            first_line_indent: 0.0,
        }
    }

    #[cfg(test)]
    pub(super) const fn with_first_line_indent(mut self, first_line_indent: f32) -> Self {
        self.first_line_indent = first_line_indent;
        self
    }
}

#[derive(Clone, Copy)]
pub(super) struct CellRenderBox {
    origin: PdfPoint,
    col_width: f32,
    row_height: f32,
    /// Extra downward offset applied to this cell's content so a
    /// `vertical-align: baseline` cell's first text baseline lines up with the
    /// common baseline of the other baseline-aligned cells in the same row. 0.0
    /// when the cell is not baseline-aligned or shares the row's tallest
    /// baseline (the common case, so existing single-font rows are unaffected).
    baseline_shift: f32,
}

impl CellRenderBox {
    pub(super) const fn new(origin: PdfPoint, col_width: f32, row_height: f32) -> Self {
        Self {
            origin,
            col_width,
            row_height,
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
    cell: &CellBox,
    custom_fonts: &HashMap<String, TtfFont>,
) -> Option<f32> {
    let line = cell
        .content
        .lines
        .iter()
        .find(|line| line.runs.iter().any(|run| !run.text.is_empty()))?;
    let metrics = line_box_metrics(line, custom_fonts);
    Some(cell.box_model.content_insets.top + metrics.half_leading + metrics.ascender)
}

/// Per-cell baseline shifts for one row: each `vertical-align: baseline` cell
/// with text is offset down so its first baseline matches the row's deepest
/// baseline. Index i corresponds to `cells[i]`; non-baseline / text-less cells
/// get 0.0. All-equal rows (same font + line-height) yield all-zero shifts, so
/// uniform tables render exactly as before.
pub(super) fn row_baseline_shifts<T: CellBoxHolder>(
    cells: &[T],
    custom_fonts: &HashMap<String, TtfFont>,
) -> Vec<f32> {
    let baselines: Vec<Option<f32>> = cells
        .iter()
        .map(|cell| {
            let layout = cell.cell_box();
            if layout.alignment.block == VerticalAlign::Baseline {
                table_cell_first_baseline(layout, custom_fonts)
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

#[cfg(test)]
pub(super) struct NestedTextBlock<'a> {
    pub(super) lines: &'a [TextLine],
    pub(super) text_align: TextAlign,
    pub(super) padding: EdgeSizes,
    pub(super) border: crate::layout::engine::LayoutBorder,
    pub(super) block_width: Option<f32>,
    pub(super) block_height: Option<f32>,
    /// Whether the box clips overflow (`overflow: hidden`/`scroll`). When true a
    /// definite `block_height` is a hard size and content is clipped to it rather
    /// than growing the box.
    pub(super) clips: bool,
    pub(super) background_color: Option<crate::types::Color>,
    pub(super) background_svg: Option<&'a crate::parser::svg::SvgTree>,
    pub(super) background_blur_radius: f32,
    pub(super) background_size: BackgroundSize,
    pub(super) background_position: BackgroundPosition,
    pub(super) background_repeat: BackgroundRepeat,
    pub(super) background_origin: BackgroundOrigin,
    pub(super) background_clip: BackgroundClip,
    pub(super) background_blur_canvas_box: Option<SvgViewportBox>,
    pub(super) border_radii: CornerRadii,
    /// CSS `text-indent` applied to the first line only. List items use a
    /// negative value here to hang an `outside` marker into the padding band.
    pub(super) text_indent: f32,
}

/// Compute a grid row's painted height. Unlike a table row, a grid track size is
/// resolved during layout (css-grid-1 §11): the row track already accounts for
/// each item's definite/auto height, and a grid item with a definite height does
/// NOT grow its track when its content is taller — the content overflows the box
/// instead. So the painted row height is the track height carried on each cell as
/// `min_content_height`, never grown by the cells' intrinsic content height.
pub(super) fn compute_grid_row_height(cells: &[GridCell]) -> f32 {
    cells
        .iter()
        .map(|cell| cell.layout.box_model.minimum_block_size)
        .fold(0.0f32, f32::max)
}

pub(super) fn render_cell_content(
    content: &mut String,
    cell: &CellBox,
    placement: CellRenderBox,
    inherited_abs_origins: &HashMap<usize, PdfPoint>,
    ctx: &mut PageRenderContext<'_>,
) {
    let content_top =
        cell_content_top(cell, placement.origin.y, placement.row_height) - placement.baseline_shift;
    if !cell.content.children.is_empty() {
        let text_h: f32 = cell.content.lines.iter().map(|line| line.height).sum();
        render_cell_text(
            content,
            cell,
            CellTextPlacement::new(
                PdfPoint::new(placement.origin.x, content_top),
                placement.col_width,
            ),
            ctx,
        );
        render_cell_child_elements(
            content,
            &cell.content.children,
            NestedLayoutFrame::new(
                PdfPoint::new(
                    placement.origin.x + cell.box_model.content_insets.left,
                    // `content_top` is already the content-box top (row top minus the
                    // cell's top padding). Nested block content starts just below any
                    // cell text; it must NOT be shifted down by the bottom padding.
                    content_top - text_h,
                ),
                PdfPoint::new(
                    placement.origin.x + cell.box_model.border_insets.left,
                    placement.origin.y - cell.box_model.border_insets.top,
                ),
                (placement.col_width - cell.box_model.content_insets.horizontal()).max(0.0),
            ),
            if cell.establishes_stacking_context() {
                StackingScope::Local
            } else {
                StackingScope::Ancestor
            },
            cell,
            inherited_abs_origins,
            ctx,
        );
        return;
    }

    render_cell_text(
        content,
        cell,
        CellTextPlacement::new(
            PdfPoint::new(placement.origin.x, content_top),
            placement.col_width,
        ),
        ctx,
    );
}

fn render_cell_child_elements(
    content: &mut String,
    elements: &[LayoutNode],
    frame: NestedLayoutFrame,
    stacking_scope: StackingScope,
    cell: &CellBox,
    inherited_abs_origins: &HashMap<usize, PdfPoint>,
    ctx: &mut PageRenderContext<'_>,
) {
    let mut abs_origins = inherited_abs_origins.clone();
    if let Some(depth) = cell.established_containing_block_depth() {
        abs_origins.insert(depth, frame.initial_origin);
    }
    render_container_children(
        content,
        elements,
        ContainerFrame::new(
            frame.origin,
            crate::types::Size::new(frame.available_width, f32::INFINITY),
            frame.initial_origin,
        ),
        &mut abs_origins,
        ctx,
        ContainerRenderOptions {
            stacking_scope,
            ..Default::default()
        },
    );
}

pub(super) fn render_cell_text(
    content: &mut String,
    cell: &CellBox,
    placement: CellTextPlacement,
    ctx: &mut PageRenderContext<'_>,
) {
    let cell_inner_w = placement.col_width - cell.box_model.content_insets.horizontal();
    let mut baseline_cursor = TextBaselineCursor::new(
        placement.origin.y,
        ctx.text.pdf_writer.page_content_transform,
    );
    let mut first_drawn_line = true;
    for line in &cell.content.lines {
        let metrics = line_box_metrics(line, ctx.text.custom_fonts);
        let text_y = baseline_cursor.next_horizontal(metrics);
        let line_annotation_bottom = text_y - metrics.descender - metrics.half_leading;
        let line_annotation_height =
            metrics.ascender + metrics.descender + 2.0 * metrics.half_leading;
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
        let merged = crate::text::coalesce_text_runs(&line.runs);
        let line_width: f32 = merged
            .iter()
            .map(|run| estimate_run_width_with_fonts(run, ctx.text.custom_fonts))
            .sum();
        let text_x = match cell.alignment.inline {
            TextAlign::Right => {
                placement.origin.x
                    + cell.box_model.content_insets.left
                    + (cell_inner_w - line_width).max(0.0)
            }
            TextAlign::Center => {
                placement.origin.x
                    + cell.box_model.content_insets.left
                    + ((cell_inner_w - line_width) / 2.0).max(0.0)
            }
            _ => placement.origin.x + cell.box_model.content_insets.left + first_line_indent,
        };
        // Line-box edges for inline-box vertical alignment, mirroring the
        // page-level text painter's geometry.
        let line_top_y = text_y + metrics.ascender + metrics.half_leading;
        let line_bottom_y = text_y - metrics.descender - metrics.half_leading;
        let (text_ascent, text_descent) = line_text_content_extents(line, ctx.text.custom_fonts);
        let line_text_top_y = if text_ascent > 0.0 {
            text_y + text_ascent
        } else {
            line_top_y
        };
        let line_text_bottom_y = if text_descent > 0.0 {
            text_y - text_descent
        } else {
            line_bottom_y
        };
        let mut x = text_x;
        for (run_index, run) in merged.iter().enumerate() {
            // Atomic inline box (display: inline-block) in the cell's text
            // flow: paint the box and its inner content, then advance —
            // the same treatment the page-level line painter applies.
            if let Some(inline) = run.inline_box.as_deref() {
                if !run.is_inline_edge() {
                    render_inline_box(
                        content,
                        inline,
                        x + inline.margin_left,
                        text_y,
                        ctx.text.page_height,
                        line_top_y,
                        line_bottom_y,
                        line_text_top_y,
                        line_text_bottom_y,
                        run.font_size,
                        run_line_height_for_vertical_align(run),
                        line_primary_x_height_ratio(&merged, ctx.text.custom_fonts),
                        ctx.text.custom_fonts,
                        ctx.text.prepared_custom_fonts,
                        ctx.page_ext_gstates,
                        ctx.bg_alpha_counter,
                        ctx.shadings,
                        ctx.shading_counter,
                        ctx.text.pdf_writer,
                        ctx.text.page_images,
                    );
                }
                x += run.atomic_inline_advance().unwrap_or_default();
                continue;
            }
            if let Some(advance) = run.atomic_inline_advance() {
                x += advance;
                continue;
            }
            if run.text.is_empty() {
                continue;
            }
            let run_width = estimate_run_width_with_fonts(run, ctx.text.custom_fonts);
            let previous = merged[..run_index]
                .iter()
                .rev()
                .find(|previous| previous.inline_box.is_none() && !previous.text.is_empty());
            let decoration =
                HorizontalRunDecorations::new(run, x, run_width, text_y, ctx.text.custom_fonts)
                    .continuing_after(previous);

            if let Some(background) = run.background_color {
                let (background_r, background_g, background_b) = background.to_f32_rgb();
                let rx = x - run.padding.left;
                let ry = text_y - 2.0 - run.padding.bottom;
                let rw2 = run_width + run.padding.horizontal();
                let rh = run.font_size + 2.0 + run.padding.vertical();
                content.push_str(&format!(
                    "{background_r} {background_g} {background_b} rg\n"
                ));
                content.push_str(
                    &PdfRect::new(rx, ry, rw2, rh)
                        .rounded(run.border_radii)
                        .path_or_rect(),
                );
                content.push_str("f\n");
            }

            decoration.paint_text(
                content,
                crate::layout::text::line_primary_font_size(&merged),
                ctx.text.prepared_custom_fonts,
                0.0,
                ctx.text.pdf_writer,
                ctx.text.page_images,
            );

            if let Some(annotation) = text_run_link_annotation(
                run,
                PdfRect::new(x, line_annotation_bottom, run_width, line_annotation_height),
            ) {
                ctx.text.annotations.push(annotation);
            }

            x += run_width;
        }
    }
}

fn cell_content_top(cell: &CellBox, row_y: f32, row_height: f32) -> f32 {
    let offset = cell.content_block_offset(row_height);
    row_y - offset - cell.box_model.content_insets.top
}

#[cfg(test)]
mod table_cell_alignment_tests {
    use super::*;

    #[test]
    fn middle_cell_rounds_a_half_css_pixel_remainder_toward_block_start() {
        let cell = CellBox {
            content: crate::layout::cells::CellContent {
                lines: vec![TextLine {
                    // A 14px font at line-height: 1.5 occupies 21 CSS pixels.
                    height: 21.0 * crate::fonts::PT_PER_CSS_PX,
                    ..Default::default()
                }],
                ..Default::default()
            },
            alignment: crate::layout::cells::CellAlignment {
                block: VerticalAlign::Middle,
                ..Default::default()
            },
            ..Default::default()
        };

        // A 40px row has 19px of surplus. Chrome assigns its half-pixel to
        // the block-start side, so the content begins 10px below the top.
        assert_eq!(
            cell_content_top(
                &cell,
                40.0 * crate::fonts::PT_PER_CSS_PX,
                40.0 * crate::fonts::PT_PER_CSS_PX,
            ),
            30.0 * crate::fonts::PT_PER_CSS_PX,
        );
    }
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
pub(super) use test_support::*;
