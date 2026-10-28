use super::{
    BlockFlow, BlockFlowOwner, BlockFlowParticipant, BlockFragmentationSource, BoxFragmentation,
    BoxFragmentationOwner, BoxModel, BoxPaint, BoxPaintOwner, ChildContainer,
    ContainingBlockConsumer, FragmentBreakQuery, InlineFlowExtent, LayoutElement, LayoutNode,
    LayoutVisitor, LayoutVisitorMut, OverflowBehavior, PageContentRole, PaintGroupOwner,
    Positioning, PositioningOwner,
};
use crate::layout::flow_metrics::{BlockMargins, MarginHolder};

/// A structural CSS box that owns nested layout nodes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Container {
    pub(crate) children: Vec<LayoutNode>,
    pub(crate) box_model: BoxModel,
    pub(crate) paint: BoxPaint,
    pub(crate) flow: BlockFlow,
    pub(crate) positioning: Positioning,
    pub(crate) fragmentation: BoxFragmentation,
    pub(crate) overflow: OverflowBehavior,
}

impl Container {
    /// Build the structural box properties that map directly from one computed
    /// style. Callers provide only the already-resolved box geometry.
    pub(crate) fn from_style(
        children: Vec<LayoutNode>,
        style: &crate::style::computed::ComputedStyle,
        box_model: BoxModel,
    ) -> Self {
        Self {
            children,
            paint: BoxPaint::from_style(style, box_model.size),
            flow: BlockFlow {
                float: style.float,
                clear: style.clear,
            },
            positioning: Positioning::from_style(style),
            fragmentation: BoxFragmentation::from_style(style),
            overflow: OverflowBehavior {
                combined: style.overflow,
                x: style.overflow_x,
                y: style.overflow_y,
            },
            box_model,
        }
    }
}

impl Container {
    /// A copy of this box holding `children` in place of its own. Fragmenting a
    /// box replaces its children, so copying them first would be wasted work.
    pub(crate) fn with_children(&self, children: Vec<LayoutNode>) -> Self {
        Self {
            children,
            box_model: self.box_model,
            paint: self.paint.clone(),
            flow: self.flow,
            positioning: self.positioning.clone(),
            fragmentation: self.fragmentation,
            overflow: self.overflow,
        }
    }
}

impl MarginHolder for Container {
    fn margins(&self) -> &BlockMargins {
        &self.box_model.margins
    }

    fn margins_mut(&mut self) -> &mut BlockMargins {
        &mut self.box_model.margins
    }
}

impl InlineFlowExtent for Container {
    fn normal_flow_right_edge(&self) -> Option<f32> {
        let width = self.box_model.size.width.fixed_value()?;
        self.positioning
            .is_in_normal_flow()
            .then_some((self.positioning.insets.left + width).max(0.0))
            .filter(|right| right.is_finite())
    }

    fn max_content_outer_extent(&self) -> Option<f32> {
        let children = self
            .children
            .iter()
            .filter_map(|child| child.inline_flow_extent()?.max_content_outer_extent())
            .fold(0.0f32, f32::max);
        if children > 0.0 {
            Some(
                children
                    + self.box_model.padding.horizontal()
                    + self.box_model.border.horizontal_width(),
            )
        } else {
            self.normal_flow_right_edge()
        }
    }
}

impl BlockFlowParticipant for Container {
    fn collapses_outer_margins(&self) -> bool {
        true
    }

    fn is_in_flow_block(&self) -> bool {
        !self.positioning.scheme.is_absolute()
            && self.flow.float == crate::style::computed::Float::None
    }
}

impl ContainingBlockConsumer for Container {
    fn resolve_containing_block(
        &mut self,
        containing_block: crate::layout::engine::ContainingBlock,
    ) {
        let width = self
            .box_model
            .size
            .width
            .fixed_value()
            .unwrap_or(containing_block.width);
        self.positioning.resolve_final_containing_block(
            containing_block,
            crate::types::Size::new(width, self.block_extent()),
        );
    }
}

impl PositioningOwner for Container {
    fn positioning(&self) -> &Positioning {
        &self.positioning
    }

    fn positioning_mut(&mut self) -> &mut Positioning {
        &mut self.positioning
    }
}

impl BlockFlowOwner for Container {
    fn block_flow(&self) -> &BlockFlow {
        &self.flow
    }
}

impl BoxPaintOwner for Container {
    fn box_paint(&self) -> &BoxPaint {
        &self.paint
    }

    fn box_paint_mut(&mut self) -> &mut BoxPaint {
        &mut self.paint
    }
}

fn child_fragmentation_outer_extent(child: &dyn LayoutElement) -> f32 {
    child
        .fragmentable_outer_block_extent()
        .unwrap_or_else(|| crate::layout::paginate::estimate_element_height(child))
}

impl BlockFragmentationSource for Container {
    fn block_extent(&self) -> f32 {
        let descendants = self
            .children
            .iter()
            .filter(|child| child.contributes_to_normal_flow())
            .map(|child| child_fragmentation_outer_extent(child.as_ref()))
            .sum::<f32>();
        let natural = self.box_model.border.vertical_width()
            + self.box_model.padding.vertical()
            + descendants;
        self.box_model.size.height.resolve(natural)
    }

    fn find_block_break(&self, query: FragmentBreakQuery) -> Option<f32> {
        let content_start = self.box_model.border.top.width + self.box_model.padding.top;
        let mut cursor = content_start;
        let mut selected = None;
        let in_flow = self
            .children
            .iter()
            .filter(|child| child.contributes_to_normal_flow())
            .collect::<Vec<_>>();
        for (index, child) in in_flow.iter().enumerate() {
            let child_extent = child_fragmentation_outer_extent(child.as_ref());
            let child_start = cursor;
            let child_end = child_start + child_extent;
            if let Some(source) = child.block_fragmentation_source() {
                let child_query = query.translated(child_start);
                if let Some(child_break) = source.find_block_break(child_query) {
                    selected = query.select(selected, child_start + child_break);
                }
            }
            if index + 1 < in_flow.len() {
                selected = query.select(selected, child_end);
            }
            cursor = child_end;
        }
        selected
    }
}

impl BoxFragmentationOwner for Container {
    fn fragmentation_box_model(&self) -> &BoxModel {
        &self.box_model
    }

    fn box_fragmentation(&self) -> &BoxFragmentation {
        &self.fragmentation
    }

    fn box_fragmentation_mut(&mut self) -> &mut BoxFragmentation {
        &mut self.fragmentation
    }
}

impl ChildContainer for Container {
    fn visit_layout_children(&self, visitor: &mut dyn FnMut(&dyn LayoutElement)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn visit_layout_children_mut(&mut self, visitor: &mut dyn FnMut(&mut dyn LayoutElement)) {
        for child in &mut self.children {
            visitor(child.as_mut());
        }
    }

    fn visit_layout_child_nodes_mut(&mut self, visitor: &mut dyn FnMut(&mut LayoutNode)) {
        for child in &mut self.children {
            visitor(child);
        }
    }
}

impl LayoutElement for Container {
    fn clone_box(&self) -> LayoutNode {
        Box::new(self.clone())
    }

    fn accept(&self, visitor: &mut dyn LayoutVisitor) {
        visitor.visit_container(self);
    }

    fn accept_mut(&mut self, visitor: &mut dyn LayoutVisitorMut) {
        visitor.visit_container(self);
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

    fn page_content_role(&self) -> PageContentRole {
        self.fragmentation
            .content_role
            .for_position(self.positioning.scheme)
    }

    fn containing_block_consumer_mut(&mut self) -> Option<&mut dyn ContainingBlockConsumer> {
        Some(self)
    }

    fn positioning_owner(&self) -> Option<&dyn PositioningOwner> {
        Some(self)
    }

    fn positioning_owner_mut(&mut self) -> Option<&mut dyn PositioningOwner> {
        Some(self)
    }

    fn block_flow_owner(&self) -> Option<&dyn BlockFlowOwner> {
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

    fn block_fragmentation_source(&self) -> Option<&dyn BlockFragmentationSource> {
        Some(self)
    }

    fn box_fragmentation_owner(&self) -> Option<&dyn BoxFragmentationOwner> {
        Some(self)
    }

    fn box_fragmentation_owner_mut(&mut self) -> Option<&mut dyn BoxFragmentationOwner> {
        Some(self)
    }

    fn exact_vector_filter_source(
        &self,
    ) -> Option<&dyn crate::layout::filter::ExactVectorFilterSource> {
        Some(self)
    }
}
