use crate::layout::elements::{
    BlockSize, BoxPaint, Container, FlexContent, FlexRow, InlineSize, IntoLayoutNode,
    LayoutElement, LayoutNode, LayoutVisitor, LayoutVisitorMut, PageBreak, Positioning,
    SizeConstraints, StackingRole, TextBlock,
};
use crate::layout::flow_metrics::BlockMargins;
use crate::parser::css::{AncestorInfo, CssRule, CssValue, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::style::computed::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, ComputedStyle, Display, FlexDirection,
    FlexWrap, FontWeight, IntrinsicWidthKeyword, JustifyContent, Overflow, OverflowWrap, Position,
    WhiteSpace, ZIndex, compute_style_with_context_with_font_metrics,
};
use crate::types::{CornerRadii, EdgeSizes, Rect, Size};

use super::box_model::ResolvedBoxDimensions;
use super::context::{ContainingBlock, LayoutContext, LayoutEnv};
use super::engine::{
    CounterState, ElementSiblingPosition, FilterApplication, FlexCell, FlexItemFragmentation,
    FlexLineId, FlexNestedOrigin, ForcedFlexLineBreak, LayoutBorder, LayoutTreeContext,
    PageBreakSide, PseudoBoxContext, TextLine, aspect_ratio_height, collects_as_inline_text,
    emit_page_break_after, emit_page_break_before, flatten_element, has_background_paint,
    measure_runs_width, resolve_padding_box_height,
};
use super::inline_formatting::{
    GeneratedBox, GeneratedContentStyles, GeneratedInlineContent, InlineContentSequence,
};
use super::paginate::estimate_element_height;
use super::roundoff::{equal_with_roundoff, exceeds_with_roundoff, is_positive_with_roundoff};
use super::text::{
    InlineRunCollector, InlineTextSequence, TextWrapOptions, estimate_word_width,
    is_collapsible_space, measure_text_intrinsic_widths, parent_line_strut,
    resolve_style_font_family, text_run_line_height_factor, used_font_size, wrap_text_runs,
};

#[derive(Debug, Clone, Copy)]
struct ElementFlexItemIndex(usize);

/// Source identity and replay state retained by a laid-out flex item.
///
/// Generated boxes have source order but no DOM element index. Keeping that
/// distinction typed prevents generated items from being accidentally
/// re-laid-out through an unrelated element's private traversal path.
#[derive(Clone)]
struct FlexItemSource {
    order: usize,
    element: Option<ElementFlexItemIndex>,
    counter_replay: CounterState,
}

#[allow(dead_code)]
struct FlexItem {
    elements: Vec<LayoutNode>,
    break_before: Option<PageBreakSide>,
    break_after: Option<PageBreakSide>,
    width: f32,
    base_width: f32,
    flex_grow: f32,
    flex_shrink: f32,
    height: f32,
    natural_height: f32,
    has_explicit_width: bool,
    fragmentation: FlexItemFragmentation,
    align_self: AlignSelf,
    order: i32,
    source: FlexItemSource,
    main_constraints: SizeConstraints,
    cross_min: f32,
    cross_max: f32,
    is_flex_container: bool,
    is_table: bool,
    uses_general_layout: bool,
    contains_nested_forced_break: bool,
    margin_main_start_auto: bool,
    margin_main_end_auto: bool,
    margin_main_start: f32,
    margin_main_end: f32,
    margin_cross_start: f32,
    rel_left: f32,
    rel_top: f32,
    is_relative: bool,
    z_index: ZIndex,
    aspect_ratio: Option<f32>,
}

/// Containing-block and static-position state shared by every out-of-flow child
/// of one flex container.
#[derive(Clone, Copy)]
struct AbsoluteFlexContext<'a, 'dom> {
    container_style: &'a ComputedStyle,
    layout: &'a LayoutContext,
    ancestors: &'a [AncestorInfo<'dom>],
    inner_width: f32,
    content_height: f32,
    containing_block: Option<ContainingBlock>,
    positioned_depth: usize,
}

impl AbsoluteFlexContext<'_, '_> {
    fn layout_element(
        self,
        element: &ElementNode,
        style: &ComputedStyle,
        source_position: &ElementSiblingPosition,
        env: &mut LayoutEnv,
        output: &mut Vec<LayoutNode>,
    ) {
        let child_outer_width = style
            .width
            .map(|width| flex_style_outer_width(style, width))
            .unwrap_or_default();
        let child_outer_height = style
            .height
            .map(|height| flex_style_outer_height(style, height))
            .or_else(|| {
                style
                    .width
                    .zip(style.aspect_ratio)
                    .map(|(width, ratio)| flex_style_outer_width(style, width) / ratio)
            })
            .unwrap_or_default();
        let static_main_free = (self.inner_width - child_outer_width).max(0.0);
        let mut static_x = match self.container_style.justify_content {
            JustifyContent::FlexStart | JustifyContent::SpaceBetween => 0.0,
            JustifyContent::FlexEnd => static_main_free,
            JustifyContent::Center
            | JustifyContent::SafeCenter
            | JustifyContent::SpaceAround
            | JustifyContent::SpaceEvenly => static_main_free / 2.0,
        };
        let row_mirrors = self.container_style.flex_direction.is_row()
            && ((self.container_style.flex_direction == FlexDirection::RowReverse)
                ^ self.container_style.direction_rtl);
        if row_mirrors {
            static_x = self.inner_width - static_x - child_outer_width;
        }
        let effective_align = match style.align_self {
            AlignSelf::Auto => self.container_style.align_items,
            AlignSelf::FlexStart => AlignItems::FlexStart,
            AlignSelf::FlexEnd => AlignItems::FlexEnd,
            AlignSelf::Center => AlignItems::Center,
            AlignSelf::Baseline => AlignItems::Baseline,
            AlignSelf::Stretch => AlignItems::Stretch,
        };
        let static_cross_free = (self.content_height - child_outer_height).max(0.0);
        let static_y = match effective_align {
            AlignItems::FlexStart | AlignItems::Baseline | AlignItems::Stretch => 0.0,
            AlignItems::FlexEnd => static_cross_free,
            AlignItems::Center => static_cross_free / 2.0,
        } + self.container_style.border.top.used_width();
        let child_context = self
            .layout
            .with_parent_and_basis(
                self.inner_width,
                self.inner_width,
                Some(self.content_height),
                self.container_style.font_size,
            )
            .with_containing_block(self.containing_block);
        let mut children = Vec::new();
        flatten_element(
            element,
            LayoutTreeContext::new(self.container_style, &child_context, self.ancestors)
                .with_positioned_ancestor_depth(self.positioned_depth)
                .for_element(source_position.as_context()),
            &mut children,
            env,
        );
        if let Some(containing_block) = self.containing_block {
            crate::layout::helpers::resolve_absolute_descendants_containing_block(
                &mut children,
                containing_block,
            );
        }
        for child in &mut children {
            let Some(positioning) = child
                .positioning_owner_mut()
                .map(|owner| owner.positioning_mut())
            else {
                continue;
            };
            if style.left.is_none() && style.right.is_none() {
                positioning.insets.left += static_x;
            }
            if style.top.is_none() && style.bottom.is_none() {
                positioning.insets.top += static_y;
            }
        }
        output.extend(children);
    }

    fn layout_generated(
        self,
        generated: GeneratedBox<'_>,
        env: &mut LayoutEnv,
        output: &mut Vec<LayoutNode>,
    ) {
        let style = generated.style();
        let counter_scope = env.counter_state.enter_element(style);
        output.push(super::helpers::build_pseudo_block(
            style,
            generated.originating_element(),
            PseudoBoxContext::new(
                self.inner_width,
                env.fonts,
                env.filter_defs,
                &mut *env.resources,
            )
            .with_containing_block(self.containing_block)
            .with_positioned_ancestor_depth(self.positioned_depth),
            env.counter_state,
            style.display == Display::ListItem,
        ));
        env.counter_state.leave_element(counter_scope);
    }
}

/// Each child is laid out as a TextBlock at a computed position. The container
/// emits one TextBlock per flex item with an `offset_left` / `offset_top` that
/// encodes its position inside the flex row/column. The container itself emits
/// a wrapper TextBlock for its background/border first, then the items.
#[allow(clippy::too_many_arguments)]
/// Max-content border-box width of layout-capable descendants inside a flex
/// item's flattened content. Used to shrink-wrap a `flex: 0 0 auto` item around
/// nested tables and replaced descendants instead of keeping the equal-share
/// fallback and then constraining those children down to that accidental width.
fn flex_probe_outer_extent(elements: &[LayoutNode]) -> f32 {
    elements
        .iter()
        .filter_map(|element| element.inline_flow_extent()?.max_content_outer_extent())
        .fold(0.0, f32::max)
}

fn update_container_size(
    element: &mut dyn LayoutElement,
    width: Option<f32>,
    height: Option<f32>,
) -> bool {
    struct SizeUpdate {
        width: Option<f32>,
        height: Option<f32>,
        updated: bool,
    }

    impl LayoutVisitorMut for SizeUpdate {
        fn visit_container(&mut self, element: &mut Container) {
            if let Some(width) = self.width {
                element.box_model.size.width = InlineSize::fixed(width);
            }
            if let Some(height) = self.height {
                element.box_model.size.height = BlockSize::definite(height);
            }
            self.updated = true;
        }
    }

    let mut update = SizeUpdate {
        width,
        height,
        updated: false,
    };
    element.accept_mut(&mut update);
    update.updated
}

fn text_block_background_height(element: &dyn LayoutElement) -> Option<f32> {
    struct BackgroundHeight(Option<f32>);

    impl LayoutVisitor for BackgroundHeight {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = element
                .box_model
                .size
                .height
                .used()
                .map(|height| height + element.box_model.border.vertical_width());
        }
    }

    let mut height = BackgroundHeight(None);
    element.accept(&mut height);
    height.0
}

fn text_block_border_height(element: &dyn LayoutElement) -> Option<f32> {
    struct BorderHeight(Option<f32>);

    impl LayoutVisitor for BorderHeight {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = Some(element.box_model.border.vertical_width());
        }
    }

    let mut height = BorderHeight(None);
    element.accept(&mut height);
    height.0
}

fn update_text_block_height(element: &mut dyn LayoutElement, height: f32) -> bool {
    struct HeightUpdate {
        height: f32,
        updated: bool,
    }

    impl LayoutVisitorMut for HeightUpdate {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            element.box_model.size.height = BlockSize::definite(self.height);
            self.updated = true;
        }
    }

    let mut update = HeightUpdate {
        height,
        updated: false,
    };
    element.accept_mut(&mut update);
    update.updated
}

fn update_text_block_layout(
    element: &mut dyn LayoutElement,
    lines: Option<Vec<TextLine>>,
    width: f32,
    height: Option<f32>,
    clip_height: Option<f32>,
) -> bool {
    struct LayoutUpdate {
        lines: Option<Vec<TextLine>>,
        width: f32,
        height: Option<f32>,
        clip_height: Option<f32>,
        updated: bool,
    }

    impl LayoutVisitorMut for LayoutUpdate {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            if let Some(lines) = self.lines.take() {
                element.lines = lines;
            }
            element.box_model.size.width = InlineSize::fixed(self.width);
            element.box_model.size.height = BlockSize::from_definite(self.height);
            if let Some(clip_height) = self.clip_height {
                element.clipping.rect = Some(Rect::from_xywh(0.0, 0.0, self.width, clip_height));
            }
            self.updated = true;
        }
    }

    let mut update = LayoutUpdate {
        lines,
        width,
        height,
        clip_height,
        updated: false,
    };
    element.accept_mut(&mut update);
    update.updated
}

fn is_borderless_text_block(element: &dyn LayoutElement) -> bool {
    struct BorderlessText(bool);

    impl LayoutVisitor for BorderlessText {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = !element.box_model.border.has_any();
        }
    }

    let mut result = BorderlessText(false);
    element.accept(&mut result);
    result.0
}

fn is_clipped_text_block(element: &dyn LayoutElement) -> bool {
    struct ClippedText(bool);

    impl LayoutVisitor for ClippedText {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = element.clipping.rect.is_some();
        }
    }

    let mut result = ClippedText(false);
    element.accept(&mut result);
    result.0
}

fn merge_text_block_into_cell(
    element: &dyn LayoutElement,
    merged_lines: &mut Vec<TextLine>,
    first_background: &mut Option<crate::types::Color>,
    first_padding: &mut EdgeSizes,
    first_radii: &mut CornerRadii,
    is_first: &mut bool,
) {
    struct Merger<'a> {
        merged_lines: &'a mut Vec<TextLine>,
        first_background: &'a mut Option<crate::types::Color>,
        first_padding: &'a mut EdgeSizes,
        first_radii: &'a mut CornerRadii,
        is_first: &'a mut bool,
    }

    impl LayoutVisitor for Merger<'_> {
        fn visit_text_block(&mut self, element: &TextBlock) {
            if *self.is_first {
                *self.first_background = element.paint.background.color;
                *self.first_padding = element.box_model.padding;
                *self.first_radii = element.paint.border_radii;
                *self.is_first = false;
            }
            if !self.merged_lines.is_empty() && element.box_model.margins.start > 0.0 {
                self.merged_lines.push(TextLine {
                    runs: Vec::new(),
                    height: element.box_model.margins.start,
                    baseline_ascent: None,
                    x_offset: 0.0,
                    metadata: Default::default(),
                });
            }
            self.merged_lines.extend(element.lines.iter().cloned());
        }
    }

    element.accept(&mut Merger {
        merged_lines,
        first_background,
        first_padding,
        first_radii,
        is_first,
    });
}

fn is_empty_pullback_spacer(element: &dyn LayoutElement) -> bool {
    struct Pullback(bool);

    impl LayoutVisitor for Pullback {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = element.lines.is_empty() && element.box_model.margins.start < 0.0;
        }
    }

    let mut pullback = Pullback(false);
    element.accept(&mut pullback);
    pullback.0
}

fn flex_item_content_height(element: &dyn LayoutElement) -> f32 {
    struct ContentHeight(Option<f32>);

    impl LayoutVisitor for ContentHeight {
        fn visit_text_block(&mut self, element: &TextBlock) {
            let text_height = element.lines.iter().map(|line| line.height).sum::<f32>();
            let natural = element.box_model.padding.vertical()
                + text_height
                + element.box_model.border.vertical_width();
            self.0 = Some(
                element
                    .box_model
                    .size
                    .height
                    .used()
                    .map_or(natural, |height| natural.max(height)),
            );
        }

        fn visit_flex_row(&mut self, element: &FlexRow) {
            let content_height = element
                .box_model
                .size
                .height
                .resolve(element.content.row_height);
            self.0 = Some(
                element.box_model.margins.total()
                    + element.box_model.padding.vertical()
                    + content_height
                    + element.box_model.border.vertical_width(),
            );
        }
    }

    let mut height = ContentHeight(None);
    element.accept(&mut height);
    height.0.unwrap_or_else(|| estimate_element_height(element))
}

fn flex_cell_from_text_block(
    element: &dyn LayoutElement,
    x_offset: f32,
    y_offset: f32,
    width: f32,
    is_positioned: bool,
    z_index: ZIndex,
) -> Option<FlexCell> {
    struct CellBuilder {
        x_offset: f32,
        y_offset: f32,
        width: f32,
        is_positioned: bool,
        z_index: ZIndex,
        cell: Option<FlexCell>,
    }

    impl LayoutVisitor for CellBuilder {
        fn visit_text_block(&mut self, element: &TextBlock) {
            let text_height = element.lines.iter().map(|line| line.height).sum::<f32>();
            let natural_content_height = element.box_model.padding.vertical()
                + text_height
                + element.box_model.border.vertical_width();
            let natural_height = element
                .box_model
                .size
                .height
                .used()
                .map(|height| height + element.box_model.border.vertical_width())
                .unwrap_or(natural_content_height);
            let mut positioning = element.positioning.clone();
            if self.is_positioned && positioning.scheme == Position::Static {
                positioning.scheme = Position::Relative;
            }
            let mut paint = element.paint.clone();
            paint.group.stacking.z_index = self.z_index;
            paint.group.stacking.role = StackingRole::FlexItem;
            self.cell = Some(FlexCell {
                lines: element.lines.clone(),
                x_offset: self.x_offset,
                y_offset: self.y_offset,
                width: self.width,
                text_align: element.text.alignment,
                padding: element.box_model.padding,
                border: element.box_model.border,
                natural_height,
                line_cross_size: natural_height,
                fragmentation: FlexItemFragmentation {
                    block_size: super::engine::FlexItemBlockSize::Definite,
                    box_fragmentation: element.fragmentation.box_fragmentation,
                    fragment_block_extent: None,
                },
                align_self: AlignSelf::FlexStart,
                paint: crate::layout::cells::CellPaint {
                    box_paint: paint,
                    ..Default::default()
                },
                positioning,
                ..Default::default()
            });
        }
    }

    let mut builder = CellBuilder {
        x_offset,
        y_offset,
        width,
        is_positioned,
        z_index,
        cell: None,
    };
    element.accept(&mut builder);
    builder.cell
}

/// Make one formatting-context cell the sole owner of a structured flex
/// item's post-layout paint group.
///
/// A structured item is stored as nested layout nodes, but filters and other
/// item-level compositing attach to the cell. Moving the principal node's group
/// to that cell gives ordinary paint, filtered replacement paint, and recursive
/// descendants the same transform/effect scope instead of leaving two possible
/// owners for one CSS box.
fn flex_cell_with_nested_item(elements: &[LayoutNode], mut cell: FlexCell) -> FlexCell {
    cell.nested_elements = elements.to_vec();
    for element in &mut cell.nested_elements {
        let Some(owner) = element.paint_group_owner_mut() else {
            continue;
        };
        cell.paint.box_paint.group = std::mem::take(owner.paint_group_mut());
        break;
    }
    cell.paint.box_paint.group.stacking.role = StackingRole::FlexItem;
    cell
}

fn flex_item_positioning(elements: &[LayoutNode], is_positioned: bool) -> Positioning {
    let mut positioning = elements
        .iter()
        .find_map(|element| element.positioning_owner())
        .map(|owner| owner.positioning().clone())
        .unwrap_or_default();
    if is_positioned && positioning.scheme == Position::Static {
        positioning.scheme = Position::Relative;
    }
    positioning
}

fn flex_cell_positioning(
    cell: &FlexCell,
    elements: &[LayoutNode],
    is_positioned: bool,
) -> Positioning {
    if cell.nested_elements.is_empty() {
        return flex_item_positioning(elements, is_positioned);
    }

    Positioning::default().with_scheme(if is_positioned {
        Position::Relative
    } else {
        Position::Static
    })
}

#[allow(clippy::too_many_arguments)]
fn flex_row_node(
    style: &ComputedStyle,
    cells: Vec<FlexCell>,
    forced_line_breaks: Vec<ForcedFlexLineBreak>,
    fragment_role: super::engine::FlexFragmentRole,
    row_height: f32,
    margins: BlockMargins,
    inline_offset: f32,
    width: f32,
    paint_height: f32,
    alignment: AlignItems,
    containing_block_depth: usize,
) -> LayoutNode {
    let size = crate::layout::elements::LayoutSize::fixed(width, None);
    let mut paint = crate::layout::elements::BoxPaint::from_style(style, size);
    paint.border_radii = style.resolve_corner_radii(width, paint_height);
    FlexRow {
        content: FlexContent {
            cells,
            forced_line_breaks,
            fragment_role,
            row_height,
            alignment,
        },
        box_model: crate::layout::elements::BoxModel {
            size,
            margins,
            padding: style.padding,
            border: LayoutBorder::from_computed(&style.border, style.color),
        },
        paint,
        positioning: crate::layout::elements::Positioning::from_style(style)
            .with_containing_block_depth(containing_block_depth),
        inline_offset: crate::layout::elements::InlineOffset::new(inline_offset),
        overflow: crate::layout::elements::OverflowBehavior {
            combined: style.overflow,
            x: style.overflow_x,
            y: style.overflow_y,
        },
    }
    .boxed()
}

#[derive(Debug, Clone, Copy, Default)]
struct ColumnTextMetrics {
    is_text: bool,
    margins: BlockMargins,
    border_height: f32,
    height: Option<f32>,
}

impl LayoutVisitor for ColumnTextMetrics {
    fn visit_text_block(&mut self, element: &TextBlock) {
        self.is_text = true;
        self.margins = element.box_model.margins;
        self.border_height = element.box_model.border.vertical_width();
        self.height = element.box_model.size.height.used();
    }
}

fn column_text_metrics(element: &dyn LayoutElement) -> ColumnTextMetrics {
    let mut metrics = ColumnTextMetrics::default();
    element.accept(&mut metrics);
    metrics
}

fn adapt_column_text_block(
    element: &mut dyn LayoutElement,
    margins: BlockMargins,
    width: Option<f32>,
    height: Option<f32>,
    inline_offset: f32,
    force_relative: bool,
) {
    struct Adapter {
        margins: BlockMargins,
        width: Option<f32>,
        height: Option<f32>,
        inline_offset: f32,
        force_relative: bool,
    }

    impl LayoutVisitorMut for Adapter {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            element.box_model.margins = self.margins;
            element.box_model.size.width = InlineSize::from_fixed_value(self.width);
            element.box_model.size.height = BlockSize::from_definite(self.height);
            element.flow = Default::default();
            if self.force_relative {
                element.positioning.scheme = Position::Relative;
            }
            element.positioning.insets = EdgeSizes::new(0.0, 0.0, 0.0, self.inline_offset);
            element.positioning.containing_block = None;
            element.paint.group.stacking = Default::default();
            element.positioning.containing_block_depth = 0;
            element.semantics = Default::default();
        }
    }

    element.accept_mut(&mut Adapter {
        margins,
        width,
        height,
        inline_offset,
        force_relative,
    });
}

fn prepare_continuation_background(element: &mut dyn LayoutElement, height: f32) -> f32 {
    struct Continuation {
        height: f32,
        flow_height: f32,
    }

    impl LayoutVisitorMut for Continuation {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            element.box_model.margins.start = 0.0;
            element.box_model.padding.top = 0.0;
            element.box_model.border.top.width = 0.0;
            element.paint.border_radii = element.paint.border_radii.clear_top();
            element.box_model.size.height = BlockSize::fragment(self.height);
            self.flow_height = self.height + element.box_model.border.vertical_width();
        }
    }

    let mut continuation = Continuation {
        height,
        flow_height: height,
    };
    element.accept_mut(&mut continuation);
    continuation.flow_height
}

fn set_text_block_start_margin(element: &mut dyn LayoutElement, margin: f32) {
    struct MarginUpdate(f32);

    impl LayoutVisitorMut for MarginUpdate {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            element.box_model.margins.start = self.0;
        }
    }

    element.accept_mut(&mut MarginUpdate(margin));
}

fn flex_style_outer_width(style: &ComputedStyle, content_width: f32) -> f32 {
    if style.box_sizing == BoxSizing::ContentBox {
        content_width + style.padding.horizontal() + style.border.horizontal_width()
    } else {
        content_width
    }
}

fn flex_style_outer_height(style: &ComputedStyle, content_height: f32) -> f32 {
    if style.box_sizing == BoxSizing::ContentBox {
        content_height + style.padding.vertical() + style.border.vertical_width()
    } else {
        content_height
    }
}

/// `flatten_element` normally emits forced breaks around the box it receives.
/// A row flex item's breaks instead belong to its flex line (CSS Flexbox §10),
/// so remove only those outer markers before the item becomes paint-only nested
/// content. Breaks created by descendants remain inside the item.
fn strip_row_flex_item_break_markers(
    elements: &mut Vec<LayoutNode>,
    break_before: bool,
    break_after: bool,
) {
    if break_before
        && elements
            .first()
            .is_some_and(|element| is_page_break(element.as_ref()))
    {
        elements.remove(0);
    }
    if break_after
        && elements
            .last()
            .is_some_and(|element| is_page_break(element.as_ref()))
    {
        elements.pop();
    }
}

fn is_page_break(element: &dyn LayoutElement) -> bool {
    struct IsPageBreak(bool);

    impl LayoutVisitor for IsPageBreak {
        fn visit_page_break(&mut self, _element: &crate::layout::elements::PageBreak) {
            self.0 = true;
        }
    }

    let mut visitor = IsPageBreak(false);
    element.accept(&mut visitor);
    visitor.0
}

fn contains_page_break(element: &dyn LayoutElement) -> bool {
    if is_page_break(element) {
        return true;
    }
    let mut found = false;
    element.visit_children(&mut |child| {
        if !found {
            found = contains_page_break(child);
        }
    });
    found
}

fn sequence_contains_page_break(elements: &[LayoutNode]) -> bool {
    elements
        .iter()
        .any(|element| contains_page_break(element.as_ref()))
}

/// The two widths an intrinsic flex base needs in the browser print model.
///
/// Its painted flex base is CSS-pixel snapped, while its inline formatter still
/// uses the exact shaped advance. Keeping both values together prevents the
/// painted box from being correct at the cost of changing line breaks.
#[derive(Clone, Copy)]
struct FlexIntrinsicWidth {
    paint: f32,
    text_wrap: f32,
}

/// One item's main-axis inputs and resolved target size from Flexbox section
/// 9.7. Keeping the unclamped flex base separate from the target is essential:
/// min/max constraints are ignored while finding the base, then applied while
/// freezing and resolving flexible lengths.
#[derive(Clone, Copy, Debug)]
struct FlexibleLength {
    base: f32,
    target: f32,
    constraints: SizeConstraints,
    grow: f32,
    shrink: f32,
    fixed_outer: f32,
    frozen: bool,
    violation: f32,
}

impl FlexibleLength {
    fn new(
        base: f32,
        constraints: SizeConstraints,
        grow: f32,
        shrink: f32,
        fixed_outer: f32,
    ) -> Self {
        Self {
            base,
            target: base,
            constraints,
            grow,
            shrink,
            fixed_outer,
            frozen: false,
            violation: 0.0,
        }
    }

    fn hypothetical(self) -> f32 {
        self.constraints.constrain(self.base)
    }

    fn outer_base(self) -> f32 {
        self.base + self.fixed_outer
    }

    fn outer_target(self) -> f32 {
        self.target + self.fixed_outer
    }
}

#[derive(Clone, Copy)]
enum FlexResolutionMode {
    Grow,
    Shrink,
}

impl FlexResolutionMode {
    fn factor(self, item: FlexibleLength) -> f32 {
        match self {
            Self::Grow => item.grow,
            Self::Shrink => item.shrink,
        }
    }
}

/// Resolve a flex line's used main sizes according to CSS Flexbox section 9.7.
/// `available` excludes the line's fixed `gap` space; fixed item margins remain
/// part of each item's outer size throughout the algorithm.
fn resolve_flexible_lengths(items: &mut [FlexibleLength], available: f32) {
    let hypothetical_outer: f32 = items
        .iter()
        .map(|item| item.hypothetical() + item.fixed_outer)
        .sum();
    let mode = if hypothetical_outer < available {
        FlexResolutionMode::Grow
    } else {
        FlexResolutionMode::Shrink
    };

    for item in items.iter_mut() {
        let hypothetical = item.hypothetical();
        let inflexible = mode.factor(*item) <= 0.0
            || match mode {
                FlexResolutionMode::Grow => item.base > hypothetical,
                FlexResolutionMode::Shrink => item.base < hypothetical,
            };
        if inflexible {
            item.target = hypothetical;
            item.frozen = true;
        }
    }

    let initial_free_space = available
        - items
            .iter()
            .map(|item| {
                if item.frozen {
                    item.outer_target()
                } else {
                    item.outer_base()
                }
            })
            .sum::<f32>();

    for _ in 0..=items.len() {
        if items.iter().all(|item| item.frozen) {
            break;
        }

        let mut remaining = available
            - items
                .iter()
                .map(|item| {
                    if item.frozen {
                        item.outer_target()
                    } else {
                        item.outer_base()
                    }
                })
                .sum::<f32>();
        let factor_sum: f32 = items
            .iter()
            .filter(|item| !item.frozen)
            .map(|item| mode.factor(*item))
            .sum();
        if factor_sum < 1.0 {
            let scaled = initial_free_space * factor_sum;
            if scaled.abs() < remaining.abs() {
                remaining = scaled;
            }
        }

        match mode {
            FlexResolutionMode::Grow if factor_sum > 0.0 => {
                for item in items.iter_mut().filter(|item| !item.frozen) {
                    item.target = item.base + remaining * item.grow / factor_sum;
                }
            }
            FlexResolutionMode::Shrink => {
                let scaled_sum: f32 = items
                    .iter()
                    .filter(|item| !item.frozen)
                    .map(|item| item.shrink * item.base.max(0.0))
                    .sum();
                if scaled_sum > 0.0 {
                    for item in items.iter_mut().filter(|item| !item.frozen) {
                        let scaled = item.shrink * item.base.max(0.0);
                        item.target = item.base - remaining.abs() * scaled / scaled_sum;
                    }
                }
            }
            FlexResolutionMode::Grow => {}
        }

        let mut total_violation = 0.0;
        for item in items.iter_mut().filter(|item| !item.frozen) {
            let unclamped = item.target;
            item.target = item.constraints.constrain(unclamped).max(0.0);
            item.violation = item.target - unclamped;
            total_violation += item.violation;
        }

        if equal_with_roundoff(total_violation, 0.0) {
            for item in items.iter_mut().filter(|item| !item.frozen) {
                item.frozen = true;
            }
        } else {
            let freeze_min_violations = total_violation > 0.0;
            for item in items.iter_mut().filter(|item| !item.frozen) {
                let is_violation = if freeze_min_violations {
                    item.violation > 0.0
                } else {
                    item.violation < 0.0
                };
                if is_violation {
                    item.frozen = true;
                }
            }
        }
    }

    for item in items.iter_mut() {
        item.target = item.constraints.constrain(item.target).max(0.0);
    }
}

impl FlexIntrinsicWidth {
    fn from_content(style: &ComputedStyle, content_width: f32) -> Self {
        let text_wrap =
            content_width + style.padding.horizontal() + style.border.horizontal_width();
        Self {
            paint: crate::fonts::round_to_css_pixel(text_wrap),
            text_wrap,
        }
    }
}

fn flex_direct_text_width(
    text: &str,
    style: &ComputedStyle,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    let family = resolve_style_font_family(style, fonts);
    let mut width = 0.0;
    let mut first = true;
    for word in text
        .split(is_collapsible_space)
        .filter(|word| !word.is_empty())
    {
        if !first {
            width += estimate_word_width(
                " ",
                style.font_size,
                &family,
                style.font_weight == FontWeight::Bold,
                style.font_style.is_slanted(),
                fonts,
            );
        }
        width += estimate_word_width(
            word,
            style.font_size,
            &family,
            style.font_weight == FontWeight::Bold,
            style.font_style.is_slanted(),
            fonts,
        );
        first = false;
    }
    width
}

fn authored_align_items_last_baseline(
    el: &ElementNode,
    ancestors: &[AncestorInfo],
    rules: &[CssRule],
) -> bool {
    let classes = el.class_list();
    let selector_ctx = crate::layout::helpers::selector_context_from_ancestors(ancestors, el);
    let mut best: Option<(bool, u32, usize, bool)> = None;
    for (source_idx, rule) in rules.iter().enumerate() {
        if rule.pseudo_element.is_some()
            || !crate::parser::css::selector_matches_with_context(
                &rule.selector,
                el.tag_name(),
                &classes,
                el.id(),
                &el.attributes,
                &selector_ctx,
            )
        {
            continue;
        }
        let Some(CssValue::Keyword(raw)) = rule.declarations.properties.get("align-items") else {
            continue;
        };
        let important = rule
            .declarations
            .important
            .get("align-items")
            .copied()
            .unwrap_or(false);
        let specificity = crate::parser::css::specificity(&rule.selector);
        let is_last = raw.trim().eq_ignore_ascii_case("last baseline");
        if best.is_none_or(|(best_important, best_spec, best_source, _)| {
            (important, specificity, source_idx) >= (best_important, best_spec, best_source)
        }) {
            best = Some((important, specificity, source_idx, is_last));
        }
    }
    if let Some(inline) = el
        .style_attr()
        .map(crate::parser::css::parse_inline_style)
        .and_then(|map| match map.properties.get("align-items") {
            Some(CssValue::Keyword(raw)) => Some(raw.trim().eq_ignore_ascii_case("last baseline")),
            _ => None,
        })
    {
        return inline;
    }
    best.is_some_and(|(_, _, _, is_last)| is_last)
}

#[allow(clippy::too_many_arguments)]
fn flex_intrinsic_container_width(
    el: &ElementNode,
    style: &ComputedStyle,
    available_width: f32,
    ancestors: &[AncestorInfo],
    env: &LayoutEnv,
) -> Option<f32> {
    let keyword = style.width_keyword?;
    if !style.flex_direction.is_row() {
        return None;
    }

    let mut contributions = Vec::new();
    let mut element_idx = 0usize;
    let element_siblings: Vec<&ElementNode> = el
        .children
        .iter()
        .filter_map(|node| match node {
            DomNode::Element(element) => Some(element),
            DomNode::Text(_) => None,
        })
        .collect();
    let sibling_positions: Vec<ElementSiblingPosition> = (0..element_siblings.len())
        .map(|index| ElementSiblingPosition::from_element_siblings(&element_siblings, index))
        .collect();

    for child in &el.children {
        match child {
            DomNode::Text(text) => {
                if text
                    .split(is_collapsible_space)
                    .any(|word| !word.is_empty())
                {
                    contributions.push(flex_direct_text_width(text, style, env.fonts));
                }
            }
            DomNode::Element(child_el) => {
                let classes = child_el.class_list();
                let selector_ctx = sibling_positions[element_idx]
                    .as_context()
                    .selector_context(ancestors, child_el.children.is_empty());
                element_idx += 1;
                let child_style = compute_style_with_context_with_font_metrics(
                    child_el.tag,
                    child_el.style_attr(),
                    style,
                    env.rules,
                    child_el.tag_name(),
                    &classes,
                    child_el.id(),
                    &child_el.attributes,
                    &selector_ctx,
                    env.font_metrics(),
                );
                if child_style.display == Display::None || child_style.position.is_absolute() {
                    continue;
                }
                let preferred_width = child_style.flex_basis.definite_length().or_else(|| {
                    child_style
                        .flex_basis
                        .content_keyword()
                        .is_none()
                        .then_some(child_style.width)
                        .flatten()
                });
                let width = preferred_width
                    .map(|w| flex_style_outer_width(&child_style, w))
                    .or_else(|| {
                        child_style
                            .height
                            .zip(child_style.aspect_ratio)
                            .map(|(h, ratio)| flex_style_outer_height(&child_style, h) * ratio)
                    })
                    .unwrap_or_else(|| {
                        let content_basis = child_style.flex_basis.content_keyword();
                        let widths = if content_basis.is_some() {
                            crate::layout::intrinsic_width::content_intrinsic_border_box_widths(
                                child_el,
                                &child_style,
                                env.rules,
                                env.fonts,
                                ancestors,
                                &selector_ctx,
                            )
                        } else {
                            crate::layout::intrinsic_width::intrinsic_border_box_widths(
                                child_el,
                                &child_style,
                                env.rules,
                                env.fonts,
                                ancestors,
                                &selector_ctx,
                            )
                        };
                        widths.resolve(content_basis.unwrap_or(keyword), available_width)
                    })
                    + child_style.margin.horizontal();
                contributions.push(width);
            }
        }
    }

    if contributions.is_empty() {
        return None;
    }

    let main_gap = if style.flex_direction.is_row() {
        style.column_gap.max(style.gap)
    } else {
        style.row_gap.max(style.gap)
    };
    let gaps = main_gap * contributions.len().saturating_sub(1) as f32;
    let content = match keyword {
        IntrinsicWidthKeyword::MinContent if style.flex_wrap.wraps() => {
            contributions.into_iter().fold(0.0f32, f32::max)
        }
        IntrinsicWidthKeyword::MinContent | IntrinsicWidthKeyword::MaxContent => {
            contributions.into_iter().sum::<f32>() + gaps
        }
        IntrinsicWidthKeyword::FitContent => {
            let max_content = contributions.into_iter().sum::<f32>() + gaps;
            max_content.min(available_width.max(0.0))
        }
    };

    Some(content + style.padding.horizontal() + style.border.horizontal_width())
}

fn flex_cell_first_baseline(cell: &FlexCell) -> f32 {
    let Some(first) = cell
        .lines
        .iter()
        .find(|line| line.runs.iter().any(|run| !run.text.is_empty()))
    else {
        return cell.natural_height;
    };
    cell.border.top.width + cell.padding.top + first.baseline_ascent.unwrap_or(first.height)
}

fn apply_row_baseline_offsets(cells: &mut [FlexCell]) {
    let mut line_baselines = std::collections::HashMap::new();
    for cell in cells
        .iter()
        .filter(|cell| matches!(cell.align_self, AlignSelf::Auto | AlignSelf::Baseline))
    {
        let baseline = flex_cell_first_baseline(cell);
        line_baselines
            .entry(cell.line_id)
            .and_modify(|line_baseline: &mut f32| *line_baseline = line_baseline.max(baseline))
            .or_insert(baseline);
    }
    for cell in cells {
        if let Some(line_baseline) = line_baselines.get(&cell.line_id)
            && matches!(cell.align_self, AlignSelf::Auto | AlignSelf::Baseline)
        {
            cell.y_offset += (line_baseline - flex_cell_first_baseline(cell)).max(0.0);
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct FlexItemContents {
    structured: bool,
    out_of_flow: bool,
}

impl FlexItemContents {
    const fn requires_general_layout(self) -> bool {
        self.structured || self.out_of_flow
    }

    const fn only_out_of_flow_structure(self) -> bool {
        self.out_of_flow && !self.structured
    }
}

fn classify_flex_item_contents(
    item: &ElementNode,
    item_style: &ComputedStyle,
    ancestors: &[AncestorInfo],
    env: &LayoutEnv,
) -> FlexItemContents {
    let mut contents = FlexItemContents::default();
    let sibling_count = item
        .children
        .iter()
        .filter(|node| matches!(node, DomNode::Element(_)))
        .count();

    for (node_index, node) in item.children.iter().enumerate() {
        let DomNode::Element(child) = node else {
            continue;
        };
        let sibling = |node: &DomNode| match node {
            DomNode::Element(element) => Some((
                element.tag_name().to_string(),
                element
                    .class_list()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            )),
            DomNode::Text(_) => None,
        };
        let selector_ctx = SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index: item.children[..node_index]
                .iter()
                .filter(|node| matches!(node, DomNode::Element(_)))
                .count(),
            sibling_count,
            preceding_siblings: item.children[..node_index]
                .iter()
                .filter_map(sibling)
                .collect(),
            following_siblings: item.children[node_index + 1..]
                .iter()
                .filter_map(sibling)
                .collect(),
            is_empty: child.children.is_empty(),
        };
        let style = compute_style_with_context_with_font_metrics(
            child.tag,
            child.style_attr(),
            item_style,
            env.rules,
            child.tag_name(),
            &child.class_list(),
            child.id(),
            &child.attributes,
            &selector_ctx,
            env.font_metrics(),
        );
        if style.display == Display::None {
            continue;
        }
        // Out-of-flow descendants still require the item's general block
        // formatting context: it owns their containing-block resolution and
        // emits them separately from the item's in-flow inline runs. The
        // text-only flex shortcut intentionally cannot represent that split.
        if style.position.is_absolute() {
            contents.out_of_flow = true;
            continue;
        }
        if matches!(child.tag, HtmlTag::Img | HtmlTag::Svg | HtmlTag::Table)
            || matches!(
                style.display,
                Display::Block
                    | Display::ListItem
                    | Display::InlineBlock
                    | Display::Flex
                    | Display::InlineFlex
                    | Display::Grid
                    | Display::InlineGrid
                    | Display::Table
                    | Display::InlineTable
                    | Display::TableRowGroup
                    | Display::TableHeaderGroup
                    | Display::TableFooterGroup
                    | Display::TableRow
                    | Display::TableCell
                    | Display::TableColumnGroup
                    | Display::TableColumn
                    | Display::TableCaption
            )
        {
            contents.structured = true;
        }
    }
    contents
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_flex_container(
    el: &ElementNode,
    style: &ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutNode>,
    ancestors: &[AncestorInfo],
    generated_content: GeneratedInlineContent<'_>,
    positioned_depth: usize,
    env: &mut LayoutEnv,
) {
    let flex_writing_mode = style.writing_mode;
    let available_width = ctx.available_width();
    // Keep the border-box and content-box contracts explicit. In particular,
    // an authored `border-box` width is already the outer width, while a
    // `content-box` width must grow by its padding and border. Treating the
    // resolved item content width as the flex container's outer width silently
    // removed its edges from nested flex containers.
    let dimensions = ResolvedBoxDimensions::from_style(style, Size::new(available_width, 0.0));
    let mut block_w = if style.width_keyword.is_some() {
        flex_intrinsic_container_width(el, style, available_width, ancestors, env)
            .unwrap_or(dimensions.border_box.width)
    } else {
        dimensions.border_box.width
    };
    block_w = block_w.max(style.padding.horizontal() + style.border.horizontal_width());

    // Horizontal offset of the flex container's border box from the containing
    // block's content-left edge. A flex container is a block-level box, so it
    // honours its own `margin-left` (and `margin: 0 auto` centering) exactly
    // like `block.rs` does for normal blocks. Without this the top-level
    // renderer painted every flex container flush at the page content-left,
    // dropping its horizontal margin (vertical margin was already applied via
    // `margin_top`/`margin_bottom`). Centering only applies when the container
    // has a definite width narrower than the available space.
    let h_offset =
        crate::layout::elements::InlineOffset::resolve_block_start(style, available_width, block_w)
            .value();

    // `block_w` is normalized to the border box for both box-sizing modes.
    let inner_width =
        (block_w - style.border.horizontal_width() - style.padding.horizontal()).max(0.0);

    // Collect child elements and lay each one out into a temporary buffer.
    // Per CSS Flexbox §4.1, an absolutely-positioned child of a flex container
    // does NOT participate in flex layout (it is taken out of flow). We collect
    // such children separately and emit them as positioned boxes anchored to the
    // flex container's padding box, while the in-flow children become flex items.
    let all_child_elements: Vec<&ElementNode> = el
        .children
        .iter()
        .filter_map(|c| {
            if let DomNode::Element(e) = c {
                Some(e)
            } else {
                None
            }
        })
        .collect();
    let total_child_count = all_child_elements.len();
    let all_child_positions: Vec<ElementSiblingPosition> = (0..total_child_count)
        .map(|index| ElementSiblingPosition::from_element_siblings(&all_child_elements, index))
        .collect();
    // Identify which children are absolutely/fixed positioned (out of flow). We
    // compute each child's position against the container style here; full styles
    // for in-flow items are recomputed in the item loop below.
    let child_is_abs: Vec<bool> = all_child_elements
        .iter()
        .enumerate()
        .map(|(idx, child_el)| {
            let classes = child_el.class_list();
            let selector_ctx = all_child_positions[idx]
                .as_context()
                .selector_context(ancestors, child_el.children.is_empty());
            let cs = compute_style_with_context_with_font_metrics(
                child_el.tag,
                child_el.style_attr(),
                style,
                env.rules,
                child_el.tag_name(),
                &classes,
                child_el.id(),
                &child_el.attributes,
                &selector_ctx,
                env.font_metrics(),
            );
            cs.position.is_absolute()
        })
        .collect();
    // In-flow flex items (abs children excluded). Text directly inside a flex
    // container is wrapped in an anonymous block flex item (css-flexbox-1 §4).
    let mut flex_child_storage: Vec<ElementNode> = Vec::new();
    let mut flex_child_positions: Vec<ElementSiblingPosition> = Vec::new();
    let mut flex_child_is_absolute = Vec::new();
    let mut element_idx = 0usize;
    for child in &el.children {
        match child {
            DomNode::Element(child_el) => {
                flex_child_storage.push((*child_el).clone());
                flex_child_positions.push(all_child_positions[element_idx].clone());
                flex_child_is_absolute.push(child_is_abs[element_idx]);
                element_idx += 1;
            }
            DomNode::Text(text) => {
                if text
                    .split(is_collapsible_space)
                    .any(|word| !word.is_empty())
                {
                    let mut anon = ElementNode::new(HtmlTag::Div);
                    anon.children.push(DomNode::Text(text.clone()));
                    flex_child_storage.push(anon);
                    // Anonymous flex items have no authored sibling identity and
                    // therefore must not inherit the source position of a nearby
                    // element merely because they share a box list.
                    flex_child_positions.push(ElementSiblingPosition::default());
                    flex_child_is_absolute.push(false);
                }
            }
        }
    }
    let child_elements: Vec<&ElementNode> = flex_child_storage.iter().collect();
    let generated_before = generated_content
        .before()
        .filter(|generated| !generated.style().position.is_absolute());
    let generated_after = generated_content
        .after()
        .filter(|generated| !generated.style().position.is_absolute());
    let child_count = flex_child_is_absolute
        .iter()
        .filter(|is_absolute| !**is_absolute)
        .count()
        + usize::from(generated_before.is_some())
        + usize::from(generated_after.is_some());

    // Lay out absolutely-positioned children (out of flow) against this flex
    // container's padding box. The container establishes a containing block when
    // it is positioned or transformed; otherwise the abs child resolves against
    // an ancestor and we leave its CB unstamped (forwarded by the renderer).
    let establishes_cb = crate::layout::helpers::establishes_containing_block(style);
    let abs_cb_depth = if establishes_cb { positioned_depth } else { 0 };
    let content_height = style
        .height
        .map(|height| match style.box_sizing {
            BoxSizing::BorderBox => {
                (height - style.border.vertical_width() - style.padding.vertical()).max(0.0)
            }
            BoxSizing::ContentBox => height,
        })
        .unwrap_or(0.0);
    let local_containing_block = ContainingBlock {
        x: h_offset + style.border.left.used_width(),
        width: inner_width.max(0.0) + style.padding.horizontal(),
        height: content_height + style.padding.vertical(),
        depth: abs_cb_depth,
    };
    let descendant_containing_block = if establishes_cb {
        Some(local_containing_block)
    } else {
        ctx.containing_block
    };
    let mut abs_output: Vec<LayoutNode> = Vec::new();
    let absolute_context = AbsoluteFlexContext {
        container_style: style,
        layout: ctx,
        ancestors,
        inner_width: inner_width.max(0.0),
        content_height: content_height.max(0.0),
        containing_block: descendant_containing_block,
        positioned_depth,
    };

    // Resolve an item's outer (border-box) main-axis min/max clamps from its
    // computed min/max width-or-height for the container's main axis. Content-
    // box values are inflated by the item's padding+border so the clamp applies
    // to the border-box main size used throughout flex resolution.
    let main_constraints = |child_style: &ComputedStyle| -> SizeConstraints {
        let extra = if child_style.box_sizing == BoxSizing::ContentBox {
            child_style.padding.horizontal() + child_style.border.horizontal_width()
        } else {
            0.0
        };
        let extra_v = if child_style.box_sizing == BoxSizing::ContentBox {
            child_style.padding.vertical() + child_style.border.vertical_width()
        } else {
            0.0
        };
        if style.flex_direction.is_row() {
            SizeConstraints::new(
                child_style.min_width.map(|value| value + extra),
                child_style.max_width.map(|value| value + extra),
            )
        } else {
            SizeConstraints::new(
                child_style.min_height.map(|value| value + extra_v),
                child_style.max_height.map(|value| value + extra_v),
            )
        }
    };

    // Resolve an item's outer (border-box) CROSS-axis min/max clamps: the
    // opposite axis from `main_constraints`. For a row container the cross axis is
    // the block axis (min/max-height); for a column container it is the inline
    // axis (min/max-width). These clamp the used cross size — both the stretched
    // size (css-flexbox-1 §9.4 step 11) and a non-stretch item's cross size.
    let cross_min_max = |child_style: &ComputedStyle| -> (f32, f32) {
        let extra_h = if child_style.box_sizing == BoxSizing::ContentBox {
            child_style.padding.horizontal() + child_style.border.horizontal_width()
        } else {
            0.0
        };
        let extra_v = if child_style.box_sizing == BoxSizing::ContentBox {
            child_style.padding.vertical() + child_style.border.vertical_width()
        } else {
            0.0
        };
        if style.flex_direction.is_row() {
            let min = child_style.min_height.map_or(0.0, |v| v + extra_v);
            let max = child_style
                .max_height
                .map_or(f32::INFINITY, |v| v + extra_v);
            (min, max)
        } else {
            let min = child_style.min_width.map_or(0.0, |v| v + extra_h);
            let max = child_style.max_width.map_or(f32::INFINITY, |v| v + extra_h);
            (min, max)
        }
    };

    let mut items: Vec<FlexItem> = Vec::new();

    // For percentage width resolution, children need the actual container width
    // as the parent reference (not the CSS width which may be None).
    // Subtract total gap space so that percentage widths + gaps fit within the container.
    let total_gaps = style.gap * (child_count.saturating_sub(1)) as f32;
    let width_for_percentages = (inner_width - total_gaps).max(0.0);
    let mut parent_for_children = style.clone();
    if parent_for_children.width.is_none() {
        parent_for_children.width = Some(width_for_percentages);
    }

    let generated_flex_item =
        |generated: GeneratedBox<'_>, source_order: usize, env: &mut LayoutEnv| {
            let generated_style = generated.style();
            let counter_replay = env.counter_state.clone();
            let measurement_scope = env.counter_state.enter_element(generated_style);
            let mut runs = Vec::new();
            generated.append_measurement_run(
                &mut runs,
                env.fonts,
                env.counter_state,
                &mut *env.resources,
            );
            env.counter_state.leave_element(measurement_scope);
            *env.counter_state = counter_replay.clone();
            runs.as_mut_slice().resolve_unclaimed_boundaries(
                crate::layout::elements::TextSpacing::from_style(generated_style),
            );

            let intrinsic = measure_text_intrinsic_widths(
                runs.clone(),
                TextWrapOptions::new(
                    f32::MAX,
                    used_font_size(generated_style, env.fonts),
                    text_run_line_height_factor(generated_style, env.fonts),
                    generated_style.overflow_wrap,
                )
                .with_white_space(generated_style.white_space)
                .with_parent_strut(parent_line_strut(generated_style, env.fonts))
                .with_rtl(generated_style.direction_rtl)
                .with_bidi_override(generated_style.bidi_override),
                !matches!(
                    generated_style.white_space,
                    WhiteSpace::NoWrap | WhiteSpace::Pre
                ),
                env.fonts,
            );
            let resolved_basis = style
                .flex_direction
                .is_row()
                .then(|| generated_style.flex_basis.resolve(inner_width))
                .flatten();
            let has_explicit_width = resolved_basis.is_some() || generated_style.width.is_some();
            let box_floor =
                generated_style.padding.horizontal() + generated_style.border.horizontal_width();
            let inflate_outer = |value: f32| {
                if generated_style.box_sizing == BoxSizing::ContentBox {
                    value + box_floor
                } else {
                    value
                }
            };
            let natural_width = (intrinsic.max_content + box_floor).min(width_for_percentages);
            let width = resolved_basis
                .or(generated_style.width)
                .map(inflate_outer)
                .unwrap_or_else(|| {
                    if style.flex_direction.is_row() {
                        natural_width
                    } else {
                        width_for_percentages
                    }
                })
                .max(box_floor);

            let counter_scope = env.counter_state.enter_element(generated_style);
            let element = super::helpers::build_pseudo_block(
                generated_style,
                generated.originating_element(),
                PseudoBoxContext::new(width, env.fonts, env.filter_defs, &mut *env.resources)
                    .with_positioned_ancestor_depth(positioned_depth),
                env.counter_state,
                generated_style.display == Display::ListItem,
            );
            env.counter_state.leave_element(counter_scope);
            let height = estimate_element_height(element.as_ref());
            let (margin_main_start_auto, margin_main_end_auto, margin_cross_start) =
                if style.flex_direction.is_row() {
                    (
                        generated_style.margin_left_auto,
                        generated_style.margin_right_auto,
                        generated_style.margin.top,
                    )
                } else {
                    (
                        generated_style.margin_top_auto,
                        generated_style.margin_bottom_auto,
                        generated_style.margin.left,
                    )
                };
            let (margin_main_start, margin_main_end) = if style.flex_direction.is_row() {
                (generated_style.margin.left, generated_style.margin.right)
            } else {
                (generated_style.margin.top, generated_style.margin.bottom)
            };
            let is_relative = generated_style.position.is_relative();
            let rel_left = if is_relative {
                {
                    generated_style
                        .left
                        .or_else(|| generated_style.right.map(|right| -right))
                        .unwrap_or_default()
                }
            } else {
                Default::default()
            };
            let rel_top = if is_relative {
                {
                    generated_style
                        .top
                        .or_else(|| generated_style.bottom.map(|bottom| -bottom))
                        .unwrap_or_default()
                }
            } else {
                Default::default()
            };

            FlexItem {
                elements: vec![element],
                break_before: generated_style
                    .break_before
                    .forces_break()
                    .then(|| PageBreakSide::from(generated_style.break_before)),
                break_after: generated_style
                    .break_after
                    .forces_break()
                    .then(|| PageBreakSide::from(generated_style.break_after)),
                width,
                base_width: width,
                flex_grow: generated_style.flex_grow,
                flex_shrink: generated_style.flex_shrink,
                height,
                natural_height: height,
                has_explicit_width,
                fragmentation: FlexItemFragmentation::from_style(generated_style),
                align_self: generated_style.align_self,
                order: generated_style.order,
                source: FlexItemSource {
                    order: source_order,
                    element: None,
                    counter_replay,
                },
                main_constraints: main_constraints(generated_style),
                cross_min: cross_min_max(generated_style).0,
                cross_max: cross_min_max(generated_style).1,
                is_flex_container: false,
                is_table: false,
                uses_general_layout: false,
                contains_nested_forced_break: false,
                margin_main_start_auto,
                margin_main_end_auto,
                margin_main_start,
                margin_main_end,
                margin_cross_start,
                rel_left,
                rel_top,
                is_relative,
                z_index: generated_style.z_index,
                aspect_ratio: generated_style.aspect_ratio,
            }
        };

    if let Some(before) = generated_content.before() {
        if before.style().position.is_absolute() {
            absolute_context.layout_generated(before, env, &mut abs_output);
        } else {
            items.push(generated_flex_item(before, 0, env));
        }
    }

    for (idx, child_el) in child_elements.iter().enumerate() {
        let classes = child_el.class_list();
        let selector_ctx = flex_child_positions[idx]
            .as_context()
            .selector_context(ancestors, child_el.children.is_empty());
        let child_style = compute_style_with_context_with_font_metrics(
            child_el.tag,
            child_el.style_attr(),
            &parent_for_children,
            env.rules,
            child_el.tag_name(),
            &classes,
            child_el.id(),
            &child_el.attributes,
            &selector_ctx,
            env.font_metrics(),
        );
        if child_style.display == Display::None {
            continue;
        }
        if flex_child_is_absolute[idx] {
            absolute_context.layout_element(
                child_el,
                &child_style,
                &flex_child_positions[idx],
                env,
                &mut abs_output,
            );
            continue;
        }
        let generated_styles = GeneratedContentStyles::resolve(
            child_el,
            &child_style,
            env.rules,
            &selector_ctx,
            env.fonts,
        );
        let generated_children = generated_styles.boxes(child_el);
        let counter_replay = env.counter_state.clone();

        // Auto margins on a flex item (css-flexbox-1 §8.1). Map the four physical
        // `auto` flags onto the container's main/cross axes. Main-axis autos are
        // carried on the `FlexItem` and absorb main free space during placement;
        // cross-axis autos override `align-self` here: both → center, a single
        // leading auto → push to the cross-end, a single trailing auto → cross-start.
        // (Per §8.3 a cross auto margin also suppresses `align-items: stretch`,
        // which the Center/FlexEnd/FlexStart mapping does implicitly.)
        let (m_main_start_auto, m_main_end_auto, m_cross_start_auto, m_cross_end_auto) =
            if style.flex_direction.is_row() {
                (
                    child_style.margin_left_auto,
                    child_style.margin_right_auto,
                    child_style.margin_top_auto,
                    child_style.margin_bottom_auto,
                )
            } else {
                (
                    child_style.margin_top_auto,
                    child_style.margin_bottom_auto,
                    child_style.margin_left_auto,
                    child_style.margin_right_auto,
                )
            };
        let item_align_self = if m_cross_start_auto && m_cross_end_auto {
            AlignSelf::Center
        } else if m_cross_start_auto {
            AlignSelf::FlexEnd
        } else if m_cross_end_auto {
            AlignSelf::FlexStart
        } else {
            child_style.align_self
        };

        // Fixed (non-auto) flex-item margins mapped onto the container's axes.
        // An `auto` margin contributes 0 here (the auto flag drives it instead).
        let (m_main_start, m_main_end, m_cross_start) = if style.flex_direction.is_row() {
            (
                child_style.margin.left,
                child_style.margin.right,
                child_style.margin.top,
            )
        } else {
            (
                child_style.margin.top,
                child_style.margin.bottom,
                child_style.margin.left,
            )
        };
        // `position: relative` offsets on a flex item. `left`/`top` win over
        // `right`/`bottom`; an unset axis is 0. The item lays out statically and
        // is painted shifted by these deltas.
        let item_is_relative = child_style.position.is_relative();
        let (item_rel_left, item_rel_top) = if item_is_relative {
            (
                child_style
                    .left
                    .or_else(|| child_style.right.map(|r| -r))
                    .unwrap_or(0.0),
                child_style
                    .top
                    .or_else(|| child_style.bottom.map(|b| -b))
                    .unwrap_or(0.0),
            )
        } else {
            (0.0, 0.0)
        };

        // Determine child width: flex-basis takes priority, then explicit width.
        // Flex base size for grow/shrink distribution:
        // - With flex-basis or width: use that value
        // - flex-grow > 0 without basis/width: use 0 so all space is distributed
        //   proportionally by grow factors
        // - flex-grow == 0 without basis/width: use equal share, then shrink to
        //   natural content width (for justify-content)
        //
        // For `box-sizing: content-box` (the CSS default), the specified width
        // is the *content* width, so the outer box used for flex main-axis
        // layout is `width + padding + border`. For `border-box`, the
        // specified width is already the outer box.
        // Resolve a percentage `flex-basis` against the container's main-axis
        // content size. For a row container that is `inner_width`; for a column
        // container the main axis is the (often indefinite) height, where a
        // percentage basis behaves like `auto`, so we only resolve it for row
        // direction. The resolved length then feeds the same path as an explicit
        // `flex-basis` length.
        // `flex-basis` (and a percentage basis) is a MAIN-axis base size. For a
        // ROW container the main axis is inline, so the basis feeds the item's
        // width. For a COLUMN container the main axis is the block (height) axis,
        // so the basis must NOT leak into the item's cross-axis WIDTH — doing so
        // defeated `align-items: stretch` (a `flex: 1 1 0` column item rendered
        // width 0, a `flex: 0 0 40px` item rendered 40px wide instead of filling
        // the column). The column main-axis basis is applied to the item height
        // further below (see the `!is_row()` `item_border_box_h` branch).
        let resolved_basis = if style.flex_direction.is_row() {
            child_style.flex_basis.resolve(inner_width)
        } else {
            None
        };
        let content_basis = child_style.flex_basis.content_keyword();
        let has_explicit_width = resolved_basis.is_some() || child_style.width.is_some();
        let fragmentation = FlexItemFragmentation::from_style(&child_style);
        let inflate_outer = |w: f32| -> f32 {
            if child_style.box_sizing == BoxSizing::ContentBox {
                w + child_style.padding.horizontal() + child_style.border.horizontal_width()
            } else {
                w
            }
        };
        // An item's outer (border-box) main size can never be smaller than its
        // own border + padding — the content box floors at 0, not the border
        // box. Under `box-sizing: border-box` a `flex-basis: 0` therefore yields
        // an outer width equal to the horizontal border + padding, NOT 0; the
        // grow free space is then `inner - Σ(these floors)` and each item's
        // final width = its floor + its share. Without this floor a bordered
        // `flex-basis: 0` item lost its border thickness from the distribution
        // (e.g. widths 78/156/78 instead of Chrome's 78.75/154.5/78.75).
        let item_box_floor =
            child_style.border.horizontal_width() + child_style.padding.horizontal();
        let transferred_aspect_width =
            if style.flex_direction.is_row() && child_style.width.is_none() {
                child_style
                    .height
                    .zip(child_style.aspect_ratio)
                    .map(|(h, ratio)| flex_style_outer_height(&child_style, h) * ratio)
            } else {
                None
            };
        let auto_item_width = if style.flex_direction.is_row() {
            width_for_percentages / child_count as f32
        } else {
            width_for_percentages
        };
        let child_w_initial = match resolved_basis
            .or(child_style.width)
            .or(transferred_aspect_width)
        {
            Some(w) => inflate_outer(w).max(item_box_floor),
            None => {
                if child_style.flex_grow > 0.0 {
                    item_box_floor
                } else {
                    auto_item_width
                }
            }
        };
        // A flexible item whose content base is exactly zero is measured at its
        // eventual share before grow distribution. A positive authored basis,
        // however small, remains distinct and must not be reclassified as zero.
        let grows_from_zero_base = child_style.flex_grow > 0.0
            && resolved_basis
                .or(child_style.width)
                .or(transferred_aspect_width)
                .is_none_or(|width| width == 0.0);
        let wrap_width = if grows_from_zero_base {
            auto_item_width
        } else {
            child_w_initial
        };

        // Include the child element itself in the ancestor chain so that
        // descendant selectors like `.card h3` can match.
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors
            .push(flex_child_positions[idx].ancestor(child_el, child_el.children.is_empty()));

        // The current element's layout context receives its resolved outer
        // flex-item width. The element's own block/flex layout then derives its
        // content box from its padding and border. Passing that already-inset
        // content width here made the element inset itself a second time.
        let child_w_for_flex = match resolved_basis
            .or(child_style.width)
            .or(transferred_aspect_width)
        {
            Some(w) => inflate_outer(w).max(item_box_floor),
            None => auto_item_width,
        };
        let child_w_for_layout = if child_style.flex_grow > 0.0
            && child_style.flex_basis.is_zero()
            && child_style.width.is_none()
        {
            width_for_percentages
        } else {
            child_w_for_flex
        };

        // Check if this flex item or any of its descendants must use the normal
        // block/replaced layout path instead of the text-only collector.
        let item_contents =
            classify_flex_item_contents(child_el, &child_style, &child_ancestors, env);
        let item_has_block_children =
            matches!(child_el.tag, HtmlTag::Img | HtmlTag::Svg | HtmlTag::Table)
                || item_contents.requires_general_layout()
                || generated_styles.requires_box_layout();
        let is_table = matches!(child_style.display, Display::Table | Display::InlineTable);

        // flex: 0 0 auto wrapping a nested layout-capable child (table, image,
        // SVG): Chrome sizes the item to that child's max-content contribution,
        // not the equal-share fallback. Probe the child's flattened border-box
        // width with a throwaway layout at the full container width, then hug it.
        // With grow:0 the base width is also the final width.
        let mut hugged_item_width = if item_contents.only_out_of_flow_structure()
            && !has_explicit_width
            && child_style.flex_grow == 0.0
            && resolved_basis.is_none()
        {
            let mut in_flow_runs = Vec::new();
            let live_counters = env.counter_state.clone();
            let counter_scope = env.counter_state.enter_element(&child_style);
            InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources)
                .collect(
                    InlineContentSequence::with_generated(&child_el.children, generated_children),
                    &child_style,
                    &mut in_flow_runs,
                    None,
                    &child_ancestors,
                );
            env.counter_state.leave_element(counter_scope);
            *env.counter_state = live_counters;
            (!in_flow_runs.is_empty()).then(|| {
                (measure_runs_width(&in_flow_runs, env.fonts)
                    + child_style.padding.horizontal()
                    + child_style.border.horizontal_width())
                .min(width_for_percentages)
            })
        } else {
            None
        };
        let intrinsic_keyword = content_basis.or(child_style.width_keyword).or_else(|| {
            (!has_explicit_width && child_style.flex_grow == 0.0)
                .then_some(IntrinsicWidthKeyword::MaxContent)
        });
        let intrinsic_width = (item_has_block_children && resolved_basis.is_none())
            .then_some(intrinsic_keyword)
            .flatten()
            .map(|keyword| {
                let widths = if content_basis.is_some() {
                    crate::layout::intrinsic_width::content_intrinsic_border_box_widths(
                        child_el,
                        &child_style,
                        env.rules,
                        env.fonts,
                        ancestors,
                        &selector_ctx,
                    )
                } else {
                    crate::layout::intrinsic_width::intrinsic_border_box_widths(
                        child_el,
                        &child_style,
                        env.rules,
                        env.fonts,
                        ancestors,
                        &selector_ctx,
                    )
                };
                widths
                    .resolve(keyword, width_for_percentages)
                    .max(item_box_floor)
            })
            .filter(|width| {
                content_basis.is_some()
                    || child_style.width_keyword.is_some()
                    || *width > item_box_floor
            });
        let (child_w_for_flex, child_w_for_layout) = if let Some(hugged) = hugged_item_width {
            (hugged, hugged)
        } else if let Some(intrinsic) = intrinsic_width {
            hugged_item_width = Some(intrinsic);
            (intrinsic, intrinsic)
        } else if item_has_block_children
            && !has_explicit_width
            && child_style.flex_grow == 0.0
            && resolved_basis.is_none()
        {
            let mut probe_buf = Vec::new();
            let probe_ctx = ctx
                .with_parent_and_basis(
                    width_for_percentages,
                    width_for_percentages,
                    Some(10000.0),
                    style.font_size,
                )
                .with_containing_block(descendant_containing_block);
            flatten_element(
                child_el,
                LayoutTreeContext::new(style, &probe_ctx, ancestors)
                    .with_positioned_ancestor_depth(positioned_depth)
                    .for_element(flex_child_positions[idx].as_context())
                    .with_filter_application(FilterApplication::DeferToFormattingItem),
                &mut probe_buf,
                env,
            );
            let probed_w = flex_probe_outer_extent(&probe_buf);
            if probed_w > 0.0 {
                let hugged = probed_w.max(item_box_floor);
                hugged_item_width = Some(hugged);
                (hugged, hugged)
            } else {
                (child_w_for_flex, child_w_for_layout)
            }
        } else {
            (child_w_for_flex, child_w_for_layout)
        };

        // For complex flex items (with block children like <h2>, <p>, <div>),
        // use flatten_element to get a proper list of layout elements with
        // margins and structure preserved.
        if item_has_block_children {
            let mut child_elements_buf = Vec::new();
            // Percentage-height children resolve against the item's OWN definite
            // height. A height-less item has an indefinite block size during this
            // intrinsic-measurement pass, so percentage heights resolve to `auto`
            // (not against an arbitrary placeholder that would balloon the item
            // and poison the container cross size). When the item later stretches
            // (`align-items: stretch`) to a definite cross size, the percentage
            // children are re-resolved against that size (see the stretch loop).
            let item_content_height_basis: Option<f32> = if fragmentation.block_size.is_explicit() {
                child_style.height.map(|h| match child_style.box_sizing {
                    BoxSizing::ContentBox => h,
                    BoxSizing::BorderBox => {
                        (h - child_style.padding.vertical() - child_style.border.vertical_width())
                            .max(0.0)
                    }
                })
            } else {
                None
            };
            let child_ctx = ctx
                .with_parent_and_basis(
                    child_w_for_layout,
                    width_for_percentages,
                    item_content_height_basis,
                    style.font_size,
                )
                .with_containing_block(descendant_containing_block);
            flatten_element(
                child_el,
                LayoutTreeContext::new(style, &child_ctx, ancestors)
                    .with_positioned_ancestor_depth(positioned_depth)
                    .for_element(flex_child_positions[idx].as_context())
                    .with_filter_application(FilterApplication::DeferToFormattingItem),
                &mut child_elements_buf,
                env,
            );
            if style.flex_direction.is_row() {
                strip_row_flex_item_break_markers(
                    &mut child_elements_buf,
                    child_style.break_before.forces_break(),
                    child_style.break_after.forces_break(),
                );
            }
            // For a shrink-wrapped table item the leading Container paints the
            // item's own background/border; stamp the hugged border-box width on it
            // so it paints at the item width (flex base size), not the laid-out
            // content width. The nested table is left-aligned and intrinsic, so its
            // position is unaffected.
            if let Some(hw) = hugged_item_width {
                if let Some(element) = child_elements_buf.first_mut() {
                    update_container_size(element.as_mut(), Some(hw), None);
                }
            }
            // A nested flex/block container that paints its own background emits
            // a leading background TextBlock (carrying the container's full
            // padding-box `block_height`) immediately followed by a
            // negative-margin spacer that pulls the flowed children back *inside*
            // that background. In that layout the background block already
            // accounts for the children's vertical extent, so summing the
            // pulled-back children as well double-counts the column's height.
            // Detect that pattern and take the background block's border-box
            // height as the item's natural height instead.
            let self_bg_natural = child_elements_buf
                .first()
                .and_then(|background| text_block_background_height(background.as_ref()))
                .filter(|_| {
                    child_elements_buf
                        .get(1)
                        .is_some_and(|spacer| is_empty_pullback_spacer(spacer.as_ref()))
                });
            let mut child_h = self_bg_natural.unwrap_or_else(|| {
                child_elements_buf
                    .iter()
                    .map(|element| {
                        if is_table {
                            // A table decoration is a paint-only box whose
                            // negative end margin pulls the row grid over it.
                            // Generic flex measurement intentionally ignores
                            // that pullback, so it double-counts the table box.
                            // The shared block-flow estimator honors the paired
                            // height/margin and returns the table's real outer
                            // flow extent, including captions and expanded rows.
                            estimate_element_height(element.as_ref())
                        } else {
                            flex_item_content_height(element.as_ref())
                        }
                    })
                    .sum::<f32>()
            });
            if let Some(specified_height) = child_style.height {
                let specified_outer = flex_style_outer_height(&child_style, specified_height);
                child_h = if is_table {
                    child_h.max(specified_outer)
                } else {
                    specified_outer
                };
            }
            if hugged_item_width.is_some() {
                if let Some(element) = child_elements_buf.first_mut() {
                    update_container_size(element.as_mut(), None, Some(child_h));
                }
            }

            let contains_nested_forced_break = sequence_contains_page_break(&child_elements_buf);
            items.push(FlexItem {
                elements: child_elements_buf,
                break_before: child_style
                    .break_before
                    .forces_break()
                    .then(|| PageBreakSide::from(child_style.break_before)),
                break_after: child_style
                    .break_after
                    .forces_break()
                    .then(|| PageBreakSide::from(child_style.break_after)),
                width: child_w_for_flex,
                base_width: child_w_for_flex,
                flex_grow: child_style.flex_grow,
                flex_shrink: child_style.flex_shrink,
                height: child_h,
                natural_height: child_h, // Natural height for align-items flex-start
                has_explicit_width,
                fragmentation,
                align_self: item_align_self,
                order: child_style.order,
                source: FlexItemSource {
                    order: idx + 1,
                    element: Some(ElementFlexItemIndex(idx)),
                    counter_replay,
                },
                main_constraints: main_constraints(&child_style),
                cross_min: cross_min_max(&child_style).0,
                cross_max: cross_min_max(&child_style).1,
                is_flex_container: matches!(
                    child_style.display,
                    Display::Flex | Display::InlineFlex
                ),
                is_table,
                uses_general_layout: true,
                contains_nested_forced_break,
                margin_main_start_auto: m_main_start_auto,
                margin_main_end_auto: m_main_end_auto,
                margin_main_start: m_main_start,
                margin_main_end: m_main_end,
                margin_cross_start: m_cross_start,
                rel_left: item_rel_left,
                rel_top: item_rel_top,
                is_relative: item_is_relative,
                z_index: child_style.z_index,
                aspect_ratio: child_style.aspect_ratio,
            });
            continue;
        }

        // Simple flex items: collect text runs and wrap
        let mut runs = Vec::new();
        let counter_scope = env.counter_state.enter_element(&child_style);
        InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources)
            .collect(
                InlineContentSequence::with_generated(&child_el.children, generated_children),
                &child_style,
                &mut runs,
                None,
                &child_ancestors,
            );
        env.counter_state.leave_element(counter_scope);
        // Automatic minimum sizing and `flex-basis:min-content` must use the
        // same tokenization as line layout. In particular, a forced `<br>`
        // separates intrinsic lines while adjacent styled runs without a wrap
        // opportunity remain one unbreakable group.
        let intrinsic_text_widths = (!runs.is_empty()).then(|| {
            measure_text_intrinsic_widths(
                runs.clone(),
                TextWrapOptions::new(
                    f32::MAX,
                    used_font_size(&child_style, env.fonts),
                    text_run_line_height_factor(&child_style, env.fonts),
                    child_style.overflow_wrap,
                )
                .with_white_space(child_style.white_space)
                .with_parent_strut(parent_line_strut(&child_style, env.fonts))
                .with_rtl(child_style.direction_rtl)
                .with_bidi_override(child_style.bidi_override),
                !matches!(
                    child_style.white_space,
                    WhiteSpace::NoWrap | WhiteSpace::Pre
                ),
                env.fonts,
            )
        });

        // `flex-basis: content` sizes the flex base to the item's max-content
        // size, ignoring any `width` (css-flexbox-1 §7.2.3). Measure the run
        // width and inflate by padding/border, capped at the container — this
        // overrides the explicit `width` that `has_explicit_width` reflects.
        // When no explicit width/flex-basis and flex-grow is 0, measure the
        // natural (intrinsic) content width so the item shrinks to fit. A
        // stretched column item is the exception: its used cross size is the
        // container width, so wrapping must use that width from the outset.
        let stretches_column_cross_axis = !style.flex_direction.is_row()
            && !has_explicit_width
            && (item_align_self == AlignSelf::Stretch
                || (item_align_self == AlignSelf::Auto
                    && style.align_items == AlignItems::Stretch));
        let mut intrinsic_text_wrap_width = None;
        let child_w = if let Some(content_basis) = content_basis.filter(|_| !runs.is_empty()) {
            let natural_text_w = if content_basis == IntrinsicWidthKeyword::MinContent {
                intrinsic_text_widths
                    .map(|widths| widths.min_content)
                    .unwrap_or_default()
            } else {
                measure_runs_width(&runs, env.fonts)
            };
            let intrinsic = FlexIntrinsicWidth::from_content(&child_style, natural_text_w);
            intrinsic_text_wrap_width = Some(intrinsic.text_wrap.min(width_for_percentages));
            intrinsic.paint.min(width_for_percentages)
        } else if !stretches_column_cross_axis
            && !has_explicit_width
            && child_style.flex_grow == 0.0
            && !runs.is_empty()
        {
            let natural_text_w = measure_runs_width(&runs, env.fonts);
            let pad_h = child_style.padding.horizontal();
            let border_h = child_style.border.horizontal_width();
            // Outer width = text + padding + border (capped at container)
            (natural_text_w + pad_h + border_h).min(width_for_percentages)
        } else {
            child_w_initial
        };

        // Automatic minimum size (css-flexbox-1 §4.5). For a row container the
        // main axis is inline, and a flex item whose `min-width` is `auto` (the
        // default) and that is not a scroll container (overflow:visible) must not
        // shrink below its content-based minimum. The used automatic minimum is
        // min(content size suggestion, specified size suggestion) clamped by the
        // item's max main size — so it never exceeds the item's own specified
        // width, and items with `min-width:0`/clipped overflow keep collapsing.
        // Only the row main axis is handled here (column main = block height is
        // left at 0 to avoid disturbing column sizing).
        let resolved_main_constraints = main_constraints(&child_style);
        let resolved_min_main = resolved_main_constraints.minimum().unwrap_or(0.0);
        let resolved_max_main = resolved_main_constraints.maximum().unwrap_or(f32::INFINITY);
        let mut auto_min_main = if style.flex_direction.is_row()
            && child_style.min_width.is_none()
            && child_style.overflow_x == Overflow::Visible
            && child_style.overflow_y == Overflow::Visible
            && child_style.overflow_wrap != OverflowWrap::Anywhere
            && !runs.is_empty()
        {
            let content_min = intrinsic_text_widths
                .map(|widths| widths.min_content)
                .unwrap_or_default()
                + child_style.padding.horizontal()
                + child_style.border.horizontal_width();
            let specified = if has_explicit_width {
                child_w_initial
            } else {
                f32::INFINITY
            };
            content_min.min(specified).min(resolved_max_main)
        } else {
            resolved_min_main
        };

        // A structurally zero grow base uses the provisional share above; every
        // positive authored base reaches text measurement unchanged.
        let wrap_w = if content_basis.is_some()
            && child_style.flex_grow == 0.0
            && child_style.flex_shrink == 0.0
        {
            intrinsic_text_wrap_width.unwrap_or(child_w)
        } else if child_style.flex_grow > 0.0 && !has_explicit_width {
            wrap_width
        } else {
            child_w
        };
        // wrap_w is always the outer box width (after content-box inflation),
        // so the inner content area is outer - padding - border.
        let child_inner_w =
            (wrap_w - child_style.padding.horizontal() - child_style.border.horizontal_width())
                .max(0.0);
        let text_indent = child_style.text_indent.resolve(child_inner_w);

        let lines = if !runs.is_empty() {
            wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    child_inner_w,
                    used_font_size(&child_style, env.fonts),
                    text_run_line_height_factor(&child_style, env.fonts),
                    child_style.overflow_wrap,
                )
                .with_white_space(child_style.white_space)
                .with_parent_strut(parent_line_strut(&child_style, env.fonts))
                .with_text_indent(text_indent)
                .with_rtl(child_style.direction_rtl)
                .with_bidi_override(child_style.bidi_override),
                env.fonts,
            )
        } else {
            Vec::new()
        };

        let text_height: f32 = lines.iter().map(|l| l.height).sum();
        let aspect_h = child_style
            .height
            .is_none()
            .then(|| aspect_ratio_height(child_w, &child_style))
            .flatten();
        let mut child_h = resolve_padding_box_height(
            text_height,
            child_style.height,
            child_style.padding,
            child_style.border.widths(),
            child_style.box_sizing,
        );
        if !style.flex_direction.is_row()
            && child_style.min_height.is_none()
            && child_style.overflow_x == Overflow::Visible
            && child_style.overflow_y == Overflow::Visible
            && text_height > 0.0
        {
            auto_min_main = (child_h + child_style.border.vertical_width()).min(resolved_max_main);
        }
        if let Some(aspect_h) = aspect_h {
            child_h = child_h.max(aspect_h);
        }

        let block_height = child_style
            .height
            .map(|_| child_h)
            .or(aspect_h.map(|_| child_h));
        let mut elem = TextBlock::from_style(
            lines,
            &child_style,
            crate::layout::elements::BoxModel::from_style(
                &child_style,
                BlockMargins::new(child_style.margin.top, child_style.margin.bottom),
            ),
        );
        elem.box_model.size = crate::layout::elements::LayoutSize::fixed(child_w, block_height);
        elem.flow = Default::default();
        elem.positioning.insets = EdgeSizes::ZERO;
        elem.paint.border_radii = child_style.resolve_corner_radii(child_w, child_h);
        elem.text.indent = text_indent;
        elem.clipping.rect = child_style
            .overflow
            .clips()
            .then(|| Rect::from_xywh(0.0, 0.0, child_w, child_h));

        // `child_h` is the item's *padding-box* height (the TextBlock
        // convention used for `block_height`). The flex *item*'s main- and
        // cross-axis extent is its border box, so add the border back here —
        // otherwise a `box-sizing: border-box` item with an explicit height
        // measured short by its border, collapsing wrapped-line cross sizes and
        // column main-axis spacing.
        let mut item_border_box_h = child_h + child_style.border.vertical_width();
        // For a column container the main axis is the block axis, so `flex-basis`
        // (a main-size) sets the item's height when no explicit `height` is
        // given. Without this an empty `flex-basis: 150px` column item measured
        // its content height (~0) and collapsed. A percentage basis resolves
        // against the container's main (cross_size already folds the height) —
        // we approximate it against `inner_cross_size` which is the resolved
        // content height, computed later, so only the length basis is used here.
        if !style.flex_direction.is_row() && !fragmentation.block_size.is_explicit() {
            // A percentage `flex-basis` resolves against the container's inner
            // main (block) size when that size is DEFINITE (css-flexbox-1 §9.2).
            // For a column container the main size is the container's content
            // height; fall back to the length basis / content size when the
            // height is indefinite.
            let container_main_content: Option<f32> =
                style.height.map(|h| match style.box_sizing {
                    BoxSizing::ContentBox => h,
                    BoxSizing::BorderBox => {
                        (h - style.padding.vertical() - style.border.vertical_width()).max(0.0)
                    }
                });
            let basis_len = child_style.flex_basis.definite_length().or_else(|| {
                container_main_content.and_then(|main| child_style.flex_basis.resolve(main))
            });
            if let Some(basis) = basis_len {
                let bb = if child_style.box_sizing == BoxSizing::ContentBox {
                    basis + child_style.padding.vertical() + child_style.border.vertical_width()
                } else {
                    basis
                };
                item_border_box_h = bb;
            }
        }
        let mut item_elements = Vec::new();
        let row_direction = style.flex_direction.is_row();
        if !row_direction {
            emit_page_break_before(&child_style, &mut item_elements);
        }
        item_elements.push(elem.boxed());
        if !row_direction {
            emit_page_break_after(&child_style, &mut item_elements);
        }

        let contains_nested_forced_break = sequence_contains_page_break(&item_elements);
        items.push(FlexItem {
            elements: item_elements,
            break_before: child_style
                .break_before
                .forces_break()
                .then(|| PageBreakSide::from(child_style.break_before)),
            break_after: child_style
                .break_after
                .forces_break()
                .then(|| PageBreakSide::from(child_style.break_after)),
            width: child_w,
            base_width: child_w,
            flex_grow: child_style.flex_grow,
            flex_shrink: child_style.flex_shrink,
            height: item_border_box_h + child_style.margin.vertical(),
            natural_height: item_border_box_h + child_style.margin.vertical(),
            has_explicit_width,
            fragmentation,
            align_self: item_align_self,
            order: child_style.order,
            source: FlexItemSource {
                order: idx + 1,
                element: Some(ElementFlexItemIndex(idx)),
                counter_replay,
            },
            main_constraints: SizeConstraints::new(
                Some(auto_min_main),
                resolved_main_constraints.maximum(),
            ),
            cross_min: cross_min_max(&child_style).0,
            cross_max: cross_min_max(&child_style).1,
            is_flex_container: matches!(child_style.display, Display::Flex | Display::InlineFlex),
            is_table: matches!(child_style.display, Display::Table | Display::InlineTable),
            uses_general_layout: false,
            contains_nested_forced_break,
            margin_main_start_auto: m_main_start_auto,
            margin_main_end_auto: m_main_end_auto,
            margin_main_start: m_main_start,
            margin_main_end: m_main_end,
            margin_cross_start: m_cross_start,
            rel_left: item_rel_left,
            rel_top: item_rel_top,
            is_relative: item_is_relative,
            z_index: child_style.z_index,
            aspect_ratio: child_style.aspect_ratio,
        });
    }

    if let Some(after) = generated_content.after() {
        if after.style().position.is_absolute() {
            absolute_context.layout_generated(after, env, &mut abs_output);
        } else {
            items.push(generated_flex_item(after, child_elements.len() + 1, env));
        }
    }

    if items.is_empty() {
        let container_height = style
            .height
            .or_else(|| aspect_ratio_height(block_w, style))
            .map(|height| match style.box_sizing {
                BoxSizing::ContentBox => height + style.padding.vertical(),
                BoxSizing::BorderBox => (height - style.border.vertical_width()).max(0.0),
            })
            .unwrap_or_default();
        let mut background = TextBlock::from_style(
            Vec::new(),
            style,
            crate::layout::elements::BoxModel::from_style(
                style,
                BlockMargins::new(style.margin.top, style.margin.bottom),
            ),
        );
        background.box_model.size =
            crate::layout::elements::LayoutSize::fixed(block_w, Some(container_height));
        background.paint.border_radii = style.resolve_corner_radii(block_w, container_height);
        background.positioning.containing_block_depth = positioned_depth;
        background.clipping.rect = style
            .overflow
            .clips()
            .then(|| Rect::from_xywh(0.0, 0.0, block_w, container_height));
        output.push(background.boxed());
        output.append(&mut abs_output);
        return;
    }
    // Reorder items by CSS `order` (ascending), with document order breaking
    // ties. Layout/placement and visual paint order both follow `order`.
    if items.iter().any(|it| it.order != 0) {
        items.sort_by_key(|it| (it.order, it.source.order));
    }

    let direction = style.flex_direction;
    let justify = style.justify_content;
    let align = style.align_items;
    let align_last_baseline = authored_align_items_last_baseline(el, ancestors, env.rules);
    let wrap = style.flex_wrap;
    // Resolve percentage gaps against the flex container's OWN content box (CSS
    // Box Alignment §8.3): column-gap% against the content-box inline size
    // (width), row-gap% against the content-box block size (height). The parser
    // stores these as fraction hints (`column_gap_pct`/`row_gap_pct`) precisely
    // so they bind to this box, not the parent/ICB width.
    let resolved_column_gap = match style.column_gap_pct {
        Some(frac) => (inner_width * frac).max(0.0),
        None => style.column_gap,
    };
    let resolved_row_gap = match style.row_gap_pct {
        Some(frac) => {
            let content_h = match style.height {
                Some(h) => match style.box_sizing {
                    BoxSizing::ContentBox => h,
                    BoxSizing::BorderBox => {
                        (h - style.padding.vertical() - style.border.vertical_width()).max(0.0)
                    }
                },
                // Indefinite block size => percentage row-gap resolves to 0.
                None => 0.0,
            };
            (content_h * frac).max(0.0)
        }
        None => style.row_gap,
    };
    // Per-axis gaps. `column_gap` separates items along the inline axis,
    // `row_gap` along the block axis. For a row container the main-axis gap is
    // the column gap and the line (cross) gap is the row gap; for a column
    // container they swap. `style.gap` is kept as the legacy single value.
    let (main_gap, line_gap) = if direction.is_row() {
        (resolved_column_gap, resolved_row_gap)
    } else {
        (resolved_row_gap, resolved_column_gap)
    };
    // `gap` is the main-axis gap used throughout the per-line packing math.
    let gap = main_gap;
    let column_wrap_limit = if direction.is_row() {
        None
    } else {
        style.height.map(|h| match style.box_sizing {
            BoxSizing::ContentBox => h,
            BoxSizing::BorderBox => {
                (h - style.padding.vertical() - style.border.vertical_width()).max(0.0)
            }
        })
    };

    // Group items into lines (for flex-wrap)
    #[derive(Default)]
    struct FlexLine {
        item_indices: Vec<usize>,
        main_size: f32,
        cross_size: f32,
        break_before: Option<PageBreakSide>,
        break_after: Option<PageBreakSide>,
    }

    let mut lines: Vec<FlexLine> = Vec::new();

    match direction {
        FlexDirection::Row | FlexDirection::RowReverse => {
            let max_main = inner_width;
            let mut current_line = FlexLine::default();

            for (i, item) in items.iter().enumerate() {
                // Flexbox section 9.3 forms lines from each item's outer
                // hypothetical main size, not its unconstrained flex base.
                let item_main = item.main_constraints.constrain(item.width)
                    + item.margin_main_start
                    + item.margin_main_end;
                let gap_extra = if current_line.item_indices.is_empty() {
                    0.0
                } else {
                    gap
                };

                // A forced item break can restart line formation only in a
                // multi-line row flex container. In a single-line (`nowrap`)
                // container every item belongs to the same flex line, so the
                // break propagates to that line and therefore to the flex
                // container boundary (CSS Flexbox §10).
                let starts_forced_line = wrap.wraps() && item.break_before.is_some();
                let wraps_here = wrap.wraps()
                    && !current_line.item_indices.is_empty()
                    && current_line.main_size + gap_extra + item_main > max_main;
                if !current_line.item_indices.is_empty() && (starts_forced_line || wraps_here) {
                    lines.push(current_line);
                    current_line = FlexLine {
                        break_before: item.break_before,
                        ..Default::default()
                    };
                } else if current_line.item_indices.is_empty()
                    || (!wrap.wraps() && item.break_before.is_some())
                {
                    current_line.break_before = item.break_before;
                }

                if !current_line.item_indices.is_empty() {
                    current_line.main_size += gap;
                }
                current_line.main_size += item_main;
                current_line.cross_size = current_line.cross_size.max(item.height);
                current_line.item_indices.push(i);
                if let Some(side) = item.break_after {
                    current_line.break_after = Some(side);
                    if wrap.wraps() {
                        lines.push(current_line);
                        current_line = FlexLine::default();
                    }
                }
            }
            if !current_line.item_indices.is_empty() {
                lines.push(current_line);
            }
        }
        FlexDirection::Column | FlexDirection::ColumnReverse => {
            // In column direction the main axis is vertical. With `flex-wrap:
            // wrap` and a definite container height, items that overflow that
            // height start a new column (a new flex line on the horizontal
            // cross axis).
            let mut line = FlexLine::default();
            for (i, item) in items.iter().enumerate() {
                let gap_extra = if line.item_indices.is_empty() {
                    0.0
                } else {
                    gap
                };
                if wrap.wraps()
                    && !line.item_indices.is_empty()
                    && column_wrap_limit
                        .is_some_and(|max_main| line.main_size + gap_extra + item.height > max_main)
                {
                    lines.push(line);
                    line = FlexLine::default();
                }
                if !line.item_indices.is_empty() {
                    line.main_size += gap;
                }
                line.main_size += item.height;
                line.cross_size = line.cross_size.max(item.width);
                line.item_indices.push(i);
            }
            if !line.item_indices.is_empty() {
                lines.push(line);
            }
        }
    }

    // Compute container dimensions
    let total_cross: f32 = if direction.is_row() {
        lines.iter().map(|l| l.cross_size).sum::<f32>()
            + if lines.len() > 1 {
                (lines.len() - 1) as f32 * line_gap
            } else {
                0.0
            }
    } else if lines.len() > 1 {
        lines.iter().map(|l| l.cross_size).sum::<f32>() + (lines.len() - 1) as f32 * line_gap
    } else {
        lines.iter().map(|l| l.cross_size).fold(0.0f32, f32::max)
    };

    let total_main: f32 = if direction.is_row() {
        inner_width
    } else if lines.len() > 1 {
        lines.iter().map(|l| l.main_size).fold(0.0f32, f32::max)
    } else {
        lines.iter().map(|l| l.main_size).sum::<f32>()
    };

    let container_height = if direction.is_row() {
        total_cross
    } else {
        total_main
    };

    // Resolve the natural flex content through the same box-dimension boundary
    // used by inline flex and grid items. This applies max before min and keeps
    // content-box/border-box conversion out of the flex algorithm.
    let pad_v = style.padding.vertical();
    let border_v = style.border.vertical_width();
    let natural_border_box_height = container_height + pad_v + border_v;
    let container_h =
        (ResolvedBoxDimensions::from_style(style, Size::new(block_w, natural_border_box_height))
            .border_box
            .height
            - border_v)
            .max(0.0);
    // Cross-axis inner size once height/min-height have been honored. For
    // row direction with a single line this is what each item should
    // stretch to (align-items: stretch) and what flex-end/center measure
    // against — otherwise a tall `min-height` container collapses visually
    // to the natural item height.
    let inner_cross_size = (container_h - style.padding.vertical()).max(0.0);

    // Cross-axis stretch for nested flex containers (row direction).
    //
    // A flex item with the default `align-items: stretch` and no definite cross
    // size (here `height`) must stretch to the container's content cross size.
    // For a *nested flex container* item, that stretched height is also its main
    // size when laid out as its own column flex, so its internal
    // `justify-content` (e.g. `space-between`) distributes against the stretched
    // height — not its natural content height. The first flatten produced the
    // item at natural height; re-flatten it with the stretched height forced so
    // its inner layout (and its painted background/border) fill the cross axis.
    if direction.is_row() && lines.len() == 1 && inner_cross_size > 0.0 {
        for item in items.iter_mut() {
            let stretches = match item.align_self {
                AlignSelf::Stretch => true,
                AlignSelf::Auto => align == AlignItems::Stretch,
                _ => false,
            };
            if !stretches
                || item.fragmentation.block_size.is_explicit()
                || !exceeds_with_roundoff(inner_cross_size, item.height)
            {
                continue;
            }
            let Some(ElementFlexItemIndex(child_idx)) = item.source.element else {
                continue;
            };
            let child_el = child_elements[child_idx];
            // Only flex containers carry their own main-axis distribution that
            // depends on the stretched height. Simple items are stretched purely
            // visually by the renderer (cell_render_h = line_cross).
            let classes = child_el.class_list();
            let selector_ctx = flex_child_positions[child_idx]
                .as_context()
                .selector_context(ancestors, child_el.children.is_empty());
            let mut child_style = compute_style_with_context_with_font_metrics(
                child_el.tag,
                child_el.style_attr(),
                &parent_for_children,
                env.rules,
                child_el.tag_name(),
                &classes,
                child_el.id(),
                &child_el.attributes,
                &selector_ctx,
                env.font_metrics(),
            );
            // Force the item's cross size (its main size as a column flex) to the
            // stretched height. Translate the padding-box `inner_cross_size` to a
            // value the container's box-sizing interprets as that border-box.
            let forced_h = match child_style.box_sizing {
                BoxSizing::BorderBox => inner_cross_size,
                BoxSizing::ContentBox => (inner_cross_size
                    - child_style.border.vertical_width()
                    - child_style.padding.vertical())
                .max(0.0),
            };
            child_style.height = Some(forced_h);
            let generated_styles = GeneratedContentStyles::resolve(
                child_el,
                &child_style,
                env.rules,
                &selector_ctx,
                env.fonts,
            );

            let mut child_ancestors = ancestors.to_vec();
            child_ancestors.push(
                flex_child_positions[child_idx].ancestor(child_el, child_el.children.is_empty()),
            );
            // The item's own content-box dimensions once stretched: percentage
            // children resolve their heights against this definite cross size.
            let item_content_w = (item.width
                - child_style.padding.horizontal()
                - child_style.border.horizontal_width())
            .max(0.0);
            let mut buf = Vec::new();
            if matches!(child_style.display, Display::Flex | Display::InlineFlex) {
                // A nested flex container carries its own main-axis distribution
                // that depends on the stretched height; re-layout it as a flex.
                let child_ctx = ctx
                    .with_parent_and_basis(
                        item.width,
                        width_for_percentages,
                        Some(inner_cross_size),
                        style.font_size,
                    )
                    .with_containing_block(descendant_containing_block);
                layout_flex_container(
                    child_el,
                    &child_style,
                    &child_ctx,
                    &mut buf,
                    &child_ancestors,
                    generated_styles.boxes(child_el),
                    positioned_depth,
                    env,
                );
            } else {
                // A stretched plain block item whose block children include a
                // percentage-height box: the first (intrinsic) pass treated those
                // percentages as `auto` because the item was height-less. Now that
                // align-items:stretch gives the item a definite height, re-flatten
                // it with that height so `height: 50%` descendants resolve against
                // it. Items with only inline/text content need no re-flatten — the
                // renderer stretches their cell visually (cell_render_h = line_cross).
                let has_block_kids = child_el.children.iter().any(|c| {
                    matches!(c, DomNode::Element(e) if e.tag.is_block() && !collects_as_inline_text(e.tag))
                });
                if !has_block_kids {
                    continue;
                }
                // `flatten_element` recomputes the item's own style from its
                // attributes, so the stretched height must be injected there.
                // Clone the item and append `height:<forced_h>pt` to its inline
                // style (inline declarations win the cascade, and layout units are
                // points). `forced_h` is already expressed in the item's own
                // box-sizing. With a now-definite block size, `height:50%`
                // descendants resolve against it.
                let mut forced_el = child_el.clone();
                let mut style_decl = forced_el
                    .attributes
                    .get("style")
                    .cloned()
                    .unwrap_or_default();
                if !style_decl.trim_end().is_empty() && !style_decl.trim_end().ends_with(';') {
                    style_decl.push(';');
                }
                style_decl.push_str(&format!("height:{forced_h}pt"));
                forced_el.attributes.insert("style".to_string(), style_decl);
                let child_ctx = ctx
                    .with_parent_and_basis(
                        item_content_w,
                        width_for_percentages,
                        Some(forced_h),
                        style.font_size,
                    )
                    .with_containing_block(descendant_containing_block);
                flatten_element(
                    &forced_el,
                    LayoutTreeContext::new(style, &child_ctx, ancestors)
                        .with_positioned_ancestor_depth(positioned_depth)
                        .for_element(flex_child_positions[child_idx].as_context())
                        .with_filter_application(FilterApplication::DeferToFormattingItem),
                    &mut buf,
                    env,
                );
            }
            if !buf.is_empty() {
                item.elements = buf;
                item.height = inner_cross_size;
                item.natural_height = inner_cross_size;
            }
        }
    }

    if direction.is_row() && lines.len() == 1 {
        if let Some(line) = lines.first_mut() {
            line.cross_size = line.cross_size.max(inner_cross_size);
        }
    }
    // Recompute total_cross after possibly growing a single line.
    let total_cross: f32 = if direction.is_row() {
        lines.iter().map(|l| l.cross_size).sum::<f32>()
            + if lines.len() > 1 {
                (lines.len() - 1) as f32 * line_gap
            } else {
                0.0
            }
    } else if lines.len() > 1 {
        lines.iter().map(|l| l.cross_size).sum::<f32>() + (lines.len() - 1) as f32 * line_gap
    } else {
        lines.iter().map(|l| l.cross_size).fold(0.0f32, f32::max)
    };

    // align-content distributes wrapped flex LINES along the cross axis when the
    // container has more than one line and spare cross space. For rows the cross
    // axis is vertical; for column-wrap it is horizontal.
    let line_count = lines.len();
    let cross_axis_extent = if direction.is_row() {
        inner_cross_size
    } else {
        inner_width
    };
    let (ac_lead, ac_between, ac_line_stretch) = if line_count > 1 {
        let lines_cross: f32 = lines.iter().map(|l| l.cross_size).sum::<f32>();
        let base_gaps = (line_count - 1) as f32 * line_gap;
        // Signed cross free space — kept negative on overflow so center/flex-end
        // honor alignment past the edge (css-flexbox-1 §8.4 + css-align-3 §9).
        let ac_free = cross_axis_extent - lines_cross - base_gaps;
        let neg = ac_free < 0.0;
        // flex-wrap:wrap-reverse swaps the cross-start/cross-end edges
        // (css-flexbox-1 §5.3), so flex-start/flex-end exchange leads. The line
        // *order* is reversed separately at placement time (`line_order`).
        let effective_ac = if wrap == FlexWrap::WrapReverse {
            match style.align_content {
                AlignContent::FlexStart => AlignContent::FlexEnd,
                AlignContent::FlexEnd => AlignContent::FlexStart,
                other => other,
            }
        } else {
            style.align_content
        };
        match effective_ac {
            AlignContent::FlexStart => (0.0, 0.0, 0.0),
            AlignContent::FlexEnd => (ac_free, 0.0, 0.0),
            AlignContent::Center => (ac_free / 2.0, 0.0, 0.0),
            AlignContent::SpaceBetween => {
                // Negative free space behaves as flex-start (§8.4).
                if neg || line_count <= 1 {
                    (0.0, 0.0, 0.0)
                } else {
                    (0.0, ac_free / (line_count - 1) as f32, 0.0)
                }
            }
            AlignContent::SpaceAround => {
                // Distributed alignment falls back to safe center, which
                // becomes cross-start when overflow would otherwise occur.
                if neg {
                    (0.0, 0.0, 0.0)
                } else {
                    let around = ac_free / line_count as f32;
                    (around / 2.0, around, 0.0)
                }
            }
            AlignContent::SpaceEvenly => {
                // Its fallback is likewise safe on overflow.
                if neg {
                    (0.0, 0.0, 0.0)
                } else {
                    let ev = ac_free / (line_count + 1) as f32;
                    (ev, ev, 0.0)
                }
            }
            // stretch grows each line equally to fill the spare cross space, but
            // never shrinks lines when the space is negative.
            AlignContent::Stretch => {
                if neg {
                    (0.0, 0.0, 0.0)
                } else {
                    (0.0, 0.0, ac_free / line_count as f32)
                }
            }
        }
    } else {
        (0.0, 0.0, 0.0)
    };
    if ac_line_stretch > 0.0 {
        for line in lines.iter_mut() {
            line.cross_size += ac_line_stretch;
        }
    }
    let column_wrap_lines = !direction.is_row() && lines.len() > 1;

    if flex_writing_mode.is_vertical() && !wrap.wraps() {
        let mut cursor = 0.0_f32;
        let mut vertical_cells = Vec::new();
        let ordered_items: Vec<usize> = if direction == FlexDirection::ColumnReverse
            || direction == FlexDirection::RowReverse
        {
            (0..items.len()).rev().collect()
        } else {
            (0..items.len()).collect()
        };
        for (pos, item_idx) in ordered_items.iter().copied().enumerate() {
            if pos > 0 {
                cursor += gap;
            }
            let item = &items[item_idx];
            let (x_offset, y_offset) = if direction.is_row() {
                let x = if flex_writing_mode.block_axis_reversed() {
                    inner_width - item.width
                } else {
                    0.0
                };
                (x + item.margin_cross_start, cursor + item.margin_main_start)
            } else {
                let x = if flex_writing_mode.block_axis_reversed() {
                    inner_width - cursor - item.width
                } else {
                    cursor
                };
                (x + item.margin_main_start, item.margin_cross_start)
            };
            let mut cell = item
                .elements
                .iter()
                .find_map(|element| {
                    flex_cell_from_text_block(
                        element.as_ref(),
                        x_offset,
                        y_offset,
                        item.width,
                        item.is_relative,
                        item.z_index,
                    )
                })
                .unwrap_or_else(|| {
                    flex_cell_with_nested_item(
                        &item.elements,
                        FlexCell {
                            x_offset,
                            width: item.width,
                            natural_height: item.height,
                            fragmentation: FlexItemFragmentation::definite(),
                            align_self: AlignSelf::FlexStart,
                            y_offset,
                            line_cross_size: item.height,
                            positioning: flex_item_positioning(&item.elements, item.is_relative),
                            ..Default::default()
                        },
                    )
                });
            if item.is_relative {
                cell.x_offset += item.rel_left;
                cell.y_offset += item.rel_top;
            }
            cell.positioning = flex_cell_positioning(&cell, &item.elements, item.is_relative);
            cell.paint.box_paint.group.stacking.z_index = item.z_index;
            cell.paint.box_paint.group.stacking.role = StackingRole::FlexItem;
            vertical_cells.push(cell);
            cursor += if direction.is_row() {
                item.height
            } else {
                item.width
            };
        }
        output.push(flex_row_node(
            style,
            vertical_cells,
            Vec::new(),
            Default::default(),
            inner_cross_size,
            BlockMargins::new(style.margin.top, style.margin.bottom),
            h_offset,
            block_w,
            inner_cross_size,
            AlignItems::FlexStart,
            abs_cb_depth,
        ));
        output.append(&mut abs_output);
        return;
    }

    // For single-line column direction, emit container background separately.
    // Multi-line column-wrap uses a FlexRow wrapper below so the items can be
    // positioned in additional columns while the container remains one flow box.
    let column_auto_overflows_fragmentainer = !direction.is_row()
        && !column_wrap_lines
        && style.height.is_none()
        && container_h + style.border.vertical_width() + style.margin.vertical()
            > ctx.available_height();
    let emitted_column_bg = !direction.is_row()
        && !column_wrap_lines
        && !column_auto_overflows_fragmentainer
        && (has_background_paint(style)
            || style.has_border_decoration()
            || !style.box_shadow.is_empty());
    if emitted_column_bg {
        // Emit the container background/border as a visual element.
        // It advances y by its full height in paginate.  We then emit a
        // negative-margin spacer to pull y back so children flow *inside*
        // the background rather than after it.
        // The background block is a bordered TextBlock, so it advances the
        // cursor by its *border-box* height (`block_height` + vertical border)
        // in the flow. The pull-back spacer undoes that whole advance back to
        // the border-box top; the first item then re-adds the container's
        // top border + padding in its own leading to flow inside the box.
        let bg_flow_height = container_h + style.border.vertical_width();
        let mut background = TextBlock::from_style(
            Vec::new(),
            style,
            crate::layout::elements::BoxModel::from_style(
                style,
                BlockMargins::new(style.margin.top, 0.0),
            ),
        );
        background.box_model.size =
            crate::layout::elements::LayoutSize::fixed(block_w, Some(container_h));
        background.paint.border_radii = style.resolve_corner_radii(block_w, inner_cross_size);
        background.paint.group.stacking = Default::default();
        background.clipping.rect = style
            .overflow
            .clips()
            .then(|| Rect::from_xywh(0.0, 0.0, block_w, container_h));
        output.push(background.boxed());

        let mut pullback = TextBlock::empty_spacer();
        pullback.box_model.margins = BlockMargins::new(-bg_flow_height, 0.0);
        output.push(pullback.boxed());
    }
    let column_bg_pair = if emitted_column_bg && output.len() >= 2 {
        let len = output.len();
        Some((output[len - 2].clone(), output[len - 1].clone()))
    } else {
        None
    };

    // Position items within the flex container and emit them. align-content
    // leading bumps the first line away from the cross-start edge.
    let mut cross_offset = ac_lead;
    // All flex cells across every line, merged into a single FlexRow for
    // row direction. This keeps container borders/backgrounds around every
    // wrapped line and keeps pagination flow correct.
    let mut all_flex_cells: Vec<FlexCell> = Vec::new();

    // `flex-wrap: wrap-reverse` stacks the wrapped lines from the cross-end
    // toward the cross-start, i.e. the visual line order is reversed. We keep
    // the cross_offset accumulation forward (cross-start downward) but feed the
    // lines in reversed order so the last source line lands at the top.
    let line_order: Vec<usize> = if wrap == FlexWrap::WrapReverse {
        (0..lines.len()).rev().collect()
    } else {
        (0..lines.len()).collect()
    };

    for (visual_pos, &line_idx) in line_order.iter().enumerate() {
        let line = &lines[line_idx];
        let line_id = FlexLineId::from_index(line_idx);
        let line_cells_start = all_flex_cells.len();
        if visual_pos > 0 {
            cross_offset += ac_between;
        }
        let line_items: Vec<usize> = line.item_indices.clone();
        let line_item_count = line_items.len();

        match direction {
            FlexDirection::Row | FlexDirection::RowReverse => {
                let total_gap = if line_item_count > 1 {
                    (line_item_count - 1) as f32 * gap
                } else {
                    0.0
                };
                let mut flexible_lengths = line_items
                    .iter()
                    .map(|&index| {
                        let item = &items[index];
                        FlexibleLength::new(
                            item.base_width,
                            item.main_constraints,
                            item.flex_grow,
                            item.flex_shrink,
                            item.margin_main_start + item.margin_main_end,
                        )
                    })
                    .collect::<Vec<_>>();
                resolve_flexible_lengths(&mut flexible_lengths, (inner_width - total_gap).max(0.0));
                for (&index, resolved) in line_items.iter().zip(flexible_lengths) {
                    items[index].width = resolved.target;
                }

                for &i in &line_items {
                    if direction.is_row() && !items[i].fragmentation.block_size.is_explicit() {
                        let Some(ratio) = items[i].aspect_ratio else {
                            continue;
                        };
                        if ratio <= 0.0 {
                            continue;
                        }
                        let border_h = items[i]
                            .elements
                            .first()
                            .and_then(|element| text_block_border_height(element.as_ref()))
                            .unwrap_or(0.0);
                        let border_box_h = (items[i].width / ratio).max(border_h);
                        let pad_box_h = (border_box_h - border_h).max(0.0);
                        if let Some(element) = items[i].elements.first_mut() {
                            update_text_block_height(element.as_mut(), pad_box_h);
                        }
                        items[i].height = border_box_h;
                        items[i].natural_height = border_box_h;
                    }
                }

                // Re-layout every item whose resolved main size differs from
                // its flex base. Grow, shrink, and min/max freezing all establish
                // the same final containing-block width for descendants.
                for &i in &line_items {
                    if !equal_with_roundoff(items[i].width, items[i].base_width) {
                        let final_w = items[i].width;
                        let Some(ElementFlexItemIndex(child_idx)) = items[i].source.element else {
                            continue;
                        };
                        let child_el = child_elements[child_idx];
                        let live_counters = env.counter_state.clone();
                        *env.counter_state = items[i].source.counter_replay.clone();
                        if items[i].uses_general_layout {
                            // `final_w` is the item's resolved BORDER-box main size.
                            // `flatten_element` derives the child's block (border-box)
                            // width by adding the child's own horizontal border to the
                            // available width it is handed. Passing the border-box
                            // width verbatim therefore double-counted the child's
                            // border: a bordered auto-width child (e.g. a nested grid
                            // host) rendered `final_w + its border` wide, overflowing
                            // the flex item. Subtract the child's own horizontal
                            // border so its border-box lands exactly on `final_w`.
                            let relayout_classes = child_el.class_list();
                            let relayout_selector_ctx = flex_child_positions[child_idx]
                                .as_context()
                                .selector_context(ancestors, child_el.children.is_empty());
                            let relayout_child_style = compute_style_with_context_with_font_metrics(
                                child_el.tag,
                                child_el.style_attr(),
                                &parent_for_children,
                                env.rules,
                                child_el.tag_name(),
                                &relayout_classes,
                                child_el.id(),
                                &child_el.attributes,
                                &relayout_selector_ctx,
                                env.font_metrics(),
                            );
                            let relayout_generated_styles = GeneratedContentStyles::resolve(
                                child_el,
                                &relayout_child_style,
                                env.rules,
                                &relayout_selector_ctx,
                                env.fonts,
                            );
                            // Only auto-width children fill the available width (and
                            // thus need the border-deduction); an explicit width
                            // resolves the child's box itself, so leave `final_w`.
                            let relayout_avail = if relayout_child_style.width.is_some() {
                                final_w
                            } else {
                                (final_w - relayout_child_style.border.horizontal_width()).max(0.0)
                            };
                            // A nested flex container must re-run its OWN flex
                            // algorithm at the final (grown) main-axis width, and —
                            // when it stretches — with the line's cross size forced
                            // as its definite height, so its flex-grow children
                            // distribute against a real main size. The generic
                            // `flatten_element` re-flatten below lays a flex item out
                            // at an indefinite height, collapsing its grow children
                            // to zero (the pre-grow stretch pass above used the
                            // ungrown width, which the grow re-flatten then clobbered).
                            if matches!(
                                relayout_child_style.display,
                                Display::Flex | Display::InlineFlex
                            ) && direction.is_row()
                                && inner_cross_size > 0.0
                            {
                                let mut fstyle = relayout_child_style.clone();
                                let stretches = matches!(items[i].align_self, AlignSelf::Stretch)
                                    || (matches!(items[i].align_self, AlignSelf::Auto)
                                        && align == AlignItems::Stretch);
                                if stretches && fstyle.height.is_none() {
                                    fstyle.height = Some(match fstyle.box_sizing {
                                        BoxSizing::BorderBox => inner_cross_size,
                                        BoxSizing::ContentBox => (inner_cross_size
                                            - fstyle.border.vertical_width()
                                            - fstyle.padding.vertical())
                                        .max(0.0),
                                    });
                                }
                                let mut fbuf = Vec::new();
                                let mut fancestors = ancestors.to_vec();
                                fancestors.push(
                                    flex_child_positions[child_idx]
                                        .ancestor(child_el, child_el.children.is_empty()),
                                );
                                let fctx = ctx
                                    .with_parent_and_basis(
                                        final_w,
                                        width_for_percentages,
                                        Some(inner_cross_size),
                                        style.font_size,
                                    )
                                    .with_containing_block(descendant_containing_block);
                                let counter_scope = env.counter_state.enter_element(&fstyle);
                                layout_flex_container(
                                    child_el,
                                    &fstyle,
                                    &fctx,
                                    &mut fbuf,
                                    &fancestors,
                                    relayout_generated_styles.boxes(child_el),
                                    positioned_depth,
                                    env,
                                );
                                env.counter_state.leave_element(counter_scope);
                                if !fbuf.is_empty() {
                                    items[i].elements = fbuf;
                                    items[i].height = if stretches {
                                        inner_cross_size
                                    } else {
                                        items[i]
                                            .elements
                                            .iter()
                                            .map(|element| {
                                                estimate_element_height(element.as_ref())
                                            })
                                            .sum()
                                    };
                                }
                                *env.counter_state = live_counters;
                                continue;
                            }
                            let mut relayout_buf = Vec::new();
                            let relayout_ctx = ctx
                                .with_parent_and_basis(
                                    relayout_avail,
                                    width_for_percentages,
                                    Some(10000.0),
                                    style.font_size,
                                )
                                .with_containing_block(descendant_containing_block);
                            flatten_element(
                                child_el,
                                LayoutTreeContext::new(style, &relayout_ctx, ancestors)
                                    .with_positioned_ancestor_depth(positioned_depth)
                                    .for_element(flex_child_positions[child_idx].as_context())
                                    .with_filter_application(
                                        FilterApplication::DeferToFormattingItem,
                                    ),
                                &mut relayout_buf,
                                env,
                            );
                            if !relayout_buf.is_empty() {
                                items[i].elements = relayout_buf;
                                items[i].height = items[i]
                                    .elements
                                    .iter()
                                    .map(|element| estimate_element_height(element.as_ref()))
                                    .sum();
                            }
                        } else {
                            let relayout_classes = child_el.class_list();
                            let relayout_selector_ctx = flex_child_positions[child_idx]
                                .as_context()
                                .selector_context(ancestors, child_el.children.is_empty());
                            let relayout_child_style = compute_style_with_context_with_font_metrics(
                                child_el.tag,
                                child_el.style_attr(),
                                &parent_for_children,
                                env.rules,
                                child_el.tag_name(),
                                &relayout_classes,
                                child_el.id(),
                                &child_el.attributes,
                                &relayout_selector_ctx,
                                env.font_metrics(),
                            );
                            let relayout_generated_styles = GeneratedContentStyles::resolve(
                                child_el,
                                &relayout_child_style,
                                env.rules,
                                &relayout_selector_ctx,
                                env.fonts,
                            );
                            let mut runs = Vec::new();
                            let mut relayout_ancestors = ancestors.to_vec();
                            relayout_ancestors.push(
                                flex_child_positions[child_idx]
                                    .ancestor(child_el, child_el.children.is_empty()),
                            );
                            let counter_scope =
                                env.counter_state.enter_element(&relayout_child_style);
                            InlineRunCollector::new(
                                env.rules,
                                env.fonts,
                                env.counter_state,
                                &mut *env.resources,
                            )
                            .collect(
                                InlineContentSequence::with_generated(
                                    &child_el.children,
                                    relayout_generated_styles.boxes(child_el),
                                ),
                                &relayout_child_style,
                                &mut runs,
                                None,
                                &relayout_ancestors,
                            );
                            env.counter_state.leave_element(counter_scope);
                            let content_w = (final_w
                                - relayout_child_style.padding.horizontal()
                                - relayout_child_style.border.horizontal_width())
                            .max(0.0);
                            let lines = if runs.is_empty() {
                                Vec::new()
                            } else {
                                wrap_text_runs(
                                    runs,
                                    TextWrapOptions::new(
                                        content_w,
                                        used_font_size(&relayout_child_style, env.fonts),
                                        text_run_line_height_factor(
                                            &relayout_child_style,
                                            env.fonts,
                                        ),
                                        relayout_child_style.overflow_wrap,
                                    )
                                    .with_white_space(relayout_child_style.white_space)
                                    .with_parent_strut(parent_line_strut(
                                        &relayout_child_style,
                                        env.fonts,
                                    ))
                                    .with_rtl(relayout_child_style.direction_rtl)
                                    .with_bidi_override(relayout_child_style.bidi_override),
                                    env.fonts,
                                )
                            };
                            let text_h: f32 = lines.iter().map(|l| l.height).sum();
                            let mut border_box_h = resolve_padding_box_height(
                                text_h,
                                relayout_child_style.height,
                                relayout_child_style.padding,
                                relayout_child_style.border.widths(),
                                relayout_child_style.box_sizing,
                            ) + relayout_child_style.border.vertical_width();
                            if relayout_child_style.height.is_none() {
                                if let Some(ratio) = relayout_child_style.aspect_ratio {
                                    if ratio > 0.0 {
                                        border_box_h = border_box_h.max(final_w / ratio);
                                    }
                                }
                            }
                            if let Some(element) = items[i].elements.first_mut() {
                                let height = (relayout_child_style.height.is_some()
                                    || relayout_child_style.aspect_ratio.is_some())
                                .then(|| {
                                    (border_box_h - relayout_child_style.border.vertical_width())
                                        .max(0.0)
                                });
                                let clip_height = relayout_child_style
                                    .overflow
                                    .clips()
                                    .then_some(border_box_h);
                                update_text_block_layout(
                                    element.as_mut(),
                                    Some(lines),
                                    final_w,
                                    height,
                                    clip_height,
                                );
                            }
                            items[i].height = border_box_h + relayout_child_style.margin.vertical();
                            items[i].natural_height = items[i].height;
                        }
                        *env.counter_state = live_counters;
                    }
                }

                // Recompute the true remaining main free space from the FINAL
                // item widths after grow/shrink. With `flex-shrink:0` items that
                // overflow the line this stays NEGATIVE, so `justify-content`
                // positions from the proper edge (center/flex-end/space-*) instead
                // of collapsing to flex-start. The earlier `free_space` was forced
                // to 0 by the shrink pass, masking real overflow.
                let final_item_width: f32 = line_items.iter().map(|&i| items[i].width).sum();
                // Fixed (non-auto) main-axis item margins consume free space too,
                // so subtract them before justify-content / auto-margin packing.
                let total_main_margin: f32 = line_items
                    .iter()
                    .map(|&i| items[i].margin_main_start + items[i].margin_main_end)
                    .sum();
                let free_space = inner_width - final_item_width - total_gap - total_main_margin;

                // css-flexbox-1 §8.1: before justify-content runs, positive main
                // free space is split equally among the line's `auto` main-axis
                // margins, which then override justify-content. With no auto
                // margins this is inert and justify-content distributes normally.
                let auto_main_count: u32 = line_items
                    .iter()
                    .map(|&i| {
                        items[i].margin_main_start_auto as u32
                            + items[i].margin_main_end_auto as u32
                    })
                    .sum();
                let use_auto_margins = auto_main_count > 0 && free_space > 0.0;
                let auto_share = if use_auto_margins {
                    free_space / auto_main_count as f32
                } else {
                    0.0
                };
                let justify_free = if use_auto_margins {
                    0.0
                } else {
                    free_space.max(0.0)
                };

                // Calculate starting x and spacing based on justify-content.
                // Distributed values use safe fallbacks on overflow; authored
                // center/flex-end remain unsafe and may overflow the start edge.
                let (mut x, extra_gap) = if free_space < 0.0 && !use_auto_margins {
                    match justify {
                        JustifyContent::FlexStart
                        | JustifyContent::SafeCenter
                        | JustifyContent::SpaceBetween
                        | JustifyContent::SpaceAround
                        | JustifyContent::SpaceEvenly => (0.0, 0.0),
                        JustifyContent::FlexEnd => (free_space, 0.0),
                        JustifyContent::Center => (free_space / 2.0, 0.0),
                    }
                } else {
                    match justify {
                        JustifyContent::FlexStart => (0.0, 0.0),
                        JustifyContent::FlexEnd => (justify_free, 0.0),
                        JustifyContent::Center | JustifyContent::SafeCenter => {
                            (justify_free / 2.0, 0.0)
                        }
                        JustifyContent::SpaceBetween => {
                            if line_item_count > 1 {
                                (0.0, justify_free / (line_item_count - 1) as f32)
                            } else {
                                (0.0, 0.0)
                            }
                        }
                        JustifyContent::SpaceAround => {
                            let around = justify_free / line_item_count as f32;
                            (around / 2.0, around)
                        }
                        JustifyContent::SpaceEvenly => {
                            let ev = justify_free / (line_item_count + 1) as f32;
                            (ev, ev)
                        }
                    }
                };

                // Build FlexCells for this row line.
                let mut flex_cells = Vec::new();
                // A trailing `auto` main margin on a prior item pushes the next
                // item along the main axis; carry it forward into the cursor.
                let mut pending_trailing_auto = 0.0_f32;
                for &item_idx in &line_items {
                    // Apply the previous item's trailing auto margin, then this
                    // item's leading auto margin, before placing its cell.
                    x += pending_trailing_auto;
                    if items[item_idx].margin_main_start_auto {
                        x += auto_share;
                    }
                    pending_trailing_auto = if items[item_idx].margin_main_end_auto {
                        auto_share
                    } else {
                        0.0
                    };
                    // Honor the item's fixed leading main-axis margin: it offsets
                    // the cursor so the cell sits after the margin, and the
                    // trailing margin is added to the advance below.
                    x += items[item_idx].margin_main_start;
                    let item = &items[item_idx];

                    if item.is_table
                        && item.contains_nested_forced_break
                        && let Some((decoration_index, decoration)) = item
                            .elements
                            .iter()
                            .enumerate()
                            .find_map(|(index, element)| {
                                element
                                    .table_box_decoration_owner()
                                    .map(|owner| (index, owner.decoration()))
                            })
                        && let Some(mut cell) = flex_cell_from_text_block(
                            decoration,
                            x,
                            0.0,
                            item.width,
                            item.is_relative,
                            item.z_index,
                        )
                    {
                        cell.lines.clear();
                        cell.width = item.width;
                        cell.natural_height = item.height;
                        cell.line_cross_size = item.height;
                        cell.fragmentation = item.fragmentation;
                        cell.cross_min = item.cross_min;
                        cell.cross_max = item.cross_max;
                        cell.align_self = item.align_self;
                        cell.nested_elements = item
                            .elements
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| *index != decoration_index)
                            .map(|(_, element)| element.clone())
                            .collect();
                        cell.nested_origin = FlexNestedOrigin::TableBorderBox;
                        flex_cells.push(cell);
                        x += item.width + gap + extra_gap + item.margin_main_end;
                        continue;
                    }

                    // A flex item that is itself a flex container establishes an
                    // independent formatting context: its `elements` already carry
                    // every inner box's own background/width/height/x-offset (a
                    // nested `FlexRow`, or a column's per-child TextBlocks). The
                    // text-merge path below would keep only the first box's
                    // background and drop the rest (blank nested rows, vanished
                    // column children), so route the whole sub-layout through
                    // `nested_elements` for the renderer to paint each inner box.
                    if item.is_flex_container {
                        flex_cells.push(flex_cell_with_nested_item(
                            &item.elements,
                            FlexCell {
                                x_offset: x,
                                width: item.width,
                                natural_height: item.height,
                                fragmentation: item.fragmentation,
                                cross_min: item.cross_min,
                                cross_max: item.cross_max,
                                align_self: item.align_self,
                                positioning: flex_item_positioning(
                                    &item.elements,
                                    item.is_relative,
                                ),
                                ..Default::default()
                            },
                        ));
                        x += item.width + gap + extra_gap + item.margin_main_end;
                        continue;
                    }

                    // Complex items (multiple elements): merge all lines
                    // into a single FlexCell, inserting margin spacing
                    if item.elements.len() > 1 {
                        let mut merged_lines = Vec::new();
                        let mut first_bg = None;
                        let mut first_padding = EdgeSizes::ZERO;
                        let mut first_radii = CornerRadii::ZERO;
                        let mut is_first = true;
                        // Check if all elements are TextBlocks without borders (mergeable).
                        // TextBlocks with borders must go through nested_elements
                        // so the renderer can draw their individual borders.
                        let all_text_blocks = item
                            .elements
                            .iter()
                            .all(|element| is_borderless_text_block(element.as_ref()));

                        if !all_text_blocks {
                            // Mixed elements (e.g. TextBlock + TableRow):
                            // store in nested_elements for the renderer to handle
                            flex_cells.push(flex_cell_with_nested_item(
                                &item.elements,
                                FlexCell {
                                    x_offset: x,
                                    width: item.width,
                                    natural_height: item.height,
                                    fragmentation: item.fragmentation,
                                    cross_min: item.cross_min,
                                    cross_max: item.cross_max,
                                    align_self: item.align_self,
                                    positioning: flex_item_positioning(
                                        &item.elements,
                                        item.is_relative,
                                    ),
                                    ..Default::default()
                                },
                            ));
                            x += item.width + gap + extra_gap + item.margin_main_end;
                            continue;
                        }

                        for element in &item.elements {
                            merge_text_block_into_cell(
                                element.as_ref(),
                                &mut merged_lines,
                                &mut first_bg,
                                &mut first_padding,
                                &mut first_radii,
                                &mut is_first,
                            );
                        }
                        // Calculate natural height for merged item
                        let natural_h: f32 = merged_lines.iter().map(|l| l.height).sum();
                        flex_cells.push(FlexCell {
                            lines: merged_lines,
                            x_offset: x,
                            width: item.width,
                            natural_height: natural_h,
                            fragmentation: item.fragmentation,
                            cross_min: item.cross_min,
                            cross_max: item.cross_max,
                            align_self: item.align_self,
                            padding: first_padding,
                            paint: crate::layout::cells::CellPaint {
                                box_paint: BoxPaint {
                                    background: crate::layout::elements::BackgroundPaint {
                                        color: first_bg,
                                        ..Default::default()
                                    },
                                    border_radii: first_radii,
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                            positioning: flex_item_positioning(&item.elements, item.is_relative),
                            ..Default::default()
                        });
                        x += item.width + gap + extra_gap + item.margin_main_end;
                        continue;
                    }

                    // Simple items: extract into FlexCell
                    if let Some(mut cell) = item.elements.first().and_then(|element| {
                        flex_cell_from_text_block(
                            element.as_ref(),
                            x,
                            0.0,
                            item.width,
                            item.is_relative,
                            item.z_index,
                        )
                    }) {
                        let natural_height = cell.natural_height;
                        if item
                            .elements
                            .first()
                            .is_some_and(|element| is_clipped_text_block(element.as_ref()))
                        {
                            let mut nested = item.elements.clone();
                            if let Some(element) = nested.first_mut() {
                                let border_height =
                                    text_block_border_height(element.as_ref()).unwrap_or_default();
                                update_text_block_layout(
                                    element.as_mut(),
                                    None,
                                    item.width,
                                    Some((natural_height - border_height).max(0.0)),
                                    Some(natural_height),
                                );
                            }
                            flex_cells.push(flex_cell_with_nested_item(
                                &nested,
                                FlexCell {
                                    x_offset: x,
                                    width: item.width,
                                    natural_height,
                                    fragmentation: item.fragmentation,
                                    cross_min: item.cross_min,
                                    cross_max: item.cross_max,
                                    align_self: item.align_self,
                                    positioning: flex_item_positioning(
                                        &item.elements,
                                        item.is_relative,
                                    ),
                                    ..Default::default()
                                },
                            ));
                            x += item.width + gap + extra_gap + item.margin_main_end;
                            continue;
                        }
                        cell.fragmentation = item.fragmentation;
                        cell.cross_min = item.cross_min;
                        cell.cross_max = item.cross_max;
                        cell.align_self = item.align_self;
                        flex_cells.push(cell);
                    } else {
                        // Single non-TextBlock element (e.g. Container): store
                        // in nested_elements for the renderer to handle.
                        flex_cells.push(flex_cell_with_nested_item(
                            &item.elements,
                            FlexCell {
                                x_offset: x,
                                width: item.width,
                                natural_height: item.height,
                                fragmentation: item.fragmentation,
                                cross_min: item.cross_min,
                                cross_max: item.cross_max,
                                align_self: item.align_self,
                                positioning: flex_item_positioning(
                                    &item.elements,
                                    item.is_relative,
                                ),
                                ..Default::default()
                            },
                        ));
                    }

                    x += item.width + gap + extra_gap + item.margin_main_end;
                }

                // The row main-start edge is right only when `row-reverse` and
                // `direction:rtl` disagree. With both set, the reversals cancel:
                // visual main-start is left again.
                if (direction == FlexDirection::RowReverse) ^ style.direction_rtl {
                    for cell in flex_cells.iter_mut() {
                        cell.x_offset = inner_width - cell.x_offset - cell.width;
                    }
                }

                // `flex-wrap: wrap-reverse` flips the cross-start edge to the
                // cross-end. Items that would anchor to a line's top (the
                // default for non-stretched items) instead anchor to the line
                // bottom. The renderer positions each cell within its line by
                // `align`, so for a flex-start-anchored, non-stretching item we
                // pre-shift its y by the slack inside the line to land it at the
                // cross-end. (Stretch items already fill the line.)
                let wrap_reversed = wrap == FlexWrap::WrapReverse;
                let resolved_line_cross_size = line_items
                    .iter()
                    .map(|&i| items[i].height)
                    .fold(line.cross_size, f32::max);

                // Stamp each cell with its cross-axis position within the
                // container so a single FlexRow can span every wrapped line.
                for cell in flex_cells.iter_mut() {
                    let anchor_start = matches!(
                        cell.align_self,
                        AlignSelf::Auto | AlignSelf::FlexStart | AlignSelf::Baseline
                    ) && (align == AlignItems::FlexStart
                        || align == AlignItems::Baseline
                        || cell.align_self == AlignSelf::FlexStart
                        || cell.align_self == AlignSelf::Baseline
                        || (matches!(cell.align_self, AlignSelf::Auto)
                            && align == AlignItems::Stretch
                            && cell.fragmentation.block_size.is_explicit()));
                    let cross_pad = if wrap_reversed && anchor_start {
                        (resolved_line_cross_size - cell.natural_height).max(0.0)
                    } else {
                        0.0
                    };
                    cell.y_offset = cross_offset + cross_pad;
                    cell.line_cross_size = resolved_line_cross_size;
                }

                // Apply each item's fixed cross-axis leading margin (margin-top
                // for a row container) and its `position: relative` paint offset.
                // Cells are 1:1 with `line_items` in placement order, so zip them.
                // Relative offsets are physical and applied after the row-reverse
                // mirror; a relatively-offset item is flagged positioned so it
                // paints above its in-flow siblings.
                for (cell, &item_idx) in flex_cells.iter_mut().zip(line_items.iter()) {
                    let it = &items[item_idx];
                    cell.y_offset += it.margin_cross_start;
                    if it.is_relative {
                        cell.x_offset += it.rel_left;
                        cell.y_offset += it.rel_top;
                    }
                    cell.positioning = flex_cell_positioning(cell, &it.elements, it.is_relative);
                    cell.paint.box_paint.group.stacking.z_index = it.z_index;
                    cell.paint.box_paint.group.stacking.role = StackingRole::FlexItem;
                }

                if align_last_baseline {
                    let line_cross = if lines.len() == 1 && inner_cross_size > 0.0 {
                        inner_cross_size
                    } else {
                        resolved_line_cross_size
                    };
                    for cell in flex_cells.iter_mut() {
                        let effective_align = match cell.align_self {
                            AlignSelf::Auto => AlignItems::Baseline,
                            AlignSelf::FlexStart => AlignItems::FlexStart,
                            AlignSelf::FlexEnd => AlignItems::FlexEnd,
                            AlignSelf::Center => AlignItems::Center,
                            AlignSelf::Baseline => AlignItems::Baseline,
                            AlignSelf::Stretch => AlignItems::Stretch,
                        };
                        if effective_align == AlignItems::Baseline {
                            cell.y_offset += (line_cross - cell.natural_height).max(0.0);
                            cell.line_cross_size = line_cross;
                        }
                    }
                }
                all_flex_cells.extend(flex_cells);
            }
            FlexDirection::Column | FlexDirection::ColumnReverse => {
                let total_gap = if line_item_count > 1 {
                    (line_item_count - 1) as f32 * gap
                } else {
                    0.0
                };

                // Column main-axis flex grow/shrink: the main axis is the block
                // (vertical) axis, so distribute/absorb the container's spare
                // height across the items, clamped to each item's min/max main
                // (min-height / max-height). Only run when the container has a
                // definite main size (`inner_cross_size > 0`). Mirrors the row
                // resolution but along the height.
                if inner_cross_size > 0.0 {
                    let sum_h: f32 = line_items.iter().map(|&i| items[i].height).sum();
                    let mut col_free = inner_cross_size - sum_h - total_gap;
                    let total_grow: f32 = line_items.iter().map(|&i| items[i].flex_grow).sum();
                    if col_free > 0.0 && total_grow > 0.0 {
                        let mut frozen = vec![false; line_items.len()];
                        // §9.7 step 4.b: cap the distributed space to the flex
                        // factor sum when it is below 1 (the rest stays free).
                        let mut remaining = if total_grow < 1.0 {
                            col_free * total_grow
                        } else {
                            col_free
                        };
                        for _ in 0..=line_items.len() {
                            let active: f32 = line_items
                                .iter()
                                .enumerate()
                                .filter(|(li, _)| !frozen[*li])
                                .map(|(_, &i)| items[i].flex_grow)
                                .sum();
                            if active <= 0.0 || !is_positive_with_roundoff(remaining) {
                                break;
                            }
                            let mut froze = false;
                            let mut consumed = 0.0;
                            for (li, &i) in line_items.iter().enumerate() {
                                if frozen[li] {
                                    continue;
                                }
                                let share = remaining * (items[i].flex_grow / active);
                                let target = items[i].height + share;
                                if let Some(maximum) = items[i].main_constraints.maximum()
                                    && target >= maximum
                                {
                                    consumed += maximum - items[i].height;
                                    items[i].height = maximum;
                                    frozen[li] = true;
                                    froze = true;
                                } else {
                                    items[i].height = target;
                                    consumed += share;
                                }
                            }
                            remaining -= consumed;
                            if !froze {
                                break;
                            }
                        }
                        col_free = 0.0;
                    }
                    if col_free < 0.0 {
                        let mut frozen = vec![false; line_items.len()];
                        // §9.7 step 4.b (shrink): absorb only the flex-shrink
                        // factor sum's fraction of the deficit when it is below 1.
                        let total_shrink: f32 =
                            line_items.iter().map(|&i| items[i].flex_shrink).sum();
                        let mut deficit = if total_shrink < 1.0 {
                            -col_free * total_shrink
                        } else {
                            -col_free
                        };
                        for _ in 0..=line_items.len() {
                            let weight_sum: f32 = line_items
                                .iter()
                                .enumerate()
                                .filter(|(li, _)| !frozen[*li])
                                .map(|(_, &i)| items[i].flex_shrink * items[i].height)
                                .sum();
                            if weight_sum <= 0.0 || !is_positive_with_roundoff(deficit) {
                                break;
                            }
                            let mut froze = false;
                            let mut removed = 0.0;
                            for (li, &i) in line_items.iter().enumerate() {
                                if frozen[li] {
                                    continue;
                                }
                                let weight = items[i].flex_shrink * items[i].height;
                                let reduce = deficit * (weight / weight_sum);
                                let target = items[i].height - reduce;
                                let floor =
                                    items[i].main_constraints.minimum().unwrap_or(0.0).max(0.0);
                                if target <= floor {
                                    removed += items[i].height - floor;
                                    items[i].height = floor;
                                    frozen[li] = true;
                                    froze = true;
                                } else {
                                    items[i].height = target;
                                    removed += reduce;
                                }
                            }
                            deficit -= removed;
                            if !froze {
                                break;
                            }
                        }
                    }
                    // Keep natural_height in sync so cross-axis emission uses the
                    // resolved main size.
                    for &i in &line_items {
                        items[i].natural_height = items[i].height;
                    }
                }

                let total_item_height: f32 = line_items.iter().map(|&i| items[i].height).sum();
                // Main-axis (vertical) free space within the container's content
                // box. `justify-content` distributes it as leading before the
                // first item and extra spacing between items. `inner_cross_size`
                // is the resolved content height once an explicit `height` /
                // `min-height` has been honored.
                // Real (signed) main-axis free space: keep it negative when the
                // items overflow a definite container height so justify-content
                // packs from the proper edge instead of collapsing to flex-start
                // (css-align-3 §9 Overflow Alignment).
                let main_free_space = inner_cross_size - total_item_height - total_gap;
                // For column-reverse the main axis points up (main-start is the
                // bottom). We lay items out in reverse source order (top to
                // bottom = last to first); swapping flex-start/flex-end then
                // packs the free space on the correct (top) side so the visual
                // result matches a bottom-anchored start edge.
                let effective_justify = if direction == FlexDirection::ColumnReverse {
                    match justify {
                        JustifyContent::FlexStart => JustifyContent::FlexEnd,
                        JustifyContent::FlexEnd => JustifyContent::FlexStart,
                        other => other,
                    }
                } else {
                    justify
                };
                let (leading, extra_gap) = if main_free_space < 0.0 {
                    // Distributed values use safe fallbacks on overflow;
                    // authored center/flex-end remain unsafe.
                    match effective_justify {
                        JustifyContent::FlexStart
                        | JustifyContent::SafeCenter
                        | JustifyContent::SpaceBetween
                        | JustifyContent::SpaceAround
                        | JustifyContent::SpaceEvenly => (0.0, 0.0),
                        JustifyContent::FlexEnd => (main_free_space, 0.0),
                        JustifyContent::Center => (main_free_space / 2.0, 0.0),
                    }
                } else {
                    let main_free_space = main_free_space.max(0.0);
                    match effective_justify {
                        JustifyContent::FlexStart => (0.0, 0.0),
                        JustifyContent::FlexEnd => (main_free_space, 0.0),
                        JustifyContent::Center | JustifyContent::SafeCenter => {
                            (main_free_space / 2.0, 0.0)
                        }
                        JustifyContent::SpaceBetween => {
                            if line_item_count > 1 {
                                (0.0, main_free_space / (line_item_count - 1) as f32)
                            } else {
                                (0.0, 0.0)
                            }
                        }
                        JustifyContent::SpaceAround => {
                            let around = main_free_space / line_item_count as f32;
                            (around / 2.0, around)
                        }
                        JustifyContent::SpaceEvenly => {
                            let ev = main_free_space / (line_item_count + 1) as f32;
                            (ev, ev)
                        }
                    }
                };

                let mut y = 0.0;
                // Leading is applied as part of the first item's top spacing
                // (which already folds in the container's border + padding); a
                // nonzero leading bumps `y` so subsequent gap math stays correct.
                let mut pending_leading = leading;
                // Per css-flexbox-1 § 6, flex-item margins never collapse — not
                // with each other, nor with the container. The downstream block
                // flow *does* collapse adjacent sibling margins, so we fold the
                // previous item's bottom margin into the next item's leading and
                // emit each item with `margin_bottom: 0`. That keeps the full
                // `prev.margin_bottom + next.margin_top` gap (e.g. 40 + 30 = 70px)
                // instead of the collapsed `max(40, 30) = 40px` of block flow.
                let mut prev_item_margin_bottom = 0.0_f32;

                // `flex-direction: column-reverse` flips the main axis: the
                // first source item is placed at the bottom. Iterating the line
                // in reverse source order packs them bottom-to-top.
                let column_order: Vec<usize> = if direction == FlexDirection::ColumnReverse {
                    line_items.iter().rev().copied().collect()
                } else {
                    line_items.clone()
                };

                if column_wrap_lines {
                    let mut y = leading;
                    for (item_pos, &item_idx) in column_order.iter().enumerate() {
                        let item = &items[item_idx];
                        if item_pos > 0 {
                            y += gap + extra_gap;
                        }

                        let effective_align = match item.align_self {
                            AlignSelf::Auto => align,
                            AlignSelf::FlexStart => AlignItems::FlexStart,
                            AlignSelf::FlexEnd => AlignItems::FlexEnd,
                            AlignSelf::Center => AlignItems::Center,
                            AlignSelf::Baseline => AlignItems::FlexStart,
                            AlignSelf::Stretch => AlignItems::Stretch,
                        };
                        let used_width =
                            if effective_align == AlignItems::Stretch && !item.has_explicit_width {
                                line.cross_size
                            } else {
                                item.width
                            };
                        let mut x_offset = cross_offset
                            + match effective_align {
                                AlignItems::FlexStart | AlignItems::Baseline => 0.0,
                                AlignItems::FlexEnd => line.cross_size - used_width,
                                AlignItems::Center => (line.cross_size - used_width) / 2.0,
                                AlignItems::Stretch => 0.0,
                            };
                        let mut y_offset = y + item.margin_main_start;
                        if item.is_relative {
                            x_offset += item.rel_left;
                            y_offset += item.rel_top;
                        }

                        if let Some(cell) = item.elements.first().and_then(|element| {
                            flex_cell_from_text_block(
                                element.as_ref(),
                                x_offset,
                                y_offset,
                                used_width,
                                item.is_relative,
                                item.z_index,
                            )
                        }) {
                            all_flex_cells.push(cell);
                        } else {
                            all_flex_cells.push(flex_cell_with_nested_item(
                                &item.elements,
                                FlexCell {
                                    x_offset,
                                    width: used_width,
                                    natural_height: item.height,
                                    fragmentation: FlexItemFragmentation::definite(),
                                    align_self: AlignSelf::FlexStart,
                                    y_offset,
                                    line_cross_size: item.height,
                                    positioning: flex_item_positioning(
                                        &item.elements,
                                        item.is_relative,
                                    ),
                                    ..Default::default()
                                },
                            ));
                        }
                        y += item.height;
                    }
                } else {
                    for (item_pos, &item_idx) in column_order.iter().enumerate() {
                        let item = &items[item_idx];

                        // `align-self` overrides the container's `align-items` on the
                        // cross axis (horizontal, for a column container).
                        let effective_align = match item.align_self {
                            AlignSelf::Auto => align,
                            AlignSelf::FlexStart => AlignItems::FlexStart,
                            AlignSelf::FlexEnd => AlignItems::FlexEnd,
                            AlignSelf::Center => AlignItems::Center,
                            // Baseline has no first-baseline notion on the cross axis
                            // of a column container; fall back to flex-start (the
                            // cross-start edge), matching browser behaviour for
                            // baseline alignment of empty boxes.
                            AlignSelf::Baseline => AlignItems::FlexStart,
                            AlignSelf::Stretch => AlignItems::Stretch,
                        };

                        // Calculate cross-axis (horizontal) alignment
                        let x_offset = match effective_align {
                            AlignItems::FlexStart | AlignItems::Baseline => 0.0,
                            AlignItems::FlexEnd => inner_width - item.width,
                            AlignItems::Center => (inner_width - item.width) / 2.0,
                            AlignItems::Stretch => 0.0,
                        };

                        // align-items: stretch only stretches items whose cross size
                        // (width, for a column container) is auto. An item with an
                        // explicit width keeps it.
                        let effective_width =
                            if effective_align == AlignItems::Stretch && !item.has_explicit_width {
                                Some(inner_width)
                            } else {
                                Some(item.width)
                            };

                        // Extra main-axis spacing this item contributes from
                        // `justify-content`: the leading for the first item, an
                        // even slice between items otherwise. Applied only to the
                        // item's first emitted element so multi-element items aren't
                        // over-spaced.
                        let item_justify_lead = if item_pos == 0 {
                            std::mem::take(&mut pending_leading)
                        } else {
                            extra_gap
                        };
                        let mut item_first_elem = true;
                        // The bottom margin of this item's last emitted element,
                        // folded into the next item's leading (flex margins don't
                        // collapse). Reset per item.
                        let mut item_last_margin_bottom = 0.0_f32;

                        for elem in &item.elements {
                            if is_page_break(elem.as_ref()) {
                                output.push(elem.clone());
                                if let Some((bg_fragment, spacer_fragment)) = &column_bg_pair {
                                    let start = if item_first_elem {
                                        item_pos
                                    } else {
                                        item_pos + 1
                                    };
                                    let remaining_count = column_order.len().saturating_sub(start);
                                    if remaining_count > 0 {
                                        let remaining_h: f32 = column_order[start..]
                                            .iter()
                                            .map(|&idx| items[idx].height)
                                            .sum::<f32>()
                                            + gap * remaining_count.saturating_sub(1) as f32;
                                        let mut bg_fragment = bg_fragment.clone();
                                        let mut spacer_fragment = spacer_fragment.clone();
                                        let bg_flow_height = prepare_continuation_background(
                                            bg_fragment.as_mut(),
                                            remaining_h + style.padding.bottom,
                                        );
                                        set_text_block_start_margin(
                                            spacer_fragment.as_mut(),
                                            -bg_flow_height,
                                        );
                                        output.push(bg_fragment);
                                        output.push(spacer_fragment);
                                    }
                                }
                                continue;
                            }
                            let metrics = column_text_metrics(elem.as_ref());
                            if metrics.is_text {
                                // `justify-content` leading/spacing applies once per
                                // item, to its first emitted element.
                                let justify_lead = if item_first_elem {
                                    item_first_elem = false;
                                    item_justify_lead
                                } else {
                                    0.0
                                };
                                item_last_margin_bottom = metrics.margins.end;
                                let resolved_height = if item.elements.len() == 1 {
                                    Some(
                                        (item.height
                                            - metrics.margins.total()
                                            - metrics.border_height)
                                            .max(0.0),
                                    )
                                } else {
                                    metrics.height
                                };
                                let leading = if y == 0.0 && !emitted_column_bg {
                                    style.margin.top
                                        + style.border.top.used_width()
                                        + style.padding.top
                                        + justify_lead
                                        + metrics.margins.start
                                } else if y == 0.0 {
                                    style.border.top.used_width()
                                        + style.padding.top
                                        + justify_lead
                                        + metrics.margins.start
                                } else {
                                    gap + justify_lead
                                        + prev_item_margin_bottom
                                        + metrics.margins.start
                                };
                                let inline_offset =
                                    x_offset + style.padding.left + style.border.left.used_width();
                                let mut emitted = elem.clone();
                                adapt_column_text_block(
                                    emitted.as_mut(),
                                    BlockMargins::new(leading, 0.0),
                                    effective_width,
                                    resolved_height,
                                    inline_offset,
                                    inline_offset > 0.0,
                                );
                                output.push(emitted);
                            } else {
                                // Non-TextBlock flex item (e.g. a Container emitted
                                // for a padded child). Wrap it so the column's
                                // main-axis (vertical) leading and cross-axis
                                // (horizontal) alignment are applied; otherwise the
                                // element would be silently dropped by this loop.
                                let justify_lead = if item_first_elem {
                                    item_first_elem = false;
                                    item_justify_lead
                                } else {
                                    0.0
                                };
                                let leading = if y == 0.0 && !emitted_column_bg {
                                    style.margin.top
                                        + style.border.top.used_width()
                                        + style.padding.top
                                        + justify_lead
                                } else if y == 0.0 {
                                    style.border.top.used_width() + style.padding.top + justify_lead
                                } else {
                                    gap + justify_lead + prev_item_margin_bottom
                                };
                                output.push(
                                    Container {
                                        children: vec![elem.clone()],
                                        box_model: crate::layout::elements::BoxModel {
                                            size: crate::layout::elements::LayoutSize {
                                                width: InlineSize::from_fixed_value(
                                                    effective_width,
                                                ),
                                                height: BlockSize::AUTO,
                                            },
                                            margins: BlockMargins::new(leading, 0.0),
                                            ..Default::default()
                                        },
                                        positioning: crate::layout::elements::Positioning::default(
                                        )
                                        .with_scheme(
                                            if x_offset > 0.0
                                                || style.padding.left > 0.0
                                                || style.border.left.used_width() > 0.0
                                            {
                                                Position::Relative
                                            } else {
                                                Position::Static
                                            },
                                        )
                                        .with_resolved_insets(EdgeSizes::new(
                                            0.0,
                                            0.0,
                                            0.0,
                                            x_offset
                                                + style.padding.left
                                                + style.border.left.used_width(),
                                        )),
                                        ..Default::default()
                                    }
                                    .boxed(),
                                );
                            }
                        }

                        y += item.height + gap;
                        prev_item_margin_bottom = item_last_margin_bottom;
                    }
                }
            }
        }

        for cell in &mut all_flex_cells[line_cells_start..] {
            cell.line_id = line_id;
        }
        cross_offset += line.cross_size + line_gap;
    }

    // Emit a single FlexRow carrying every line's cells for row direction.
    // The row's height is the container's inner cross size so pagination and
    // the visual border both include every wrapped line. Each cell's own
    // y_offset and line_cross_size handle per-line alignment internally.
    if (direction.is_row() || column_wrap_lines) && !all_flex_cells.is_empty() {
        // CSS Flexbox §10 propagates forced breaks on row items to their flex
        // line. Keep that structure through layout so pagination can split the
        // container decoration at the line boundary, rather than smuggling a
        // `PageBreak` into a paint-only FlexCell.
        let leading_forced_break = direction
            .is_row()
            .then(|| lines.first().and_then(|line| line.break_before))
            .flatten();
        let forced_line_breaks = if direction.is_row() {
            lines
                .windows(2)
                .enumerate()
                .filter_map(|(index, pair)| {
                    pair[1]
                        .break_before
                        .or(pair[0].break_after)
                        .map(|side| ForcedFlexLineBreak {
                            before: FlexLineId::from_index(index + 1),
                            side,
                        })
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut resolved_row_align = if column_wrap_lines || align_last_baseline {
            AlignItems::FlexStart
        } else {
            align
        };
        if direction.is_row() && resolved_row_align == AlignItems::Baseline {
            apply_row_baseline_offsets(&mut all_flex_cells);
            resolved_row_align = AlignItems::FlexStart;
        }
        let row_height = if column_wrap_lines || (direction.is_row() && style.height.is_some()) {
            inner_cross_size
        } else if lines.len() == 1 {
            all_flex_cells
                .iter()
                .map(|cell| {
                    if cell.line_cross_size > 0.0 {
                        cell.line_cross_size
                    } else {
                        cell.natural_height
                    }
                })
                .fold(total_cross.max(inner_cross_size), f32::max)
        } else {
            all_flex_cells
                .iter()
                .map(|cell| {
                    cell.y_offset
                        + if cell.line_cross_size > 0.0 {
                            cell.line_cross_size
                        } else {
                            cell.natural_height
                        }
                })
                .fold(total_cross.max(inner_cross_size), f32::max)
        };
        if let Some(side) = leading_forced_break {
            output.push(
                PageBreak {
                    side,
                    page_name: None,
                }
                .boxed(),
            );
        }
        output.push(flex_row_node(
            style,
            all_flex_cells,
            forced_line_breaks,
            Default::default(),
            row_height,
            BlockMargins::new(style.margin.top, 0.0),
            h_offset,
            block_w,
            inner_cross_size,
            resolved_row_align,
            abs_cb_depth,
        ));
    }

    // Emit out-of-flow absolute children after the in-flow flex content so they
    // paint above it (CSS painting order) and anchor to the container's padding
    // box via the containing block stamped above.
    output.append(&mut abs_output);

    if direction.is_row()
        && let Some(side) = lines.last().and_then(|line| line.break_after)
    {
        output.push(
            PageBreak {
                side,
                page_name: None,
            }
            .boxed(),
        );
    }

    // Emit trailing margin (include bottom padding when bg spacer shifted y back)
    let trailing = if emitted_column_bg {
        style.padding.bottom + style.margin.bottom
    } else {
        style.margin.bottom
    };
    if trailing > 0.0 {
        let mut spacer = TextBlock::empty_spacer();
        spacer.box_model.margins = BlockMargins::new(trailing, 0.0);
        output.push(spacer.boxed());
    }
}

#[cfg(test)]
mod cutoff_tests {
    use super::{
        FlexIntrinsicWidth, FlexibleLength, apply_row_baseline_offsets, flex_cell_with_nested_item,
        flex_direct_text_width, resolve_flexible_lengths,
    };
    use crate::layout::elements::{
        BoxPaint, BoxTransform, IntoLayoutNode, LayoutElement, LayoutElementTestExt, PaintGroup,
        SizeConstraints, TextBlock,
    };
    use crate::layout::engine::{FlexCell, FlexLineId, layout};
    use crate::parser::html::{parse_html, parse_html_with_styles};
    use crate::style::computed::{ComputedStyle, Transform};
    use crate::types::{Margin, PageSize};

    #[test]
    fn direct_flex_text_keeps_non_breaking_space_advance() {
        let style = ComputedStyle::default();
        let fonts = std::collections::HashMap::new();

        let ordinary = flex_direct_text_width("A B", &style, &fonts);
        let non_breaking = flex_direct_text_width("A\u{00a0}\u{00a0}B", &style, &fonts);

        assert!(non_breaking > ordinary);
    }

    fn flex_rows_in_element(element: &dyn LayoutElement, rows: &mut Vec<(Vec<FlexCell>, f32)>) {
        if let Some(row) = element.inspect_flex(|row| {
            (
                row.content.cells.clone(),
                row.box_model.size.width.fixed_value().unwrap_or_default(),
            )
        }) {
            rows.push(row);
        }
        element.visit_children(&mut |child| flex_rows_in_element(child, rows));
    }

    #[test]
    fn baseline_offsets_are_scoped_to_semantic_flex_lines() {
        let first = FlexLineId::from_index(0);
        let second = FlexLineId::from_index(1);
        let mut cells = vec![
            FlexCell {
                natural_height: 4.0,
                line_id: first,
                y_offset: 10.0,
                ..Default::default()
            },
            FlexCell {
                natural_height: 7.0,
                line_id: first,
                y_offset: 10.0,
                ..Default::default()
            },
            FlexCell {
                natural_height: 19.0,
                line_id: second,
                y_offset: 10.005,
                ..Default::default()
            },
        ];

        apply_row_baseline_offsets(&mut cells);

        assert_eq!(cells[0].y_offset, 13.0);
        assert_eq!(cells[1].y_offset, 10.0);
        assert_eq!(cells[2].y_offset, 10.005);
    }

    #[test]
    fn intrinsic_flex_base_snaps_paint_without_narrowing_text_wrap() {
        let width = FlexIntrinsicWidth::from_content(&ComputedStyle::default(), 89.519_53);

        assert_eq!(width.paint, 89.25);
        assert_eq!(width.text_wrap, 89.519_53);
    }

    #[test]
    fn inflexible_main_size_freezes_to_its_maximum() {
        let mut lengths = [FlexibleLength::new(
            108.75,
            SizeConstraints::new(None, Some(99.0)),
            0.0,
            1.0,
            0.0,
        )];

        resolve_flexible_lengths(&mut lengths, 115.5);

        assert_eq!(lengths[0].target, 99.0);
    }

    #[test]
    fn centered_flex_item_uses_its_post_clamp_main_size() {
        let nodes = parse_html(
            r#"<div style="display:flex;width:115.5pt;justify-content:center">
                <div style="box-sizing:border-box;width:108.75pt;max-width:99pt;height:10pt"></div>
            </div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut rows = Vec::new();
        for (_, element) in &pages[0].elements {
            flex_rows_in_element(element.as_ref(), &mut rows);
        }
        let cell = rows
            .iter()
            .find_map(|(cells, width)| {
                if (*width - 115.5).abs() < 0.001 {
                    cells.first()
                } else {
                    None
                }
            })
            .expect("centered flex item");

        assert!((cell.width - 99.0).abs() < 0.001);
        assert!((cell.x_offset - 8.25).abs() < 0.001);
    }

    #[test]
    fn overflowing_space_around_uses_its_safe_start_fallback() {
        let nodes = parse_html(
            r#"<div style="display:flex;width:78pt;gap:3pt;justify-content:space-around">
                <div style="width:32pt;height:10pt;flex-grow:0;flex-shrink:0"></div>
                <div style="width:69pt;height:10pt;flex-grow:0;flex-shrink:0"></div>
            </div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut rows = Vec::new();
        for (_, element) in &pages[0].elements {
            flex_rows_in_element(element.as_ref(), &mut rows);
        }
        let cells = rows
            .iter()
            .find_map(|(cells, width)| {
                ((*width - 78.0).abs() < 0.001 && cells.len() == 2).then_some(cells)
            })
            .expect("overflowing flex line");

        assert!((cells[0].x_offset - 0.0).abs() < 0.001);
        assert!((cells[1].x_offset - 35.0).abs() < 0.001);
    }

    #[test]
    fn automatic_minimum_keeps_forced_lines_separate() {
        let nodes = parse_html(
            r#"<div style="display:flex;width:260pt">
                <div style="padding:0 8pt">A<br>B</div>
                <div style="padding:0 8pt">AB</div>
            </div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut rows = Vec::new();
        for (_, element) in &pages[0].elements {
            flex_rows_in_element(element.as_ref(), &mut rows);
        }
        let cells = rows
            .iter()
            .find_map(|(cells, width)| {
                ((*width - 260.0).abs() < 0.001 && cells.len() == 2).then_some(cells)
            })
            .expect("two-item flex line");

        assert!(
            cells[0].width < cells[1].width,
            "a forced break must make A/B narrower than the unbroken AB: {:?}",
            cells.iter().map(|cell| cell.width).collect::<Vec<_>>()
        );
    }

    #[test]
    fn nested_one_item_space_around_retains_centered_offset() {
        let nodes = parse_html(
            r#"<div style="display:inline-flex;width:117pt;height:123pt;align-items:center;justify-content:center">
                <div style="display:flex;box-sizing:border-box;width:94.5pt;height:51pt;padding:5.25pt;border:1.5pt solid;align-items:center;justify-content:space-around">
                    <div style="height:16.5pt;white-space:nowrap"><span>Ag</span><span>Bb</span><img style="display:none" alt=""></div>
                </div>
            </div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut rows = Vec::new();
        for page in &pages {
            for (_, element) in &page.elements {
                flex_rows_in_element(element, &mut rows);
            }
        }

        assert!(
            rows.iter().any(|(cells, width)| cells.len() == 1
                && (*width - 94.5).abs() < 0.001
                && cells[0].x_offset > 10.0),
            "nested item must be centered: {rows:?}"
        );
    }

    #[test]
    fn flex_container_retains_its_group_transform() {
        fn contains_transformed_flex(element: &dyn LayoutElement) -> bool {
            if element
                .inspect_flex(|row| row.paint.group.transform.value.is_some())
                .unwrap_or(false)
            {
                return true;
            }
            let mut found = false;
            element.visit_children(&mut |child| {
                found |= contains_transformed_flex(child);
            });
            found
        }

        let nodes = parse_html(
            r#"<div style="display:flex;width:80pt;height:30pt;transform:translate(2pt,-1pt) rotate(5deg)"><span>A</span></div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        assert!(pages.iter().any(|page| {
            page.elements
                .iter()
                .any(|(_, element)| contains_transformed_flex(element.as_ref()))
        }));
    }

    #[test]
    fn structured_flex_item_group_moves_to_its_cell() {
        let nested = TextBlock {
            paint: BoxPaint {
                group: PaintGroup {
                    transform: BoxTransform {
                        value: Some(Transform::Rotate(5.0)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
        .boxed();

        let cell = flex_cell_with_nested_item(&[nested], FlexCell::default());

        assert!(cell.paint.group.transform.value.is_some());
        assert!(
            cell.nested_elements[0]
                .paint_group_owner()
                .is_some_and(|owner| owner.paint_group().is_identity())
        );
    }

    #[test]
    fn shrink_wrapped_structured_item_counts_its_border_once() {
        let nodes = parse_html(
            r#"<div style="display:flex;align-items:flex-start">
                <div style="box-sizing:border-box;padding:6pt;border:1.5pt solid">
                    <table style="border-collapse:collapse"><tr><td style="box-sizing:border-box;width:45pt;height:27pt;border:1.5pt solid"></td></tr><tr><td style="box-sizing:border-box;width:45pt;height:27pt;border:1.5pt solid"></td></tr></table>
                </div>
            </div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let mut rows = Vec::new();
        for (_, element) in &pages[0].elements {
            flex_rows_in_element(element.as_ref(), &mut rows);
        }
        let cell = rows
            .iter()
            .find_map(|(cells, _)| (cells.len() == 1).then(|| &cells[0]))
            .expect("one shrink-wrapped flex item");

        // Two collapsed 27pt rows share their middle 1.5pt border, producing a
        // 55.5pt table grid. The item's 12pt padding and 3pt border then yield
        // one 70.5pt border box; no trailing border is added a second time.
        assert_eq!(cell.natural_height, 70.5);
    }

    #[test]
    fn row_item_forced_break_splits_at_its_flex_line() {
        let parsed = parse_html_with_styles(
            r#"<!doctype html>
            <html><head><style>
                html { font-family: sans-serif; line-height: 1.5; }
                * { margin: 0; box-sizing: border-box; }
                .flex { display: flex; flex-wrap: wrap; align-content: flex-start;
                    row-gap: 10px; column-gap: 10px; width: 220px;
                    background: #eee; border: 2px solid #222; }
                .item { width: 60px; height: 50px; }
                .item:nth-child(4) { break-before: page; }
            </style></head><body>
                <div class="flex">
                    <div class="item"></div><div class="item"></div>
                    <div class="item"></div><div class="item"></div>
                    <div class="item"></div><div class="item"></div>
                    <div class="item"></div><div class="item"></div>
                    <div class="item"></div><div class="item"></div>
                    <div class="item"></div><div class="item"></div>
                </div>
            </body></html>"#,
        )
        .unwrap();
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| crate::parser::css::parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = crate::layout::engine::layout_with_rules(
            &parsed.nodes,
            PageSize::new(180.0, 138.0),
            Margin::uniform(0.0),
            &rules,
        );

        let flex = pages
            .iter()
            .enumerate()
            .flat_map(|(page_index, page)| {
                page.elements.iter().filter_map(move |(y, element)| {
                    element.inspect_flex(|row| {
                        (
                            page_index,
                            *y,
                            row.content.forced_line_breaks.clone(),
                            row.content
                                .cells
                                .iter()
                                .map(|cell| (cell.line_id, cell.y_offset, cell.width))
                                .collect::<Vec<_>>(),
                            row.content.row_height,
                            row.content.fragment_role,
                        )
                    })
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(pages.len(), 2, "flex layout: {flex:?}");
        assert_eq!(
            flex.iter()
                .map(|(_, _, _, cells, _, _)| cells.len())
                .collect::<Vec<_>>(),
            [3, 9],
            "forced break must preserve every flex item: {flex:?}",
        );
        assert!(
            flex.iter().all(|(_, _, _, _, _, role)| {
                *role == crate::layout::engine::FlexFragmentRole::Normal
            }),
            "line fragments remain normal document flow: {flex:?}",
        );
    }

    #[test]
    fn half_point_flex_growth_relayouts_percentage_child() {
        let nodes = parse_html(
            r#"<div style="display:flex;width:100.5pt">
                <div style="flex-grow:1;flex-shrink:0;flex-basis:100pt;min-width:0">
                    <div style="box-sizing:border-box;width:50%;height:1pt;border:0.1pt solid red"></div>
                    <div style="width:0;height:1pt;border:0.1pt solid transparent"></div>
                </div>
            </div>"#,
        )
        .unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let flex = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_flex(|row| {
                    (row.content.cells.len() == 1).then(|| row.content.cells.clone())
                })?
            })
            .expect("one-item flex row");
        assert!((flex[0].width - 100.5).abs() < 0.0001);
        let child_width = flex[0]
            .nested_elements
            .iter()
            .find_map(|element| {
                element
                    .inspect_text(|block| block.box_model.size.width.fixed_value())
                    .or_else(|| {
                        element.inspect_container(|container| {
                            container.box_model.size.width.fixed_value()
                        })
                    })
                    .flatten()
            })
            .expect("percentage-width block child");
        assert!(
            (child_width - 50.25).abs() < 0.0001,
            "50% child must use final 100.5pt flex width, got {child_width}pt"
        );
    }

    fn single_flex_cell_width(html: &str) -> f32 {
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_flex(|row| {
                    (row.content.cells.len() == 1).then(|| row.content.cells[0].width)
                })?
            })
            .expect("one-item flex row")
    }

    #[test]
    fn calc_percent_flex_basis_uses_the_inner_main_size() {
        let width = single_flex_cell_width(
            r#"<div style="display:flex;box-sizing:border-box;width:200pt;border:2pt solid #222">
                <div style="flex:0 0 calc(25% - 10pt);height:1pt"></div>
            </div>"#,
        );

        // 25% resolves against the 196pt content box, not the 200pt border box:
        // 196 * .25 - 10 = 39pt.
        assert!(
            (width - 39.0).abs() < 0.000_1,
            "calc percentage flex basis used the wrong box: {width}pt"
        );
    }

    #[test]
    fn five_thousandths_of_a_point_is_distributed_by_grow_and_shrink() {
        let grown = single_flex_cell_width(
            r#"<div style="display:flex;width:100.005pt"><div style="height:1pt;flex-grow:1;flex-shrink:0;flex-basis:100pt"></div></div>"#,
        );
        let shrunk = single_flex_cell_width(
            r#"<div style="display:flex;width:99.995pt"><div style="height:1pt;min-width:0;flex-grow:0;flex-shrink:1;flex-basis:100pt"></div></div>"#,
        );

        assert!((grown - 100.005).abs() < 0.0001, "grown width: {grown}");
        assert!((shrunk - 99.995).abs() < 0.0001, "shrunk width: {shrunk}");
    }

    fn flex_item_lines_at_authored_basis(basis: f32) -> (f32, usize) {
        let remainder = 10.0 - basis;
        let nodes = parse_html(&format!(
            r#"<div style="display:flex;width:10pt;font-size:0.5pt;line-height:1">
                <div style="min-width:0;flex-grow:1;flex-shrink:0;flex-basis:{basis}pt;overflow-wrap:anywhere">i i i i</div>
                <div style="min-width:0;flex:0 0 {remainder}pt"></div>
            </div>"#
        ))
        .expect("valid flex fixture");
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_flex(|row| {
                    (row.content.cells.len() == 2)
                        .then(|| (row.content.cells[0].width, row.content.cells[0].lines.len()))
                })?
            })
            .expect("two-item flex row")
    }

    #[test]
    fn positive_subpoint_flex_bases_are_not_reclassified_as_zero() {
        let (half_width, half_lines) = flex_item_lines_at_authored_basis(0.5);
        let (thousandth_width, thousandth_lines) = flex_item_lines_at_authored_basis(0.001);

        assert_eq!(half_width, 0.5);
        assert_eq!(thousandth_width, 0.001);
        assert!(half_lines > 1, "0.5pt basis was measured at an equal share");
        assert!(
            thousandth_lines > half_lines,
            "0.001pt basis was measured as a zero or half-point flex base: {thousandth_lines} vs {half_lines} lines"
        );
    }
}
