use std::collections::HashMap;

use crate::layout::elements::TableGridIdentity;

/// Position of one cell in the normalized row sequence of a table grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct TableCellPosition {
    row: NormalizedTableRow,
    cell: NormalizedTableCell,
}

impl TableCellPosition {
    pub(super) const fn new(normalized_row: usize, normalized_cell: usize) -> Self {
        Self {
            row: NormalizedTableRow(normalized_row),
            cell: NormalizedTableCell(normalized_cell),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NormalizedTableRow(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NormalizedTableCell(usize);

/// Hashable table width admitted at the memo boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TableSizingWidth(u32);

impl TableSizingWidth {
    fn parse(points: f32) -> Option<Self> {
        if !points.is_finite() || points < 0.0 {
            return None;
        }
        let normalized = if points == 0.0 { 0.0 } else { points };
        Some(Self(normalized.to_bits()))
    }
}

/// Intrinsic outer widths computed for one table cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TableCellIntrinsicWidths {
    preferred_outer: f32,
    minimum_outer: f32,
}

impl TableCellIntrinsicWidths {
    pub(super) fn parse(preferred_outer: f32, minimum_outer: f32) -> Option<Self> {
        if !preferred_outer.is_finite()
            || !minimum_outer.is_finite()
            || preferred_outer < 0.0
            || minimum_outer < 0.0
            || minimum_outer > preferred_outer
        {
            return None;
        }
        Some(Self {
            preferred_outer,
            minimum_outer,
        })
    }

    pub(super) const fn preferred_outer(self) -> f32 {
        self.preferred_outer
    }

    pub(super) const fn minimum_outer(self) -> f32 {
        self.minimum_outer
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CellMeasurementKey {
    position: TableCellPosition,
    width: TableSizingWidth,
}

#[derive(Default)]
struct TableMeasurements {
    cells: HashMap<CellMeasurementKey, TableCellIntrinsicWidths>,
}

/// Conversion-owned intrinsic-width memo for table cells.
#[derive(Default)]
pub(crate) struct TableCellSizingMemo {
    tables: HashMap<TableGridIdentity, TableMeasurements>,
}

impl TableCellSizingMemo {
    pub(super) fn lookup(
        &self,
        table: &TableGridIdentity,
        position: TableCellPosition,
        inner_width: f32,
    ) -> Option<TableCellIntrinsicWidths> {
        let width = TableSizingWidth::parse(inner_width)?;
        let key = CellMeasurementKey { position, width };
        self.tables.get(table)?.cells.get(&key).copied()
    }

    pub(super) fn remember(
        &mut self,
        table: &TableGridIdentity,
        position: TableCellPosition,
        inner_width: f32,
        widths: TableCellIntrinsicWidths,
    ) {
        let Some(width) = TableSizingWidth::parse(inner_width) else {
            return;
        };
        let key = CellMeasurementKey { position, width };
        if let Some(measurements) = self.tables.get_mut(table) {
            measurements.cells.insert(key, widths);
        } else {
            let mut measurements = TableMeasurements::default();
            measurements.cells.insert(key, widths);
            self.tables.insert(table.clone(), measurements);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(path: impl IntoIterator<Item = usize>) -> TableGridIdentity {
        TableGridIdentity::from_source_path(path)
    }

    #[test]
    fn sizing_width_parses_only_finite_non_negative_points() {
        assert_eq!(TableSizingWidth::parse(-0.0), TableSizingWidth::parse(0.0));
        assert!(TableSizingWidth::parse(12.5).is_some());
        assert!(TableSizingWidth::parse(-1.0).is_none());
        assert!(TableSizingWidth::parse(f32::NAN).is_none());
        assert!(TableSizingWidth::parse(f32::INFINITY).is_none());
    }

    #[test]
    fn memo_keys_measurements_by_semantic_table_cell_and_width() {
        let first_grid = grid([0, 2]);
        let other_grid = grid([0, 3]);
        let position = TableCellPosition::new(1, 4);
        let widths = TableCellIntrinsicWidths::parse(80.0, 24.0).expect("valid widths");
        let mut memo = TableCellSizingMemo::default();

        memo.remember(&first_grid, position, 120.0, widths);
        assert_eq!(memo.lookup(&first_grid, position, 120.0), Some(widths));
        assert_eq!(memo.lookup(&other_grid, position, 120.0), None);
        assert_eq!(
            memo.lookup(&first_grid, TableCellPosition::new(2, 4), 120.0),
            None
        );
        assert_eq!(
            memo.lookup(&first_grid, TableCellPosition::new(1, 5), 120.0),
            None
        );
        assert_eq!(memo.lookup(&first_grid, position, 121.0), None);
    }

    #[test]
    fn separate_layout_memos_never_share_entries() {
        let grid = grid([4, 2]);
        let position = TableCellPosition::new(3, 1);
        let widths = TableCellIntrinsicWidths::parse(48.0, 12.0).expect("valid widths");
        let mut first_layout = TableCellSizingMemo::default();
        let second_layout = TableCellSizingMemo::default();

        first_layout.remember(&grid, position, 90.0, widths);
        assert_eq!(second_layout.lookup(&grid, position, 90.0), None);
    }

    #[test]
    fn invalid_measurements_are_not_cached() {
        let grid = grid([1]);
        let position = TableCellPosition::new(0, 0);
        let valid = TableCellIntrinsicWidths::parse(48.0, 12.0).expect("valid widths");
        let mut memo = TableCellSizingMemo::default();

        memo.remember(&grid, position, f32::NAN, valid);
        assert_eq!(memo.lookup(&grid, position, f32::NAN), None);
        assert!(TableCellIntrinsicWidths::parse(f32::NAN, 12.0).is_none());
        assert!(TableCellIntrinsicWidths::parse(48.0, -1.0).is_none());
    }
}
