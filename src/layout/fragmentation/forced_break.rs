//! Forced breaks retained inside otherwise paint-atomic layout structures.
//!
//! Layout boxes carry complex descendants as nested layout elements. Pagination
//! must split that nested flow before PDF painting; otherwise a descendant
//! `break-before: page` becomes inert merely because an ancestor owns an
//! independent formatting context.

use crate::layout::elements::{
    BlockSize, BoxFragmentSlice, ColumnRule, Container, FlexRow, FragmentPlacementOwner, GridRow,
    IntoLayoutNode, LayoutElement, LayoutNode, LayoutVisitor, MulticolColumn, MulticolContainer,
    PageBreak, Table, TableRow,
};
use crate::layout::engine::{
    FlexCell, FlexFragmentRole, FlexItemFragmentation, FlexNestedOrigin, PageBreakSide,
};
use crate::layout::flow_metrics::BlockMargins;
use crate::layout::paginate::{estimate_element_height, simulate_block_flow};
use crate::layout::roundoff::{
    equal_with_roundoff, exceeds_with_roundoff, is_positive_with_roundoff,
};
#[cfg(test)]
use crate::style::computed::Overflow;
use crate::style::computed::{AlignItems, BoxDecorationBreak};
use crate::types::{CornerRadii, EdgeSizes};

#[derive(Debug)]
struct ForcedBreak<T> {
    before: T,
    after: T,
    target: ForcedBreakTarget,
}

impl<T> ForcedBreak<T> {
    fn map_sides<U>(self, mut map: impl FnMut(T) -> U) -> ForcedBreak<U> {
        ForcedBreak {
            before: map(self.before),
            after: map(self.after),
            target: self.target,
        }
    }
}

/// Remaining block-axis room from a layout-flow origin to the fragmentainer
/// edge. This is intentionally a domain value rather than an unlabelled `f32`:
/// recursive fragmentation must consume ancestor and sibling offsets exactly
/// once as it descends toward a forced break.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FragmentainerSpace {
    remaining: f32,
}

/// Continuous containing-block geometry shared by positioned descendants on
/// both sides of one fragmentation break.
#[derive(Clone, Copy, Debug)]
struct PositionedFragmentPlan {
    containing_block_depth: usize,
    composite_block_size: f32,
    continuation_offset: f32,
}

impl PositionedFragmentPlan {
    fn from_fragments(before: &Container, after: &Container) -> Option<Self> {
        let first = before.fragmentation.reference_slice?;
        let continuation = after.fragmentation.reference_slice?;
        Some(Self {
            containing_block_depth: before.positioning.containing_block_depth,
            composite_block_size: first.composite_block_size(),
            continuation_offset: continuation.block_offset(),
        })
        .filter(|plan| plan.containing_block_depth > 0)
    }
}

fn positioned_in_plan(element: &dyn LayoutElement, plan: PositionedFragmentPlan) -> bool {
    element.positioning_owner().is_some_and(|owner| {
        let positioning = owner.positioning();
        positioning.scheme.is_absolute()
            && positioning
                .containing_block
                .is_some_and(|block| block.depth == plan.containing_block_depth)
    })
}

fn rebase_positioned_element(
    element: &mut dyn LayoutElement,
    plan: PositionedFragmentPlan,
    fragment_offset: f32,
) -> f32 {
    let extent = element.block_fragmentation_source().map_or_else(
        || fragment_box_extent(element),
        |source| source.block_extent(),
    );
    element.positioning_owner_mut().map_or(0.0, |owner| {
        owner.positioning_mut().resolve_fragmented_block_offset(
            plan.composite_block_size,
            extent,
            fragment_offset,
        )
    })
}

/// Re-resolve matching absolute descendants already retained by a fragment.
/// The visitor follows every structural child store through the same operation
/// instead of adding a pagination-only path for one concrete box type.
fn rebase_positioned_descendants(
    elements: &mut [LayoutNode],
    plan: PositionedFragmentPlan,
    fragment_offset: f32,
) {
    struct Rebase {
        plan: PositionedFragmentPlan,
        fragment_offset: f32,
    }

    impl crate::layout::elements::LayoutVisitorMut for Rebase {
        fn visit_container(&mut self, element: &mut Container) {
            rebase_positioned_descendants(&mut element.children, self.plan, self.fragment_offset);
        }

        fn visit_flex_row(&mut self, element: &mut FlexRow) {
            for cell in &mut element.content.cells {
                rebase_positioned_descendants(
                    &mut cell.nested_elements,
                    self.plan,
                    self.fragment_offset,
                );
            }
        }

        fn visit_grid_row(&mut self, element: &mut GridRow) {
            for cell in &mut element.content.cells {
                rebase_positioned_descendants(
                    &mut cell.layout.content.children,
                    self.plan,
                    self.fragment_offset,
                );
            }
        }

        fn visit_table_row(&mut self, element: &mut TableRow) {
            for cell in &mut element.content.cells {
                rebase_positioned_descendants(
                    &mut cell.layout.content.children,
                    self.plan,
                    self.fragment_offset,
                );
            }
        }
    }

    for element in elements {
        if positioned_in_plan(element.as_ref(), plan) {
            rebase_positioned_element(element.as_mut(), plan, fragment_offset);
        } else {
            element.accept_mut(&mut Rebase {
                plan,
                fragment_offset,
            });
        }
    }
}

/// Remove absolute descendants whose continuous block-start lies after the
/// break. They are returned as direct out-of-flow children of the continued
/// containing block; their stored containing-block identity preserves the
/// correct padding-box anchor while ordinary intermediary boxes remain in
/// normal flow on the first fragment.
fn take_positioned_continuations(
    elements: &mut Vec<LayoutNode>,
    plan: PositionedFragmentPlan,
) -> Vec<LayoutNode> {
    struct Take<'a> {
        plan: PositionedFragmentPlan,
        moved: &'a mut Vec<LayoutNode>,
    }

    impl crate::layout::elements::LayoutVisitorMut for Take<'_> {
        fn visit_container(&mut self, element: &mut Container) {
            self.moved.extend(take_positioned_continuations(
                &mut element.children,
                self.plan,
            ));
        }

        fn visit_flex_row(&mut self, element: &mut FlexRow) {
            for cell in &mut element.content.cells {
                self.moved.extend(take_positioned_continuations(
                    &mut cell.nested_elements,
                    self.plan,
                ));
            }
        }

        fn visit_grid_row(&mut self, element: &mut GridRow) {
            for cell in &mut element.content.cells {
                self.moved.extend(take_positioned_continuations(
                    &mut cell.layout.content.children,
                    self.plan,
                ));
            }
        }

        fn visit_table_row(&mut self, element: &mut TableRow) {
            for cell in &mut element.content.cells {
                self.moved.extend(take_positioned_continuations(
                    &mut cell.layout.content.children,
                    self.plan,
                ));
            }
        }
    }

    let mut moved = Vec::new();
    let mut index = 0;
    while index < elements.len() {
        if positioned_in_plan(elements[index].as_ref(), plan) {
            let continuous_offset = rebase_positioned_element(elements[index].as_mut(), plan, 0.0);
            if !exceeds_with_roundoff(plan.continuation_offset, continuous_offset) {
                let mut continuation = elements.remove(index);
                rebase_positioned_element(continuation.as_mut(), plan, plan.continuation_offset);
                moved.push(continuation);
                continue;
            }
        } else {
            let mut nested = Vec::new();
            elements[index].accept_mut(&mut Take {
                plan,
                moved: &mut nested,
            });
            moved.extend(nested);
        }
        index += 1;
    }
    moved
}

impl FragmentainerSpace {
    fn new(remaining: f32) -> Self {
        Self {
            remaining: remaining.max(0.0),
        }
    }

    fn after(self, leading_extent: f32) -> Self {
        Self::new(self.remaining - leading_extent.max(0.0))
    }
}

/// Destination selected by a forced page break retained inside nested layout.
/// Keeping the named-page request with its side prevents flex fragmentation
/// from silently degrading `page: <name>` into an ordinary page break.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ForcedBreakTarget {
    side: PageBreakSide,
    page_name: Option<String>,
}

impl ForcedBreakTarget {
    pub(crate) fn into_layout_element(self) -> LayoutNode {
        PageBreak {
            side: self.side,
            page_name: self.page_name,
        }
        .boxed()
    }
}

fn forced_break_target(element: &dyn LayoutElement) -> Option<ForcedBreakTarget> {
    #[derive(Default)]
    struct TargetVisitor(Option<ForcedBreakTarget>);

    impl LayoutVisitor for TargetVisitor {
        fn visit_page_break(&mut self, element: &PageBreak) {
            self.0 = Some(ForcedBreakTarget {
                side: element.side,
                page_name: element.page_name.clone(),
            });
        }
    }

    let mut visitor = TargetVisitor::default();
    element.accept(&mut visitor);
    visitor.0
}

fn split_sequence(
    elements: &[LayoutNode],
    space: FragmentainerSpace,
) -> Option<ForcedBreak<Vec<LayoutNode>>> {
    let mut consumed = 0.0;
    for (index, element) in elements.iter().enumerate() {
        if let Some(target) = forced_break_target(element.as_ref()) {
            return Some(ForcedBreak {
                before: elements[..index].to_vec(),
                after: elements[index + 1..].to_vec(),
                target,
            });
        }
        let margins = element
            .margin_holder()
            .map(|holder| *holder.margins())
            .unwrap_or(BlockMargins::ZERO);
        let element_space = space.after(consumed + margins.start);
        let Some(split) = split_element(element, element_space) else {
            consumed += estimate_element_height(element);
            continue;
        };
        let split_table_wrapper = is_table_row(element.as_ref());
        let continuation_extent = split
            .after
            .as_deref()
            .map(fragment_box_extent)
            .unwrap_or_default()
            + elements[index + 1..]
                .iter()
                .take_while(|element| is_table_row(element.as_ref()))
                .map(|element| fragment_box_extent(element.as_ref()))
                .sum::<f32>();
        let mut before = elements[..index].to_vec();
        before.extend(split.before);
        let mut after = Vec::new();
        after.extend(split.after);
        after.extend_from_slice(&elements[index + 1..]);
        if split_table_wrapper {
            split_preceding_table_decoration(&mut before, &mut after, space, continuation_extent);
        }
        return Some(ForcedBreak {
            before,
            after,
            target: split.target,
        });
    }
    None
}

fn fragment_box_extent(element: &dyn LayoutElement) -> f32 {
    let margins = element
        .margin_holder()
        .map(|holder| holder.margins().total())
        .unwrap_or_default();
    (estimate_element_height(element) - margins).max(0.0)
}

fn is_table_row(element: &dyn LayoutElement) -> bool {
    #[derive(Default)]
    struct TableRowProbe(bool);

    impl LayoutVisitor for TableRowProbe {
        fn visit_table_row(&mut self, _element: &TableRow) {
            self.0 = true;
        }
    }

    let mut probe = TableRowProbe::default();
    element.accept(&mut probe);
    probe.0
}

fn split_preceding_table_decoration(
    before: &mut Vec<LayoutNode>,
    after: &mut Vec<LayoutNode>,
    space: FragmentainerSpace,
    continuation_extent: f32,
) {
    let Some(decoration_index) = before
        .iter()
        .rposition(|element| element.table_box_decoration_owner().is_some())
    else {
        return;
    };
    let leading_extent = before[..decoration_index]
        .iter()
        .map(|element| estimate_element_height(element.as_ref()))
        .sum::<f32>();
    let decoration_margin_start = before[decoration_index]
        .margin_holder()
        .map(|holder| holder.margins().start)
        .unwrap_or_default();
    let first_extent = space
        .after(leading_extent + decoration_margin_start)
        .remaining;
    let Some(owner) = before[decoration_index].table_box_decoration_owner() else {
        return;
    };
    let first = owner.open_fragment(first_extent);
    let continuation = owner.continuation_fragment(continuation_extent);
    before[decoration_index] = first;
    after.insert(0, continuation);
}

fn split_element(
    element: &dyn LayoutElement,
    space: FragmentainerSpace,
) -> Option<ForcedBreak<Option<LayoutNode>>> {
    struct SplitVisitor {
        space: FragmentainerSpace,
        split: Option<ForcedBreak<Option<LayoutNode>>>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_container(&mut self, element: &Container) {
            if element.overflow.combined.clips() {
                return;
            }
            let child_space = self
                .space
                .after(element.box_model.border.top.width + element.box_model.padding.top);
            self.split = split_sequence(&element.children, child_space)
                .map(|split| split_container(element, split, self.space));
        }

        fn visit_multicol_container(&mut self, element: &MulticolContainer) {
            let principal = &element.principal;
            if principal.overflow.combined.clips() {
                return;
            }
            let child_space = self
                .space
                .after(principal.box_model.border.top.width + principal.box_model.padding.top);
            self.split = split_sequence(&principal.children, child_space)
                .map(|split| split_multicol_container(element, split, self.space));
        }

        fn visit_multicol_column(&mut self, element: &MulticolColumn) {
            let principal = &element.principal;
            let child_space = self
                .space
                .after(principal.box_model.border.top.width + principal.box_model.padding.top);
            self.split = split_sequence(&principal.children, child_space)
                .map(|split| split_multicol_column(element, split, self.space));
        }

        fn visit_table(&mut self, element: &Table) {
            let principal = &element.principal;
            if principal.overflow.combined.clips() {
                return;
            }
            let child_space = self
                .space
                .after(principal.box_model.border.top.width + principal.box_model.padding.top);
            self.split = split_sequence(&principal.children, child_space)
                .map(|split| split_table(element, split, self.space));
        }

        fn visit_flex_row(&mut self, element: &FlexRow) {
            self.split = split_flex_row(element, self.space).map(|split| split.map_sides(Some));
        }

        fn visit_table_row(&mut self, element: &TableRow) {
            self.split = split_table_row(element, self.space).map(|split| split.map_sides(Some));
        }

        fn visit_grid_row(&mut self, element: &GridRow) {
            self.split = split_grid_row(element, self.space).map(|split| split.map_sides(Some));
        }
    }

    let mut visitor = SplitVisitor { space, split: None };
    element.accept(&mut visitor);
    visitor.split
}

fn split_container(
    element: &Container,
    split: ForcedBreak<Vec<LayoutNode>>,
    space: FragmentainerSpace,
) -> ForcedBreak<Option<LayoutNode>> {
    split_container_principal(element, split, space, |_| BlockSize::AUTO)
        .map_sides(|fragment| fragment.map(IntoLayoutNode::boxed))
}

fn split_multicol_container(
    element: &MulticolContainer,
    split: ForcedBreak<Vec<LayoutNode>>,
    space: FragmentainerSpace,
) -> ForcedBreak<Option<LayoutNode>> {
    let mut fragments = split_container_principal(
        &element.principal,
        split,
        space,
        multicol_continuation_block_size,
    );
    for principal in [&mut fragments.before, &mut fragments.after]
        .into_iter()
        .flatten()
    {
        retain_supported_column_rules(principal);
    }
    fragments.map_sides(|fragment| fragment.map(|box_| MulticolContainer::new(box_).boxed()))
}

fn split_table(
    element: &Table,
    split: ForcedBreak<Vec<LayoutNode>>,
    space: FragmentainerSpace,
) -> ForcedBreak<Option<LayoutNode>> {
    split_container_principal(&element.principal, split, space, |_| BlockSize::AUTO)
        .map_sides(|fragment| fragment.map(|principal| Table::new(principal).boxed()))
}

fn split_multicol_column(
    element: &MulticolColumn,
    split: ForcedBreak<Vec<LayoutNode>>,
    space: FragmentainerSpace,
) -> ForcedBreak<Option<LayoutNode>> {
    split_container_principal(&element.principal, split, space, |_| BlockSize::AUTO)
        .map_sides(|fragment| fragment.map(|principal| element.with_principal(principal).boxed()))
}

fn retain_supported_column_rules(principal: &mut Container) {
    #[derive(Default)]
    struct ChildIdentity {
        column: Option<(usize, f32)>,
        rule: Option<(usize, f32)>,
    }

    impl LayoutVisitor for ChildIdentity {
        fn visit_multicol_column(&mut self, element: &MulticolColumn) {
            if !element.principal.children.is_empty() {
                self.column = Some((element.index, element.fragment_placement().block_offset()));
            }
        }

        fn visit_column_rule(&mut self, element: &ColumnRule) {
            self.rule = Some((
                element.gap_after,
                element.fragment_placement().block_offset(),
            ));
        }
    }

    let columns = principal
        .children
        .iter()
        .filter_map(|child| {
            let mut identity = ChildIdentity::default();
            child.accept(&mut identity);
            identity.column
        })
        .collect::<Vec<_>>();
    principal.children.retain(|child| {
        let mut identity = ChildIdentity::default();
        child.accept(&mut identity);
        let Some((gap_after, line_top)) = identity.rule else {
            return true;
        };
        [gap_after, gap_after + 1].into_iter().all(|index| {
            columns
                .iter()
                .any(|(column, top)| *column == index && equal_with_roundoff(*top, line_top))
        })
    });
}

fn split_container_principal(
    element: &Container,
    split: ForcedBreak<Vec<LayoutNode>>,
    space: FragmentainerSpace,
    continuation_block_size: impl FnOnce(&Container) -> BlockSize,
) -> ForcedBreak<Option<Container>> {
    let ForcedBreak {
        before: before_children,
        after: after_children,
        target,
    } = split;

    // Edge breaks propagate to the container instead of creating an empty box
    // fragment. Removing the nested marker while bubbling an absent side lets
    // the same rule apply through arbitrarily deep ordinary block wrappers.
    if before_children.is_empty() {
        let mut after = element.clone();
        after.children = after_children;
        return ForcedBreak {
            before: None,
            after: Some(after),
            target,
        };
    }
    if after_children.is_empty() {
        let mut before = element.clone();
        before.children = before_children;
        return ForcedBreak {
            before: Some(before),
            after: None,
            target,
        };
    }

    let mut before = element.clone();
    let mut after = element.clone();
    before.children = before_children;
    let fixed_height = before.box_model.size.height.is_definite();
    let overflow_content_insets =
        (before.box_model.padding + before.box_model.border.widths()).horizontal_only();

    after.children = after_children;
    after.box_model.margins.start = 0.0;
    if fixed_height {
        // CSS Break 3 §5.5: content after a fixed-height box is an
        // overflow-only fragment. It has an empty, undecorated box at the
        // fragmentainer start, while graphical effects such as transforms
        // remain active on the overflow content.
        after.paint.background = Default::default();
        after.box_model.border = Default::default();
        after.paint.border_radii = CornerRadii::ZERO;
        // The overflow-only fragment has no block-axis decoration, but its
        // descendants retain the original inline content origin.
        after.box_model.padding = overflow_content_insets;
        after.box_model.margins.end = 0.0;
        after.box_model.size.height = BlockSize::definite(0.0);
        after.fragmentation.content_role =
            crate::layout::elements::PageContentRole::OverflowContinuation;
        after.paint.shadows.clear();
        after.paint.outline = Default::default();
    } else {
        after.box_model.padding.top = 0.0;
        after.box_model.border.top.width = 0.0;
        after.paint.border_radii = after.paint.border_radii.clear_top();
        after.box_model.size.height = continuation_block_size(&after);
    }
    if !fixed_height {
        // CSS Break 3 §5.3: an open principal box fragment consumes all
        // remaining fragmentainer space before its content resumes. Record
        // that used fragment extent explicitly; the continuation remains
        // content-sized and therefore fragmentable at any later descendant
        // break.
        before.box_model.size.height = BlockSize::fragment(space.remaining);
        before.box_model.padding.bottom = 0.0;
        before.box_model.border.bottom.width = 0.0;
        before.paint.border_radii = before.paint.border_radii.clear_bottom();
        before.box_model.margins.end = 0.0;
        if element.fragmentation.decoration == BoxDecorationBreak::Slice {
            let minimum_continuation = element
                .box_model
                .size
                .height
                .used()
                .map_or(0.0, |height| (height - space.remaining).max(0.0));
            let continuation_extent = estimate_element_height(&after).max(minimum_continuation);
            let (first_slice, continuation_slice) =
                BoxFragmentSlice::split(space.remaining, continuation_extent, &element.box_model);
            before.fragmentation.reference_slice = Some(first_slice);
            after.fragmentation.reference_slice = Some(continuation_slice);
        }
    }
    if let Some(plan) = PositionedFragmentPlan::from_fragments(&before, &after) {
        let moved = take_positioned_continuations(&mut before.children, plan);
        rebase_positioned_descendants(&mut after.children, plan, plan.continuation_offset);
        after.children.extend(moved);
    }
    ForcedBreak {
        before: Some(before),
        after: Some(after),
        target,
    }
}

/// Size a new multicol line from the retained fragments it contains.
///
/// Multicol columns are anonymous fragmentainers with physical placements, so
/// ordinary block-flow measurement intentionally ignores them.
/// At a page break they nevertheless define the block size of the continuing
/// principal multicol box.
fn multicol_continuation_block_size(element: &Container) -> BlockSize {
    let content_bottom = element
        .children
        .iter()
        .filter_map(|child| {
            let placement = child.fragment_placement_owner()?.fragment_placement();
            Some(placement.block_offset() + placement.size.height)
        })
        .fold(0.0_f32, f32::max);
    BlockSize::fragment(
        content_bottom + element.box_model.padding.bottom + element.box_model.border.bottom.width,
    )
}

fn overflow_continuation_cell(cell: &FlexCell, nested: Vec<LayoutNode>) -> FlexCell {
    FlexCell {
        lines: Vec::new(),
        padding: EdgeSizes::ZERO,
        border: Default::default(),
        natural_height: 0.0,
        fragmentation: FlexItemFragmentation::definite(),
        paint: Default::default(),
        positioning: Default::default(),
        nested_elements: nested,
        y_offset: 0.0,
        line_cross_size: 0.0,
        ..cell.clone()
    }
}

fn fragmented_continuation_cell(cell: &FlexCell, nested: Vec<LayoutNode>) -> FlexCell {
    let clone = cell.fragmentation.box_fragmentation.decoration == BoxDecorationBreak::Clone;
    let mut continuation = cell.clone();
    continuation.lines.clear();
    continuation.nested_elements = nested;
    continuation.paint.group = Default::default();
    continuation.y_offset = 0.0;
    continuation.line_cross_size = 0.0;
    continuation.fragmentation.fragment_block_extent = None;

    if !clone {
        continuation.padding.top = 0.0;
        continuation.border.top.width = 0.0;
        continuation.paint.border_radii = continuation.paint.border_radii.clear_top();
    }

    let nested_height = simulate_block_flow(&continuation.nested_elements).height;
    continuation.natural_height = match continuation.nested_origin {
        FlexNestedOrigin::ContentBox => {
            nested_height + continuation.padding.vertical() + continuation.border.vertical_width()
        }
        // Table rows already measure the principal table border box. The flex
        // cell supplies its paint but must not add that border to the row
        // extent a second time when sizing a continuation.
        FlexNestedOrigin::TableBorderBox => nested_height,
    };
    continuation
}

fn open_flex_cell_fragment(cell: &mut FlexCell, fragment_extent: f32) {
    cell.fragmentation.fragment_block_extent = Some(fragment_extent);
    if cell.fragmentation.box_fragmentation.decoration == BoxDecorationBreak::Slice {
        cell.padding.bottom = 0.0;
        cell.border.bottom.width = 0.0;
        cell.paint.border_radii = cell.paint.border_radii.clear_bottom();
    }
}

fn clear_flex_container_continuation(element: &mut FlexRow) {
    let overflow_content_insets =
        (element.box_model.padding + element.box_model.border.widths()).horizontal_only();
    element.content.forced_line_breaks.clear();
    element.content.fragment_role = FlexFragmentRole::ParallelOverflowContinuation;
    element.content.row_height = 0.0;
    element.box_model.margins = BlockMargins::ZERO;
    element.paint.background = Default::default();
    element.box_model.padding = overflow_content_insets;
    element.box_model.border = Default::default();
    element.paint.border_radii = CornerRadii::ZERO;
    element.paint.shadows.clear();
    element.content.alignment = AlignItems::FlexStart;
}

/// Consume the first item break propagated to a row flex line by CSS Flexbox
/// §10. The break divides whole flex lines; it must never split one item's
/// parallel contents away from its siblings.
fn split_flex_row_at_line_break(
    element: &FlexRow,
    space: FragmentainerSpace,
) -> Option<ForcedBreak<LayoutNode>> {
    let available_inner = space
        .after(element.box_model.border.top.width + element.box_model.padding.top)
        .remaining;
    let (marker, cut_y) = element
        .content
        .forced_line_breaks
        .iter()
        .filter_map(|marker| {
            element
                .content
                .cells
                .iter()
                .find(|cell| cell.line_id == marker.before)
                .map(|cell| (*marker, cell.y_offset))
        })
        .filter(|(_, top)| {
            is_positive_with_roundoff(*top) && !exceeds_with_roundoff(*top, available_inner)
        })
        .min_by(|(_, left), (_, right)| left.total_cmp(right))?;

    let mut first_cells = Vec::new();
    let mut continuation_cells = Vec::new();
    for cell in &element.content.cells {
        if exceeds_with_roundoff(cut_y, cell.y_offset) {
            first_cells.push(cell.clone());
        } else {
            let mut continuation = cell.clone();
            continuation.y_offset = (continuation.y_offset - cut_y).max(0.0);
            continuation_cells.push(continuation);
        }
    }
    if first_cells.is_empty() || continuation_cells.is_empty() {
        return None;
    }

    let mut before = element.clone();
    before.content.cells = first_cells;
    before.content.forced_line_breaks.retain(|candidate| {
        candidate.before != marker.before
            && before
                .content
                .cells
                .iter()
                .any(|cell| cell.line_id == candidate.before)
    });
    before.content.row_height = available_inner.max(cut_y);
    before.box_model.margins.end = 0.0;
    before.box_model.padding.bottom = 0.0;
    before.box_model.border.bottom.width = 0.0;
    before.paint.border_radii = before.paint.border_radii.clear_bottom();

    let mut after = element.clone();
    after.content.cells = continuation_cells;
    after.content.forced_line_breaks.retain(|candidate| {
        candidate.before != marker.before
            && after
                .content
                .cells
                .iter()
                .any(|cell| cell.line_id == candidate.before)
    });
    after.content.row_height = (after.content.row_height - cut_y).max(0.0);
    after.box_model.margins.start = 0.0;
    after.box_model.padding.top = 0.0;
    after.box_model.border.top.width = 0.0;
    after.paint.border_radii = after.paint.border_radii.clear_top();

    Some(ForcedBreak {
        before: before.boxed(),
        after: after.boxed(),
        target: ForcedBreakTarget {
            side: marker.side,
            page_name: None,
        },
    })
}

fn split_flex_row(element: &FlexRow, space: FragmentainerSpace) -> Option<ForcedBreak<LayoutNode>> {
    if let Some(split) = split_flex_row_at_line_break(element, space) {
        return Some(split);
    }
    for (index, cell) in element.content.cells.iter().enumerate() {
        let text_height = cell.lines.iter().map(|line| line.height).sum::<f32>();
        // Baseline alignment without a textual baseline falls back to
        // cross-start. A descendant layout flow has no inline baseline of its
        // own, so zero is the correct shift for the space calculation here.
        let cross = cell.cross_geometry(element.content.row_height, element.content.alignment, 0.0);
        let cell_leading =
            element.box_model.border.top.width + element.box_model.padding.top + cross.offset;
        let cell_space = space.after(cell_leading);
        let nested_leading = cell.border.top.width + cell.padding.top + text_height;
        let Some(split) = split_sequence(&cell.nested_elements, cell_space.after(nested_leading))
        else {
            continue;
        };
        let mut before = element.clone();
        let mut after = element.clone();
        before.content.cells[index].nested_elements = split.before;
        if cell.fragmentation.block_size.fragments_principal_box() {
            open_flex_cell_fragment(&mut before.content.cells[index], cell_space.remaining);
            after.content.cells = vec![fragmented_continuation_cell(cell, split.after)];
        } else {
            after.content.cells = vec![overflow_continuation_cell(cell, split.after)];
        }
        clear_flex_container_continuation(&mut after);
        return Some(ForcedBreak {
            before: before.boxed(),
            after: after.boxed(),
            target: split.target,
        });
    }
    None
}

fn split_table_row(
    element: &TableRow,
    space: FragmentainerSpace,
) -> Option<ForcedBreak<LayoutNode>> {
    // The row's block extent starts with its internal table spacing (the
    // table's top border, padding and `border-spacing` on a first row), so the
    // cells start, and the first fragment's cells end, that much further down.
    let space = space.after(element.flow.internal.start);
    for (index, cell) in element.content.cells.iter().enumerate() {
        if cell.table.clips {
            continue;
        }
        let text_height = cell
            .layout
            .content
            .lines
            .iter()
            .map(|line| line.height)
            .sum::<f32>();
        let Some(split) = split_sequence(
            &cell.layout.content.children,
            space.after(cell.layout.box_model.content_insets.top + text_height),
        ) else {
            continue;
        };
        let mut before = element.clone();
        let mut after = element.clone();
        before.flow.margins.end = 0.0;
        before.flow.internal.end = 0.0;
        before.flow.extra_end = 0.0;
        for cell in &mut before.content.cells {
            cell.layout.box_model.border.bottom.width = 0.0;
            cell.layout.box_model.border_insets.bottom = 0.0;
            cell.layout.box_model.content_insets.bottom = 0.0;
            cell.layout.box_model.minimum_block_size = space.remaining;
        }
        before.collapsed_borders.open_fragment_end();
        before.content.cells[index].layout.content.children = split.before;
        after.flow.margins.start = 0.0;
        after.flow.internal.start = 0.0;
        for cell in &mut after.content.cells {
            cell.layout.content.lines.clear();
            cell.layout.content.children.clear();
            cell.layout.box_model.border.top.width = 0.0;
            cell.layout.box_model.border_insets.top = 0.0;
            cell.layout.box_model.content_insets.top = 0.0;
            cell.layout.box_model.minimum_block_size = 0.0;
        }
        after.collapsed_borders.open_fragment_start();
        after.content.cells[index].layout.content.children = split.after;
        return Some(ForcedBreak {
            before: before.boxed(),
            after: after.boxed(),
            target: split.target,
        });
    }
    None
}

fn split_grid_row(element: &GridRow, space: FragmentainerSpace) -> Option<ForcedBreak<LayoutNode>> {
    for (index, cell) in element.content.cells.iter().enumerate() {
        if cell.placement.clips {
            continue;
        }
        let text_height = cell
            .layout
            .content
            .lines
            .iter()
            .map(|line| line.height)
            .sum::<f32>();
        let Some(split) = split_sequence(
            &cell.layout.content.children,
            space.after(cell.layout.box_model.content_insets.top + text_height),
        ) else {
            continue;
        };
        let mut before = element.clone();
        let mut after = element.clone();
        before.content.cells[index].layout.content.children = split.before;
        let after_cell = &mut after.content.cells[index];
        after_cell.layout.content.lines.clear();
        after_cell.layout.content.children = split.after;
        after_cell.layout.paint.background = Default::default();
        after_cell.layout.box_model.content_insets = EdgeSizes::ZERO;
        after_cell.layout.box_model.border = Default::default();
        after_cell.layout.box_model.minimum_block_size = 0.0;
        return Some(ForcedBreak {
            before: before.boxed(),
            after: after.boxed(),
            target: split.target,
        });
    }
    None
}

/// Split any nested layout flow at its first retained forced break.
///
/// Ordinary block edge breaks bubble through absent fragment sides until they
/// reach their applicable class-A boundary. Internal block breaks slice their
/// ancestors, while flex lines, table cells, and grid cells retain their
/// parallel-fragmentation behavior.
pub(crate) fn split_flow_at_descendant_break(
    element: &dyn LayoutElement,
    available_block_size: f32,
) -> Option<(Option<LayoutNode>, Option<LayoutNode>, ForcedBreakTarget)> {
    split_element(element, FragmentainerSpace::new(available_block_size))
        .map(|split| (split.before, split.after, split.target))
}

#[cfg(test)]
mod tests;
