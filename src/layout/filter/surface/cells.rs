//! Flex and grid item sources for retained filters.

use crate::layout::cells::{CellBox, GridCell};
use crate::layout::elements::{BoxModel, FlexRow, GridRow, Positioning};
use crate::layout::engine::FlexCell;
use crate::style::computed::AlignItems;
use crate::types::{EdgeSizes, Point, Size, Vector};

use super::canvas::{PaintBounds, RasterCanvas, SurfaceRect};
use super::geometry::{SourceGeometry, SourceGraphic, SourceRasterGeometry, SourceRasterSpace};
use super::gradient::FilterBackground;
use super::overflow::flex_cell_paint_overflow;
use super::painter::{DescendantPaintArea, ElementPaintSpace, RootEffectHandling, SourcePainter};
use super::text::{flex_cell_baseline, flex_line_max_baseline};

mod frame;
mod table;

pub(crate) use frame::CellSourceFrame;
pub(crate) use table::table_cell_source_frames;

/// Resolve every grid item's concrete border box from retained track geometry.
/// Source painting and post-pagination filter materialization share this one
/// calculation.
pub(crate) fn grid_cell_source_frames(grid: &GridRow) -> Vec<CellSourceFrame> {
    let content_offset = Vector::new(
        grid.box_model.border.left.width + grid.box_model.padding.left,
        grid.box_model.border.top.width + grid.box_model.padding.top,
    );
    let row_height = grid
        .content
        .cells
        .iter()
        .map(|cell| cell.layout.box_model.minimum_block_size)
        .fold(0.0_f32, f32::max);
    grid.content
        .cells
        .iter()
        .map(|cell| {
            let column = cell.placement.column_start;
            let span = cell.placement.column_span.max(1);
            let track_x = grid.content.column_widths.iter().take(column).sum::<f32>()
                + grid.content.gap * column as f32;
            let track_width = grid
                .content
                .column_widths
                .iter()
                .skip(column)
                .take(span)
                .sum::<f32>()
                + grid.content.gap * span.saturating_sub(1) as f32;
            let (inset_offset, size) = cell.placement.inset.map_or(
                (Vector::ZERO, Size::new(track_width, row_height)),
                |inset| (Vector::new(inset.offset.x, inset.offset.y), inset.size),
            );
            CellSourceFrame::new(
                size,
                content_offset + Vector::new(track_x + inset_offset.x, inset_offset.y),
            )
        })
        .collect()
}

impl SourcePainter<'_> {
    pub(super) fn paint_flex_cell(
        &mut self,
        cell: &FlexCell,
        flex: &FlexRow,
        content: SurfaceRect,
        max_baseline: Option<f32>,
    ) -> Option<()> {
        let alignment = cell.effective_cross_alignment(flex.content.alignment);
        let baseline_shift = if alignment == AlignItems::Baseline {
            match (flex_cell_baseline(cell, self.fonts), max_baseline) {
                (Some(own), Some(maximum)) => (maximum - own).max(0.0),
                _ => 0.0,
            }
        } else {
            0.0
        };
        let cross = cell.cross_geometry(
            flex.content.row_height,
            flex.content.alignment,
            baseline_shift,
        );
        let rect = SurfaceRect::new(
            Point::new(
                content.origin.x + cell.x_offset,
                content.origin.y + cross.offset,
            ),
            Size::new(cell.width, cross.size),
        );
        self.paint_flex_cell_box(cell, rect, RootEffectHandling::Paint)
    }

    pub(super) fn paint_flex_cell_box(
        &mut self,
        cell: &FlexCell,
        rect: SurfaceRect,
        effects: RootEffectHandling,
    ) -> Option<()> {
        self.paint_group(
            self.space.for_descendant_box(rect, effects),
            &cell.paint.group,
            Some(cell),
            |painter| painter.paint_flex_cell_source(cell, rect),
        )
    }

    fn paint_flex_cell_source(&mut self, cell: &FlexCell, rect: SurfaceRect) -> Option<()> {
        if let Some(output) = &cell.paint.filter_output {
            return self.canvas.paint_filter_output(output, rect);
        }
        let model = BoxModel {
            size: crate::layout::elements::LayoutSize::fixed(
                rect.size.width,
                Some(rect.size.height),
            ),
            padding: cell.padding,
            border: cell.border,
            ..Default::default()
        };
        let background = FilterBackground::resolve(
            &cell.paint.background,
            &model,
            rect,
            cell.paint.border_radii,
            cell.paint.border_image.as_ref(),
        )?;
        self.canvas
            .paint_outset_shadows(rect, &cell.paint.shadows, self.filter_dpi)?;
        background.paint(&mut self.canvas);
        let padding_box = rect.inset(cell.border.widths());
        self.canvas
            .paint_inset_shadows(padding_box, &cell.paint.shadows, self.filter_dpi)?;
        self.canvas
            .paint_border(rect, &cell.border, cell.paint.border_radii)?;
        let area = DescendantPaintArea {
            padding_box,
            content_box: rect.inset(cell.border.widths() + cell.padding),
            absolute_containing_block: Some(padding_box),
            direct_child_effects: RootEffectHandling::Paint,
        };
        self.paint_text_lines(&cell.lines, area.content_box, cell.text_align, 0.0)?;
        let text_height = cell.lines.iter().map(|line| line.height).sum::<f32>();
        self.paint_children(&cell.nested_elements, area.after_normal_flow(text_height))
    }

    pub(super) fn paint_grid_cell(
        &mut self,
        cell: &GridCell,
        rect: SurfaceRect,
        effects: RootEffectHandling,
    ) -> Option<()> {
        self.paint_cell_box(
            &cell.layout,
            rect,
            cell.placement.clips,
            false,
            0.0,
            effects,
        )
    }

    pub(super) fn paint_cell_box(
        &mut self,
        cell: &CellBox,
        rect: SurfaceRect,
        clips: bool,
        suppressed: bool,
        baseline_shift: f32,
        effects: RootEffectHandling,
    ) -> Option<()> {
        if suppressed {
            return Some(());
        }
        self.paint_group(
            self.space.for_descendant_box(rect, effects),
            &cell.paint.group,
            Some(&cell.box_model),
            |painter| painter.paint_cell_source(cell, rect, clips, baseline_shift),
        )
    }

    fn paint_cell_source(
        &mut self,
        cell: &CellBox,
        rect: SurfaceRect,
        clips: bool,
        baseline_shift: f32,
    ) -> Option<()> {
        if let Some(output) = &cell.paint.filter_output {
            return self.canvas.paint_filter_output(output, rect);
        }
        let border = cell.box_model.border;
        let model = BoxModel {
            size: crate::layout::elements::LayoutSize::fixed(
                rect.size.width,
                Some(rect.size.height),
            ),
            padding: cell.box_model.padding(),
            border,
            ..Default::default()
        };
        let background = FilterBackground::resolve(
            &cell.paint.background,
            &model,
            rect,
            cell.paint.border_radii,
            cell.paint.border_image.as_ref(),
        )?;
        self.canvas
            .paint_outset_shadows(rect, &cell.paint.shadows, self.filter_dpi)?;
        background.paint(&mut self.canvas);
        let padding_box = rect.inset(border.widths());
        self.canvas
            .paint_inset_shadows(padding_box, &cell.paint.shadows, self.filter_dpi)?;
        self.canvas
            .paint_border(rect, &border, cell.paint.border_radii)?;
        let mut content_box = rect.inset(cell.box_model.content_insets);
        let block_offset = cell.content_block_offset(rect.size.height) + baseline_shift;
        content_box.origin.y += block_offset;
        content_box.size.height = (content_box.size.height - block_offset).max(0.0);
        let area = DescendantPaintArea {
            padding_box,
            content_box,
            absolute_containing_block: Some(padding_box),
            direct_child_effects: RootEffectHandling::Paint,
        };
        let paint_contents = |painter: &mut SourcePainter<'_>| {
            painter.paint_text_lines(
                &cell.content.lines,
                area.content_box,
                cell.alignment.inline,
                0.0,
            )?;
            let text_height = cell
                .content
                .lines
                .iter()
                .map(|line| line.height)
                .sum::<f32>();
            painter.paint_children(&cell.content.children, area.after_normal_flow(text_height))
        };
        if clips {
            let clip = crate::render::borders::CssRoundedRect::new(rect, cell.paint.border_radii)
                .inset(border.widths());
            self.paint_clipped_descendants(clip, paint_contents)
        } else {
            paint_contents(self)
        }
    }
}

/// Paint one grid item's complete border-box source before applying its filter.
pub(crate) fn paint_grid_cell_source(
    cell: &GridCell,
    size: Size,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
    raster_space: SourceRasterSpace,
) -> Option<SourceGraphic> {
    let layout = SourceGeometry {
        size,
        flow: crate::layout::flow_metrics::BlockFlowSpacing::default(),
        positioning: Positioning::default(),
    };
    let geometry =
        SourceRasterGeometry::resolve(layout, EdgeSizes::ZERO, filter_dpi, raster_space)?;
    let dimensions = geometry.dimensions();
    let mut pixels = crate::render::raster_pixels::PremultipliedRgba8::transparent(
        dimensions.width,
        dimensions.height,
    );
    let mut paint_bounds = PaintBounds::default();
    {
        let canvas = RasterCanvas {
            pixels: &mut pixels,
            pixels_per_point: crate::render::raster_scale::RasterScale::at_dpi(filter_dpi)
                .pixels_per_point(),
            paint_bounds: &mut paint_bounds,
        };
        let border_box = SurfaceRect::new(geometry.border_origin(), size);
        let mut painter = SourcePainter::new(
            canvas,
            ElementPaintSpace::root(border_box, RootEffectHandling::DeferToOwner),
            fonts,
            filter_dpi,
        );
        painter.paint_grid_cell(cell, border_box, RootEffectHandling::DeferToOwner)?;
    }
    Some(SourceGraphic {
        pixels,
        geometry,
        paint_bounds: paint_bounds.resolve(),
    })
}

/// Resolve every flex item's concrete border-box frame after line alignment.
///
/// The returned order is the cell order. Keeping size and position together in
/// the SourceGraphic domain prevents filter materialization and PDF flex paint
/// from quantizing the same item independently.
pub(crate) fn flex_cell_source_frames(
    flex: &FlexRow,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> Vec<CellSourceFrame> {
    let max_baseline = flex_line_max_baseline(&flex.content.cells, flex.content.alignment, fonts);
    flex.content
        .cells
        .iter()
        .map(|cell| {
            let alignment = cell.effective_cross_alignment(flex.content.alignment);
            let baseline_shift = if alignment == AlignItems::Baseline {
                match (flex_cell_baseline(cell, fonts), max_baseline) {
                    (Some(own), Some(maximum)) => (maximum - own).max(0.0),
                    _ => 0.0,
                }
            } else {
                0.0
            };
            let cross = cell.cross_geometry(
                flex.content.row_height,
                flex.content.alignment,
                baseline_shift,
            );
            CellSourceFrame::new(
                Size::new(cell.width, cross.size),
                Vector::new(
                    flex.box_model.border.left.width + flex.box_model.padding.left + cell.x_offset,
                    flex.box_model.border.top.width + flex.box_model.padding.top + cross.offset,
                ),
            )
        })
        .collect()
}

/// Paint one flex item's complete border-box source before applying its
/// retained filter.
pub(crate) fn paint_flex_cell_source(
    cell: &FlexCell,
    size: Size,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
    raster_space: SourceRasterSpace,
) -> Option<SourceGraphic> {
    let layout = SourceGeometry {
        size,
        flow: crate::layout::flow_metrics::BlockFlowSpacing::default(),
        positioning: Positioning::default(),
    };
    let geometry = SourceRasterGeometry::resolve(
        layout,
        flex_cell_paint_overflow(cell, size, filter_dpi)?,
        filter_dpi,
        raster_space,
    )?;
    let dimensions = geometry.dimensions();
    let mut pixels = crate::render::raster_pixels::PremultipliedRgba8::transparent(
        dimensions.width,
        dimensions.height,
    );
    let border_box = SurfaceRect::new(geometry.border_origin(), size);
    let mut paint_bounds = PaintBounds::default();
    {
        let canvas = RasterCanvas {
            pixels: &mut pixels,
            pixels_per_point: crate::render::raster_scale::RasterScale::at_dpi(filter_dpi)
                .pixels_per_point(),
            paint_bounds: &mut paint_bounds,
        };
        let mut painter = SourcePainter::new(
            canvas,
            ElementPaintSpace::root(border_box, RootEffectHandling::DeferToOwner),
            fonts,
            filter_dpi,
        );
        painter.paint_flex_cell_box(cell, border_box, RootEffectHandling::DeferToOwner)?;
    }
    Some(SourceGraphic {
        pixels,
        geometry,
        paint_bounds: paint_bounds.resolve(),
    })
}
