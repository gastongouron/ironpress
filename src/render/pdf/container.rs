use super::*;
use crate::layout::elements::{
    ColumnRule, Container, FlexRow, FragmentPlacementOwner, GridRow, HorizontalRule, Image,
    LayoutNode, LayoutVisitor, MathBlock, ProgressBar, Svg, TableRow, TextBlock,
};

mod flex;
mod media;
mod nested;
mod text;

use flex::render_flex_child;
use media::{render_image_child, render_svg_child};
use nested::render_nested_container;
use text::render_text_child;

#[derive(Clone, Copy)]
pub(super) struct ContainerFrame {
    content_origin: PdfPoint,
    size: crate::types::Size,
    padding_origin: PdfPoint,
}

impl ContainerFrame {
    pub(super) const fn new(
        content_origin: PdfPoint,
        size: crate::types::Size,
        padding_origin: PdfPoint,
    ) -> Self {
        Self {
            content_origin,
            size,
            padding_origin,
        }
    }

    const fn width(self) -> f32 {
        self.size.width
    }
}

#[derive(Clone, Copy)]
pub(super) struct ContainerRenderOptions {
    pub(super) device_space_available: bool,
    pub(super) paint_phase: ElementPaintPhase,
    pub(super) stacking_scope: StackingScope,
}

impl Default for ContainerRenderOptions {
    fn default() -> Self {
        Self {
            device_space_available: false,
            paint_phase: ElementPaintPhase::All,
            stacking_scope: StackingScope::Local,
        }
    }
}

#[derive(Clone, Copy)]
struct ContainerFlowContext<'a> {
    frame: ContainerFrame,
    container_top_y: f32,
    flow_top_by_index: &'a HashMap<usize, f32>,
    float_top_by_index: &'a HashMap<usize, f32>,
    left_float_bottom: f32,
    right_float_bottom: f32,
    device_space_available: bool,
    paint_phase: ElementPaintPhase,
}

impl ContainerFlowContext<'_> {
    fn with_paint_phase(mut self, paint_phase: ElementPaintPhase) -> Self {
        self.paint_phase = paint_phase;
        self
    }
}

fn is_nested_row(element: &dyn LayoutElement) -> bool {
    #[derive(Default)]
    struct NestedRow(bool);

    impl LayoutVisitor for NestedRow {
        fn visit_table_row(&mut self, _element: &TableRow) {
            self.0 = true;
        }

        fn visit_grid_row(&mut self, _element: &GridRow) {
            self.0 = true;
        }
    }

    let mut nested = NestedRow::default();
    element.accept(&mut nested);
    nested.0
}

struct DirectChildRenderer<'call, 'flow, 'page> {
    content: &'call mut String,
    child_index: usize,
    flow: &'call ContainerFlowContext<'flow>,
    position: FlowPosition,
    abs_origins: &'call mut HashMap<usize, PdfPoint>,
    ctx: &'call mut PageRenderContext<'page>,
    handled: bool,
    result: Option<FlowPosition>,
}

impl DirectChildRenderer<'_, '_, '_> {
    fn finish(&mut self, position: FlowPosition) {
        self.handled = true;
        self.result = Some(position);
    }

    fn render(&mut self, element: &dyn LayoutElement) {
        if let Some(placed) = element.fragment_placement_owner() {
            self.render_placed_fragment(placed);
        } else {
            element.accept(self);
        }
    }

    fn render_placed_fragment(&mut self, placed: &dyn FragmentPlacementOwner) {
        let placement = placed.fragment_placement();
        let anchor = if placement.uses_padding_box() {
            self.flow.frame.padding_origin
        } else {
            self.flow.frame.content_origin
        };
        let offset = placement.offset();
        let origin = PdfPoint::new(anchor.x + offset.x, anchor.y - offset.y);
        let planned_flow_top = HashMap::from([(0, origin.y)]);
        let flow = ContainerFlowContext {
            frame: ContainerFrame::new(origin, placement.size, origin),
            container_top_y: origin.y,
            flow_top_by_index: &planned_flow_top,
            float_top_by_index: self.flow.float_top_by_index,
            left_float_bottom: self.flow.left_float_bottom,
            right_float_bottom: self.flow.right_float_bottom,
            device_space_available: self.flow.device_space_available,
            paint_phase: self.flow.paint_phase,
        };
        let position = FlowPosition::new(origin.y, origin.y, 0.0);
        let mut renderer = DirectChildRenderer {
            content: self.content,
            child_index: 0,
            flow: &flow,
            position,
            abs_origins: self.abs_origins,
            ctx: self.ctx,
            handled: false,
            result: None,
        };
        placed.fragment_source().accept(&mut renderer);
        self.handled = renderer.handled;
        self.result = Some(self.position);
    }
}

impl LayoutVisitor for DirectChildRenderer<'_, '_, '_> {
    fn visit_text_block(&mut self, element: &TextBlock) {
        let result = render_text_child(
            self.content,
            element,
            self.child_index,
            self.flow,
            self.position,
            self.abs_origins,
            self.ctx,
        );
        self.finish(result);
    }

    fn visit_column_rule(&mut self, element: &ColumnRule) {
        if self.flow.paint_phase.paints_decoration() {
            paint_column_rule_line(
                self.content,
                self.flow.frame.content_origin.x,
                self.flow.frame.content_origin.y,
                element.paint.width,
                element.height,
                &element.paint,
                self.ctx.page_ext_gstates,
                self.ctx.bg_alpha_counter,
            );
        }
        self.finish(self.position);
    }

    fn visit_container(&mut self, element: &Container) {
        let result = render_nested_container(
            self.content,
            element,
            self.child_index,
            self.flow,
            self.position,
            self.abs_origins,
            self.ctx,
        );
        self.finish(result);
    }

    fn visit_image(&mut self, element: &Image) {
        let result = render_image_child(
            self.content,
            element,
            self.child_index,
            self.flow,
            self.position,
            self.abs_origins,
            self.ctx,
        );
        self.finish(result);
    }

    fn visit_svg(&mut self, element: &Svg) {
        let result = render_svg_child(
            self.content,
            element,
            self.child_index,
            self.flow,
            self.position,
            self.abs_origins,
            self.ctx,
        );
        self.finish(result);
    }

    fn visit_flex_row(&mut self, element: &FlexRow) {
        let result = render_flex_child(
            self.content,
            element,
            self.child_index,
            self.flow,
            self.position,
            self.abs_origins,
            self.ctx,
        );
        self.finish(result);
    }

    fn visit_horizontal_rule(&mut self, element: &HorizontalRule) {
        let mut position = self.position;
        let planned_flow_top = self.flow.flow_top_by_index.get(&self.child_index).copied();
        if let Some(top) = planned_flow_top {
            position.y = top;
        } else {
            position.cursor_y -=
                collapsed_margin_top_extra(element.margins.start, position.previous_margin_bottom);
            position.y = position.cursor_y;
        }
        let layout_geometry = LayoutBoxGeometry::new(
            PdfRect::from_top(
                self.flow.frame.content_origin.x,
                position.y,
                self.flow.frame.width(),
                1.0,
            ),
            EdgeSizes::ZERO,
            EdgeSizes::ZERO,
        );
        let box_geometry = self
            .ctx
            .text
            .pdf_writer
            .resolve_box_geometry(layout_geometry);
        let paint_geometry = box_geometry.painting();
        let geometry = box_geometry.fragment(Default::default());
        if self.flow.paint_phase.paints_contents() {
            let group = PaintGroupScope::begin(self.content, element, geometry, self.ctx);
            paint_horizontal_rule(
                self.content,
                PdfPoint::new(
                    paint_geometry.border_box.left,
                    paint_geometry.border_box.top(),
                ),
                paint_geometry.border_box.width,
            );
            group.finish(self.content, self.ctx);
        }
        if planned_flow_top.is_none() {
            position.cursor_y -= 1.0 + element.margins.end;
            position.y = position.cursor_y;
        }
        position.previous_margin_bottom = element.margins.end;
        self.finish(position);
    }

    fn visit_progress_bar(&mut self, element: &ProgressBar) {
        let mut position = self.position;
        let planned_flow_top = self.flow.flow_top_by_index.get(&self.child_index).copied();
        let top = if let Some(top) = planned_flow_top {
            top
        } else {
            position.cursor_y -=
                collapsed_margin_top_extra(element.margins.start, position.previous_margin_bottom);
            position.cursor_y
        };
        let rect = PdfRect::from_top(
            self.flow.frame.content_origin.x,
            top,
            element.size.width,
            element.size.height,
        );
        let layout_geometry = LayoutBoxGeometry::new(rect, EdgeSizes::ZERO, EdgeSizes::ZERO);
        let box_geometry = self
            .ctx
            .text
            .pdf_writer
            .resolve_box_geometry(layout_geometry);
        let paint_geometry = box_geometry.painting();
        let geometry = box_geometry.fragment(Default::default());
        if self.flow.paint_phase.paints_contents() {
            let group = PaintGroupScope::begin(self.content, element, geometry, self.ctx);
            paint_progress_bar(self.content, element, paint_geometry.border_box);
            group.finish(self.content, self.ctx);
        }
        if planned_flow_top.is_none() {
            position.cursor_y -= element.size.height + element.margins.end;
            position.y = position.cursor_y;
        }
        position.previous_margin_bottom = element.margins.end;
        self.finish(position);
    }

    fn visit_math_block(&mut self, element: &MathBlock) {
        let mut position = self.position;
        let planned_flow_top = self.flow.flow_top_by_index.get(&self.child_index).copied();
        let top = if let Some(top) = planned_flow_top {
            top
        } else {
            position.cursor_y -=
                collapsed_margin_top_extra(element.margins.start, position.previous_margin_bottom);
            position.cursor_y
        };
        let layout_geometry = LayoutBoxGeometry::new(
            PdfRect::from_top(
                self.flow.frame.content_origin.x,
                top,
                self.flow.frame.width(),
                element.layout.height(),
            ),
            EdgeSizes::ZERO,
            EdgeSizes::ZERO,
        );
        let geometry = self
            .ctx
            .text
            .pdf_writer
            .resolve_box_geometry(layout_geometry)
            .fragment(Default::default());
        if self.flow.paint_phase.paints_contents() {
            let group = PaintGroupScope::begin(self.content, element, geometry, self.ctx);
            paint_math_block(
                self.content,
                element,
                PdfPoint::new(self.flow.frame.content_origin.x, top),
                self.flow.frame.width(),
                &mut self.ctx.text,
            );
            group.finish(self.content, self.ctx);
        }
        if planned_flow_top.is_none() {
            position.cursor_y -= element.layout.height() + element.margins.end;
            position.y = position.cursor_y;
        }
        position.previous_margin_bottom = element.margins.end;
        self.finish(position);
    }
}

pub(super) fn render_container_children(
    content: &mut String,
    children: &[LayoutNode],
    frame: ContainerFrame,
    abs_origins: &mut HashMap<usize, PdfPoint>,
    ctx: &mut PageRenderContext<'_>,
    options: ContainerRenderOptions,
) {
    // Depth zero denotes the initial containing block. In paged media fixed
    // positioned descendants resolve against the page area, regardless of the
    // positioned ancestors they happen to be nested under.
    abs_origins.entry(0).or_insert(ctx.initial_fixed_origin);
    // Padding-box origin (left x, top y in PDF coords) of THIS container, used
    // as the default anchor for absolutely-positioned children. An abs child
    // whose containing block names a *different* positioned ancestor (because it
    // is nested inside static intermediates) overrides this via `abs_origins`.
    let x = frame.content_origin.x;
    let mut y = frame.content_origin.y;
    let device_space_available = options.device_space_available;
    let paint_phase = options.paint_phase;
    let stacking_scope = options.stacking_scope;
    let mut stacking_plan = StackingPaintPlan::default();

    // Separate children handled by render_nested_table_rows from those rendered
    // directly (Container, Svg, etc.).
    // We process all children in order, flushing accumulated nested-layout
    // batches when we hit a directly-handled type.
    let mut nested_batch: Vec<&dyn LayoutElement> = Vec::new();
    let mut cursor_y = y;
    // Save original y for absolute positioning (must not be affected by
    // flow children advancing the cursor).
    let container_top_y = y;
    // Track the previous in-flow block sibling's margin-bottom so adjacent
    // vertical margins collapse (CSS) instead of summing. Nested table batches
    // return the same state, while true barriers and out-of-flow content reset
    // it according to their flow capability.
    let mut prev_margin_bottom: f32 = 0.0;
    let mut nested_batch_position = FlowPosition::new(y, cursor_y, prev_margin_bottom);

    // Simplified CSS floats: precompute each floated child's top (relative to the
    // content-box top) and per-side running bottoms via the shared flow
    // simulator, keyed by source index. Floats are removed from normal flow — the
    // cursor below does not advance for them — but in-flow blocks with `clear`
    // are pushed below the relevant float bottoms. Only computed when a child
    // actually floats, so the common case pays nothing.
    let has_floats = children
        .iter()
        .any(|c| crate::layout::paginate::element_float(c) != Float::None);
    let (float_top_by_index, left_float_bottom, right_float_bottom) = if has_floats {
        let flow = crate::layout::paginate::simulate_block_flow(children);
        let tops: HashMap<usize, f32> = flow.floats.iter().map(|f| (f.index, f.top)).collect();
        // Float bottoms in PDF y (down = smaller y) for `clear`. Floats always
        // precede the blocks that clear them in source order, so the per-side
        // totals from the simulator are exactly what those blocks must clear.
        (
            tops,
            container_top_y - flow.left_float_bottom,
            container_top_y - flow.right_float_bottom,
        )
    } else {
        (HashMap::new(), container_top_y, container_top_y)
    };

    // Geometry and flow are resolved in source order. Paint order is a
    // separate concern: non-in-flow fragments are either collected by this
    // box's stacking context or deferred through ordinary ancestors to the
    // nearest context. This is required for a positioned grandchild to stack
    // against its ancestor's siblings instead of being trapped in the
    // grandparent's direct-child fragment.
    let flow_top_by_index = HashMap::new();
    let flow_context = ContainerFlowContext {
        frame,
        container_top_y,
        flow_top_by_index: &flow_top_by_index,
        float_top_by_index: &float_top_by_index,
        left_float_bottom,
        right_float_bottom,
        device_space_available,
        paint_phase,
    };

    for (child_index, child) in children
        .iter()
        .enumerate()
        .map(|(index, child)| (index, child.as_ref()))
    {
        let handled_by_nested = is_nested_row(child);
        if handled_by_nested {
            if nested_batch.is_empty() {
                nested_batch_position = FlowPosition::new(y, cursor_y, prev_margin_bottom);
            }
            nested_batch.push(child);
            continue;
        }

        // Flush any accumulated nested batch before handling this element
        if !nested_batch.is_empty() {
            let marker = ctx.stacking.marker();
            let mut batch_content = String::new();
            let position = render_rows(
                &mut batch_content,
                &nested_batch,
                x,
                NestedRowsFlow::pending(nested_batch_position),
                paint_phase.paints_contents(),
                abs_origins,
                ctx,
            );
            let descendants = ctx.stacking.take_since(marker);
            ctx.stacking.commit(
                stacking_scope,
                content,
                &mut stacking_plan,
                crate::layout::elements::StackingLevel::in_flow(),
                batch_content,
                descendants,
            );
            y = position.y;
            cursor_y = position.cursor_y;
            prev_margin_bottom = position.previous_margin_bottom;
            nested_batch.clear();
        }

        let child_position = FlowPosition::new(y, cursor_y, prev_margin_bottom);
        let split_in_flow = paint_phase == ElementPaintPhase::All
            && child_paint_order(child).is_in_flow()
            && child
                .in_flow_paint_phase_owner()
                .is_some_and(crate::layout::elements::BoxPaintOwner::supports_phased_paint);
        let (handled, result) = if split_in_flow {
            let render_phase = |phase,
                                abs_origins: &mut HashMap<usize, PdfPoint>,
                                ctx: &mut PageRenderContext<'_>| {
                let marker = ctx.stacking.marker();
                let mut phase_content = String::new();
                let phase_flow = flow_context.with_paint_phase(phase);
                let (handled, result) = {
                    let mut renderer = DirectChildRenderer {
                        content: &mut phase_content,
                        child_index,
                        flow: &phase_flow,
                        position: child_position,
                        abs_origins,
                        ctx,
                        handled: false,
                        result: None,
                    };
                    renderer.render(child);
                    (renderer.handled, renderer.result)
                };
                let descendants = ctx.stacking.take_since(marker);
                (handled, result, phase_content, descendants)
            };
            let (decoration_handled, _, decoration, decoration_descendants) =
                render_phase(ElementPaintPhase::Decoration, abs_origins, ctx);
            ctx.stacking.commit(
                stacking_scope,
                content,
                &mut stacking_plan,
                crate::layout::elements::StackingLevel::in_flow_decoration(),
                decoration,
                decoration_descendants,
            );
            let (contents_handled, result, contents, contents_descendants) =
                render_phase(ElementPaintPhase::Contents, abs_origins, ctx);
            ctx.stacking.commit(
                stacking_scope,
                content,
                &mut stacking_plan,
                crate::layout::elements::StackingLevel::in_flow_contents(),
                contents,
                contents_descendants,
            );
            (decoration_handled || contents_handled, result)
        } else {
            let marker = ctx.stacking.marker();
            let mut child_content = String::new();
            let (handled, result) = {
                let mut renderer = DirectChildRenderer {
                    content: &mut child_content,
                    child_index,
                    flow: &flow_context,
                    position: child_position,
                    abs_origins,
                    ctx,
                    handled: false,
                    result: None,
                };
                renderer.render(child);
                (renderer.handled, renderer.result)
            };
            let descendants = ctx.stacking.take_since(marker);
            ctx.stacking.commit(
                stacking_scope,
                content,
                &mut stacking_plan,
                child_paint_order(child),
                child_content,
                descendants,
            );
            (handled, result)
        };
        if let Some(position) = result {
            y = position.y;
            cursor_y = position.cursor_y;
            prev_margin_bottom = position.previous_margin_bottom;
        } else if !handled {
            let height = crate::layout::paginate::estimate_element_height(child);
            if let Some(top) = flow_top_by_index.get(&child_index).copied() {
                y = top - height;
            } else {
                cursor_y -= height;
                y = cursor_y;
            }
            // Unknown/other element: its full estimated height (including any
            // margins) was consumed; do not collapse the next sibling.
            prev_margin_bottom = 0.0;
        }
    }

    // Flush any remaining nested batch
    if !nested_batch.is_empty() {
        let marker = ctx.stacking.marker();
        let mut batch_content = String::new();
        let _ = render_rows(
            &mut batch_content,
            &nested_batch,
            x,
            NestedRowsFlow::pending(nested_batch_position),
            paint_phase.paints_contents(),
            abs_origins,
            ctx,
        );
        let descendants = ctx.stacking.take_since(marker);
        ctx.stacking.commit(
            stacking_scope,
            content,
            &mut stacking_plan,
            crate::layout::elements::StackingLevel::in_flow(),
            batch_content,
            descendants,
        );
    }

    if stacking_scope.is_local() {
        ctx.stacking.paint_plan(stacking_plan, content);
    }
}
