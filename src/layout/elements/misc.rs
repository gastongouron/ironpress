use super::{
    BlockFlowParticipant, FragmentPlacement, FragmentPlacementOwner, InlineFlowExtent,
    LayoutElement, LayoutNode, LayoutVisitor, LayoutVisitorMut, PaintGroup, PaintGroupOwner,
};
use crate::layout::engine::LayoutBorderSide;
use crate::layout::flow_metrics::{BlockMargins, MarginHolder};
use crate::types::{Color, Size};

#[derive(Debug, Clone, Default)]
pub(crate) struct ColumnRule {
    /// Document-order column immediately before this rule.
    pub(crate) gap_after: usize,
    pub(crate) placement: FragmentPlacement,
    pub(crate) height: f32,
    pub(crate) paint: LayoutBorderSide,
}

impl FragmentPlacementOwner for ColumnRule {
    fn fragment_placement(&self) -> FragmentPlacement {
        self.placement
    }

    fn fragment_source(&self) -> &dyn LayoutElement {
        self
    }
}

impl LayoutElement for ColumnRule {
    fn clone_box(&self) -> LayoutNode {
        Box::new(self.clone())
    }

    fn accept(&self, visitor: &mut dyn LayoutVisitor) {
        visitor.visit_column_rule(self);
    }

    fn accept_mut(&mut self, visitor: &mut dyn LayoutVisitorMut) {
        visitor.visit_column_rule(self);
    }

    fn fragment_placement_owner(&self) -> Option<&dyn FragmentPlacementOwner> {
        Some(self)
    }

    fn contributes_to_normal_flow(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HorizontalRule {
    pub(crate) margins: BlockMargins,
    pub(crate) group: PaintGroup,
}

impl MarginHolder for HorizontalRule {
    fn margins(&self) -> &BlockMargins {
        &self.margins
    }

    fn margins_mut(&mut self) -> &mut BlockMargins {
        &mut self.margins
    }
}

impl PaintGroupOwner for HorizontalRule {
    fn paint_group(&self) -> &PaintGroup {
        &self.group
    }

    fn paint_group_mut(&mut self) -> &mut PaintGroup {
        &mut self.group
    }
}

impl LayoutElement for HorizontalRule {
    fn clone_box(&self) -> LayoutNode {
        Box::new(self.clone())
    }

    fn accept(&self, visitor: &mut dyn LayoutVisitor) {
        visitor.visit_horizontal_rule(self);
    }

    fn accept_mut(&mut self, visitor: &mut dyn LayoutVisitorMut) {
        visitor.visit_horizontal_rule(self);
    }

    fn margin_holder(&self) -> Option<&dyn MarginHolder> {
        Some(self)
    }

    fn margin_holder_mut(&mut self) -> Option<&mut dyn MarginHolder> {
        Some(self)
    }

    fn block_flow_participant(&self) -> Option<&dyn BlockFlowParticipant> {
        Some(self)
    }

    fn block_flow_participant_mut(&mut self) -> Option<&mut dyn BlockFlowParticipant> {
        Some(self)
    }

    fn paint_group_owner(&self) -> Option<&dyn PaintGroupOwner> {
        Some(self)
    }

    fn paint_group_owner_mut(&mut self) -> Option<&mut dyn PaintGroupOwner> {
        Some(self)
    }
}

impl BlockFlowParticipant for HorizontalRule {
    fn collapses_outer_margins(&self) -> bool {
        true
    }

    fn is_in_flow_block(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProgressColors {
    pub(crate) fill: Color,
    pub(crate) track: Color,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProgressBar {
    pub(crate) fraction: f32,
    pub(crate) size: Size,
    pub(crate) colors: ProgressColors,
    pub(crate) margins: BlockMargins,
    pub(crate) group: PaintGroup,
}

impl MarginHolder for ProgressBar {
    fn margins(&self) -> &BlockMargins {
        &self.margins
    }

    fn margins_mut(&mut self) -> &mut BlockMargins {
        &mut self.margins
    }
}

impl InlineFlowExtent for ProgressBar {
    fn normal_flow_right_edge(&self) -> Option<f32> {
        self.size
            .width
            .is_finite()
            .then_some(self.size.width.max(0.0))
    }
}

impl PaintGroupOwner for ProgressBar {
    fn paint_group(&self) -> &PaintGroup {
        &self.group
    }

    fn paint_group_mut(&mut self) -> &mut PaintGroup {
        &mut self.group
    }
}

impl LayoutElement for ProgressBar {
    fn clone_box(&self) -> LayoutNode {
        Box::new(self.clone())
    }

    fn accept(&self, visitor: &mut dyn LayoutVisitor) {
        visitor.visit_progress_bar(self);
    }

    fn accept_mut(&mut self, visitor: &mut dyn LayoutVisitorMut) {
        visitor.visit_progress_bar(self);
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

    fn paint_group_owner(&self) -> Option<&dyn PaintGroupOwner> {
        Some(self)
    }

    fn paint_group_owner_mut(&mut self) -> Option<&mut dyn PaintGroupOwner> {
        Some(self)
    }
}

impl BlockFlowParticipant for ProgressBar {
    fn collapses_outer_margins(&self) -> bool {
        true
    }

    fn is_in_flow_block(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MathBlock {
    pub(crate) layout: crate::layout::math::MathLayout,
    pub(crate) display: bool,
    pub(crate) margins: BlockMargins,
    pub(crate) group: PaintGroup,
}

impl MarginHolder for MathBlock {
    fn margins(&self) -> &BlockMargins {
        &self.margins
    }

    fn margins_mut(&mut self) -> &mut BlockMargins {
        &mut self.margins
    }
}

impl BlockFlowParticipant for MathBlock {
    fn collapses_outer_margins(&self) -> bool {
        true
    }

    fn is_in_flow_block(&self) -> bool {
        true
    }
}

impl InlineFlowExtent for MathBlock {
    fn normal_flow_right_edge(&self) -> Option<f32> {
        self.layout
            .width
            .is_finite()
            .then_some(self.layout.width.max(0.0))
    }
}

impl PaintGroupOwner for MathBlock {
    fn paint_group(&self) -> &PaintGroup {
        &self.group
    }

    fn paint_group_mut(&mut self) -> &mut PaintGroup {
        &mut self.group
    }
}

impl LayoutElement for MathBlock {
    fn clone_box(&self) -> LayoutNode {
        Box::new(self.clone())
    }

    fn accept(&self, visitor: &mut dyn LayoutVisitor) {
        visitor.visit_math_block(self);
    }

    fn accept_mut(&mut self, visitor: &mut dyn LayoutVisitorMut) {
        visitor.visit_math_block(self);
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

    fn paint_group_owner(&self) -> Option<&dyn PaintGroupOwner> {
        Some(self)
    }

    fn paint_group_owner_mut(&mut self) -> Option<&mut dyn PaintGroupOwner> {
        Some(self)
    }
}
