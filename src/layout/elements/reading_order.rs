//! Numbering of lines across the document, for the reading order of marks.

use super::{FlexRow, GridRow, LayoutVisitorMut, TableRow, TextBlock, visit_layout_tree_mut};
use crate::layout::engine::{Page, TextLine};

/// Numbers every line of every page once pages are final, so that the marks
/// painted from one line can be put in its logical order (UAX #9) while
/// marks of different lines never mix.
pub(crate) struct LineNumbering {
    next: u32,
}

impl LineNumbering {
    pub(crate) fn number(pages: &mut [Page]) {
        let mut numbering = Self { next: 1 };
        for page in pages {
            for (_, element) in &mut page.elements {
                visit_layout_tree_mut(element.as_mut(), &mut numbering);
            }
        }
    }

    /// Number each line and, where the bidi algorithm left the runs in their
    /// logical order (no run carries a logical position), give every run its
    /// offset in the line, so marks painted from it in later passes (an
    /// inline box's image, say) still read in place. Inline boxes take their
    /// run's position.
    fn number_lines(&mut self, lines: &mut [TextLine]) {
        for line in lines {
            let number = self.next;
            self.next += 1;
            let reordered = line.runs.iter().any(|run| {
                run.metadata.reading.logical_start != 0 || run.metadata.reading.reversed
            });
            let mut offset = 0;
            for run in &mut line.runs {
                run.metadata.reading.line = Some(number);
                if !reordered {
                    run.metadata.reading.logical_start = offset;
                }
                // Measured as the bidi pass measures it: an inline box with no
                // text stands for one U+FFFC.
                offset += if run.text.is_empty() && run.inline_box.is_some() {
                    '\u{FFFC}'.len_utf8()
                } else {
                    run.text.len()
                };
                if let Some(inline) = &mut run.inline_box {
                    inline.reading = run.metadata.reading;
                    self.number_lines(&mut inline.lines);
                }
            }
        }
    }
}

impl LayoutVisitorMut for LineNumbering {
    fn visit_text_block(&mut self, element: &mut TextBlock) {
        self.number_lines(&mut element.lines);
    }

    fn visit_flex_row(&mut self, element: &mut FlexRow) {
        for cell in &mut element.content.cells {
            self.number_lines(&mut cell.lines);
        }
    }

    fn visit_grid_row(&mut self, element: &mut GridRow) {
        for cell in &mut element.content.cells {
            self.number_lines(&mut cell.layout.content.lines);
        }
    }

    fn visit_table_row(&mut self, element: &mut TableRow) {
        for cell in &mut element.content.cells {
            self.number_lines(&mut cell.layout.content.lines);
        }
    }
}
