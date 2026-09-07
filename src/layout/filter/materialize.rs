//! Post-fragmentation materialization of retained CSS filters.
//!
//! Traversal is always performed through `LayoutElement::visit_child_nodes_mut`.
//! Formatting contexts refine child page geometry, while one inherited paint
//! space carries graphical transforms through every nesting level.

use crate::layout::elements::{FlexRow, GridRow, LayoutElement, LayoutNode, LayoutVisitorMut};
use crate::types::{EdgeSizes, Point};

use super::paint_space::{InheritedFilterPaintSpace, PageBoxAnchor};

mod child_frames;
#[cfg(test)]
mod tests;
mod traversal;

use child_frames::ChildPaintFrames;
use traversal::TraversalFrame;

/// Materialize every retained filter after pagination, deepest descendants
/// first. A single generic traversal applies to every concrete layout node.
pub(crate) fn materialize_page_filters(
    pages: &mut [crate::layout::engine::Page],
    document_margin: crate::types::Margin,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
) {
    for page in pages {
        let margin = page
            .geometry
            .map(crate::layout::page_context::PageGeometry::flow_margin)
            .unwrap_or(document_margin);
        for (y, element) in &mut page.elements {
            let anchor =
                page_border_box_anchor(element.as_ref(), Point::new(margin.left, margin.top + *y));
            materialize_node_filter(
                element,
                TraversalFrame {
                    anchor,
                    inherited_space: Default::default(),
                },
                fonts,
                filter_dpi,
            );
        }
        for element in page.generated_content.running_elements_mut() {
            let anchor =
                page_border_box_anchor(element.as_ref(), Point::new(margin.left, margin.top));
            materialize_node_filter(
                element,
                TraversalFrame {
                    anchor,
                    inherited_space: Default::default(),
                },
                fonts,
                filter_dpi,
            );
        }
    }
}

fn materialize_node_filter(
    element: &mut LayoutNode,
    frame: TraversalFrame,
    fonts: &dyn crate::font_registry::FontRegistry,
    filter_dpi: f32,
) {
    let element_space = frame.enter(element.as_ref());
    let fallback = TraversalFrame {
        anchor: frame.anchor,
        inherited_space: element_space.descendant_space,
    };
    let mut child_frames =
        ChildPaintFrames::resolve(element.as_ref(), element_space, fonts).into_iter(fallback);
    element.visit_child_nodes_mut(&mut |child| {
        materialize_node_filter(child, child_frames.next(), fonts, filter_dpi);
    });

    struct CellFilterMaterializer<'a> {
        anchor: PageBoxAnchor,
        inherited_space: InheritedFilterPaintSpace,
        fonts: &'a dyn crate::font_registry::FontRegistry,
        filter_dpi: f32,
    }

    impl LayoutVisitorMut for CellFilterMaterializer<'_> {
        fn visit_flex_row(&mut self, element: &mut FlexRow) {
            super::cells::materialize_flex_row(
                element,
                self.anchor,
                self.inherited_space,
                self.fonts,
                self.filter_dpi,
            );
        }

        fn visit_grid_row(&mut self, element: &mut GridRow) {
            super::cells::materialize_grid_row(
                element,
                self.anchor,
                self.inherited_space,
                self.fonts,
                self.filter_dpi,
            );
        }
    }

    element.accept_mut(&mut CellFilterMaterializer {
        anchor: frame.anchor,
        inherited_space: element_space.descendant_space,
        fonts,
        filter_dpi,
    });

    let Some(filter) = element
        .filter_holder_mut()
        .and_then(crate::layout::elements::FilterHolder::take_filter)
    else {
        return;
    };
    let Some(box_space) = element_space.box_space else {
        filter.apply_primitive_fallback(std::slice::from_mut(element));
        return;
    };
    let raster_space = box_space.source_raster_space(filter.matrix_capability());
    if let Some(graphic) =
        super::composite_source(element.as_ref(), &filter, fonts, filter_dpi, raster_space)
    {
        *element = graphic.into_layout_node();
    } else {
        filter.apply_primitive_fallback(std::slice::from_mut(element));
    }
}

fn page_border_box_anchor(element: &dyn LayoutElement, flow_origin: Point) -> PageBoxAnchor {
    let insets = super::surface::source_geometry(element)
        .map(|geometry| geometry.positioning.insets)
        .or_else(|| {
            element
                .positioning_owner()
                .map(|owner| owner.positioning().insets)
        })
        .unwrap_or(EdgeSizes::ZERO);
    PageBoxAnchor::at(Point::new(
        flow_origin.x + insets.left,
        flow_origin.y + insets.top,
    ))
}
