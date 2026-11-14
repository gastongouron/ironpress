//! What a painted picture stands for in the structure tree.

use super::{
    FlexRow, GridRow, Image, LayoutElement, LayoutNode, LayoutVisitor, Svg, TableRow, TextBlock,
};
use crate::layout::cells::CellContent;
use crate::layout::engine::TextLine;

/// The text of boxes painted as one picture, in tree order: lines joined by
/// spaces, except after a synthesized hyphen, whose U+00AD already ends the
/// line (ISO 32000-1 14.9.4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SpokenText(String);

impl SpokenText {
    /// The text of `element` and everything inside it.
    pub(crate) fn of_element(element: &dyn LayoutElement) -> Self {
        let mut text = Self::default();
        text.push_element(element);
        text
    }

    /// The text of lines painted as one picture.
    pub(crate) fn of_lines(lines: &[TextLine]) -> Self {
        let mut text = Self::default();
        text.push_lines(lines);
        text
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn push_lines(&mut self, lines: &[TextLine]) {
        for line in lines {
            let spoken: String = line
                .runs
                .iter()
                .map(|run| run.metadata.hyphenation.spoken_text(&run.text))
                .collect();
            self.push_str(spoken.trim());
        }
    }

    fn push_str(&mut self, spoken: &str) {
        if spoken.is_empty() {
            return;
        }
        if !self.0.is_empty() && !self.0.ends_with('\u{AD}') {
            self.0.push(' ');
        }
        self.0.push_str(spoken);
    }

    fn push_children(&mut self, children: &[LayoutNode]) {
        for child in children {
            self.push_element(child.as_ref());
        }
    }

    fn push_cell(&mut self, content: &CellContent) {
        self.push_lines(&content.lines);
        self.push_children(&content.children);
    }

    fn push_element(&mut self, element: &dyn LayoutElement) {
        let descend = {
            let mut collector = Collector {
                text: self,
                descend: true,
            };
            element.accept(&mut collector);
            collector.descend
        };
        if descend {
            element.visit_children(&mut |child| self.push_element(child));
        }
    }
}

/// Collects the text of one element; rows interleave each cell's lines with
/// its nested boxes, so they take over the descent.
struct Collector<'a> {
    text: &'a mut SpokenText,
    descend: bool,
}

impl LayoutVisitor for Collector<'_> {
    fn visit_text_block(&mut self, element: &TextBlock) {
        self.text.push_lines(&element.lines);
    }

    fn visit_flex_row(&mut self, element: &FlexRow) {
        for cell in &element.content.cells {
            self.text.push_lines(&cell.lines);
            self.text.push_children(&cell.nested_elements);
        }
        self.descend = false;
    }

    fn visit_grid_row(&mut self, element: &GridRow) {
        for cell in &element.content.cells {
            self.text.push_cell(&cell.layout.content);
        }
        self.descend = false;
    }

    fn visit_table_row(&mut self, element: &TableRow) {
        for cell in &element.content.cells {
            self.text.push_cell(&cell.layout.content);
        }
        self.descend = false;
    }

    fn visit_image(&mut self, element: &Image) {
        if let PictureMeaning::RasterizedBoxes(text) = &element.paint.meaning {
            self.text.push_str(text.as_str());
        }
    }
}

/// What a painted picture stands for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum PictureMeaning {
    /// A replaced image: the Figure its element names.
    #[default]
    Figure,
    /// Boxes a filter rasterized; the reader sees their text.
    RasterizedBoxes(SpokenText),
}

impl PictureMeaning {
    /// What a filter's picture of `element` stands for: a replaced image or
    /// SVG keeps its Figure, anything else is read as its text.
    pub(crate) fn of_element(element: &dyn LayoutElement) -> Self {
        struct Replaced(Option<PictureMeaning>);

        impl LayoutVisitor for Replaced {
            fn visit_image(&mut self, element: &Image) {
                self.0 = Some(element.paint.meaning.clone());
            }

            fn visit_svg(&mut self, _element: &Svg) {
                self.0 = Some(PictureMeaning::Figure);
            }
        }

        let mut replaced = Replaced(None);
        element.accept(&mut replaced);
        replaced
            .0
            .unwrap_or_else(|| Self::RasterizedBoxes(SpokenText::of_element(element)))
    }

    /// What a filter's picture of a flex item stands for.
    pub(crate) fn of_flex_cell(cell: &crate::layout::engine::FlexCell) -> Self {
        let mut text = SpokenText::default();
        text.push_lines(&cell.lines);
        text.push_children(&cell.nested_elements);
        Self::RasterizedBoxes(text)
    }

    /// What a filter's picture of a grid or table cell stands for.
    pub(crate) fn of_cell(content: &CellContent) -> Self {
        let mut text = SpokenText::default();
        text.push_cell(content);
        Self::RasterizedBoxes(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::{RunHyphenation, TextRun};

    fn line(words: &[(&str, RunHyphenation)]) -> TextLine {
        TextLine {
            runs: words
                .iter()
                .map(|(text, hyphenation)| {
                    let mut run = TextRun {
                        text: text.to_string(),
                        ..Default::default()
                    };
                    run.metadata.hyphenation = *hyphenation;
                    run
                })
                .collect(),
            height: 0.0,
            baseline_ascent: None,
            x_offset: 0.0,
            metadata: Default::default(),
        }
    }

    /// ISO 32000-1 14.9.4: the lines of a rasterized block read as one text,
    /// a line break as a space, except after a soft hyphen that ends the
    /// line; blank lines contribute nothing.
    #[test]
    fn lines_join_with_spaces_except_after_soft_hyphens() {
        let lines = [
            line(&[("Soft ", RunHyphenation::Authored)]),
            line(&[]),
            line(&[("hy-", RunHyphenation::SynthesizedHyphen)]),
            line(&[
                ("phen", RunHyphenation::Authored),
                ("ated", RunHyphenation::Authored),
            ]),
        ];
        assert_eq!(
            SpokenText::of_lines(&lines).as_str(),
            "Soft hy\u{AD}phenated"
        );
    }
}
