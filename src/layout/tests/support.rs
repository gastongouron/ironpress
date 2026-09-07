use crate::layout::elements::{GridRow, LayoutVisitor, TableRow, TextBlock};
use crate::layout::engine::{
    FlexRow, Page, TextRun, layout_with_rules, layout_with_rules_and_fonts, visit_layout_tree,
};
use crate::parser::css::parse_stylesheet;
use crate::parser::html::parse_html_with_styles;
use crate::types::{Margin, PageSize};

#[derive(Default)]
struct RunCollector(Vec<TextRun>);

impl LayoutVisitor for RunCollector {
    fn visit_text_block(&mut self, block: &TextBlock) {
        self.0.extend(
            block
                .lines
                .iter()
                .flat_map(|line| line.runs.iter().cloned()),
        );
    }

    fn visit_flex_row(&mut self, row: &FlexRow) {
        self.0.extend(
            row.content
                .cells
                .iter()
                .flat_map(|cell| cell.lines.iter())
                .flat_map(|line| line.runs.iter().cloned()),
        );
    }

    fn visit_grid_row(&mut self, row: &GridRow) {
        self.0.extend(
            row.content
                .cells
                .iter()
                .flat_map(|cell| &cell.layout.content.lines)
                .flat_map(|line| line.runs.iter().cloned()),
        );
    }

    fn visit_table_row(&mut self, row: &TableRow) {
        self.0.extend(
            row.content
                .cells
                .iter()
                .flat_map(|cell| &cell.layout.content.lines)
                .flat_map(|line| line.runs.iter().cloned()),
        );
    }
}

pub(super) fn layout_pages(markup: &str) -> Vec<Page> {
    layout_pages_at(markup, PageSize::new(390.0, 150.0))
}

pub(super) fn layout_pages_at(markup: &str, page_size: PageSize) -> Vec<Page> {
    let document = parse_html_with_styles(markup).expect("valid layout fixture");
    let rules = document
        .stylesheets
        .iter()
        .flat_map(|stylesheet| parse_stylesheet(stylesheet))
        .collect::<Vec<_>>();
    layout_with_rules(&document.nodes, page_size, Margin::uniform(0.0), &rules)
}

pub(super) fn layout_pages_with_fonts(
    markup: &str,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> Vec<Page> {
    layout_pages_at_with_fonts(
        markup,
        PageSize::new(390.0, 150.0),
        Margin::uniform(0.0),
        fonts,
    )
}

pub(super) fn layout_pages_at_with_fonts(
    markup: &str,
    page_size: PageSize,
    margin: Margin,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> Vec<Page> {
    let document = parse_html_with_styles(markup).expect("valid layout fixture");
    let rules = document
        .stylesheets
        .iter()
        .flat_map(|stylesheet| parse_stylesheet(stylesheet))
        .collect::<Vec<_>>();
    layout_with_rules_and_fonts(
        &document.nodes,
        page_size,
        margin,
        &rules,
        fonts,
        None,
        300.0,
        crate::layout::paginate::FootnoteAreaLayout::default(),
    )
}

fn collected_runs(markup: &str) -> Vec<TextRun> {
    let mut collector = RunCollector::default();
    for page in layout_pages(markup) {
        for (_, element) in page.elements {
            visit_layout_tree(element.as_ref(), &mut collector);
        }
    }
    collector.0
}

pub(super) fn visible_runs(markup: &str) -> Vec<TextRun> {
    collected_runs(markup)
        .into_iter()
        .filter(|run| !run.text.trim().is_empty())
        .collect()
}

pub(super) fn visible_inline_box_runs(markup: &str) -> Vec<TextRun> {
    collected_runs(markup)
        .into_iter()
        .flat_map(|run| {
            run.inline_box
                .into_iter()
                .flat_map(|inline| inline.lines.into_iter())
                .flat_map(|line| line.runs)
        })
        .filter(|run| !run.text.trim().is_empty())
        .collect()
}
