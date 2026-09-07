//! Formatting-context refinement of generic traversal frames.

use crate::layout::cells::CellPaintHolder;
use crate::layout::elements::{
    Container, FlexRow, GridRow, LayoutElement, LayoutNode, LayoutVisitor, TableRow,
};
use crate::layout::filter::paint_space::{InheritedFilterPaintSpace, PageBoxAnchor};
use crate::types::{EdgeSizes, Rect};

use super::traversal::{ElementTraversalSpace, TraversalFrame};

/// Traversal frames for every direct node exposed by one layout element, in
/// the same order as `visit_child_nodes_mut`.
///
/// An absent plan means only that this formatting context cannot refine child
/// geometry. It never changes whether those children are visited.
pub(super) struct ChildPaintFrames(Option<Vec<TraversalFrame>>);

impl ChildPaintFrames {
    pub(super) fn resolve(
        element: &dyn LayoutElement,
        space: ElementTraversalSpace,
        fonts: &dyn crate::font_registry::FontRegistry,
    ) -> Self {
        let mut resolver = Resolver {
            parent_anchor: space.frame.anchor,
            descendant_space: space.descendant_space,
            fonts,
            frames: None,
        };
        element.accept(&mut resolver);
        Self(resolver.frames)
    }

    pub(super) fn into_iter(self, fallback: TraversalFrame) -> ChildPaintFrameIter {
        ChildPaintFrameIter {
            resolved: self.0.unwrap_or_default().into_iter(),
            fallback,
        }
    }
}

pub(super) struct ChildPaintFrameIter {
    resolved: std::vec::IntoIter<TraversalFrame>,
    fallback: TraversalFrame,
}

impl ChildPaintFrameIter {
    pub(super) fn next(&mut self) -> TraversalFrame {
        self.resolved.next().unwrap_or(self.fallback)
    }
}

struct Resolver<'a> {
    parent_anchor: PageBoxAnchor,
    descendant_space: InheritedFilterPaintSpace,
    fonts: &'a dyn crate::font_registry::FontRegistry,
    frames: Option<Vec<TraversalFrame>>,
}

impl Resolver<'_> {
    fn block_frames(
        &self,
        children: &[LayoutNode],
        border_box: Rect,
        border: EdgeSizes,
        padding: EdgeSizes,
        inherited_space: InheritedFilterPaintSpace,
    ) -> Option<Vec<TraversalFrame>> {
        let padding_box = border_box.inset(border);
        let content_box = padding_box.inset(padding);
        crate::layout::filter::surface::block_child_frames(
            children,
            crate::layout::filter::surface::BlockChildSpace::new(
                content_box,
                padding_box,
                Some(padding_box),
            ),
        )
        .map(|frames| {
            frames
                .into_iter()
                .map(|frame| TraversalFrame {
                    anchor: PageBoxAnchor::at(frame.border_box.origin),
                    inherited_space,
                })
                .collect()
        })
    }

    fn cell_descendant_space(
        &self,
        anchor: PageBoxAnchor,
        size: crate::types::Size,
        group: &crate::layout::elements::PaintGroup,
        reference_box: &dyn crate::layout::elements::BoxReferenceGeometry,
    ) -> InheritedFilterPaintSpace {
        self.descendant_space
            .enter(anchor, size, Some(group), Some(reference_box))
            .descendants()
    }

    fn fallback_cell_children<'a>(
        &self,
        children: impl IntoIterator<Item = &'a LayoutNode>,
        anchor: PageBoxAnchor,
        inherited_space: InheritedFilterPaintSpace,
    ) -> impl Iterator<Item = TraversalFrame> {
        children.into_iter().map(move |_| TraversalFrame {
            anchor,
            inherited_space,
        })
    }
}

impl LayoutVisitor for Resolver<'_> {
    fn visit_container(&mut self, element: &Container) {
        let Some(geometry) = crate::layout::filter::surface::source_geometry(element) else {
            return;
        };
        let border_box = Rect::new(self.parent_anchor.border_origin(), geometry.size);
        self.frames = self.block_frames(
            &element.children,
            border_box,
            element.box_model.border.widths(),
            element.box_model.padding,
            self.descendant_space,
        );
    }

    fn visit_flex_row(&mut self, element: &FlexRow) {
        let cell_frames =
            crate::layout::filter::surface::flex_cell_source_frames(element, self.fonts);
        let mut frames = Vec::new();
        for (cell, cell_frame) in element.content.cells.iter().zip(cell_frames) {
            let cell_anchor = cell_frame.page_anchor_in(self.parent_anchor);
            let cell_space = self.cell_descendant_space(
                cell_anchor,
                cell_frame.size,
                &cell.cell_paint().group,
                cell,
            );
            let cell_box = Rect::new(cell_anchor.border_origin(), cell_frame.size);
            let nested = self.block_frames(
                &cell.nested_elements,
                cell_box,
                cell.border.widths(),
                cell.padding,
                cell_space,
            );
            match nested {
                Some(nested) => frames.extend(nested),
                None => frames.extend(self.fallback_cell_children(
                    &cell.nested_elements,
                    cell_anchor,
                    cell_space,
                )),
            }
        }
        self.frames = Some(frames);
    }

    fn visit_grid_row(&mut self, element: &GridRow) {
        let cell_frames = crate::layout::filter::surface::grid_cell_source_frames(element);
        let mut frames = Vec::new();
        for (cell, cell_frame) in element.content.cells.iter().zip(cell_frames) {
            let cell_anchor = cell_frame.page_anchor_in(self.parent_anchor);
            let cell_space = self.cell_descendant_space(
                cell_anchor,
                cell_frame.size,
                &cell.cell_paint().group,
                &cell.layout.box_model,
            );
            let cell_box = Rect::new(cell_anchor.border_origin(), cell_frame.size);
            let nested = self.block_frames(
                &cell.layout.content.children,
                cell_box,
                cell.layout.box_model.border.widths(),
                cell.layout.box_model.padding(),
                cell_space,
            );
            match nested {
                Some(nested) => frames.extend(nested),
                None => frames.extend(self.fallback_cell_children(
                    &cell.layout.content.children,
                    cell_anchor,
                    cell_space,
                )),
            }
        }
        self.frames = Some(frames);
    }

    fn visit_table_row(&mut self, element: &TableRow) {
        let cell_frames = crate::layout::filter::surface::table_cell_source_frames(element);
        let baseline_shifts = crate::layout::filter::surface::table_row_baseline_shifts(
            &element.content.cells,
            self.fonts,
        );
        let mut frames = Vec::new();
        for (index, cell) in element.content.cells.iter().enumerate() {
            let Some(cell_frame) = cell_frames.get(index).copied().flatten() else {
                frames.extend(self.fallback_cell_children(
                    &cell.layout.content.children,
                    self.parent_anchor,
                    self.descendant_space,
                ));
                continue;
            };
            let cell_anchor = cell_frame.page_anchor_in(self.parent_anchor);
            let cell_space = self.cell_descendant_space(
                cell_anchor,
                cell_frame.size,
                &cell.cell_paint().group,
                &cell.layout.box_model,
            );
            let nested = crate::layout::filter::surface::block_child_frames(
                &cell.layout.content.children,
                cell_frame.nested_child_space(
                    self.parent_anchor.border_origin(),
                    &cell.layout,
                    baseline_shifts.get(index).copied().unwrap_or_default(),
                ),
            );
            if let Some(nested) = nested {
                frames.extend(nested.into_iter().map(|frame| TraversalFrame {
                    anchor: PageBoxAnchor::at(frame.border_box.origin),
                    inherited_space: cell_space,
                }));
            } else {
                frames.extend(self.fallback_cell_children(
                    &cell.layout.content.children,
                    cell_anchor,
                    cell_space,
                ));
            }
        }
        self.frames = Some(frames);
    }
}
