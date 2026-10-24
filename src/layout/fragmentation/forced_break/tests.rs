use super::*;
use crate::layout::cells::{CellBox, CellContent, TableCell};
use crate::layout::elements::{
    BoxFragmentation, BoxModel, BoxPaint, FlexContent, LayoutSize, OverflowBehavior,
    TableBoxDecoration, TableCells, TextBlock,
};
use crate::layout::engine::{FlexItemBlockSize, LayoutBorderSide};
use crate::types::Color;

fn container(children: Vec<LayoutNode>, height: Option<f32>) -> LayoutNode {
    Container {
        children,
        box_model: BoxModel {
            size: LayoutSize::fixed(40.0, height),
            padding: EdgeSizes::uniform(2.0),
            ..Default::default()
        },
        paint: BoxPaint {
            background: crate::layout::elements::BackgroundPaint {
                color: Some(Color::from_srgb(0.1, 0.2, 0.3, 1.0)),
                ..Default::default()
            },
            ..Default::default()
        },
        fragmentation: BoxFragmentation {
            decoration: BoxDecorationBreak::Slice,
            ..Default::default()
        },
        overflow: OverflowBehavior {
            combined: Overflow::Visible,
            x: Overflow::Visible,
            y: Overflow::Visible,
        },
        ..Default::default()
    }
    .boxed()
}

fn row(nested: Vec<LayoutNode>) -> LayoutNode {
    FlexRow {
        content: FlexContent {
            cells: vec![FlexCell {
                width: 40.0,
                natural_height: 40.0,
                fragmentation: FlexItemFragmentation::definite(),
                nested_elements: nested,
                ..Default::default()
            }],
            row_height: 40.0,
            alignment: AlignItems::FlexStart,
            ..Default::default()
        },
        box_model: BoxModel {
            size: LayoutSize::fixed(40.0, None),
            ..Default::default()
        },
        paint: BoxPaint {
            background: crate::layout::elements::BackgroundPaint {
                color: Some(Color::from_srgb(0.9, 0.9, 0.9, 1.0)),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    }
    .boxed()
}

#[derive(Default)]
struct FragmentSnapshot {
    flex_cell_count: usize,
    flex_row_height: f32,
    flex_has_background: bool,
    flex_cell_has_background: bool,
    flex_cell_fragment_extent: Option<f32>,
    flex_cell_border_top: f32,
    flex_cell_border_bottom: f32,
    container_child_count: usize,
    container_height: Option<f32>,
    container_has_background: bool,
    container_reference_slice: Option<(f32, f32)>,
}

impl LayoutVisitor for FragmentSnapshot {
    fn visit_flex_row(&mut self, element: &FlexRow) {
        self.flex_cell_count = element.content.cells.len();
        self.flex_row_height = element.content.row_height;
        self.flex_has_background = element.paint.background.color.is_some();
        if let Some(cell) = element.content.cells.first() {
            self.flex_cell_has_background = cell.paint.background.color.is_some();
            self.flex_cell_fragment_extent = cell.fragmentation.fragment_block_extent;
            self.flex_cell_border_top = cell.border.top.width;
            self.flex_cell_border_bottom = cell.border.bottom.width;
        }
        if let Some(container) = element
            .content
            .cells
            .first()
            .and_then(|cell| cell.nested_elements.first())
        {
            container.accept(self);
        }
    }

    fn visit_container(&mut self, element: &Container) {
        self.container_child_count = element.children.len();
        self.container_height = element.box_model.size.height.used();
        self.container_has_background = element.paint.background.color.is_some();
        self.container_reference_slice = element
            .fragmentation
            .reference_slice
            .map(|slice| (slice.block_offset(), slice.composite_block_size()));
    }
}

fn fragment_snapshot(element: &dyn LayoutElement) -> FragmentSnapshot {
    let mut snapshot = FragmentSnapshot::default();
    element.accept(&mut snapshot);
    snapshot
}

#[test]
fn block_edge_break_bubbles_without_an_empty_box_fragment() {
    let element = container(
        vec![
            PageBreak {
                side: PageBreakSide::Any,
                page_name: None,
            }
            .boxed(),
            container(Vec::new(), None),
        ],
        Some(40.0),
    );
    let (before, after, _) = split_flow_at_descendant_break(element.as_ref(), 100.0).unwrap();

    assert!(before.is_none());
    let after = fragment_snapshot(
        after
            .as_deref()
            .expect("the principal box follows its propagated break"),
    );
    assert_eq!(after.container_child_count, 1);
    assert_eq!(after.container_height, Some(40.0));
    assert!(after.container_has_background);
}

#[test]
fn internal_block_break_slices_the_ancestor_box() {
    let element = container(
        vec![
            container(Vec::new(), None),
            PageBreak {
                side: PageBreakSide::Any,
                page_name: None,
            }
            .boxed(),
            container(Vec::new(), None),
        ],
        Some(40.0),
    );
    let (before, after, _) = split_flow_at_descendant_break(element.as_ref(), 100.0).unwrap();

    let before = fragment_snapshot(
        before
            .as_deref()
            .expect("content before the break forms a fragment"),
    );
    assert_eq!(before.container_child_count, 1);
    assert_eq!(before.container_height, Some(40.0));

    let after = after
        .as_deref()
        .expect("content after the break forms an overflow fragment");
    assert_eq!(
        after.page_content_role(),
        crate::layout::elements::PageContentRole::OverflowContinuation
    );
    let after = fragment_snapshot(after);
    assert_eq!(after.container_child_count, 1);
    assert_eq!(after.container_height, Some(0.0));
    assert!(!after.container_has_background);
}

#[test]
fn auto_container_fragments_share_one_composite_decoration_box() {
    let element = container(
        vec![
            container(Vec::new(), None),
            PageBreak {
                side: PageBreakSide::Any,
                page_name: None,
            }
            .boxed(),
            container(Vec::new(), None),
        ],
        None,
    );
    let (before, after, _) = split_flow_at_descendant_break(element.as_ref(), 100.0).unwrap();

    let before = fragment_snapshot(before.as_deref().expect("open principal fragment"));
    let after = fragment_snapshot(after.as_deref().expect("continuation fragment"));
    assert_eq!(before.container_height, Some(100.0));
    assert!(before.container_has_background);
    assert!(after.container_has_background);
    let (first_offset, first_composite) = before
        .container_reference_slice
        .expect("first reference slice");
    let (continuation_offset, continuation_composite) = after
        .container_reference_slice
        .expect("continued reference slice");
    assert_eq!(first_offset, 0.0);
    assert_eq!(continuation_offset, 100.0);
    assert_eq!(first_composite, continuation_composite);
    assert!(first_composite > continuation_offset);
}

#[test]
fn table_row_break_slices_its_wrapper_decoration() {
    let border_side = LayoutBorderSide {
        width: 1.0,
        ..Default::default()
    };
    let decoration = TableBoxDecoration::new(TextBlock {
        box_model: BoxModel {
            size: LayoutSize::fixed(40.0, Some(50.0)),
            margins: BlockMargins::new(10.0, -52.0),
            border: crate::layout::engine::LayoutBorder::uniform(border_side),
            ..Default::default()
        },
        paint: BoxPaint {
            background: crate::layout::elements::BackgroundPaint {
                color: Some(Color::from_srgb(0.1, 0.2, 0.3, 1.0)),
                ..Default::default()
            },
            ..Default::default()
        },
        fragmentation: crate::layout::elements::TextFragmentation {
            box_fragmentation: BoxFragmentation {
                decoration: BoxDecorationBreak::Slice,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    })
    .boxed();
    let table = TableRow {
        content: TableCells {
            cells: vec![TableCell {
                layout: CellBox {
                    content: CellContent {
                        children: vec![
                            container(Vec::new(), Some(10.0)),
                            PageBreak {
                                side: PageBreakSide::Any,
                                page_name: None,
                            }
                            .boxed(),
                            container(Vec::new(), Some(20.0)),
                        ],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            }],
            column_widths: vec![40.0],
        },
        ..Default::default()
    }
    .boxed();

    let split = split_sequence(&[decoration, table], FragmentainerSpace::new(100.0))
        .expect("forced break inside the table cell");
    let first = split.before[0]
        .table_box_decoration_owner()
        .expect("open table decoration")
        .decoration();
    let continuation = split.after[0]
        .table_box_decoration_owner()
        .expect("continued table decoration")
        .decoration();

    assert_eq!(first.box_model.border.top.width, 1.0);
    assert_eq!(first.box_model.border.bottom.width, 0.0);
    assert_eq!(first.box_model.size.height.used(), Some(89.0));
    assert_eq!(first.box_model.margins.end, -90.0);
    assert_eq!(continuation.box_model.border.top.width, 0.0);
    assert_eq!(continuation.box_model.border.bottom.width, 1.0);
    assert_eq!(continuation.box_model.size.height.used(), Some(19.0));
    assert_eq!(continuation.box_model.margins.start, 0.0);
    assert_eq!(continuation.box_model.margins.end, -20.0);
}

/// A first table row starts with the table's top border, padding and
/// `border-spacing`. Its open fragment fills the rest of the fragmentainer,
/// so the cells must stop that much earlier, or the fragment overflows the
/// page and pagination splits it again onto an extra page.
#[test]
fn table_row_break_fragment_ends_at_the_fragmentainer_end_after_its_internal_start() {
    let table = TableRow {
        content: TableCells {
            cells: vec![TableCell {
                layout: CellBox {
                    content: CellContent {
                        children: vec![
                            container(Vec::new(), Some(10.0)),
                            PageBreak {
                                side: PageBreakSide::Any,
                                page_name: None,
                            }
                            .boxed(),
                            container(Vec::new(), Some(20.0)),
                        ],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            }],
            column_widths: vec![40.0],
        },
        flow: crate::layout::flow_metrics::BlockFlowSpacing::from_internal_start(9.0),
        ..Default::default()
    }
    .boxed();

    let split = split_sequence(&[table], FragmentainerSpace::new(100.0))
        .expect("forced break inside the table cell");

    assert_eq!(estimate_element_height(split.before[0].as_ref()), 100.0);
}

#[test]
fn fixed_container_descendant_break_becomes_overflow_fragment() {
    let nested = container(
        vec![
            container(Vec::new(), None),
            PageBreak {
                side: PageBreakSide::Any,
                page_name: None,
            }
            .boxed(),
            container(Vec::new(), None),
        ],
        Some(40.0),
    );
    let (before, after, _) = split_flow_at_descendant_break(&row(vec![nested]), 100.0).unwrap();
    let before = before.expect("parallel flow retains a leading fragment");
    let after = after.expect("parallel flow retains a continuation fragment");

    let before = fragment_snapshot(before.as_ref());
    assert_eq!(before.flex_cell_count, 1);
    assert_eq!(before.container_child_count, 1);

    let after = fragment_snapshot(after.as_ref());
    assert_eq!(after.flex_row_height, 0.0);
    assert!(!after.flex_has_background);
    assert_eq!(after.container_child_count, 1);
    assert_eq!(after.container_height, Some(0.0));
    assert!(!after.container_has_background);
}

#[test]
fn minimum_sized_flex_item_keeps_principal_box_fragments() {
    let nested = vec![
        container(Vec::new(), None),
        PageBreak {
            side: PageBreakSide::Any,
            page_name: None,
        }
        .boxed(),
        container(Vec::new(), None),
    ];
    let mut element = row(nested);
    struct MakeFragmentable;
    impl crate::layout::elements::LayoutVisitorMut for MakeFragmentable {
        fn visit_flex_row(&mut self, element: &mut FlexRow) {
            let cell = &mut element.content.cells[0];
            cell.fragmentation.block_size = FlexItemBlockSize::Minimum;
            cell.paint.background.color = Some(Color::from_srgb(0.2, 0.3, 0.4, 1.0));
            cell.border = crate::types::PhysicalEdges::uniform(LayoutBorderSide {
                width: 1.0,
                ..Default::default()
            });
        }
    }
    element.accept_mut(&mut MakeFragmentable);

    let (before, after, _) = split_flow_at_descendant_break(element.as_ref(), 100.0).unwrap();
    let before = before.expect("parallel flow retains a leading fragment");
    let after = after.expect("parallel flow retains a continuation fragment");

    let before = fragment_snapshot(before.as_ref());
    assert!(before.flex_cell_has_background);
    assert_eq!(before.flex_cell_fragment_extent, Some(100.0));
    assert_eq!(before.flex_cell_border_top, 1.0);
    assert_eq!(before.flex_cell_border_bottom, 0.0);

    let after = fragment_snapshot(after.as_ref());
    assert!(after.flex_cell_has_background);
    assert_eq!(after.flex_cell_fragment_extent, None);
    assert_eq!(after.flex_cell_border_top, 0.0);
    assert_eq!(after.flex_cell_border_bottom, 1.0);
}
