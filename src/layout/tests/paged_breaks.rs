use super::support::layout_pages_at;
use crate::layout::cells::GridInset;
use crate::layout::elements::{
    ColumnRule, Container, FlexRow, GridRow, LayoutVisitor, MulticolContainer, TableRow, TextBlock,
};
use crate::layout::engine::{FlexFragmentRole, visit_layout_tree};
use crate::types::PageSize;

#[derive(Debug, Default)]
struct PagedFragments {
    normal: usize,
    normal_flex_cells: usize,
    normal_flex_inline_offsets: Vec<f32>,
    forced_flex_line_breaks: usize,
    parallel_overflow: usize,
    overflow_nested_nodes: usize,
    grid_rows: usize,
    grid_cells: usize,
    grid_insets: Vec<GridInset>,
    grid_row_margin_starts: Vec<f32>,
    grid_row_padding_tops: Vec<f32>,
    sliced_containers: usize,
    containers: Vec<ContainerMetrics>,
    column_rules: usize,
    multicol_fragments: Vec<MulticolFragmentMetrics>,
}

#[derive(Debug)]
struct ContainerMetrics {
    width: f32,
    padding_top: f32,
    border_top: f32,
}

#[derive(Debug)]
struct MulticolFragmentMetrics {
    width: f32,
    block_size: f32,
}

impl LayoutVisitor for PagedFragments {
    fn visit_flex_row(&mut self, row: &FlexRow) {
        match row.content.fragment_role {
            FlexFragmentRole::Normal => {
                self.normal += 1;
                self.normal_flex_cells += row.content.cells.len();
                self.normal_flex_inline_offsets
                    .push(row.inline_offset.value());
                self.forced_flex_line_breaks += row.content.forced_line_breaks.len();
            }
            FlexFragmentRole::ParallelOverflowContinuation => {
                self.parallel_overflow += 1;
                self.overflow_nested_nodes += row
                    .content
                    .cells
                    .iter()
                    .map(|cell| cell.nested_elements.len())
                    .sum::<usize>();
            }
        }
    }

    fn visit_grid_row(&mut self, row: &GridRow) {
        self.grid_rows += 1;
        self.grid_cells += row.content.cells.len();
        self.grid_row_margin_starts
            .push(row.box_model.margins.start);
        self.grid_row_padding_tops.push(row.box_model.padding.top);
        self.grid_insets.extend(
            row.content
                .cells
                .iter()
                .filter_map(|cell| cell.placement.inset),
        );
    }

    fn visit_container(&mut self, container: &Container) {
        self.sliced_containers += usize::from(container.fragmentation.reference_slice.is_some());
        if let Some(width) = container.box_model.size.width.fixed_value() {
            self.containers.push(ContainerMetrics {
                width,
                padding_top: container.box_model.padding.top,
                border_top: container.box_model.border.top.width,
            });
        }
    }

    fn visit_multicol_container(&mut self, multicol: &MulticolContainer) {
        self.visit_container(&multicol.principal);
        if let Some(width) = multicol.principal.box_model.size.width.fixed_value() {
            self.multicol_fragments.push(MulticolFragmentMetrics {
                width,
                block_size: multicol
                    .principal
                    .box_model
                    .size
                    .height
                    .used()
                    .unwrap_or_default(),
            });
        }
    }

    fn visit_column_rule(&mut self, _rule: &ColumnRule) {
        self.column_rules += 1;
    }
}

#[test]
fn nowrap_row_item_break_propagates_without_inventing_a_flex_line() {
    let pages = layout_pages_at(
        include_str!(
            "../../../tests/parity/cases/interactions/interactions-cartesian-flexbox-x-paged-media.html"
        ),
        PageSize::new(144.0, 150.0),
    );

    assert_eq!(pages.len(), 3);
    let mut page_two = PagedFragments::default();
    for (_, element) in &pages[1].elements {
        visit_layout_tree(element.as_ref(), &mut page_two);
    }

    assert_eq!(page_two.parallel_overflow, 0, "page 2: {page_two:#?}");
    assert_eq!(page_two.forced_flex_line_breaks, 0, "page 2: {page_two:#?}");
    assert!(page_two.normal_flex_cells >= 2, "page 2: {page_two:#?}");
    assert!(
        page_two
            .normal_flex_inline_offsets
            .iter()
            .any(|offset| (*offset - 10.5).abs() < 0.001),
        "page 2: {page_two:#?}"
    );
    assert!(
        page_two
            .containers
            .iter()
            .any(|metrics| (metrics.width - 117.0).abs() < 0.001),
        "page 2: {page_two:#?}"
    );
}

#[test]
fn first_grid_row_item_break_propagates_without_an_empty_grid_fragment() {
    let pages = layout_pages_at(
        include_str!(
            "../../../tests/parity/cases/interactions/interactions-cartesian-grid-x-paged-media.html"
        ),
        PageSize::new(144.0, 150.0),
    );

    assert_eq!(pages.len(), 3);
    let mut page_two = PagedFragments::default();
    for (_, element) in &pages[1].elements {
        visit_layout_tree(element.as_ref(), &mut page_two);
    }
    assert!(page_two.grid_rows > 0, "page 2: {page_two:#?}");
    assert!(page_two.grid_cells > 0, "page 2: {page_two:#?}");
    assert_eq!(page_two.parallel_overflow, 0, "page 2: {page_two:#?}");
    assert_eq!(
        page_two.grid_row_margin_starts,
        [0.0],
        "page 2: {page_two:#?}"
    );
    assert_eq!(
        page_two.grid_row_padding_tops,
        [0.0],
        "page 2: {page_two:#?}"
    );
    assert!(
        page_two.containers.iter().any(|metrics| {
            (metrics.width - 94.5).abs() < 0.001
                && (metrics.padding_top - 5.25).abs() < 0.001
                && (metrics.border_top - 1.5).abs() < 0.001
        }),
        "page 2: {page_two:#?}"
    );
}

#[test]
fn nested_clip_mask_break_fragments_ordinary_ancestor_boxes() {
    let pages = layout_pages_at(
        include_str!(
            "../../../tests/parity/cases/interactions/interactions-cartesian-clip-mask-x-paged-media.html"
        ),
        PageSize::new(144.0, 150.0),
    );

    assert_eq!(pages.len(), 4);
    let mut fragments = PagedFragments::default();
    for (_, element) in &pages[2].elements {
        visit_layout_tree(element.as_ref(), &mut fragments);
    }

    assert!(fragments.sliced_containers >= 2, "page 3: {fragments:#?}");
    assert_eq!(fragments.parallel_overflow, 0, "page 3: {fragments:#?}");
}

#[test]
fn page_break_inside_auto_multicol_keeps_ordinary_ancestors_fragmentable() {
    let pages = layout_pages_at(
        include_str!(
            "../../../tests/parity/cases/interactions/interactions-cartesian-multicol-x-paged-media.html"
        ),
        PageSize::new(144.0, 150.0),
    );

    assert_eq!(pages.len(), 4);

    let mut page_two = PagedFragments::default();
    for (_, element) in &pages[1].elements {
        visit_layout_tree(element.as_ref(), &mut page_two);
    }
    assert_eq!(page_two.parallel_overflow, 0, "page 2: {page_two:#?}");
    assert_eq!(page_two.column_rules, 0, "page 2: {page_two:#?}");

    let mut page_three = PagedFragments::default();
    for (_, element) in &pages[2].elements {
        visit_layout_tree(element.as_ref(), &mut page_three);
    }
    assert!(page_three.sliced_containers >= 1, "page 3: {page_three:#?}");
    assert_eq!(page_three.column_rules, 0, "page 3: {page_three:#?}");
    assert!(
        page_three.multicol_fragments.iter().any(|fragment| {
            (fragment.width - 94.5).abs() < 0.001 && fragment.block_size > 40.0
        }),
        "page 3: {page_three:#?}"
    );

    let mut page_four = PagedFragments::default();
    for (_, element) in &pages[3].elements {
        visit_layout_tree(element.as_ref(), &mut page_four);
    }
    assert_eq!(page_four.column_rules, 1, "page 4: {page_four:#?}");
}

#[derive(Debug, Default)]
struct AbsoluteText(Vec<String>);

impl LayoutVisitor for AbsoluteText {
    fn visit_text_block(&mut self, block: &TextBlock) {
        if !block.positioning.scheme.is_absolute() {
            return;
        }
        let text = block
            .lines
            .iter()
            .flat_map(|line| line.runs.iter().map(|run| run.text.as_str()))
            .collect::<String>();
        if !text.is_empty() {
            self.0.push(text);
        }
    }
}

#[test]
fn bottom_positioned_descendant_uses_continuous_fragmented_containing_block() {
    let pages = layout_pages_at(
        include_str!(
            "../../../tests/parity/cases/interactions/interactions-cartesian-paged-media-x-positioning.html"
        ),
        PageSize::new(144.0, 150.0),
    );

    assert_eq!(pages.len(), 4);
    let absolute_text = pages
        .iter()
        .map(|page| {
            let mut text = AbsoluteText::default();
            for (_, element) in &page.elements {
                visit_layout_tree(element.as_ref(), &mut text);
            }
            text.0
        })
        .collect::<Vec<_>>();

    assert_eq!(absolute_text[2], Vec::<String>::new());
    assert_eq!(absolute_text[3], ["Bb"]);
}

#[derive(Debug, Default)]
struct ContainerFragment {
    width: f32,
    height: Option<f32>,
    height_is_definite: bool,
    bottom_border: f32,
    child_count: usize,
}

#[derive(Debug, Default)]
struct ContainerFragments(Vec<ContainerFragment>);

impl LayoutVisitor for ContainerFragments {
    fn visit_container(&mut self, container: &Container) {
        self.0.push(ContainerFragment {
            width: container
                .box_model
                .size
                .width
                .fixed_value()
                .unwrap_or_default(),
            height: container.box_model.size.height.used(),
            height_is_definite: container.box_model.size.height.is_definite(),
            bottom_border: container.box_model.border.bottom.width,
            child_count: container.children.len(),
        });
    }
}

#[test]
fn auto_min_height_has_a_used_extent_without_becoming_definite() {
    let pages = layout_pages_at(
        r#"
            <style>
                @page { size: 144pt 150pt; margin: 0 }
                * { box-sizing: border-box; margin: 0 }
                .stage { width: 117pt; min-height: 123pt; border: 1pt solid silver }
            </style>
            <div class="stage"><div>short</div></div>
        "#,
        PageSize::new(144.0, 150.0),
    );

    assert_eq!(pages.len(), 1);
    let mut fragments = ContainerFragments::default();
    for (_, element) in &pages[0].elements {
        visit_layout_tree(element.as_ref(), &mut fragments);
    }
    assert!(
        fragments.0.iter().any(|fragment| fragment.width == 117.0
            && fragment.height == Some(123.0)
            && !fragment.height_is_definite),
        "page 1: {fragments:#?}"
    );
}

#[test]
fn descendant_break_fills_each_auto_sized_ancestor_fragment() {
    let pages = layout_pages_at(
        r#"
            <style>
                @page { size: 144pt 150pt; margin: 0 }
                * { box-sizing: border-box; margin: 0 }
                .stage { width: 117pt; min-height: 123pt; border: 1pt solid silver }
                .outer {
                    width: 94.5pt;
                    min-height: 72pt;
                    margin: 24.75pt auto 0;
                    border: 1.5pt solid navy;
                    padding: 5.25pt;
                    background: linear-gradient(135deg, gold, teal);
                }
                .before { height: 16.5pt }
                .after { width: 43.5pt; height: 36pt; break-before: page }
            </style>
            <div class="stage">
                <div class="outer">
                    <div class="before">before</div>
                    <div class="after">after</div>
                </div>
            </div>
        "#,
        PageSize::new(144.0, 150.0),
    );

    assert_eq!(pages.len(), 2);
    let mut first_page = ContainerFragments::default();
    for (_, element) in &pages[0].elements {
        visit_layout_tree(element.as_ref(), &mut first_page);
    }
    assert!(
        first_page.0.iter().any(|fragment| fragment.width == 117.0
            && fragment.height == Some(150.0)
            && fragment.bottom_border == 0.0
            && fragment.child_count == 1),
        "page 1: {first_page:#?}"
    );
    assert!(
        first_page.0.iter().any(|fragment| fragment.width == 94.5
            && fragment.height == Some(124.25)
            && fragment.bottom_border == 0.0
            && fragment.child_count == 1),
        "page 1: {first_page:#?}"
    );
}

#[derive(Debug, Default)]
struct TableRowTexts(Vec<String>);

impl LayoutVisitor for TableRowTexts {
    fn visit_table_row(&mut self, row: &TableRow) {
        let words = row
            .content
            .cells
            .iter()
            .flat_map(|cell| &cell.layout.content.lines)
            .flat_map(|line| &line.runs)
            .map(|run| run.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>();
        self.0.push(words.join(" "));
    }
}

/// CSS Fragmentation 3 §4.4: unforced breaks keep content from overflowing the
/// fragmentainer, and a forced break after a box doesn't exempt that box. A table
/// taller than the page that is followed by a forced break continues on the next
/// page instead of overflowing the first.
#[test]
fn box_taller_than_the_page_fragments_before_a_forced_break() {
    let rows = (1..=40)
        .map(|row| format!("<tr><td>Row {row}</td></tr>"))
        .collect::<String>();
    let pages = layout_pages_at(
        &format!(
            r#"
                <style>@page {{ size: 200pt 150pt; margin: 0 }} * {{ margin: 0 }}</style>
                <table>{rows}</table>
                <p style="break-before: page">After</p>
            "#
        ),
        PageSize::new(200.0, 150.0),
    );

    let mut rows_laid_out = TableRowTexts::default();
    for (page_index, page) in pages.iter().enumerate() {
        for (y, element) in &page.elements {
            let bottom = y + crate::layout::paginate::estimate_element_height(element.as_ref());
            assert!(
                bottom <= 150.0 + 0.01,
                "page {} has an element ending at {bottom}pt",
                page_index + 1
            );
            visit_layout_tree(element.as_ref(), &mut rows_laid_out);
        }
    }
    let expected = (1..=40).map(|row| format!("Row {row}")).collect::<Vec<_>>();
    assert_eq!(rows_laid_out.0, expected);
    assert!(pages.len() >= 3, "{} pages", pages.len());
}
