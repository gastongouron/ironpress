use super::LayoutNode;
use super::{
    BlockFlowParticipant, BoxPaint, BoxPaintOwner, ChildContainer, InlineFlowExtent, LayoutElement,
    LayoutVisitor, LayoutVisitorMut, OverflowBehavior, PageContentRole, PaintGroupOwner,
    Positioning, PositioningOwner,
};
use crate::layout::engine::{FlexCell, FlexFragmentRole, ForcedFlexLineBreak};
use crate::layout::flow_metrics::{BlockMargins, MarginHolder};
use crate::style::computed::AlignItems;

/// Flex-specific content and line-fragmentation state.
#[derive(Debug, Clone, Default)]
pub(crate) struct FlexContent {
    pub(crate) cells: Vec<FlexCell>,
    pub(crate) forced_line_breaks: Vec<ForcedFlexLineBreak>,
    pub(crate) fragment_role: FlexFragmentRole,
    pub(crate) row_height: f32,
    pub(crate) alignment: AlignItems,
}

impl FlexContent {
    /// Inline extent occupied by the source-ordered cells in this line.
    ///
    /// Cell offsets already include collapsed whitespace and tracking inserted
    /// by the inline cursor, so the furthest cell edge is the intrinsic line
    /// contribution independently from its later fill-available box width.
    pub(crate) fn intrinsic_inline_extent(&self) -> f32 {
        self.cells
            .iter()
            .map(|cell| cell.x_offset + cell.width)
            .filter(|extent| extent.is_finite())
            .fold(0.0_f32, f32::max)
    }
}

/// One laid-out flex line. Shared box properties use the same semantic groups
/// as text and container nodes.
#[derive(Debug, Clone, Default)]
pub(crate) struct FlexRow {
    pub(crate) content: FlexContent,
    pub(crate) box_model: super::BoxModel,
    pub(crate) paint: super::BoxPaint,
    pub(crate) positioning: super::Positioning,
    pub(crate) inline_offset: super::InlineOffset,
    pub(crate) overflow: OverflowBehavior,
}

impl MarginHolder for FlexRow {
    fn margins(&self) -> &BlockMargins {
        &self.box_model.margins
    }

    fn margins_mut(&mut self) -> &mut BlockMargins {
        &mut self.box_model.margins
    }
}

impl InlineFlowExtent for FlexRow {
    fn normal_flow_right_edge(&self) -> Option<f32> {
        let right = self.inline_offset.value() + self.box_model.size.width.fixed_value()?;
        right.is_finite().then_some(right.max(0.0))
    }
}

impl BlockFlowParticipant for FlexRow {
    fn collapses_outer_margins(&self) -> bool {
        true
    }

    fn is_in_flow_block(&self) -> bool {
        true
    }
}

impl PositioningOwner for FlexRow {
    fn positioning(&self) -> &Positioning {
        &self.positioning
    }

    fn positioning_mut(&mut self) -> &mut Positioning {
        &mut self.positioning
    }
}

impl BoxPaintOwner for FlexRow {
    fn box_paint(&self) -> &BoxPaint {
        &self.paint
    }

    fn box_paint_mut(&mut self) -> &mut BoxPaint {
        &mut self.paint
    }
}

impl ChildContainer for FlexRow {
    fn visit_layout_children(&self, visitor: &mut dyn FnMut(&dyn LayoutElement)) {
        for child in self
            .content
            .cells
            .iter()
            .flat_map(|cell| &cell.nested_elements)
        {
            visitor(child.as_ref());
        }
    }

    fn visit_layout_children_mut(&mut self, visitor: &mut dyn FnMut(&mut dyn LayoutElement)) {
        for child in self
            .content
            .cells
            .iter_mut()
            .flat_map(|cell| &mut cell.nested_elements)
        {
            visitor(child.as_mut());
        }
    }

    fn visit_layout_child_nodes_mut(&mut self, visitor: &mut dyn FnMut(&mut LayoutNode)) {
        for child in self
            .content
            .cells
            .iter_mut()
            .flat_map(|cell| &mut cell.nested_elements)
        {
            visitor(child);
        }
    }
}

impl LayoutElement for FlexRow {
    fn clone_box(&self) -> LayoutNode {
        Box::new(self.clone())
    }

    fn accept(&self, visitor: &mut dyn LayoutVisitor) {
        visitor.visit_flex_row(self);
    }

    fn accept_mut(&mut self, visitor: &mut dyn LayoutVisitorMut) {
        visitor.visit_flex_row(self);
    }

    fn margin_holder(&self) -> Option<&dyn MarginHolder> {
        Some(self)
    }

    fn margin_holder_mut(&mut self) -> Option<&mut dyn MarginHolder> {
        Some(self)
    }

    fn inline_flow_extent(&self) -> Option<&dyn InlineFlowExtent> {
        Some(self)
    }

    fn block_flow_participant(&self) -> Option<&dyn BlockFlowParticipant> {
        Some(self)
    }

    fn block_flow_participant_mut(&mut self) -> Option<&mut dyn BlockFlowParticipant> {
        Some(self)
    }
    fn visit_children(&self, visitor: &mut dyn FnMut(&dyn LayoutElement)) {
        self.visit_layout_children(visitor);
    }

    fn visit_children_mut(&mut self, visitor: &mut dyn FnMut(&mut dyn LayoutElement)) {
        self.visit_layout_children_mut(visitor);
    }

    fn visit_child_nodes_mut(&mut self, visitor: &mut dyn FnMut(&mut LayoutNode)) {
        self.visit_layout_child_nodes_mut(visitor);
    }

    fn positioning_owner(&self) -> Option<&dyn PositioningOwner> {
        Some(self)
    }

    fn positioning_owner_mut(&mut self) -> Option<&mut dyn PositioningOwner> {
        Some(self)
    }

    fn paint_group_owner(&self) -> Option<&dyn PaintGroupOwner> {
        Some(self)
    }

    fn paint_group_owner_mut(&mut self) -> Option<&mut dyn PaintGroupOwner> {
        Some(self)
    }

    fn box_reference_geometry(&self) -> Option<&dyn super::BoxReferenceGeometry> {
        Some(&self.box_model)
    }

    fn box_paint_owner(&self) -> Option<&dyn BoxPaintOwner> {
        Some(self)
    }

    fn box_paint_owner_mut(&mut self) -> Option<&mut dyn BoxPaintOwner> {
        Some(self)
    }

    fn in_flow_paint_phase_owner(&self) -> Option<&dyn BoxPaintOwner> {
        Some(self)
    }

    fn has_own_page_spanning_graphical_effect(&self) -> bool {
        self.paint.has_outset_graphical_effect()
            || self.content.cells.iter().any(|cell| {
                cell.paint.has_outset_graphical_effect()
                    || super::text_lines_have_outset_shadows(&cell.lines)
            })
    }

    fn page_content_role(&self) -> PageContentRole {
        match self.content.fragment_role {
            FlexFragmentRole::Normal => PageContentRole::MainFlow,
            FlexFragmentRole::ParallelOverflowContinuation => PageContentRole::OverflowContinuation,
        }
    }
}
