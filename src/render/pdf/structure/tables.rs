//! Table regularity and header scope (ISO 14289-1 7.5, veraPDF 7.2-41…43
//! and 7.5-1).
//!
//! veraPDF places every cell on a grid sized from the first row and fails
//! empty slots and overflowing spans, both legal in HTML. Rows are padded
//! with empty `TD`s and spans are clamped to the rows that remain. Under the
//! PDF/UA-1 flavour no header scope is inferred, so every `TH` gets one.

use super::tree::Element;
use crate::accessibility::HeaderScope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CellFacts {
    pub(super) header: bool,
    /// The cell's row is inside `<thead>`.
    pub(super) in_head: bool,
    pub(super) col_span: u32,
    pub(super) row_span: u32,
    pub(super) scope: Option<HeaderScope>,
}

pub(super) fn regularize_tables(elements: &mut Vec<Element>) {
    let tables: Vec<usize> = (0..elements.len())
        .filter(|index| elements[*index].name == "Table")
        .collect();
    for table in tables {
        let rows = table_rows(elements, table);
        let column_count = grid_columns(elements, &rows);
        pad_rows_and_attribute_cells(elements, &rows, column_count);
    }
}

fn table_rows(elements: &[Element], table: usize) -> Vec<usize> {
    let mut rows = Vec::new();
    for child in &elements[table].children {
        match elements[*child].name.as_str() {
            "TR" => rows.push(*child),
            "THead" | "TBody" | "TFoot" => rows.extend(
                elements[*child]
                    .children
                    .iter()
                    .copied()
                    .filter(|row| elements[*row].name == "TR"),
            ),
            _ => {}
        }
    }
    rows
}

fn row_cells(elements: &[Element], row: usize) -> Vec<(usize, CellFacts)> {
    elements[row]
        .children
        .iter()
        .filter_map(|cell| elements[*cell].cell.map(|facts| (*cell, facts)))
        .collect()
}

/// Per column, the number of rows (the current one included) a placed cell
/// still covers; `next_row` retires one row.
#[derive(Default)]
struct RowSpanCarry(Vec<u32>);

impl RowSpanCarry {
    fn blocked(&self, column: usize) -> bool {
        self.0.get(column).is_some_and(|remaining| *remaining > 0)
    }

    fn block(&mut self, column: usize, rows: u32) {
        if self.0.len() <= column {
            self.0.resize(column + 1, 0);
        }
        self.0[column] = self.0[column].max(rows);
    }

    fn next_row(&mut self) {
        for remaining in &mut self.0 {
            *remaining = remaining.saturating_sub(1);
        }
    }

    fn blocked_count(&self) -> usize {
        self.0.iter().filter(|remaining| **remaining > 0).count()
    }
}

/// Place every cell and return the widest column extent.
fn grid_columns(elements: &[Element], rows: &[usize]) -> usize {
    let mut carry = RowSpanCarry::default();
    let mut column_count = 0;
    for (row_index, row) in rows.iter().enumerate() {
        let rows_left = (rows.len() - row_index) as u32;
        let mut column = 0;
        for (_, facts) in row_cells(elements, *row) {
            while carry.blocked(column) {
                column += 1;
            }
            let span = facts.col_span.max(1) as usize;
            let row_span = facts.row_span.clamp(1, rows_left);
            for covered in column..column + span {
                carry.block(covered, row_span);
            }
            column += span;
        }
        column_count = column_count.max(column).max(carry.0.len());
        carry.next_row();
    }
    column_count
}

fn pad_rows_and_attribute_cells(elements: &mut Vec<Element>, rows: &[usize], column_count: usize) {
    let mut carry = RowSpanCarry::default();
    for (row_index, row) in rows.iter().enumerate() {
        let rows_left = (rows.len() - row_index) as u32;
        let cells = row_cells(elements, *row);
        let all_headers = !cells.is_empty() && cells.iter().all(|(_, facts)| facts.header);
        let mut column = 0;
        let mut used = carry.blocked_count();
        for (cell, facts) in &cells {
            while carry.blocked(column) {
                column += 1;
            }
            let span = facts.col_span.max(1) as usize;
            let row_span = facts.row_span.clamp(1, rows_left);
            for covered in column..column + span {
                carry.block(covered, row_span);
            }
            column += span;
            used += span;
            elements[*cell].attributes.push_str(&cell_attributes(
                *facts,
                span as u32,
                row_span,
                all_headers,
            ));
        }
        for _ in used..column_count {
            let padding = Element::new("TD", Some(*row));
            let index = elements.len();
            elements[*row].children.push(index);
            elements.push(padding);
        }
        carry.next_row();
    }
}

fn cell_attributes(facts: CellFacts, col_span: u32, row_span: u32, all_headers: bool) -> String {
    let mut entries = String::new();
    if facts.header {
        let scope = match facts.scope {
            Some(HeaderScope::Row | HeaderScope::RowGroup) => "Row",
            Some(HeaderScope::Column | HeaderScope::ColumnGroup) => "Column",
            None if facts.in_head || all_headers => "Column",
            None => "Row",
        };
        entries.push_str(&format!(" /Scope /{scope}"));
    }
    if col_span > 1 {
        entries.push_str(&format!(" /ColSpan {col_span}"));
    }
    if row_span > 1 {
        entries.push_str(&format!(" /RowSpan {row_span}"));
    }
    if entries.is_empty() {
        String::new()
    } else {
        format!(" /A << /O /Table{entries} >>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: &[&[(bool, u32, u32)]]) -> (Vec<Element>, usize) {
        let mut elements = vec![Element::new("Document", None)];
        let table = push(&mut elements, Element::new("Table", Some(0)));
        for row in rows {
            let row_index = push(&mut elements, Element::new("TR", Some(table)));
            for (header, col_span, row_span) in row.iter() {
                let mut cell = Element::new(if *header { "TH" } else { "TD" }, Some(row_index));
                cell.cell = Some(CellFacts {
                    header: *header,
                    in_head: false,
                    col_span: *col_span,
                    row_span: *row_span,
                    scope: None,
                });
                push(&mut elements, cell);
            }
        }
        (elements, table)
    }

    fn push(elements: &mut Vec<Element>, element: Element) -> usize {
        let index = elements.len();
        if let Some(parent) = element.parent_index() {
            elements[parent].children.push(index);
        }
        elements.push(element);
        index
    }

    fn row_names(elements: &[Element], table: usize) -> Vec<Vec<String>> {
        table_rows(elements, table)
            .iter()
            .map(|row| {
                elements[*row]
                    .children
                    .iter()
                    .map(|cell| format!("{}{}", elements[*cell].name, elements[*cell].attributes))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn ragged_rows_are_padded_to_the_widest_row() {
        // veraPDF 7.2-41: every row must fill the table's columns.
        let (mut elements, table) = table(&[
            &[(true, 1, 1), (true, 1, 1), (true, 1, 1)],
            &[(false, 1, 1)],
        ]);
        regularize_tables(&mut elements);
        assert_eq!(
            row_names(&elements, table),
            vec![
                vec![
                    "TH /A << /O /Table /Scope /Column >>".to_string(),
                    "TH /A << /O /Table /Scope /Column >>".to_string(),
                    "TH /A << /O /Table /Scope /Column >>".to_string(),
                ],
                vec!["TD".to_string(), "TD".to_string(), "TD".to_string()],
            ]
        );
    }

    #[test]
    fn spans_fill_the_grid_and_row_spans_are_clamped_to_the_remaining_rows() {
        // veraPDF 7.2-42/43: spans may not overflow the grid.
        let (mut elements, table) = table(&[
            &[(true, 1, 3), (false, 2, 1)],
            &[(false, 1, 1), (false, 1, 1)],
        ]);
        regularize_tables(&mut elements);
        assert_eq!(
            row_names(&elements, table),
            vec![
                vec![
                    "TH /A << /O /Table /Scope /Row /RowSpan 2 >>".to_string(),
                    "TD /A << /O /Table /ColSpan 2 >>".to_string(),
                ],
                vec!["TD".to_string(), "TD".to_string()],
            ]
        );
    }

    #[test]
    fn header_scope_follows_the_row_shape_when_not_authored() {
        // veraPDF 7.5-1 under PDF/UA-1: every TH needs an explicit Scope.
        let (mut elements, table) = table(&[&[(true, 1, 1), (false, 1, 1)]]);
        regularize_tables(&mut elements);
        assert_eq!(
            row_names(&elements, table)[0][0],
            "TH /A << /O /Table /Scope /Row >>"
        );
    }
}
