//! Filter ownership for layout cells.
//!
//! Grid, table, and flex formatting contexts store their items differently,
//! but a CSS filter always replaces one composited source box. This module is
//! the shared boundary that attaches that result to the concrete cell instead
//! of teaching every renderer a formatting-context-specific workaround.

use crate::layout::cells::{CellPaintHolder, GridCell};
use crate::layout::elements::{FilterHolder, FlexRow, GridRow};
use crate::layout::engine::FlexCell;
use crate::types::Size;

use super::ResolvedFilter;
use super::paint_space::{InheritedFilterPaintSpace, PageBoxAnchor};

/// Retain a grid-item filter until pagination supplies the row's absolute
/// device-space anchor.
pub(crate) fn retain_grid_cell_filter(cell: &mut GridCell, filter: ResolvedFilter) {
    if filter.requires_source_surface() {
        *cell.cell_paint_mut().filter_slot_mut() = Some(filter);
    }
}

/// Materialize filters retained on flattened flex items after pagination has
/// established the concrete cell fragment sizes.
pub(super) fn materialize_flex_row(
    flex: &mut FlexRow,
    anchor: PageBoxAnchor,
    inherited_space: InheritedFilterPaintSpace,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
) {
    let frames = super::surface::flex_cell_source_frames(flex, fonts);
    for (cell, frame) in flex.content.cells.iter_mut().zip(frames) {
        let Some(filter) = cell.cell_paint_mut().take_filter() else {
            continue;
        };
        let cell_anchor = frame.page_anchor_in(anchor);
        let paint_space = inherited_space.enter(
            cell_anchor,
            frame.size,
            Some(&cell.cell_paint().group),
            Some(cell),
        );
        if composite_flex_cell(
            cell,
            frame.size,
            paint_space.source_raster_space(filter.matrix_capability()),
            &filter,
            fonts,
            filter_dpi,
        ) {
            continue;
        }
        filter.apply_flex_cell_fallback(cell);
    }
}

fn composite_flex_cell(
    cell: &mut FlexCell,
    size: Size,
    raster_space: super::surface::SourceRasterSpace,
    filter: &ResolvedFilter,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
) -> bool {
    if !filter.requires_source_surface() {
        return false;
    }
    let compositing = super::FilterCompositing::from_group(&cell.cell_paint_mut().group);
    let Some(source) =
        super::surface::paint_flex_cell_source(cell, size, fonts, filter_dpi, raster_space)
    else {
        return false;
    };
    let Some((output, _)) =
        super::composite_source_graphic(source, filter, compositing, filter_dpi)
    else {
        return false;
    };
    cell.cell_paint_mut().filter_output = Some(output);
    true
}

/// Materialize filters retained on grid items after pagination has established
/// the concrete cell fragment sizes and device-space phase.
pub(super) fn materialize_grid_row(
    grid: &mut GridRow,
    anchor: PageBoxAnchor,
    inherited_space: InheritedFilterPaintSpace,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
) {
    let frames = super::surface::grid_cell_source_frames(grid);
    for (cell, frame) in grid.content.cells.iter_mut().zip(frames) {
        let Some(filter) = cell.cell_paint_mut().take_filter() else {
            continue;
        };
        let cell_anchor = frame.page_anchor_in(anchor);
        let paint_space = inherited_space.enter(
            cell_anchor,
            frame.size,
            Some(&cell.cell_paint().group),
            Some(&cell.layout.box_model),
        );
        if !composite_grid_cell(
            cell,
            frame.size,
            paint_space.source_raster_space(filter.matrix_capability()),
            &filter,
            fonts,
            filter_dpi,
        ) {
            *cell.cell_paint_mut().filter_slot_mut() = Some(filter);
        }
    }
}

fn composite_grid_cell(
    cell: &mut GridCell,
    size: Size,
    raster_space: super::surface::SourceRasterSpace,
    filter: &ResolvedFilter,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
) -> bool {
    if !filter.requires_source_surface() {
        return false;
    }
    let compositing = super::FilterCompositing::from_group(&cell.cell_paint_mut().group);
    let Some(source) =
        super::surface::paint_grid_cell_source(cell, size, fonts, filter_dpi, raster_space)
    else {
        return false;
    };
    let Some((output, _)) =
        super::composite_source_graphic(source, filter, compositing, filter_dpi)
    else {
        return false;
    };
    cell.cell_paint_mut().filter_output = Some(output);
    true
}
