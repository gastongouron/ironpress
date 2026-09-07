use super::cells::TableRowCells;
use super::engine::{
    FootnoteItem, GridCell, Page, PageBreakSide, ReplacedFragment, TextLine, TextRun,
    decode_footnote_link_data, target_anchor_id,
};
use super::flow_metrics::BlockMargins;
use super::fragmentation::split_flow_at_descendant_break;
use super::roundoff::{equal_with_roundoff, exceeds_with_roundoff, is_positive_with_roundoff};
use super::text::{OverflowWrap, TextWrapOptions, wrap_text_runs};
use crate::layout::elements::{
    BlockSize, Container, FlexRow, FragmentBreakRule, GridRow, HorizontalRule, Image,
    IntoLayoutNode, LayoutElement, LayoutNode, LayoutVisitor, LayoutVisitorMut, MathBlock,
    NamedString, PageContentRole, ProgressBar, RunningElement, Svg, Table, TableFragmentGroup,
    TableRow, TextBlock, visit_layout_tree,
};
use crate::style::computed::{
    BoxDecorationBreak, Clear, Float, FootnotePolicy, ObjectFit, Position,
};
use crate::types::{Color, EdgeSizes, Margin, PageSize, Size};
use std::collections::{HashMap, HashSet, VecDeque};

mod measurement;

fn advance_positioned_ancestors_after_page_break(
    positioned_y_by_depth: &mut HashMap<usize, f32>,
    consumed_height: f32,
) {
    for y in positioned_y_by_depth.values_mut() {
        *y -= consumed_height;
    }
}

fn collect_footnotes_from_runs(
    runs: &[TextRun],
    seen_links: &mut HashSet<String>,
    out: &mut Vec<FootnoteItem>,
) {
    for run in runs {
        let Some(link) = run.link_url.as_deref() else {
            continue;
        };
        if !seen_links.insert(link.to_string()) {
            continue;
        }
        let Some(data) = decode_footnote_link_data(link) else {
            continue;
        };
        out.push(FootnoteItem {
            marker: data.marker,
            text: data.text,
            body: data.body,
            marker_color: data.marker_color,
            marker_prefix: data.marker_prefix,
            formatting: data.formatting,
        });
    }
}

fn collect_footnotes_from_element(element: &dyn LayoutElement, out: &mut Vec<FootnoteItem>) {
    struct FootnoteCollector<'a> {
        seen_links: HashSet<String>,
        out: &'a mut Vec<FootnoteItem>,
    }

    impl LayoutVisitor for FootnoteCollector<'_> {
        fn visit_text_block(&mut self, element: &TextBlock) {
            for line in &element.lines {
                collect_footnotes_from_runs(&line.runs, &mut self.seen_links, self.out);
            }
        }
    }

    visit_layout_tree(
        element,
        &mut FootnoteCollector {
            seen_links: HashSet::new(),
            out,
        },
    );
}

/// The line whose `footnote-policy: line` body makes this page overflow.
///
/// A line-policy call that still fits must not move an earlier line: CSS GCPM
/// applies the break only to the reference whose body cannot fit in the current
/// footnote area. A policy break is a boundary *before* that line, so index zero
/// falls back to the ordinary whole-block page break.
fn footnote_line_policy_break_index(
    element: &dyn LayoutElement,
    current_footnotes: &[FootnoteItem],
    element_y: f32,
    element_height: f32,
    content_height: f32,
    footnote_area: FootnoteAreaLayout,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> Option<usize> {
    struct LinePolicyVisitor<'a> {
        current_footnotes: &'a [FootnoteItem],
        element_y: f32,
        element_height: f32,
        content_height: f32,
        footnote_area: FootnoteAreaLayout,
        fonts: &'a dyn crate::font_registry::FontRegistry,
        break_index: Option<usize>,
    }

    impl LayoutVisitor for LinePolicyVisitor<'_> {
        fn visit_text_block(&mut self, element: &TextBlock) {
            let fits = |pending: &[FootnoteItem]| {
                let available = (self.content_height
                    - footnote_reserved_height(
                        &[self.current_footnotes, pending],
                        self.footnote_area,
                        self.fonts,
                    ))
                .max(0.0);
                !exceeds_with_roundoff(self.element_y + self.element_height, available)
            };
            let mut seen_links = HashSet::new();
            let mut pending = Vec::new();
            for (index, line) in element.lines.iter().enumerate() {
                let mut line_footnotes = Vec::new();
                collect_footnotes_from_runs(&line.runs, &mut seen_links, &mut line_footnotes);
                for footnote in line_footnotes {
                    let fit_before = fits(&pending);
                    let policy = footnote.formatting.policy;
                    pending.push(footnote);
                    if fit_before && !fits(&pending) && policy == FootnotePolicy::Line {
                        self.break_index = Some(index);
                        return;
                    }
                }
            }
        }
    }

    let mut visitor = LinePolicyVisitor {
        current_footnotes,
        element_y,
        element_height,
        content_height,
        footnote_area,
        fonts,
        break_index: None,
    };
    element.accept(&mut visitor);
    visitor.break_index
}

/// Whether a `footnote-policy: block` call forces a break before its owning
/// block. CSS GCPM makes this distinct from ordinary footnote reservation: the
/// block stays on the current page when it fits without its footnote, but moves
/// intact when adding the block-policy body would make the page overflow.
fn footnote_block_policy_requires_break(
    pending_footnotes: &[FootnoteItem],
    current_footnotes: &[FootnoteItem],
    element_y: f32,
    element_height: f32,
    content_height: f32,
    footnote_area: FootnoteAreaLayout,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> bool {
    if !pending_footnotes
        .iter()
        .any(|footnote| footnote.formatting.policy == FootnotePolicy::Block)
    {
        return false;
    }

    let available_before = (content_height
        - footnote_reserved_height(&[current_footnotes], footnote_area, fonts))
    .max(0.0);
    let available_with_body = (content_height
        - footnote_reserved_height(
            &[current_footnotes, pending_footnotes],
            footnote_area,
            fonts,
        ))
    .max(0.0);
    !exceeds_with_roundoff(element_y + element_height, available_before)
        && exceeds_with_roundoff(element_y + element_height, available_with_body)
}

fn extract_page_state_markers(
    element: &mut dyn LayoutElement,
    generated_content: &mut super::page_values::PageGeneratedContent,
    at_page_start: bool,
    pending_target_anchors: &mut Vec<String>,
) {
    struct MarkerExtraction {
        running: Option<(String, LayoutNode)>,
        named: Option<(String, String)>,
    }

    impl LayoutVisitor for MarkerExtraction {
        fn visit_running_element(&mut self, element: &RunningElement) {
            self.running = Some((element.name.clone(), element.element.clone()));
        }

        fn visit_named_string(&mut self, element: &NamedString) {
            self.named = Some((element.name.clone(), element.value.clone()));
        }
    }

    struct ContainerMarkerExtractor<'a> {
        generated_content: &'a mut super::page_values::PageGeneratedContent,
        at_page_start: bool,
        pending_target_anchors: &'a mut Vec<String>,
    }

    impl LayoutVisitorMut for ContainerMarkerExtractor<'_> {
        fn visit_container(&mut self, element: &mut Container) {
            let mut kept = Vec::with_capacity(element.children.len());
            for mut child in element.children.drain(..) {
                let mut marker = MarkerExtraction {
                    running: None,
                    named: None,
                };
                child.accept(&mut marker);
                if let Some((name, running)) = marker.running {
                    self.generated_content
                        .capture_running(name, running, self.at_page_start);
                    continue;
                }
                if let Some((name, value)) = marker.named {
                    if target_anchor_id(&name).is_some() {
                        self.pending_target_anchors.push(name);
                    } else {
                        self.generated_content.capture_named_string(
                            name,
                            value,
                            self.at_page_start,
                        );
                    }
                    continue;
                }
                extract_page_state_markers(
                    &mut child,
                    self.generated_content,
                    self.at_page_start,
                    self.pending_target_anchors,
                );
                kept.push(child);
            }
            element.children = kept;
        }
    }

    element.accept_mut(&mut ContainerMarkerExtractor {
        generated_content,
        at_page_start,
        pending_target_anchors,
    });
}

fn apply_pending_target_anchors(
    pending_target_anchors: &mut Vec<String>,
    generated_content: &mut super::page_values::PageGeneratedContent,
    at_page_start: bool,
) {
    for name in pending_target_anchors.drain(..) {
        generated_content.capture_named_string(name, String::new(), at_page_start);
    }
}

/// Resolved top separator for the footnote area.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct FootnoteSeparator {
    pub width: f32,
    pub color: Color,
}

/// Resolved box properties shared by footnote pagination and painting.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct ResolvedFootnoteAreaStyle {
    pub padding: EdgeSizes,
    pub separator: FootnoteSeparator,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FootnoteAreaLayout {
    pub content_width: f32,
    pub max_height: Option<f32>,
    pub style: ResolvedFootnoteAreaStyle,
}

impl Default for FootnoteAreaLayout {
    fn default() -> Self {
        Self {
            content_width: f32::INFINITY,
            max_height: None,
            style: ResolvedFootnoteAreaStyle::default(),
        }
    }
}

fn footnote_lines_height(
    footnotes: &[FootnoteItem],
    content_width: f32,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    let mut total = 0.0f32;
    let mut compact_runs: Vec<TextRun> = Vec::new();
    let flush_compact = |runs: &mut Vec<TextRun>, total: &mut f32| {
        if runs.is_empty() {
            return;
        }
        let font_size = runs.first().map(|run| run.font_size).unwrap_or(12.0);
        let line_height = runs
            .first()
            .map(|run| run.line_height_factor)
            .filter(|factor| factor.is_finite())
            .unwrap_or(1.2);
        let lines = wrap_text_runs(
            std::mem::take(runs),
            TextWrapOptions::new(
                content_width.max(0.0),
                font_size,
                line_height,
                OverflowWrap::Normal,
            ),
            fonts,
        );
        *total += lines.iter().map(|line| line.height).sum::<f32>();
    };

    for footnote in footnotes {
        let runs = footnote.text_runs();
        if footnote.formatting.display.is_inline_layout() {
            compact_runs.extend(runs);
            continue;
        }
        flush_compact(&mut compact_runs, &mut total);
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(
                content_width.max(0.0),
                footnote.body.font_size,
                if footnote.body.line_height_factor.is_finite() {
                    footnote.body.line_height_factor
                } else {
                    1.2
                },
                OverflowWrap::Normal,
            ),
            fonts,
        );
        total += lines.iter().map(|line| line.height).sum::<f32>();
    }
    flush_compact(&mut compact_runs, &mut total);
    total
}

fn footnote_content_width(area: FootnoteAreaLayout) -> f32 {
    (area.content_width - area.style.padding.horizontal()).max(0.0)
}

fn footnote_content_height(
    footnotes: &[FootnoteItem],
    area: FootnoteAreaLayout,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    footnote_lines_height(footnotes, footnote_content_width(area), fonts)
}

fn footnote_reserved_height(
    groups: &[&[FootnoteItem]],
    area: FootnoteAreaLayout,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> f32 {
    if groups.iter().all(|footnotes| footnotes.is_empty()) {
        return 0.0;
    }
    let content_height = groups
        .iter()
        .map(|footnotes| footnote_content_height(footnotes, area, fonts))
        .sum::<f32>();
    if area
        .max_height
        .is_some_and(|max| exceeds_with_roundoff(content_height, max))
    {
        return 0.0;
    }
    content_height + area.style.padding.vertical() + area.style.separator.width.max(0.0)
}

pub(crate) fn move_overflow_footnotes_to_next_page(
    pages: &mut Vec<Page>,
    area: FootnoteAreaLayout,
    fonts: &dyn crate::font_registry::FontRegistry,
) {
    let Some(max_height) = area.max_height else {
        return;
    };
    let mut index = 0usize;
    while index < pages.len() {
        let height = footnote_content_height(&pages[index].footnotes, area, fonts);
        if exceeds_with_roundoff(height, max_height)
            && !pages[index].footnotes.is_empty()
            && !pages[index].elements.is_empty()
        {
            let footnotes = std::mem::take(&mut pages[index].footnotes);
            let carry = Page {
                elements: Vec::new(),
                print_content_scale: Default::default(),
                document_svg_defs: Default::default(),
                generated_content: pages[index].generated_content.following_page(),
                footnotes,
                geometry: pages[index].geometry,
                page_name: pages[index].page_name.clone(),
                is_blank: false,
            };
            pages.insert(index + 1, carry);
            index += 2;
        } else {
            index += 1;
        }
    }
}

/// A tracked float region for simplified float layout.
#[derive(Debug, Clone)]
struct FloatRegion {
    #[allow(dead_code)]
    y_start: f32,
    y_end: f32,
    #[allow(dead_code)]
    side: Float,
}

/// Estimate the height of a layout element for wrapper sizing.
pub(crate) fn estimate_element_height(element: &dyn LayoutElement) -> f32 {
    measurement::element_height(element)
}

/// Retain one CSS principal box as the reference geometry shared by the two
/// fragments produced from it. This is independent of the concrete node type;
/// every decorated box exposes the same fragmentation capability.
fn retain_reference_box(
    source: &dyn LayoutElement,
    first: &mut dyn LayoutElement,
    continuation: &mut dyn LayoutElement,
) {
    let border_box_extent = |element: &dyn LayoutElement| {
        let margins = element
            .box_fragmentation_owner()
            .map_or(0.0, |owner| owner.fragmentation_box_model().margins.total());
        (measurement::element_height(element) - margins).max(0.0)
    };
    let Some(source) = source.box_fragmentation_owner() else {
        return;
    };
    let Some((first_slice, continuation_slice)) = source.box_fragmentation().split_reference_box(
        border_box_extent(first),
        border_box_extent(continuation),
        source.fragmentation_box_model(),
    ) else {
        return;
    };
    if let Some(owner) = first.box_fragmentation_owner_mut() {
        owner.box_fragmentation_mut().reference_slice = Some(first_slice);
    }
    if let Some(owner) = continuation.box_fragmentation_owner_mut() {
        owner.box_fragmentation_mut().reference_slice = Some(continuation_slice);
    }
}

/// The CSS `float` value of a block-level layout element (`None` for anything
/// that cannot float, e.g. table rows).
pub(crate) fn element_float(element: &dyn LayoutElement) -> Float {
    element
        .block_flow_owner()
        .map_or(Float::None, |owner| owner.block_flow().float)
}

/// The CSS `clear` value of a block-level layout element.
fn element_clear(element: &dyn LayoutElement) -> Clear {
    element
        .block_flow_owner()
        .map_or(Clear::None, |owner| owner.block_flow().clear)
}

fn extend_open_column_flex_decoration_to_break(
    elements: &mut [(f32, LayoutNode)],
    content_height: f32,
) {
    struct PullbackSpacer(bool);

    impl LayoutVisitor for PullbackSpacer {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = element.lines.is_empty()
                && is_positive_with_roundoff(-element.box_model.margins.start)
                && element.paint.background.color.is_none()
                && !element.box_model.border.has_any();
        }
    }

    struct DecorationExtender {
        y: f32,
        content_height: f32,
    }

    impl LayoutVisitorMut for DecorationExtender {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            let Some(block_height) = element.box_model.size.height.used() else {
                return;
            };
            if !element.lines.is_empty() || !element.box_model.border.has_any() {
                return;
            }
            let target_flow = (self.content_height - self.y).max(0.0);
            let current_flow = block_height + element.box_model.border.vertical_width();
            if exceeds_with_roundoff(target_flow, current_flow) {
                element.box_model.border.bottom.width = 0.0;
                element.box_model.size.height = BlockSize::fragment(
                    (target_flow - element.box_model.border.vertical_width()).max(0.0),
                );
            }
        }
    }

    for idx in 0..elements.len().saturating_sub(1) {
        let mut pullback = PullbackSpacer(false);
        elements[idx + 1].1.accept(&mut pullback);
        if !pullback.0 {
            continue;
        }
        let (y_pos, element) = &mut elements[idx];
        element.accept_mut(&mut DecorationExtender {
            y: *y_pos,
            content_height,
        });
    }
}

/// Whether a layout element is out of normal flow (absolutely positioned) and so
/// contributes no height to its container and does not advance the flow cursor.
fn element_is_out_of_flow(element: &dyn LayoutElement) -> bool {
    !element.contributes_to_normal_flow()
}

/// Whether an in-flow element participates in adjacent-sibling vertical margin
/// collapse, mirroring `collapse_role` in the renderer. Flattened table rows
/// expose only the table's exterior margins through this capability; their
/// grid spacing remains separate internal geometry.
fn element_collapses_margins(element: &dyn LayoutElement) -> bool {
    element
        .block_flow_participant()
        .is_some_and(|participant| participant.collapses_outer_margins())
}

/// The collapsed vertical gap between two adjacent block margins (CSS 2.1
/// § 8.3.1): positive margins overlap, negative margins overlap, and a mixed
/// pair sums.
fn collapse_pair(margin_top: f32, prev_margin_bottom: f32) -> f32 {
    if margin_top >= 0.0 && prev_margin_bottom >= 0.0 {
        margin_top.max(prev_margin_bottom)
    } else if margin_top < 0.0 && prev_margin_bottom < 0.0 {
        margin_top.min(prev_margin_bottom)
    } else {
        margin_top + prev_margin_bottom
    }
}

/// The placement of a single floated child, relative to the container's
/// content-box top-left, computed by [`simulate_block_flow`]. The float's side
/// is read from the element itself at paint time, so only its index and top are
/// recorded here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FloatPlacement {
    /// Index of the float within the original `children` slice.
    pub index: usize,
    /// Distance of the float's border-box top below the content-box top.
    pub top: f32,
}

/// Result of simulating normal-flow block layout with simplified CSS floats.
#[derive(Debug, Clone, Default)]
pub(crate) struct BlockFlowResult {
    /// Total in-flow content height (border-box heights of in-flow children plus
    /// collapsed margins and any `clear` offsets). Floats do not extend it.
    pub height: f32,
    /// Placement of each floated child, in source order.
    pub floats: Vec<FloatPlacement>,
    /// Lowest bottom edge of any left float, below the content-box top.
    pub left_float_bottom: f32,
    /// Lowest bottom edge of any right float, below the content-box top.
    pub right_float_bottom: f32,
}

#[derive(Default)]
struct BlockFlowAccumulator {
    result: BlockFlowResult,
    prev_margin_bottom: Option<f32>,
}

impl BlockFlowAccumulator {
    fn include(&mut self, index: usize, child: &dyn LayoutElement, outer_height: f32) {
        if element_is_out_of_flow(child) {
            // Out of flow: contributes nothing and leaves the collapse chain.
            return;
        }

        let float = element_float(child);
        let margins = child
            .margin_holder()
            .map(|holder| *holder.margins())
            .unwrap_or(BlockMargins::ZERO);
        let (margin_top, margin_bottom) = (margins.start, margins.end);
        if float != Float::None {
            // A float is pinned to the current content bottom and stacked below
            // an earlier float on the same side, but does not advance normal flow.
            let side_bottom = if float == Float::Left {
                self.result.left_float_bottom
            } else {
                self.result.right_float_bottom
            };
            let float_top = (self.result.height + margin_top).max(side_bottom);
            let border_box_height = (outer_height - margin_top - margin_bottom).max(0.0);
            let float_bottom = float_top + border_box_height;
            if float == Float::Left {
                self.result.left_float_bottom = float_bottom;
            } else {
                self.result.right_float_bottom = float_bottom;
            }
            self.result.floats.push(FloatPlacement {
                index,
                top: float_top,
            });
            self.prev_margin_bottom = None;
            return;
        }

        // Clearance pushes this in-flow child below the relevant float and
        // breaks adjacent margin collapse.
        let clear = element_clear(child);
        let clear_to = match clear {
            Clear::Left => self.result.left_float_bottom,
            Clear::Right => self.result.right_float_bottom,
            Clear::Both => self
                .result
                .left_float_bottom
                .max(self.result.right_float_bottom),
            Clear::None => f32::NEG_INFINITY,
        };
        if clear != Clear::None && clear_to > self.result.height {
            self.result.height = clear_to;
            self.prev_margin_bottom = None;
        }

        self.result.height += outer_height;
        if element_collapses_margins(child) {
            if let Some(previous) = self.prev_margin_bottom {
                self.result.height -= previous + margin_top - collapse_pair(margin_top, previous);
            }
            self.prev_margin_bottom = Some(margin_bottom);
        } else {
            self.prev_margin_bottom = None;
        }
    }

    fn finish(self) -> BlockFlowResult {
        self.result
    }
}

/// Simulate normal-flow block layout of `children` with simplified floats and
/// `clear`, returning the in-flow content height and the resolved top of every
/// float. This is the single source of truth shared by the wrapper-height
/// estimate and the renderer's float placement, so the painted geometry always
/// matches the measured height.
///
/// The in-flow accumulation mirrors `collapsed_children_height` in the renderer
/// (sum of each child's outer `estimate_element_height` minus adjacent-sibling
/// margin-collapse overlap), so for the common no-float case the measured height
/// is byte-for-byte identical and nothing regresses.
///
/// Float model (block-sibling case): a `float: left|right` child is removed from
/// normal flow — it does not advance the flow cursor and does not stretch the
/// container — but it is pinned to the left/right content edge at the cursor's
/// current position (its top below the content origin). A later in-flow block
/// with `clear` is pushed below the bottom of the relevant float(s); that
/// clearance gap *does* extend the container because the cleared block is in
/// flow. Adjacent in-flow blocks collapse their vertical margins across
/// out-of-flow (float / absolute) siblings.
pub(crate) fn simulate_block_flow(children: &[LayoutNode]) -> BlockFlowResult {
    let mut flow = BlockFlowAccumulator::default();
    for (index, child) in children.iter().enumerate() {
        flow.include(index, child, estimate_element_height(child));
    }
    flow.finish()
}

/// Split a too-tall in-flow `TextBlock` at a line boundary (CSS Fragmentation 3
/// §3) so its first fragment fills the remaining fragmentainer height and the
/// rest continues on the next page. `avail_below_box_top` is the page height
/// still available below this box's *border-box top* on the current page
/// (`content_height − cursor − collapsed margin-top`).
///
/// Returns `(first_fragment, continuation)` for `box-decoration-break: slice`
/// (the default): the first fragment keeps the box's TOP border/padding but
/// drops its bottom border/padding/margin; the continuation drops its top
/// margin/border/padding and keeps the original bottom decoration. `None` is
/// returned when the box cannot be cleanly split between lines — a definite
/// `height`/clipped (overflow) box, a positioned/floated box, fewer than two
/// lines, or a boundary where every line fits or none would move — in which case
/// the caller places it whole (the pre-existing, possibly-overflowing behavior).
fn split_text_block(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        rule: FragmentBreakRule,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.result = split_text_block_node(element, self.available, self.rule);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        rule,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_text_block_node(
    element: &TextBlock,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    let lines = &element.lines;
    // Only a plain, auto-height, in-flow text block is splittable here. A box
    // with a definite height or `overflow` clip is a hard-sized box (treat as
    // monolithic); a positioned/floated box is out of normal flow and handled
    // elsewhere; a single line cannot be divided.
    if element.box_model.size.height.is_definite()
        || element.clipping.rect.is_some()
        || element.positioning.scheme != Position::Static
        || element.flow.float != Float::None
        || lines.len() < 2
    {
        return None;
    }

    // `box-decoration-break: clone` re-wraps EVERY fragment with the full
    // top+bottom border/padding/margin and background, so the first fragment's
    // line area is reduced by the bottom decoration too (the box closes on this
    // page). `slice` (default) leaves the box open at the bottom, so its lines
    // may extend to the page edge.
    let clone = element.fragmentation.box_fragmentation.decoration == BoxDecorationBreak::Clone;

    // Content-box height available for text lines on this page: the space below
    // the box's border-box top, minus the top border + top padding (and, under
    // `clone`, the bottom border + bottom padding the fragment also carries).
    let avail_lines = if clone {
        avail_below_box_top
            - element.box_model.border.vertical_width()
            - element.box_model.padding.vertical()
    } else {
        avail_below_box_top - element.box_model.border.top.width - element.box_model.padding.top
    };

    // Greedily keep whole lines that fit, but always retain at least one line on
    // this page — the forward-progress invariant: never leave a fragmentainer
    // empty / never break at the very top with zero content.
    let mut acc = 0.0f32;
    let mut idx = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let next = acc + line.height;
        if i > 0 && next > avail_lines {
            break;
        }
        acc = next;
        idx = i + 1;
    }
    if idx == 0 || idx >= lines.len() {
        // Every line fits (no overflow to split) or not even one would stay.
        return None;
    }

    // CSS Fragmentation 3 §3.4 first admits only boundaries satisfying both
    // `orphans` and `widows`. Pagination asks again with the emergency rule only
    // after a fresh-fragmentainer retry cannot yield a compliant break. Keeping
    // that relaxation explicit prevents an ordinary partially filled page from
    // silently stranding a line.
    idx = element
        .fragmentation
        .lines
        .split_index(lines.len(), idx, rule)?;

    split_text_block_at_line_node(element, idx)
}

/// Split an in-flow text block at a known line boundary.
///
/// The caller is responsible for choosing a legal boundary. This keeps
/// pagination policies that choose a semantic line boundary (such as CSS GCPM
/// `footnote-policy: line`) on the same decoration and continuation path as
/// ordinary height-driven fragmentation.
fn split_text_block_at_line(
    element: &dyn LayoutElement,
    idx: usize,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        index: usize,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.result = split_text_block_at_line_node(element, self.index);
        }
    }

    let mut visitor = SplitVisitor {
        index: idx,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_text_block_at_line_node(
    element: &TextBlock,
    idx: usize,
) -> Option<(LayoutNode, LayoutNode)> {
    if element.box_model.size.height.is_definite()
        || element.clipping.rect.is_some()
        || element.positioning.scheme != Position::Static
        || element.flow.float != Float::None
        || idx == 0
        || idx >= element.lines.len()
    {
        return None;
    }

    let clone = element.fragmentation.box_fragmentation.decoration == BoxDecorationBreak::Clone;

    // First fragment: the lines that fit. Under `slice` it keeps the box's top
    // decoration but drops its bottom border/padding/margin (the box stays open
    // at the page bottom); under `clone` it keeps the FULL decoration and closes.
    let mut first = element.clone();
    first.lines = element.lines[..idx].to_vec();
    if !clone {
        first.box_model.margins.end = 0.0;
        first.box_model.padding.bottom = 0.0;
        first.box_model.border.bottom.width = 0.0;
        first.paint.border_radii = first.paint.border_radii.clear_bottom();
    }

    // Continuation: the remaining lines. Under `slice` it drops the top
    // margin/border/padding (continuing the open box) and keeps the original
    // bottom decoration so the LAST fragment closes it; under `clone` it keeps
    // the FULL decoration so the fragment is independently wrapped.
    let mut rest = element.clone();
    rest.lines = element.lines[idx..].to_vec();
    if !clone {
        rest.box_model.margins.start = 0.0;
        rest.box_model.padding.top = 0.0;
        rest.box_model.border.top.width = 0.0;
        rest.paint.border_radii = rest.paint.border_radii.clear_top();
    }

    retain_reference_box(element, &mut first, &mut rest);

    Some((Box::new(first), Box::new(rest)))
}

/// Return the continuation size only when a split produces two genuinely
/// positive fragments. The shared roundoff bound covers arithmetic noise only;
/// no authored minimum fragment size is imposed.
fn split_remainder(total: f32, first: f32) -> Option<f32> {
    let remainder = total - first;
    (is_positive_with_roundoff(first) && is_positive_with_roundoff(remainder)).then_some(remainder)
}

/// Slice a definite-height in-flow text block at the fragmentainer edge.
///
/// A fixed-height box carries its background and border onto the next page.
/// The paginator decides whether an empty box is tall enough to start internal
/// fragmentation before calling this splitter.
fn split_fixed_height_text_block(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.result = split_fixed_height_text_block_node(element, self.available);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_fixed_height_text_block_node(
    element: &TextBlock,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    if !element.box_model.size.height.is_definite() {
        return None;
    }
    let block_height = element.box_model.size.height.used()?;
    if element.clipping.rect.is_some()
        || element.positioning.scheme != Position::Static
        || element.flow.float != Float::None
        || block_height <= 0.0
    {
        return None;
    }

    let clone = element.fragmentation.box_fragmentation.decoration == BoxDecorationBreak::Clone;
    let first_border_h = if clone {
        element.box_model.border.vertical_width()
    } else {
        element.box_model.border.top.width
    };
    let first_content_h = (avail_below_box_top - first_border_h).min(block_height);
    let rest_content_h = split_remainder(block_height, first_content_h)?;

    let first_line_space = if clone {
        first_content_h - element.box_model.padding.vertical()
    } else {
        first_content_h - element.box_model.padding.top
    };
    let mut acc = 0.0f32;
    let mut idx = 0usize;
    for (i, line) in element.lines.iter().enumerate() {
        let next = acc + line.height;
        if i > 0 && next > first_line_space {
            break;
        }
        if next > first_line_space && i == 0 {
            break;
        }
        acc = next;
        idx = i + 1;
    }

    let mut first = element.clone();
    first.lines = element.lines[..idx.min(element.lines.len())].to_vec();
    first.box_model.size.height = BlockSize::definite(first_content_h);
    if !clone {
        first.box_model.margins.end = 0.0;
        first.box_model.padding.bottom = 0.0;
        first.box_model.border.bottom.width = 0.0;
        first.paint.border_radii = first.paint.border_radii.clear_bottom();
    }

    let mut rest = element.clone();
    rest.lines = element.lines[idx.min(element.lines.len())..].to_vec();
    rest.box_model.size.height = BlockSize::definite(rest_content_h);
    if !clone {
        rest.box_model.margins.start = 0.0;
        rest.box_model.padding.top = 0.0;
        rest.box_model.border.top.width = 0.0;
        rest.paint.border_radii = rest.paint.border_radii.clear_top();
    }

    retain_reference_box(element, &mut first, &mut rest);

    Some((Box::new(first), Box::new(rest)))
}

/// Slice a too-tall in-flow raster `Image` at the page boundary (CSS
/// Fragmentation 3 §4.1: monolithic content taller than the fragmentainer is
/// sliced at the fragmentainer edge rather than discarded). `avail_below_box_top`
/// is the page height still available below the image's border-box top on the
/// current page.
///
/// Returns `(first_fragment, continuation)`: the first fragment fills the rest of
/// this page with the TOP slice of the source raster (its `flow_extra_bottom` and
/// `margin_bottom` dropped), and the continuation displays the remainder on the
/// next page (its `margin_top` dropped, the original bottom decoration kept so the
/// FINAL fragment closes the box). Each fragment records its offset into the
/// original replaced-content box, so the renderer reuses and clips the source
/// instead of decoding and resampling a new raster.
///
/// Returns `None` (caller places the image whole, the pre-existing overflow
/// behavior) when the image cannot be sliced cleanly: a non-`fill` `object-fit`
/// (the source does not map linearly onto the box, so a box slice is not a source
/// slice), a bordered box (the frame cannot be split here), a `filter` raster
/// (already feathered/padded), or no usable space on the page.
fn split_image_block(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_image(&mut self, element: &Image) {
            self.result = split_image_block_node(element, self.available);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_image_block_node(
    element: &Image,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    let height = element.geometry.size.height;
    if element.sampling.replaced.object_fit != ObjectFit::Fill
        || element.geometry.border.vertical_width() != 0.0
        || !element.paint.raster_overflow.is_zero()
        || element.paint.filter_effect.is_some()
        || height <= 0.0
    {
        return None;
    }

    // Display height of the TOP slice that fits on this page.
    let first_h = avail_below_box_top.min(height);
    let rest_h = split_remainder(height, first_h)?;

    let source_content_size = element
        .sampling
        .replaced
        .fragment
        .map_or(element.geometry.content_size(), |fragment| {
            fragment.source_content_size
        });
    if !is_positive_with_roundoff(source_content_size.width)
        || !is_positive_with_roundoff(source_content_size.height)
    {
        return None;
    }
    let fragment = element
        .sampling
        .replaced
        .fragment
        .unwrap_or_else(|| ReplacedFragment::initial(source_content_size));

    let mut first = element.clone();
    first.geometry.size.height = first_h;
    first.geometry.flow.extra_end = 0.0;
    first.geometry.flow.margins.end = 0.0;
    first.sampling.replaced.fragment = Some(fragment);

    let mut rest = element.clone();
    rest.geometry.size.height = rest_h;
    rest.geometry.flow.margins.start = 0.0;
    rest.sampling.replaced.fragment = Some(fragment.following_block(first_h));

    Some((Box::new(first), Box::new(rest)))
}

/// Slice a too-tall SVG replaced element at its content-box page edge.
///
/// The SVG tree remains whole and each page clips the original rendered
/// viewport. Rewriting the root `viewBox` would reinterpret root
/// `preserveAspectRatio` per fragment and visibly shift the content.
fn split_svg_block(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_svg(&mut self, element: &Svg) {
            self.result = split_svg_block_node(element, self.available);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_svg_block_node(
    element: &Svg,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    let height = element.geometry.size.height;
    let border = &element.geometry.border;
    if height <= 0.0 {
        return None;
    }

    let first_h = avail_below_box_top.min(height);
    let rest_h = split_remainder(height, first_h)?;
    let first_content_h = first_h - border.top.width;
    let total_content_h = height - border.vertical_width();
    if !is_positive_with_roundoff(first_content_h)
        || !exceeds_with_roundoff(total_content_h, first_content_h)
    {
        return None;
    }
    let source_content_size = element.replaced.fragment.map_or_else(
        || Size::new(element.geometry.content_size().width, total_content_h),
        |fragment| fragment.source_content_size,
    );
    if !is_positive_with_roundoff(source_content_size.width)
        || !is_positive_with_roundoff(source_content_size.height)
    {
        return None;
    }
    let fragment = element
        .replaced
        .fragment
        .unwrap_or_else(|| ReplacedFragment::initial(source_content_size));

    let mut first = element.clone();
    first.geometry.size.height = first_h;
    first.geometry.flow.extra_end = 0.0;
    first.geometry.flow.margins.end = 0.0;
    first.geometry.border.bottom.width = 0.0;
    first.replaced.fragment = Some(fragment);

    let mut rest = element.clone();
    rest.geometry.size.height = rest_h;
    rest.geometry.flow.margins.start = 0.0;
    rest.geometry.border.top.width = 0.0;
    rest.replaced.fragment = Some(fragment.following_block(first_content_h));

    Some((Box::new(first), Box::new(rest)))
}

/// Dispatch a too-tall in-flow element to the right splitter: a `TextBlock`
/// splits at a line boundary, a raster `Image` slices at the page edge, and a
/// `Container` splits between (or recurses into) its children. Returns `None`
/// for anything monolithic/out-of-flow that cannot be fragmented here. Shared by
/// `paginate` (top-level boxes) and `split_container` (a single too-tall child).
fn split_element(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    if element
        .box_fragmentation_owner()
        .is_some_and(|owner| !owner.box_fragmentation().permits_split(rule))
    {
        return None;
    }

    split_fixed_height_text_block(element, avail_below_box_top)
        .or_else(|| split_text_block(element, avail_below_box_top, rule))
        .or_else(|| split_image_block(element, avail_below_box_top))
        .or_else(|| split_svg_block(element, avail_below_box_top))
        .or_else(|| split_empty_sized_container(element, avail_below_box_top))
        .or_else(|| split_table_row(element, avail_below_box_top, rule))
        .or_else(|| split_grid_row(element, avail_below_box_top, rule))
        .or_else(|| split_flex_row(element, avail_below_box_top))
        .or_else(|| split_container(element, avail_below_box_top, rule))
}

/// Split an empty container with a used block extent only after pagination
/// establishes that it needs an internal fragment. This covers both authored
/// definite heights and content-dependent `min-height` floors. Containers with
/// children use [`split_container`] to break at child boundaries instead.
fn split_empty_sized_container(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_container(&mut self, element: &Container) {
            self.result = split_empty_sized_container_node(element, self.available);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_empty_sized_container_node(
    element: &Container,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    let block_size = element.box_model.size.height;
    let box_h = block_size.used()?;
    if element.positioning.scheme != Position::Static
        || element.flow.float != Float::None
        || !element.children.is_empty()
        || !is_positive_with_roundoff(box_h)
        || !is_positive_with_roundoff(avail_below_box_top)
        || !exceeds_with_roundoff(box_h, avail_below_box_top)
    {
        return None;
    }

    let consumed_h = avail_below_box_top.min(box_h).max(0.0);
    let (first_size, rest_size) = block_size.split_fragment_at(consumed_h)?;
    let clone = element.fragmentation.decoration == BoxDecorationBreak::Clone;

    let mut first = element.clone();
    first.box_model.size.height = first_size;
    if !clone {
        first.box_model.margins.end = 0.0;
        first.box_model.padding.bottom = 0.0;
        first.box_model.border.bottom.width = 0.0;
        first.paint.border_radii = first.paint.border_radii.clear_bottom();
    }

    let mut rest = element.clone();
    rest.box_model.size.height = rest_size;
    if !clone {
        rest.box_model.margins.start = 0.0;
        rest.box_model.padding.top = 0.0;
        rest.box_model.border.top.width = 0.0;
        rest.paint.border_radii = rest.paint.border_radii.clear_top();
    }

    retain_reference_box(element, &mut first, &mut rest);

    Some((Box::new(first), Box::new(rest)))
}

fn split_nested_rows_at(
    rows: &[LayoutNode],
    available_height: f32,
    rule: FragmentBreakRule,
) -> (Vec<LayoutNode>, Vec<LayoutNode>) {
    if rows.is_empty() || !is_positive_with_roundoff(available_height) {
        return (Vec::new(), rows.to_vec());
    }
    let mut first = Vec::new();
    let mut rest = Vec::new();
    let mut used = 0.0_f32;
    for (idx, child) in rows.iter().enumerate() {
        let child_h = estimate_element_height(child);
        if !exceeds_with_roundoff(used + child_h, available_height) {
            first.push(child.clone());
            used += child_h;
            continue;
        }
        let margin_start = child
            .margin_holder()
            .map(|holder| holder.margins().start)
            .unwrap_or_default();
        let child_avail = (available_height - used - margin_start).max(0.0);
        if is_positive_with_roundoff(child_avail) {
            if let Some((head, tail)) = split_empty_sized_container(child, child_avail)
                .or_else(|| split_element(child, child_avail, rule))
            {
                first.push(head);
                rest.push(tail);
                rest.extend_from_slice(&rows[idx + 1..]);
                return (first, rest);
            }
        }
        rest.extend_from_slice(&rows[idx..]);
        return (first, rest);
    }
    (first, rest)
}

/// Content capacity of a cell fragment after both retained logical edge
/// insets have participated in fragmentation. The first painted slice drops
/// its block-end border/padding later for `box-decoration-break: slice`, but
/// that edge still belongs to the unsplit principal box and constrains the
/// class-B line break chosen before slicing. Reserving it prevents a final
/// line from being admitted only to leave a decoration-only continuation.
fn cell_fragment_content_capacity(
    cell: &crate::layout::cells::CellBox,
    fragment_height: f32,
) -> f32 {
    (fragment_height - cell.box_model.content_insets.vertical()).max(0.0)
}

fn fitting_line_count(lines: &[TextLine], available_height: f32, rule: FragmentBreakRule) -> usize {
    let mut used = 0.0_f32;
    let mut count = 0;
    for (index, line) in lines.iter().enumerate() {
        let next = used + line.height;
        // Only the emergency pass may consume a first line taller than the
        // available content lane. An incoming ordinary row must move intact;
        // otherwise the line is clipped at the fragmentainer boundary even
        // though a legal break exists before the row.
        if exceeds_with_roundoff(next, available_height)
            && (index > 0 || rule != FragmentBreakRule::Emergency)
        {
            break;
        }
        used = next;
        count = index + 1;
    }
    count
}

fn fragment_line_count(
    cell: &crate::layout::cells::CellBox,
    available_height: f32,
    rule: FragmentBreakRule,
) -> usize {
    let line_count = cell.content.lines.len();
    let fitting = fitting_line_count(&cell.content.lines, available_height, rule).min(line_count);
    if fitting >= line_count {
        return line_count;
    }
    cell.fragmentation
        .lines
        .split_index(line_count, fitting, rule)
        .unwrap_or_default()
}

fn cell_content_has_flow(content: &crate::layout::cells::CellContent) -> bool {
    !content.lines.is_empty() || !content.children.is_empty()
}

/// Whether a break advances at least one of a row's parallel cell flows.
///
/// CSS Break 3 treats the contents of the cells in a row as parallel
/// fragmentation flows. Each flow chooses its own legal break: one cell may
/// contribute content to the current fragment while another moves all of its
/// content to the next fragment to satisfy `orphans` and `widows`. Requiring
/// every non-empty flow to advance incorrectly makes the row monolithic.
fn parallel_cell_break_advances_content<'a, 'b>(
    original: impl Iterator<Item = &'a crate::layout::cells::CellContent>,
    first_fragments: impl Iterator<Item = &'b crate::layout::cells::CellContent>,
    rule: FragmentBreakRule,
) -> bool {
    if rule == FragmentBreakRule::Emergency {
        return true;
    }

    let mut has_flow = false;
    let mut advances_flow = false;
    for (source, first) in original.zip(first_fragments) {
        has_flow |= cell_content_has_flow(source);
        advances_flow |= cell_content_has_flow(first);
    }

    !has_flow || advances_flow
}

fn cell_content_block_extent(content: &crate::layout::cells::CellContent) -> f32 {
    content.lines.iter().map(|line| line.height).sum::<f32>()
        + content
            .children
            .iter()
            .map(|child| estimate_element_height(child.as_ref()))
            .sum::<f32>()
}

fn split_cell_content(
    cell: &crate::layout::cells::CellBox,
    fragment_height: f32,
    rule: FragmentBreakRule,
) -> (
    crate::layout::cells::CellContent,
    crate::layout::cells::CellContent,
) {
    let available_content = cell_fragment_content_capacity(cell, fragment_height);
    let cut = fragment_line_count(cell, available_content, rule).min(cell.content.lines.len());
    let text_first_height = cell.content.lines[..cut]
        .iter()
        .map(|line| line.height)
        .sum::<f32>();
    let nested_available = (available_content - text_first_height).max(0.0);
    let (first_children, rest_children) =
        split_nested_rows_at(&cell.content.children, nested_available, rule);

    (
        crate::layout::cells::CellContent {
            lines: cell.content.lines[..cut].to_vec(),
            children: first_children,
        },
        crate::layout::cells::CellContent {
            lines: cell.content.lines[cut..].to_vec(),
            children: rest_children,
        },
    )
}

fn split_grid_cell(
    cell: &GridCell,
    first_h: f32,
    rest_h: f32,
    rule: FragmentBreakRule,
) -> (GridCell, GridCell) {
    let (first_content, rest_content) = split_cell_content(&cell.layout, first_h, rule);

    let mut first = cell.clone();
    first.layout.content = first_content;
    first.layout.box_model.border.bottom.width = 0.0;
    first.layout.box_model.border_insets.bottom = 0.0;
    first.layout.box_model.content_insets.bottom = 0.0;
    first.layout.box_model.minimum_block_size = first_h;
    if let Some(inset) = &mut first.placement.inset {
        inset.size.height = first_h;
        inset.offset.y = inset.offset.y.min(first_h);
    }

    let mut rest = cell.clone();
    rest.layout.content = rest_content;
    rest.layout.box_model.border.top.width = 0.0;
    rest.layout.box_model.border_insets.top = 0.0;
    rest.layout.box_model.content_insets.top = 0.0;
    let rest_intrinsic_h = rest.layout.box_model.content_insets.top
        + cell_content_block_extent(&rest.layout.content)
        + rest.layout.box_model.content_insets.bottom;
    let adjusted_rest_h = if cell.placement.row_span == 1 {
        rest_h.max(rest_intrinsic_h)
    } else {
        rest_h
    };
    rest.layout.box_model.minimum_block_size = adjusted_rest_h;
    if let Some(inset) = &mut rest.placement.inset {
        inset.size.height = adjusted_rest_h;
        inset.offset.y = 0.0;
    }

    (first, rest)
}

fn split_grid_row(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        rule: FragmentBreakRule,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_grid_row(&mut self, element: &GridRow) {
            self.result = split_grid_row_node(element, self.available, self.rule);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        rule,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_grid_row_node(
    element: &GridRow,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    let row_h = element
        .content
        .cells
        .iter()
        .map(|cell| cell.layout.box_model.minimum_block_size)
        .fold(0.0_f32, f32::max);
    let available_row_h =
        avail_below_box_top - element.box_model.border.top.width - element.box_model.padding.top;
    if !is_positive_with_roundoff(row_h)
        || !is_positive_with_roundoff(available_row_h)
        || !exceeds_with_roundoff(row_h, available_row_h)
    {
        return None;
    }

    let first_h = available_row_h.min(row_h).max(0.0);
    let rest_h = split_remainder(row_h, first_h)?;

    let split_cells: Vec<(GridCell, GridCell)> = element
        .content
        .cells
        .iter()
        .map(|cell| split_grid_cell(cell, first_h, rest_h, rule))
        .collect();
    if !parallel_cell_break_advances_content(
        element
            .content
            .cells
            .iter()
            .map(|cell| &cell.layout.content),
        split_cells.iter().map(|(first, _)| &first.layout.content),
        rule,
    ) {
        return None;
    }

    let mut first = element.clone();
    first.content.cells = split_cells.iter().map(|(cell, _)| cell.clone()).collect();
    first.box_model.margins.end = 0.0;
    first.box_model.padding.bottom = 0.0;
    first.box_model.border.bottom.width = 0.0;

    let mut rest = element.clone();
    rest.content.cells = split_cells.into_iter().map(|(_, cell)| cell).collect();
    rest.box_model.margins.start = 0.0;
    rest.box_model.padding.top = 0.0;
    rest.box_model.border.top.width = 0.0;

    Some((Box::new(first), Box::new(rest)))
}

/// Split a table row that is taller than the current fragmentainer. CSS Tables
/// fragments row boxes by slicing each cell box at the page edge; under the
/// default `box-decoration-break: slice` the first fragment keeps the top
/// border/padding and the continuation keeps the bottom edge.
fn split_table_row(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        rule: FragmentBreakRule,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_table_row(&mut self, element: &TableRow) {
            self.result = split_table_row_node(element, self.available, self.rule);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        rule,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_table_row_node(
    element: &TableRow,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    // `break-inside: avoid` constrains normal row fragmentation at every
    // nesting depth, including rows reached through a table's principal-box
    // container. Only the emergency pass may relax it for a row taller than a
    // fresh fragmentainer.
    if element.fragmentation.avoid_inside && rule != FragmentBreakRule::Emergency {
        return None;
    }

    let row_h = element.content.cells.row_block_extent();
    if !is_positive_with_roundoff(row_h)
        || !is_positive_with_roundoff(avail_below_box_top)
        || !exceeds_with_roundoff(row_h, avail_below_box_top)
    {
        return None;
    }

    let consumed_h = avail_below_box_top.min(row_h).max(0.0);
    let rest_h = split_remainder(row_h, consumed_h)?;
    // A sliced cell fragment occupies every available point up to the page
    // edge. Its retained top edge is painted within that fragment; subtracting
    // half the border here loses real content height and leaves a visible gap
    // at the fragmentainer boundary.
    let first_painted_h = consumed_h;

    let split_content = element
        .content
        .cells
        .iter()
        .map(|cell| split_cell_content(&cell.layout, first_painted_h, rule))
        .collect::<Vec<_>>();
    if !parallel_cell_break_advances_content(
        element
            .content
            .cells
            .iter()
            .map(|cell| &cell.layout.content),
        split_content.iter().map(|(first, _)| first),
        rule,
    ) {
        return None;
    }

    let mut first = element.clone();
    first.flow.margins.end = 0.0;
    first.flow.internal.end = 0.0;
    first.flow.extra_end = 0.0;
    for (cell, (content, _)) in first.content.cells.iter_mut().zip(&split_content) {
        cell.layout.content = content.clone();
        cell.layout.box_model.border.bottom.width = 0.0;
        cell.layout.box_model.border_insets.bottom = 0.0;
        cell.layout.box_model.content_insets.bottom = 0.0;
        cell.layout.box_model.minimum_block_size = first_painted_h;
    }
    first.collapsed_borders.open_fragment_end();

    let mut rest = element.clone();
    rest.flow.margins.start = 0.0;
    rest.flow.internal.start = 0.0;
    for (cell, (_, content)) in rest.content.cells.iter_mut().zip(split_content) {
        cell.layout.content = content;
        cell.layout.box_model.border.top.width = 0.0;
        cell.layout.box_model.border_insets.top = 0.0;
        cell.layout.box_model.content_insets.top = 0.0;
        cell.layout.box_model.minimum_block_size = rest_h;
    }
    rest.collapsed_borders.open_fragment_start();

    Some((Box::new(first), Box::new(rest)))
}

/// Slice a row-direction flex container at a flex-line boundary. Cells retain
/// their main-axis geometry while the continuation is rebased to its own
/// cross-axis origin; the container decoration follows `box-decoration-break:
/// slice`.
fn split_flex_row_at_line(
    element: &FlexRow,
    cut_y: f32,
    first_row_height: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    if element.content.cells.is_empty()
        || !is_positive_with_roundoff(cut_y)
        || !exceeds_with_roundoff(element.content.row_height, cut_y)
    {
        return None;
    }

    let mut first_cells = Vec::new();
    let mut rest_cells = Vec::new();
    for cell in &element.content.cells {
        if exceeds_with_roundoff(cut_y, cell.y_offset) {
            first_cells.push(cell.clone());
        } else {
            let mut rest = cell.clone();
            rest.y_offset = (rest.y_offset - cut_y).max(0.0);
            rest_cells.push(rest);
        }
    }
    if first_cells.is_empty() || rest_cells.is_empty() {
        return None;
    }

    let mut first = element.clone();
    first
        .content
        .forced_line_breaks
        .retain(|marker| first_cells.iter().any(|cell| cell.line_id == marker.before));
    first.content.cells = first_cells;
    first.content.row_height = first_row_height.max(cut_y);
    first.box_model.margins.end = 0.0;
    first.box_model.padding.bottom = 0.0;
    first.box_model.border.bottom.width = 0.0;

    let mut rest = element.clone();
    rest.content
        .forced_line_breaks
        .retain(|marker| rest_cells.iter().any(|cell| cell.line_id == marker.before));
    rest.content.cells = rest_cells;
    rest.content.row_height = (rest.content.row_height - cut_y).max(0.0);
    rest.box_model.margins.start = 0.0;
    rest.box_model.padding.top = 0.0;
    rest.box_model.border.top.width = 0.0;

    Some((Box::new(first), Box::new(rest)))
}

/// Split a wrapped row-direction flex container at the first class-A line
/// boundary that no longer fits in the current fragmentainer.
fn split_flex_row(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_flex_row(&mut self, element: &FlexRow) {
            self.result = split_flex_row_node(element, self.available);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_flex_row_node(
    element: &FlexRow,
    avail_below_box_top: f32,
) -> Option<(LayoutNode, LayoutNode)> {
    if element.content.cells.is_empty()
        || !is_positive_with_roundoff(element.content.row_height)
        || !is_positive_with_roundoff(avail_below_box_top)
    {
        return None;
    }
    let avail_inner =
        (avail_below_box_top - element.box_model.border.top.width - element.box_model.padding.top)
            .max(0.0);
    if !exceeds_with_roundoff(element.content.row_height, avail_inner) {
        return None;
    }

    let mut line_tops: Vec<f32> = element
        .content
        .cells
        .iter()
        .map(|cell| cell.y_offset)
        .collect();
    line_tops.sort_by(f32::total_cmp);
    line_tops.dedup_by(|a, b| equal_with_roundoff(*a, *b));
    if line_tops.len() <= 1 {
        return None;
    }

    let line_extent = |line_top: f32| -> f32 {
        element
            .content
            .cells
            .iter()
            .filter(|cell| equal_with_roundoff(cell.y_offset, line_top))
            .map(|cell| {
                if cell.line_cross_size > 0.0 {
                    cell.line_cross_size
                } else {
                    cell.natural_height
                }
            })
            .fold(0.0_f32, f32::max)
    };

    let cut_y = line_tops.iter().enumerate().find_map(|(idx, &top)| {
        (idx > 0 && exceeds_with_roundoff(top + line_extent(top), avail_inner)).then_some(top)
    })?;
    split_flex_row_at_line(element, cut_y, avail_inner)
}

/// Split a too-tall in-flow `Container` between its children (CSS Fragmentation 3
/// §3, class-A break point) so its first fragment fills the remaining
/// fragmentainer height and the rest continues on the next page.
/// `avail_below_box_top` is the page height still available below this box's
/// *border-box top* on the current page.
///
/// Returns `(first_fragment, continuation)`. Under `box-decoration-break: slice`
/// (the default) the first fragment keeps the box's TOP border/padding but drops
/// its bottom border/padding/margin (the box stays open at the page bottom), and
/// the continuation drops its top margin/border/padding while keeping the bottom
/// so the LAST fragment closes the box. Under `clone` EVERY fragment is
/// independently wrapped with the full border/padding/margin and background.
///
/// Returns `None` — so the caller places the box whole, the pre-existing
/// (possibly-overflowing) behavior — for any container that cannot be cleanly
/// split: a definite-`height`/clipped (overflow) box, a positioned or floated
/// box, or an empty box. The split always keeps at least the first child on this
/// page (forward progress) and the continuation carries strictly less content
/// than the original, so re-enqueuing it terminates. When the first child is
/// ALONE taller than the fragmentainer the splitter RECURSES into it (rather than
/// leaving it whole to clip), so a deeply nested too-tall box still fragments
/// across pages instead of losing data.
fn split_container(
    element: &dyn LayoutElement,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    struct SplitVisitor {
        available: f32,
        rule: FragmentBreakRule,
        result: Option<(LayoutNode, LayoutNode)>,
    }

    impl LayoutVisitor for SplitVisitor {
        fn visit_container(&mut self, element: &Container) {
            self.result = split_container_node(element, self.available, self.rule)
                .map(|(before, after)| (before.boxed(), after.boxed()));
        }

        fn visit_table(&mut self, element: &Table) {
            self.result = split_table_node(element, self.available, self.rule);
        }
    }

    let mut visitor = SplitVisitor {
        available: avail_below_box_top,
        rule,
        result: None,
    };
    element.accept(&mut visitor);
    visitor.result
}

fn split_container_node(
    element: &Container,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(Container, Container)> {
    let children = &element.children;
    // Only a plain, auto-height, in-flow container is splittable here. A definite
    // `height` or `overflow` clip makes it a hard-sized/monolithic box; a
    // positioned/floated box is out of normal flow; an empty box has nothing to
    // fragment. A single-child box has no between-children break point but may
    // still be split by RECURSING into that one (too-tall) child below.
    if element.box_model.size.height.is_definite()
        || element.overflow.combined.clips()
        || element.positioning.scheme != Position::Static
        || element.flow.float != Float::None
        || children.is_empty()
    {
        return None;
    }

    // Any out-of-flow (absolutely positioned) child is anchored, not flowed, so it
    // must not become a break boundary or move to the continuation independently.
    // Keep the split path to the simple all-in-flow case; anything else is placed
    // whole (unchanged behavior).
    if children
        .iter()
        .any(|child| element_is_out_of_flow(child.as_ref()))
    {
        return None;
    }

    let clone = element.fragmentation.decoration == BoxDecorationBreak::Clone;

    // Content-box height available for children on this page: below the box's
    // border-box top, minus the top border + top padding (and, under `clone`, the
    // bottom border + bottom padding the fragment also carries).
    let avail_children = if clone {
        avail_below_box_top
            - element.box_model.border.vertical_width()
            - element.box_model.padding.vertical()
    } else {
        avail_below_box_top - element.box_model.border.top.width - element.box_model.padding.top
    };
    let continuation_size = element
        .box_model
        .size
        .height
        .remaining_fragment_floor(avail_below_box_top);
    let has_floor_continuation = continuation_size.used().is_some();

    // The page-fit check that brought us here sums the children's outer heights
    // WITHOUT adjacent-sibling margin collapse, so a box whose children collapse
    // (CSS 2.1 §8.3.1) is over-measured and can look like it overflows when it
    // actually fits. Re-measure with the collapsed model the renderer uses
    // (`simulate_block_flow`): if the children genuinely fit, the box is not
    // overflowing — place it whole (unchanged behaviour) rather than spuriously
    // fragmenting a box that lands on a single page in Chrome.
    if !exceeds_with_roundoff(simulate_block_flow(children).height, avail_children)
        && !has_floor_continuation
    {
        return None;
    }

    // Greedily keep whole children that fit, always retaining at least the first
    // (forward progress). Children heights are summed the same way the container's
    // own auto height is measured in `paginate` (plain outer-height sum), so the
    // boundary the fit-decision saw and the boundary we cut at agree.
    let mut acc = 0.0f32;
    let mut idx = 0usize;
    for (i, child) in children.iter().enumerate() {
        let next = acc + estimate_element_height(child.as_ref());
        if i > 0 && next > avail_children {
            break;
        }
        acc = next;
        idx = i + 1;
    }

    // Partition the children into the first fragment's list and the continuation's
    // list. Normally the cut is between children at `idx`. But the first child is
    // always force-kept for forward progress, so when it ALONE overflows the
    // fragmentainer (idx == 1 and its height exceeds the space), placing it whole
    // would clip it (data loss). Instead RECURSE into that child — split it with
    // the same splitter — so its head fills this page and its tail continues. Only
    // the first child can be the too-tall one (every later kept child fit), so this
    // single check covers every nested-too-tall case (CSS Fragmentation 3 §3).
    let first_child_h = estimate_element_height(children[0].as_ref());
    let (f_children_vec, mut r_children_vec) = if idx == 1 && first_child_h > avail_children {
        let first_child = &children[0];
        // The child's border-box top sits at the container's content-box top plus
        // its own margin-top, so it has that much less room than the content box.
        let child_avail = avail_children
            - first_child
                .margin_holder()
                .map(|holder| holder.margins().start)
                .unwrap_or_default();
        match split_element(first_child.as_ref(), child_avail, rule) {
            Some((c_first, c_rest)) => {
                let mut rest_children = vec![c_rest];
                rest_children.extend_from_slice(&children[1..]);
                (vec![c_first], rest_children)
            }
            // The single too-tall child cannot be split (e.g. a definite-height /
            // clipped / replaced box). If there are later siblings, still cut after
            // it (it overflows, as before); otherwise nothing can be done — place
            // the whole container as-is (unchanged overflow behaviour).
            None if children.len() >= 2 => (children[..1].to_vec(), children[1..].to_vec()),
            None => return None,
        }
    } else if idx < children.len() {
        let next_child = &children[idx];
        let margin_start = next_child
            .margin_holder()
            .map(|holder| holder.margins().start)
            .unwrap_or_default();
        let child_avail = (avail_children - acc - margin_start).max(0.0);
        if is_positive_with_roundoff(child_avail) {
            if let Some((c_first, c_rest)) = split_element(next_child.as_ref(), child_avail, rule) {
                let mut first_children = children[..idx].to_vec();
                first_children.push(c_first);
                let mut rest_children = vec![c_rest];
                rest_children.extend_from_slice(&children[idx + 1..]);
                (first_children, rest_children)
            } else {
                (children[..idx].to_vec(), children[idx..].to_vec())
            }
        } else {
            (children[..idx].to_vec(), children[idx..].to_vec())
        }
    } else if idx >= children.len() {
        // Every child fits at this boundary. Usually there is nothing to move,
        // but a composite `min-height` can still have an unconsumed floor. Emit
        // an empty continuation for that remaining principal-box geometry.
        if has_floor_continuation {
            (children.to_vec(), Vec::new())
        } else {
            return None;
        }
    } else {
        (children[..idx].to_vec(), children[idx..].to_vec())
    };
    if let Some(first) = r_children_vec.first_mut() {
        first.suppress_first_fragment_spacing();
    }

    // First fragment: the children that fit, with the box's top decoration. Under
    // `slice` drop the bottom border/padding/margin (box stays open at the page
    // bottom); under `clone` keep the full decoration so the fragment closes.
    let mut first = element.clone();
    first.children = f_children_vec;
    if !clone {
        first.box_model.margins.end = 0.0;
        first.box_model.padding.bottom = 0.0;
        first.box_model.border.bottom.width = 0.0;
        // css-break-3 §5.4: under `slice` the fragmentation CUT edge is
        // square — only the box's real corners stay rounded. This fragment's
        // bottom edge is the cut, so drop the bottom-right/bottom-left radii.
        first.paint.border_radii = first.paint.border_radii.clear_bottom();
        // A box that continues onto the next fragmentainer occupies the FULL
        // remaining height of THIS one: its background and left/right borders
        // extend to the page bottom even though the children only fill part of
        // it (css-break-3 — the box is sliced at the fragmentainer edge, not
        // shrink-wrapped to the children that landed on this page). Pin the
        // first fragment's border-box height to that remaining space so the
        // background/side-borders reach the page bottom, matching Chrome. The
        // last fragment keeps auto height (block_height stays None) so it ends
        // at its natural content + bottom decoration.
        first.box_model.size.height = BlockSize::fragment(avail_below_box_top);
    }

    // Continuation: the remaining children. Under `slice` drop the top
    // margin/border/padding (the open box continues) and keep the bottom so the
    // LAST fragment closes it; under `clone` keep the full decoration.
    let mut rest = element.clone();
    rest.children = r_children_vec;
    rest.box_model.size.height = continuation_size;
    if !clone {
        rest.box_model.margins.start = 0.0;
        rest.box_model.padding.top = 0.0;
        rest.box_model.border.top.width = 0.0;
        // css-break-3 §5.4: the continuation's TOP edge is the cut, so it is
        // square — drop the top-left/top-right radii (the original bottom
        // corners stay rounded so the LAST fragment closes the box).
        rest.paint.border_radii = rest.paint.border_radii.clear_top();
    }

    retain_reference_box(element, &mut first, &mut rest);

    Some((first, rest))
}

fn split_table_node(
    element: &Table,
    avail_below_box_top: f32,
    rule: FragmentBreakRule,
) -> Option<(LayoutNode, LayoutNode)> {
    let principal = &element.principal;
    let headers = principal
        .children
        .iter()
        .filter(|child| table_row_pagination_state(child.as_ref()).is_header)
        .cloned()
        .collect::<Vec<_>>();
    let footers = principal
        .children
        .iter()
        .filter(|child| table_row_pagination_state(child.as_ref()).is_footer)
        .cloned()
        .collect::<Vec<_>>();
    let footer_extent = footers
        .iter()
        .map(|footer| estimate_element_height(footer.as_ref()))
        .sum::<f32>();
    let has_body_rows = principal.children.iter().any(|child| {
        let row = table_row_pagination_state(child.as_ref());
        row.is_row && !row.is_header && !row.is_footer
    });
    let available_before_footer = if has_body_rows && !footers.is_empty() {
        (avail_below_box_top - footer_extent).max(0.0)
    } else {
        avail_below_box_top
    };
    let (mut before, mut after) = split_container_node(principal, available_before_footer, rule)?;

    // A row-group with break-inside:avoid moves as a unit when the tentative
    // cut falls inside it. The table header remains on the preceding fragment;
    // this is a group keep-together break, not a normal mid-table continuation.
    let continuation_avoid_group = after.children.iter().find_map(|child| {
        let row = table_row_pagination_state(child.as_ref());
        row.is_row.then_some(row.avoid_group).flatten()
    });
    let mut kept_group_together = false;
    if let Some(group) = continuation_avoid_group {
        let trailing_group_start = before
            .children
            .iter()
            .rposition(|child| {
                let row = table_row_pagination_state(child.as_ref());
                !row.is_row || row.avoid_group != Some(group)
            })
            .map_or(0, |index| index + 1);
        if trailing_group_start < before.children.len() {
            let moved = before.children.split_off(trailing_group_start);
            if !moved.is_empty() {
                let mut continuation = moved;
                continuation.append(&mut after.children);
                after.children = continuation;
                kept_group_together = true;
            }
        }
    }

    let first_has_footer = before
        .children
        .iter()
        .any(|child| table_row_pagination_state(child.as_ref()).is_footer);
    if !first_has_footer {
        before.children.extend(footers.iter().cloned());
    }

    let continuation_has_header = after
        .children
        .iter()
        .any(|child| table_row_pagination_state(child.as_ref()).is_header);
    if !kept_group_together && !continuation_has_header && !headers.is_empty() {
        let first_row = after
            .children
            .iter()
            .position(|child| table_row_pagination_state(child.as_ref()).is_row)
            .unwrap_or(after.children.len());
        after.children.splice(first_row..first_row, headers);
    }

    Some((Table::new(before).boxed(), Table::new(after).boxed()))
}

/// Page-dependent state needed by fragmentation.
///
/// Geometry remains a cascade until a physical page exists. This prevents
/// `:first`, spread, `:blank`, and named selectors from taking separate paths
/// that can disagree about the same page.
#[derive(Debug, Clone)]
pub(crate) struct PaginationContext {
    pub(crate) geometry: super::page_context::PageGeometryContext,
    pub(crate) footnote_area: FootnoteAreaLayout,
    pub(crate) root_margin_top: f32,
}

impl PaginationContext {
    pub(crate) fn new(
        geometry: super::page_context::PageGeometryContext,
        footnote_area: FootnoteAreaLayout,
        root_margin_top: f32,
    ) -> Self {
        Self {
            geometry,
            footnote_area,
            root_margin_top,
        }
    }

    pub(crate) fn with_root_margin_top(mut self, root_margin_top: f32) -> Self {
        self.root_margin_top = root_margin_top;
        self
    }

    pub(crate) fn with_footnote_content_width(mut self, content_width: f32) -> Self {
        self.footnote_area.content_width = content_width;
        self
    }

    fn uniform(content_height: f32, root_margin_top: f32) -> Self {
        Self::new(
            super::page_context::PageGeometryContext::uniform(
                PageSize::new(content_height, content_height),
                Margin::uniform(0.0),
            ),
            FootnoteAreaLayout::default(),
            root_margin_top,
        )
    }
}

/// Named-page identity active at the current fragmentation position.
///
/// Page identity is independent from geometry: `page: Appendix` still makes
/// `@page Appendix:right` match even when no plain `@page Appendix` rule
/// supplies a margin or size override.
#[derive(Debug, Clone, Default)]
struct ActivePageType {
    name: Option<String>,
}

impl ActivePageType {
    fn select(name: Option<String>) -> Self {
        Self { name }
    }

    fn geometry(
        &self,
        cascade: &super::page_context::PageGeometryContext,
        page_number: usize,
        is_blank: bool,
    ) -> super::page_context::PageGeometry {
        cascade.resolve(crate::parser::css::PageSelectorContext {
            page_number,
            is_blank,
            page_name: self.name.as_deref(),
        })
    }
}

/// An element waiting for pagination, tagged when it is the tail of an
/// already-started internal fragment. A continuation keeps fragmenting against
/// the content box even after its remaining height falls below the physical
/// page height.
struct PendingElement {
    element: LayoutNode,
    is_fragment_continuation: bool,
    /// Adjacent class-A boxes that must stay together when they fit a fresh
    /// page. The group is still paginated element by element once admitted, so
    /// normal painting and flow stay unchanged.
    avoid_group: Option<AvoidBreakGroup>,
}

impl PendingElement {
    fn fresh(element: LayoutNode) -> Self {
        Self {
            element,
            is_fragment_continuation: false,
            avoid_group: None,
        }
    }

    fn continuation(element: LayoutNode) -> Self {
        Self {
            element,
            is_fragment_continuation: true,
            avoid_group: None,
        }
    }
}

#[derive(Default)]
struct RepeatedPageDecorations {
    elements: Vec<(f32, LayoutNode)>,
}

impl RepeatedPageDecorations {
    fn capture(&mut self, block_offset: f32, element: &LayoutNode) {
        self.elements.push((block_offset, element.clone()));
    }

    fn append_to(
        &self,
        page: &mut Vec<(f32, LayoutNode)>,
        geometry: super::page_context::PageGeometry,
    ) {
        page.extend(self.elements.iter().map(|(block_offset, element)| {
            let mut repeated = element.clone();
            if let Some(background) = repeated.page_area_background_mut() {
                background.fit_page_area(geometry.page_area_in_flow_space());
            }
            (*block_offset, repeated)
        }));
    }

    fn page_elements(&self, geometry: super::page_context::PageGeometry) -> Vec<(f32, LayoutNode)> {
        let mut elements = Vec::with_capacity(self.elements.len());
        self.append_to(&mut elements, geometry);
        elements
    }
}

/// The parts of an in-flow box that matter while judging an avoided break.
/// Keeping margins separate lets the group use the same collapse rule as the
/// normal pagination loop instead of double-counting adjacent vertical gaps.
#[derive(Debug, Clone, Copy)]
struct FlowBoxMetrics {
    content_height: f32,
    margins: BlockMargins,
}

impl FlowBoxMetrics {
    fn from_element(element: &dyn LayoutElement) -> Option<Self> {
        if element_is_out_of_flow(element) {
            return None;
        }
        let margins = *element.margin_holder()?.margins();
        Some(Self {
            content_height: (estimate_element_height(element) - margins.total()).max(0.0),
            margins,
        })
    }
}

fn collapse_vertical_margins(previous: f32, next: f32) -> f32 {
    if previous >= 0.0 && next >= 0.0 {
        previous.max(next)
    } else if previous < 0.0 && next < 0.0 {
        previous.min(next)
    } else {
        previous + next
    }
}

/// A run of in-flow boxes connected by `break-before/after: avoid`.
#[derive(Debug, Clone)]
struct AvoidBreakGroup {
    first: FlowBoxMetrics,
    following: Vec<FlowBoxMetrics>,
}

impl AvoidBreakGroup {
    fn fresh_page_height(&self) -> f32 {
        self.height_from_first(self.first.margins.start + self.first.content_height)
    }

    fn height_from_first(&self, first_height: f32) -> f32 {
        self.following
            .iter()
            .fold(
                (
                    first_height - self.first.margins.end,
                    self.first.margins.end,
                ),
                |(height, previous), next| {
                    (
                        height
                            + collapse_vertical_margins(previous, next.margins.start)
                            + next.content_height
                            + next.margins.end,
                        next.margins.end,
                    )
                },
            )
            .0
    }
}

fn prepare_pagination_work(elements: Vec<LayoutNode>) -> VecDeque<PendingElement> {
    let mut work = Vec::new();
    let mut flow_indexes = Vec::new();
    let mut joins_previous = Vec::new();
    let mut avoid_next_flow = false;
    let mut has_previous_flow = false;

    for element in elements {
        if is_avoid_page_break(element.as_ref()) {
            avoid_next_flow = true;
            continue;
        }
        if page_break_data(element.as_ref()).is_some() {
            work.push(PendingElement::fresh(element));
            avoid_next_flow = false;
            has_previous_flow = false;
            continue;
        }

        let flow_metrics = FlowBoxMetrics::from_element(&element);
        let index = work.len();
        work.push(PendingElement::fresh(element));
        if flow_metrics.is_some() {
            flow_indexes.push(index);
            joins_previous.push(avoid_next_flow && has_previous_flow);
            avoid_next_flow = false;
            has_previous_flow = true;
        }
    }

    let mut group_start = 0;
    while group_start < flow_indexes.len() {
        let mut group_end = group_start + 1;
        while group_end < flow_indexes.len() && joins_previous[group_end] {
            group_end += 1;
        }
        if group_end - group_start > 1 {
            let first_index = flow_indexes[group_start];
            let first = FlowBoxMetrics::from_element(&work[first_index].element);
            let following = flow_indexes[group_start + 1..group_end]
                .iter()
                .filter_map(|&index| FlowBoxMetrics::from_element(&work[index].element))
                .collect::<Vec<_>>();
            if let Some(first) = first.filter(|_| !following.is_empty()) {
                work[first_index].avoid_group = Some(AvoidBreakGroup { first, following });
            }
        }
        group_start = group_end;
    }
    work.into()
}

fn is_avoid_page_break(element: &dyn LayoutElement) -> bool {
    #[derive(Default)]
    struct AvoidVisitor(bool);

    impl LayoutVisitor for AvoidVisitor {
        fn visit_avoid_page_break(&mut self, _element: &crate::layout::elements::AvoidPageBreak) {
            self.0 = true;
        }
    }

    let mut visitor = AvoidVisitor::default();
    element.accept(&mut visitor);
    visitor.0
}

fn page_break_data(element: &dyn LayoutElement) -> Option<(PageBreakSide, Option<String>)> {
    #[derive(Default)]
    struct BreakVisitor(Option<(PageBreakSide, Option<String>)>);

    impl LayoutVisitor for BreakVisitor {
        fn visit_page_break(&mut self, element: &crate::layout::elements::PageBreak) {
            self.0 = Some((element.side, element.page_name.clone()));
        }
    }

    let mut visitor = BreakVisitor::default();
    element.accept(&mut visitor);
    visitor.0
}

#[derive(Default)]
struct PageStateMarker {
    running: Option<(String, LayoutNode)>,
    named: Option<(String, String)>,
}

impl LayoutVisitor for PageStateMarker {
    fn visit_running_element(&mut self, element: &RunningElement) {
        self.running = Some((element.name.clone(), element.element.clone()));
    }

    fn visit_named_string(&mut self, element: &NamedString) {
        self.named = Some((element.name.clone(), element.value.clone()));
    }
}

fn page_state_marker(element: &dyn LayoutElement) -> PageStateMarker {
    let mut marker = PageStateMarker::default();
    element.accept(&mut marker);
    marker
}

#[derive(Debug, Clone, Copy, Default)]
struct TableRowPaginationState {
    is_row: bool,
    is_header: bool,
    is_footer: bool,
    avoid_inside: bool,
    avoid_group: Option<TableFragmentGroup>,
}

impl LayoutVisitor for TableRowPaginationState {
    fn visit_table_row(&mut self, element: &TableRow) {
        self.is_row = true;
        self.is_header = element.fragmentation.repeats_as_header;
        self.is_footer = element.fragmentation.repeats_as_footer;
        self.avoid_inside = element.fragmentation.avoid_inside;
        self.avoid_group = element.fragmentation.avoid_group;
    }
}

fn table_row_pagination_state(element: &dyn LayoutElement) -> TableRowPaginationState {
    let mut state = TableRowPaginationState::default();
    element.accept(&mut state);
    state
}

#[derive(Debug, Clone, Copy)]
struct PaginationPosition {
    float: Float,
    clear: Clear,
    scheme: Position,
    insets: EdgeSizes,
    containing_block: Option<super::engine::ContainingBlock>,
    containing_block_depth: usize,
    flex_padding_box_border_top: Option<f32>,
}

impl Default for PaginationPosition {
    fn default() -> Self {
        Self {
            float: Float::None,
            clear: Clear::None,
            scheme: Position::Static,
            insets: EdgeSizes::ZERO,
            containing_block: None,
            containing_block_depth: 0,
            flex_padding_box_border_top: None,
        }
    }
}

fn pagination_position(element: &dyn LayoutElement) -> PaginationPosition {
    let mut result = PaginationPosition::default();
    if let Some(flow) = element.block_flow_owner().map(|owner| owner.block_flow()) {
        result.float = flow.float;
        result.clear = flow.clear;
    }
    if let Some(positioning) = element.positioning_owner().map(|owner| owner.positioning()) {
        result.scheme = positioning.scheme;
        result.insets = positioning.insets;
        result.containing_block = positioning.containing_block;
        result.containing_block_depth = positioning.containing_block_depth;
    }

    struct FlexPaddingBoxTop<'a>(&'a mut Option<f32>);

    impl LayoutVisitor for FlexPaddingBoxTop<'_> {
        fn visit_flex_row(&mut self, element: &FlexRow) {
            if element.positioning.containing_block_depth > 0 {
                *self.0 = Some(element.box_model.border.top.width);
            }
        }
    }

    element.accept(&mut FlexPaddingBoxTop(
        &mut result.flex_padding_box_border_top,
    ));
    result
}

fn repeats_on_each_page(element: &dyn LayoutElement) -> bool {
    element.page_content_role() == PageContentRole::RepeatedDecoration
}

fn is_empty_fixed_height_box(element: &dyn LayoutElement) -> bool {
    struct EmptyFixedBox(bool);

    impl LayoutVisitor for EmptyFixedBox {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = element.lines.is_empty() && element.box_model.size.height.is_definite();
        }

        fn visit_container(&mut self, element: &Container) {
            self.0 = element.children.is_empty() && element.box_model.size.height.is_definite();
        }
    }

    let mut result = EmptyFixedBox(false);
    element.accept(&mut result);
    result.0
}

fn is_fragmentable_fixed_text(element: &dyn LayoutElement) -> bool {
    struct FragmentableFixedText(bool);

    impl LayoutVisitor for FragmentableFixedText {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = !element.lines.is_empty()
                && element.paint.background.color.is_some()
                && element.box_model.size.height.is_definite();
        }
    }

    let mut result = FragmentableFixedText(false);
    element.accept(&mut result);
    result.0
}

#[derive(Debug, Clone, Copy, Default)]
struct ElementFlowGeometry {
    content_height: f32,
    margins: BlockMargins,
}

impl ElementFlowGeometry {
    fn set(&mut self, content_height: f32, margins: BlockMargins) {
        self.content_height = content_height;
        self.margins = margins;
    }
}

impl LayoutVisitor for ElementFlowGeometry {
    fn visit_horizontal_rule(&mut self, element: &HorizontalRule) {
        self.set(1.0, element.margins);
    }

    fn visit_table_row(&mut self, element: &TableRow) {
        let row_height = element.content.cells.row_block_extent();
        self.set(
            element.flow.content_extent(row_height),
            element.flow.margins,
        );
    }

    fn visit_grid_row(&mut self, element: &GridRow) {
        let row_height = element
            .content
            .cells
            .iter()
            .map(|cell| cell.layout.box_model.minimum_block_size)
            .fold(0.0, f32::max);
        self.set(row_height, element.box_model.margins);
    }

    fn visit_flex_row(&mut self, element: &FlexRow) {
        let content_height = element.box_model.padding.vertical()
            + element.content.row_height
            + element.box_model.border.vertical_width();
        self.set(content_height, element.box_model.margins);
    }

    fn visit_text_block(&mut self, element: &TextBlock) {
        let text_height = element.lines.iter().map(|line| line.height).sum::<f32>();
        let natural_content_height = element.box_model.padding.vertical() + text_height;
        let content_height = if element.clipping.rect.is_some() {
            element
                .box_model
                .size
                .height
                .resolve(natural_content_height)
        } else {
            element
                .box_model
                .size
                .height
                .used()
                .map_or(natural_content_height, |height| {
                    natural_content_height.max(height)
                })
        } + element.box_model.border.vertical_width();
        self.set(content_height, element.box_model.margins);
    }

    fn visit_image(&mut self, element: &Image) {
        self.set(
            element.geometry.size.height + element.geometry.flow.extra_end,
            element.geometry.flow.margins,
        );
    }

    fn visit_svg(&mut self, element: &Svg) {
        self.set(
            element.geometry.size.height + element.geometry.flow.extra_end,
            element.geometry.flow.margins,
        );
    }

    fn visit_progress_bar(&mut self, element: &ProgressBar) {
        self.set(element.size.height, element.margins);
    }

    fn visit_math_block(&mut self, element: &MathBlock) {
        self.set(element.layout.height(), element.margins);
    }

    fn visit_container(&mut self, element: &Container) {
        let children_height = element
            .children
            .iter()
            .map(|child| estimate_element_height(child.as_ref()))
            .sum::<f32>();
        let natural_content_height = element.box_model.padding.vertical()
            + children_height
            + element.box_model.border.vertical_width();
        let content_height = if element.overflow.combined.clips() {
            element
                .box_model
                .size
                .height
                .resolve(natural_content_height)
        } else {
            element
                .box_model
                .size
                .height
                .used()
                .map_or(natural_content_height, |height| {
                    natural_content_height.max(height)
                })
        };
        self.set(content_height, element.box_model.margins);
    }
}

fn element_flow_geometry(element: &dyn LayoutElement) -> ElementFlowGeometry {
    let mut geometry = ElementFlowGeometry::default();
    element.accept(&mut geometry);
    geometry
}

/// Paginate with one uniform physical-page geometry.
#[allow(dead_code)]
pub(crate) fn paginate(
    elements: Vec<LayoutNode>,
    content_height: f32,
    root_margin_top: f32,
) -> Vec<Page> {
    paginate_with_context(
        elements,
        PaginationContext::uniform(content_height, root_margin_top),
        &HashMap::new(),
    )
}

/// Paginate after resolving each physical page against the complete `@page`
/// selector cascade.
pub(crate) fn paginate_with_context(
    elements: Vec<LayoutNode>,
    context: PaginationContext,
    fonts: &dyn crate::font_registry::FontRegistry,
) -> Vec<Page> {
    let footnote_area = context.footnote_area;
    let root_margin_top = context.root_margin_top;
    let mut active_page_type = ActivePageType::default();
    let mut current_page_geometry = active_page_type.geometry(&context.geometry, 1, false);
    let mut content_height = current_page_geometry.content_height();
    let mut pages: Vec<Page> = Vec::new();
    let mut current_elements: Vec<(f32, LayoutNode)> = Vec::new();
    let mut current_generated_content = super::page_values::PageGeneratedContent::default();
    let mut pending_target_anchors: Vec<String> = Vec::new();
    let mut current_footnotes: Vec<FootnoteItem> = Vec::new();
    // Page 1 starts with body/html margin-top applied; continuation pages
    // start flush against the page margin (Chrome's print-model: body margin
    // opens the document, not every page).
    let mut y: f32 = root_margin_top;

    // Track active float regions for simplified float/clear behavior
    let mut left_floats: Vec<FloatRegion> = Vec::new();
    let mut right_floats: Vec<FloatRegion> = Vec::new();
    let mut prev_margin_bottom: f32 = 0.0;
    // CSS margin-collapse-through-root: the first in-flow block on a page has
    // its margin-top collapse with the body margin on page 1. On continuation
    // pages (after page break), the first block's margin-top applies as-is
    // because body is mid-flow and doesn't collapse with the viewport anymore.
    let mut first_on_page: bool = true;
    let mut on_first_page: bool = true;

    // Collect synthetic full-page background elements that should be repeated
    // across every page during pagination.
    let mut repeated_decorations = RepeatedPageDecorations::default();
    // Track the y-position of positioned ancestors by depth so absolute descendants
    // resolve against the nearest positioned ancestor rather than the most recent one.
    let mut positioned_y_by_depth: HashMap<usize, f32> = HashMap::new();

    // Track the header rows of the currently-active table so pagination can
    // re-emit them at the top of each page the table spans (Chrome parity).
    // Cleared as soon as a non-TableRow element is encountered.
    let mut pending_table_headers: Vec<LayoutNode> = Vec::new();
    // Track the `<tfoot>` rows of the active table so pagination can repeat them
    // as a running footer at the bottom of every page the table spans, directly
    // after the last body row (Chrome's LayoutNG table fragmentation). Collected
    // by a forward scan when the table is first entered, so their height is known
    // (and reserved) while body rows are placed — even though, after the
    // thead->tbody->tfoot reorder, the footer rows arrive LAST in the stream.
    let mut pending_table_footers: Vec<LayoutNode> = Vec::new();
    // Total reserved height of `pending_table_footers` (content height of the
    // footer rows, mirroring the repeated-header advance). Subtracted from the
    // available page height when deciding whether a body row fits.
    let mut pending_footer_height: f32 = 0.0;
    // Whether the cursor is currently inside a table's row run (between the first
    // row and the next non-row element), used to detect table entry for the
    // footer pre-scan.
    let mut in_table = false;
    #[allow(unused_assignments)]
    let mut in_table_body = false;
    let mut previous_table_avoid_group = None;

    // Content height of a table row = the tallest cell's content height (the same
    // measure used for the repeated-header advance), excluding row margins.
    let row_content_height = |element: &dyn LayoutElement| -> f32 {
        #[derive(Default)]
        struct RowHeight(f32);

        impl LayoutVisitor for RowHeight {
            fn visit_table_row(&mut self, element: &TableRow) {
                self.0 = element.content.cells.row_block_extent();
            }
        }

        let mut height = RowHeight::default();
        element.accept(&mut height);
        height.0
    };

    // Worklist of pending top-level elements. A box that is too tall for the
    // page is split (CSS Fragmentation 3 §3): its first fragment is placed and
    // the continuation is pushed back onto the FRONT so it resumes immediately
    // on the next page. Elements that already fit are processed exactly as
    // before (every existing `continue`/placement is unchanged), so the whole
    // single-page corpus is byte-for-byte identical.
    let mut work = prepare_pagination_work(elements);
    while let Some(PendingElement {
        mut element,
        is_fragment_continuation,
        avoid_group,
    }) = work.pop_front()
    {
        if is_avoid_page_break(element.as_ref()) {
            continue;
        }
        let marker = page_state_marker(element.as_ref());
        if let Some((name, running)) = marker.running {
            current_generated_content.capture_running(name, running, first_on_page);
            continue;
        }
        if let Some((name, value)) = marker.named {
            if target_anchor_id(&name).is_some() {
                pending_target_anchors.push(name);
            } else {
                current_generated_content.capture_named_string(name, value, first_on_page);
            }
            continue;
        }
        extract_page_state_markers(
            &mut element,
            &mut current_generated_content,
            first_on_page,
            &mut pending_target_anchors,
        );

        // When the FIRST row of a `break-inside: avoid` table cannot fit in the
        // space left on the current page but DOES fit on an empty one, the break
        // decision below uses the whole table's height (this value) instead of
        // just the first row's, so the entire row run is pushed to the next page
        // intact rather than split between rows (Chrome's table keep-together).
        let mut table_keep_break_height: Option<f32> = None;
        // Track <thead>/<tfoot> rows so we can repeat them across page breaks
        // that occur mid-table: the header at each page top, the footer at each
        // page bottom. Reset when leaving the table.
        let mut suppress_repeated_headers_after_break = false;
        let row_state = table_row_pagination_state(element.as_ref());
        if row_state.is_row {
            let table_avoid_group_starts_here = row_state.avoid_group.is_some()
                && row_state.avoid_group != previous_table_avoid_group;
            previous_table_avoid_group = row_state.avoid_group;
            if !in_table {
                // First row of a new table: scan ahead over the rest of this
                // table's contiguous row run to collect the `<tfoot>` rows
                // (which the thead->tbody->tfoot reorder places at the end of
                // the run) so their height is reserved while body rows are
                // placed and they can be repeated at each page bottom.
                in_table = true;
                pending_table_headers.clear();
                pending_table_footers.clear();
                pending_footer_height = 0.0;
                for w in work.iter() {
                    let queued_state = table_row_pagination_state(w.element.as_ref());
                    if !queued_state.is_row {
                        break;
                    }
                    if queued_state.is_footer {
                        pending_footer_height += row_content_height(&w.element);
                        pending_table_footers.push(w.element.clone());
                    }
                }
                // `break-inside: avoid` table keep-together (CSS Fragmentation
                // 3 §5.2 / legacy `page-break-inside: avoid`): sum the whole
                // table's row run (this first row plus every contiguous row
                // still queued). When the table is avoid-inside AND fits on a
                // full page, arm the whole-table break height so a table that
                // would straddle the boundary is moved WHOLE to the next page.
                // A table taller than a full page cannot be kept together, so
                // it falls back to the normal between-rows split.
                if row_state.avoid_inside {
                    let mut total = estimate_element_height(&element);
                    for w in work.iter() {
                        if !table_row_pagination_state(w.element.as_ref()).is_row {
                            break;
                        }
                        total += estimate_element_height(&w.element);
                    }
                    if total <= content_height {
                        table_keep_break_height = Some(total);
                    }
                }
            }
            if table_keep_break_height.is_none() && table_avoid_group_starts_here {
                let avoid_group = row_state.avoid_group;
                let mut total = estimate_element_height(&element);
                for w in work.iter() {
                    let queued_state = table_row_pagination_state(w.element.as_ref());
                    if !queued_state.is_row || queued_state.avoid_group != avoid_group {
                        break;
                    }
                    total += estimate_element_height(&w.element);
                }
                if total <= content_height {
                    table_keep_break_height = Some(total);
                }
            }
            suppress_repeated_headers_after_break =
                table_keep_break_height.is_some() && table_avoid_group_starts_here;
            // A header is collected for repetition; a footer is handled by the
            // running-footer placement (below / at page breaks); only ordinary
            // body rows count as "table body" for fit/break decisions.
            in_table_body = !row_state.is_header && !row_state.is_footer;
            if row_state.is_header {
                pending_table_headers.push(element.clone());
            }
        } else {
            pending_table_headers.clear();
            pending_table_footers.clear();
            pending_footer_height = 0.0;
            in_table = false;
            in_table_body = false;
            previous_table_avoid_group = None;
        }

        // A `<tfoot>` row reaching the normal flow is the FINAL-page footer (the
        // reorder put it after every body row): place it directly after the last
        // body row on the current page. Its height was reserved while the body
        // rows were placed, so it always fits — skip the generic fit/break path.
        if row_state.is_footer {
            let fh = row_content_height(&element);
            collect_footnotes_from_element(&element, &mut current_footnotes);
            current_elements.push((y, element));
            y += fh;
            prev_margin_bottom = 0.0;
            first_on_page = false;
            continue;
        }

        let position = pagination_position(element.as_ref());
        let elem_float = position.float;
        let elem_clear = position.clear;
        let elem_position = position.scheme;
        let elem_offset_top = position.insets.top;
        let elem_containing_block = position.containing_block;
        let elem_positioned_depth = position.containing_block_depth;

        // A flex container (emitted as a FlexRow) that establishes a containing
        // block for absolute children records its padding-box top under its
        // `positioned_depth`, so abs children emitted after it anchor correctly.
        // (`top: 0` of such a child is the padding-box edge.) The padding-box top
        // is the FlexRow's flowed border-box top plus its top border.
        let flex_cb_depth = position
            .flex_padding_box_border_top
            .map(|border_top| (position.containing_block_depth, border_top));

        // Handle clear: move y below active floats on the specified side
        match elem_clear {
            Clear::Left | Clear::Both => {
                for f in &left_floats {
                    if f.y_end > y {
                        y = f.y_end;
                    }
                }
                if elem_clear == Clear::Both {
                    for f in &right_floats {
                        if f.y_end > y {
                            y = f.y_end;
                        }
                    }
                }
            }
            Clear::Right => {
                for f in &right_floats {
                    if f.y_end > y {
                        y = f.y_end;
                    }
                }
            }
            Clear::None => {}
        }

        if let Some((mut side, mut name)) = page_break_data(element.as_ref()) {
            // CSS Fragmentation 3 break precedence is resolved at the
            // shared class-A break point. The layout flattener emits
            // `break-after` followed by `break-before`; coalesce adjacent
            // forced breaks here so the later-in-flow `break-before` value
            // wins instead of being ignored on the empty page just created.
            while let Some((next_side, next_name)) = work
                .front()
                .and_then(|pending| page_break_data(pending.element.as_ref()))
            {
                side = next_side;
                name = next_name;
                work.pop_front();
            }
            // A forced break before any real content on the current page is
            // ignored (CSS Fragmentation 3: a forced break at the very start
            // of the fragmentation flow produces no leading blank page).
            // Consecutive forced breaks likewise collapse to one. A page that
            // holds only repeated page-background elements counts as empty.
            let page_has_content = current_elements.iter().any(|(_, element)| {
                element
                    .page_content_role()
                    .interrupts_forced_break_sequence()
            });
            if !page_has_content {
                // A named box that opens the document (no preceding content)
                // still selects its page type: the leading break is suppressed,
                // but the current physical page adopts its full selector context.
                if name.is_some() {
                    active_page_type = ActivePageType::select(name);
                    current_page_geometry =
                        active_page_type.geometry(&context.geometry, pages.len() + 1, false);
                    content_height = current_page_geometry.content_height();
                }
                continue;
            }
            let consumed_height = y;
            extend_open_column_flex_decoration_to_break(&mut current_elements, content_height);
            pages.push(Page {
                elements: std::mem::take(&mut current_elements),
                print_content_scale: Default::default(),
                document_svg_defs: Default::default(),
                generated_content: current_generated_content.snapshot_and_advance(),
                footnotes: std::mem::take(&mut current_footnotes),
                geometry: Some(current_page_geometry),
                page_name: active_page_type.name.clone(),
                is_blank: false,
            });
            // CSS Paged Media 3 §3.4: a `page: <name>` break changes the page
            // type before resolving the next physical page.
            active_page_type = ActivePageType::select(name);
            current_page_geometry =
                active_page_type.geometry(&context.geometry, pages.len() + 1, false);
            content_height = current_page_geometry.content_height();
            // Sided break (`break-*: left|right|recto|verso`): force the
            // following content onto a page of the requested parity. Page 1
            // is a right/recto page (LTR), so odd 1-based pages are right and
            // even are left. When the natural next page is the wrong side,
            // insert ONE blank page (carrying any repeated background) so the
            // content lands correctly.
            if matches!(
                side,
                PageBreakSide::Left
                    | PageBreakSide::Right
                    | PageBreakSide::Recto
                    | PageBreakSide::Verso
            ) {
                let next_page_no = pages.len() + 1; // 1-based content page
                let wants_right = matches!(side, PageBreakSide::Right | PageBreakSide::Recto);
                let next_is_right = next_page_no % 2 == 1;
                if wants_right != next_is_right {
                    let blank_geometry =
                        active_page_type.geometry(&context.geometry, next_page_no, true);
                    pages.push(Page {
                        elements: repeated_decorations.page_elements(blank_geometry),
                        print_content_scale: Default::default(),
                        document_svg_defs: Default::default(),
                        generated_content: current_generated_content.snapshot_and_advance(),
                        footnotes: Vec::new(),
                        geometry: Some(blank_geometry),
                        page_name: active_page_type.name.clone(),
                        is_blank: true,
                    });
                    current_page_geometry =
                        active_page_type.geometry(&context.geometry, pages.len() + 1, false);
                    content_height = current_page_geometry.content_height();
                }
            }
            repeated_decorations.append_to(&mut current_elements, current_page_geometry);
            y = 0.0;
            prev_margin_bottom = 0.0;
            first_on_page = true;
            on_first_page = false;
            left_floats.clear();
            right_floats.clear();
            advance_positioned_ancestors_after_page_break(
                &mut positioned_y_by_depth,
                consumed_height,
            );
            continue;
        }

        let flow_geometry = element_flow_geometry(element.as_ref());
        let content_h_val = flow_geometry.content_height;
        let margin_top_val = flow_geometry.margins.start;
        let margin_bottom_val = flow_geometry.margins.end;

        // Collapse margins: adjacent vertical margins merge (larger wins for positive,
        // most negative for negative, sum for mixed).
        let collapsed_margin = collapse_vertical_margins(prev_margin_bottom, margin_top_val);
        // CSS margin collapse through the root applies ONLY on page 1 (where
        // body opens). On page 1, the first block's margin-top collapses with
        // body.margin.top: since paginate pre-seeded `y = root_margin_top`,
        // the *extra* to add is `(block_mt - root_mt).max(0)`. On continuation
        // pages (page 2+), body is already mid-flow — no collapse with root,
        // and no body margin-top at all.
        let collapsed_margin = if first_on_page && on_first_page {
            (collapsed_margin - root_margin_top).max(0.0)
        } else {
            collapsed_margin
        };
        let margin_top_val = collapsed_margin;
        let element_height = margin_top_val + content_h_val + margin_bottom_val;
        let mut pending_footnotes = Vec::new();
        collect_footnotes_from_element(&element, &mut pending_footnotes);
        let pending_non_block_footnotes: Vec<_> = pending_footnotes
            .iter()
            .filter(|footnote| footnote.formatting.policy != FootnotePolicy::Block)
            .cloned()
            .collect();
        let footnote_reserve = footnote_reserved_height(
            &[
                current_footnotes.as_slice(),
                pending_non_block_footnotes.as_slice(),
            ],
            footnote_area,
            fonts,
        );
        let content_height_before_pending_footnotes = (content_height
            - footnote_reserved_height(&[current_footnotes.as_slice()], footnote_area, fonts))
        .max(0.0);
        let effective_content_height = (content_height - footnote_reserve).max(0.0);
        let block_footnote_requires_break = y > 0.0
            && footnote_block_policy_requires_break(
                &pending_footnotes,
                &current_footnotes,
                y,
                element_height,
                content_height,
                footnote_area,
                fonts,
            );
        let physical_page_height = current_page_geometry.size.height;
        let empty_fixed_height_box = is_empty_fixed_height_box(element.as_ref());
        // An empty definite-height box can extend into page margins when its
        // border box still fits on the physical sheet. Once it exceeds that
        // sheet, or it is already a continuation, it must fragment against the
        // content box so the nonzero tail remains observable.
        let may_fragment_internally = !empty_fixed_height_box
            || is_fragment_continuation
            || exceeds_with_roundoff(content_h_val, physical_page_height);

        // Handle position: absolute -- place at fixed position, don't affect flow
        if elem_position.is_absolute() {
            let abs_y = if let Some(cb) = elem_containing_block {
                // Position relative to the containing block (nearest positioned ancestor).
                // bottom/right offsets are pre-resolved into top/left in build_pseudo_block.
                positioned_y_by_depth.get(&cb.depth).copied().unwrap_or(0.0) + elem_offset_top
            } else {
                // No containing block — position relative to page (legacy behavior).
                elem_offset_top
            };
            if elem_positioned_depth > 0 {
                positioned_y_by_depth.insert(elem_positioned_depth, abs_y);
            }
            if let Some(background) = element.page_area_background_mut() {
                background.fit_page_area(current_page_geometry.page_area_in_flow_space());
            }
            if repeats_on_each_page(element.as_ref()) {
                repeated_decorations.capture(abs_y, &element);
            }
            collect_footnotes_from_element(&element, &mut current_footnotes);
            current_elements.push((abs_y, element));
            continue;
        }

        // Reserve the repeated running-footer height while placing table body
        // rows so a body row is never laid where the footer will be re-emitted
        // at the page bottom (Chrome reserves the tfoot on every spanned page).
        let footer_reserve = if in_table_body {
            pending_footer_height
        } else {
            0.0
        };
        // For the first row of a `break-inside: avoid` table that fits a full
        // page, decide the break against the WHOLE table's height (footer
        // already included in that sum) so the entire table moves to the next
        // page intact; otherwise decide against this row plus the reserved
        // running-footer height as before.
        let avoid_break_height = avoid_group.as_ref().and_then(|group| {
            (!exceeds_with_roundoff(group.fresh_page_height(), effective_content_height))
                .then(|| group.height_from_first(element_height))
        });
        let (break_decision_height, break_footer_reserve) = table_keep_break_height
            .or(avoid_break_height)
            .map_or((element_height, footer_reserve), |height| (height, 0.0));
        // A class-A break before the box is useful only when the box (or the
        // keep-together group being judged) can fit in a fresh fragmentainer.
        // If it cannot, moving it merely wastes the current page before the
        // same internal split happens on the next one. CSS Fragmentation 3
        // requires the remaining current fragmentainer to be used in that
        // case; the internal splitter below then relaxes avoid constraints as
        // needed and preserves forward progress.
        let fresh_fit_height = if may_fragment_internally {
            effective_content_height
        } else {
            physical_page_height
        };
        let fits_fresh_fragmentainer = !exceeds_with_roundoff(
            break_decision_height + break_footer_reserve,
            fresh_fit_height,
        );
        let page_broke_mid_loop = (fits_fresh_fragmentainer
            && exceeds_with_roundoff(
                y + break_decision_height + break_footer_reserve,
                effective_content_height,
            )
            // A nonzero cursor does not prove that the fragmentainer already
            // contains flow content: page 1 starts after the projected root
            // margin/padding gutter. Breaking before its first principal box
            // would manufacture a blank leading page and retry the same box at
            // the top of page 2. Use the semantic flow state instead; the
            // internal splitter below can then consume page 1 when necessary.
            && !first_on_page)
            || block_footnote_requires_break;
        if page_broke_mid_loop {
            // CSS GCPM §2.8: when a `footnote-policy: line` body cannot fit
            // on this page, the break occurs at the start of the line with the
            // reference — not before the containing paragraph. Process the
            // two fragments through the normal work queue so page finalization,
            // footnote collection, and decoration slicing remain centralized.
            let line_policy_requires_break = avoid_group.is_none()
                && break_footer_reserve == 0.0
                && equal_with_roundoff(break_decision_height, element_height)
                && !exceeds_with_roundoff(
                    y + element_height,
                    content_height_before_pending_footnotes,
                );
            if line_policy_requires_break
                && let Some(index) = footnote_line_policy_break_index(
                    &element,
                    &current_footnotes,
                    y,
                    element_height,
                    content_height,
                    footnote_area,
                    fonts,
                )
                && let Some((first, rest)) = split_text_block_at_line(&element, index)
            {
                work.push_front(PendingElement::continuation(rest));
                work.push_front(PendingElement {
                    element: first,
                    is_fragment_continuation,
                    avoid_group: None,
                });
                continue;
            }
            let fragmentable_fixed_text = is_fragmentable_fixed_text(element.as_ref());
            if fragmentable_fixed_text
                && elem_position == Position::Static
                && elem_float == Float::None
                && !in_table_body
            {
                let avail_below_box_top = effective_content_height - (y + margin_top_val);
                if let Some((first, rest)) =
                    split_fixed_height_text_block(&element, avail_below_box_top)
                {
                    y += margin_top_val;
                    collect_footnotes_from_element(&first, &mut current_footnotes);
                    current_elements.push((y, first));
                    let consumed_height = content_height;
                    pages.push(Page {
                        elements: std::mem::take(&mut current_elements),
                        print_content_scale: Default::default(),
                        document_svg_defs: Default::default(),
                        generated_content: current_generated_content.snapshot_and_advance(),
                        footnotes: std::mem::take(&mut current_footnotes),
                        geometry: Some(current_page_geometry),
                        page_name: active_page_type.name.clone(),
                        is_blank: false,
                    });
                    current_page_geometry =
                        active_page_type.geometry(&context.geometry, pages.len() + 1, false);
                    content_height = current_page_geometry.content_height();
                    repeated_decorations.append_to(&mut current_elements, current_page_geometry);
                    y = 0.0;
                    prev_margin_bottom = 0.0;
                    first_on_page = true;
                    on_first_page = false;
                    left_floats.clear();
                    right_floats.clear();
                    advance_positioned_ancestors_after_page_break(
                        &mut positioned_y_by_depth,
                        consumed_height,
                    );
                    work.push_front(PendingElement::continuation(rest));
                    continue;
                }
            }
            if in_table_body && elem_position == Position::Static && elem_float == Float::None {
                let avail_below_box_top = effective_content_height - (y + margin_top_val);
                if let Some((first, rest)) =
                    split_table_row(&element, avail_below_box_top, FragmentBreakRule::Normal)
                {
                    y += margin_top_val;
                    collect_footnotes_from_element(&first, &mut current_footnotes);
                    current_elements.push((y, first));
                    let consumed_height = content_height;
                    pages.push(Page {
                        elements: std::mem::take(&mut current_elements),
                        print_content_scale: Default::default(),
                        document_svg_defs: Default::default(),
                        generated_content: current_generated_content.snapshot_and_advance(),
                        footnotes: std::mem::take(&mut current_footnotes),
                        geometry: Some(current_page_geometry),
                        page_name: active_page_type.name.clone(),
                        is_blank: false,
                    });
                    current_page_geometry =
                        active_page_type.geometry(&context.geometry, pages.len() + 1, false);
                    content_height = current_page_geometry.content_height();
                    repeated_decorations.append_to(&mut current_elements, current_page_geometry);
                    y = 0.0;
                    prev_margin_bottom = 0.0;
                    first_on_page = true;
                    on_first_page = false;
                    left_floats.clear();
                    right_floats.clear();
                    advance_positioned_ancestors_after_page_break(
                        &mut positioned_y_by_depth,
                        consumed_height,
                    );
                    work.push_front(PendingElement::continuation(rest));
                    continue;
                }
            }
            // Repeat the running footer at the bottom of the page being closed,
            // directly after the last body row (matching Chrome: the footer is
            // NOT flushed to the page edge — any reserved slack stays as
            // whitespace below it).
            if in_table_body && !pending_table_footers.is_empty() {
                for footer in pending_table_footers.clone() {
                    let footer_h = row_content_height(&footer);
                    collect_footnotes_from_element(&footer, &mut current_footnotes);
                    current_elements.push((y, footer));
                    y += footer_h;
                }
            }
            let consumed_height = y;
            pages.push(Page {
                elements: std::mem::take(&mut current_elements),
                print_content_scale: Default::default(),
                document_svg_defs: Default::default(),
                generated_content: current_generated_content.snapshot_and_advance(),
                footnotes: std::mem::take(&mut current_footnotes),
                geometry: Some(current_page_geometry),
                page_name: active_page_type.name.clone(),
                is_blank: false,
            });
            current_page_geometry =
                active_page_type.geometry(&context.geometry, pages.len() + 1, false);
            content_height = current_page_geometry.content_height();
            // Duplicate root background onto the new page.
            repeated_decorations.append_to(&mut current_elements, current_page_geometry);
            y = 0.0;
            on_first_page = false;
            // prev_margin_bottom and first_on_page are reset at the bottom of
            // this iteration (float or normal-flow branch overwrites both).
            left_floats.clear();
            right_floats.clear();
            advance_positioned_ancestors_after_page_break(
                &mut positioned_y_by_depth,
                consumed_height,
            );
            // Re-emit <thead> rows at the top of the new page if we're in the
            // middle of a table body (Chrome parity for long tables).
            if in_table_body
                && !suppress_repeated_headers_after_break
                && !pending_table_headers.is_empty()
            {
                for header in pending_table_headers.clone() {
                    let header_h = row_content_height(header.as_ref());
                    collect_footnotes_from_element(&header, &mut current_footnotes);
                    current_elements.push((y, header));
                    y += header_h;
                }
            }
            element.suppress_first_fragment_spacing();
        }

        // After a mid-loop page break, the current element is now the first
        // in-flow block on a continuation page. Its margin-top applies as-is
        // (no collapse with root — body is mid-flow across the page break).
        let effective_flow_geometry = element_flow_geometry(element.as_ref());
        let effective_margin_top = if page_broke_mid_loop {
            effective_flow_geometry.margins.start
        } else {
            margin_top_val
        };
        let effective_element_height = effective_margin_top
            + effective_flow_geometry.content_height
            + effective_flow_geometry.margins.end;

        // Handle floated elements (floats don't participate in margin collapsing)
        if elem_float != Float::None {
            y += effective_margin_top;
            let float_y_end = y + content_h_val;
            let region = FloatRegion {
                y_start: y,
                y_end: float_y_end,
                side: elem_float,
            };
            if elem_float == Float::Left {
                left_floats.push(region);
            } else {
                right_floats.push(region);
            }
            apply_pending_target_anchors(
                &mut pending_target_anchors,
                &mut current_generated_content,
                first_on_page,
            );
            collect_footnotes_from_element(&element, &mut current_footnotes);
            current_elements.push((y, element));
            prev_margin_bottom = 0.0;
            first_on_page = false;
            continue;
        }

        // CSS Fragmentation 3 §3: if this in-flow box STILL overflows the page
        // after the break-between handling above, it is genuinely taller than a
        // full fragmentainer and would otherwise be clipped (data loss). Split it
        // at an internal break point, place the first fragment to fill the rest
        // of this page, and resume the continuation at the top of the next one.
        //
        // The guard `y + element_height > content_height` is true ONLY for a box
        // taller than the remaining space that the break-between logic could not
        // resolve (i.e. taller than a full empty page, or a too-tall box already
        // at the page top). Every box that fits — the entire existing corpus —
        // skips this block and takes the unchanged whole-placement path below.
        let followed_by_forced_break = work
            .front()
            .is_some_and(|pending| page_break_data(pending.element.as_ref()).is_some());
        let avail_below_box_top = effective_content_height - (y + effective_margin_top);
        // A forced break retained inside this box occurs before any top-level
        // break marker that follows the box. The following marker must not make
        // the earlier descendant break inert; after splitting, the ordinary
        // adjacent-break coalescing logic resolves both in document order.
        if elem_position == Position::Static
            && let Some((first, rest, target)) =
                split_flow_at_descendant_break(&element, avail_below_box_top)
        {
            if let Some(rest) = rest {
                work.push_front(PendingElement::continuation(rest));
            }
            work.push_front(PendingElement::fresh(target.into_layout_element()));
            if let Some(first) = first {
                work.push_front(PendingElement {
                    element: first,
                    is_fragment_continuation,
                    avoid_group: None,
                });
            }
            continue;
        }
        if elem_position == Position::Static
            && !followed_by_forced_break
            && may_fragment_internally
            && exceeds_with_roundoff(y + effective_element_height, effective_content_height)
        {
            let avail_below_box_top = effective_content_height - (y + effective_margin_top);
            // A too-tall text block splits at a line boundary; a too-tall raster
            // image slices at the page edge (each page embeds only its slice); a
            // too-tall container splits between its children, re-enqueuing the
            // continuation so it resumes on the next page.
            let split = split_element(&element, avail_below_box_top, FragmentBreakRule::Normal)
                .or_else(|| {
                    split_element(&element, avail_below_box_top, FragmentBreakRule::Emergency)
                });
            if let Some((first, rest)) = split {
                // Place the first fragment at the (margin-adjusted) cursor; it
                // fills the remainder of this page.
                y += effective_margin_top;
                apply_pending_target_anchors(
                    &mut pending_target_anchors,
                    &mut current_generated_content,
                    first_on_page,
                );
                collect_footnotes_from_element(&first, &mut current_footnotes);
                current_elements.push((y, first));
                // Close the page (the fragmentainer is full) and reset flow state
                // for the continuation, mirroring a normal mid-loop page break.
                let consumed_height = content_height;
                pages.push(Page {
                    elements: std::mem::take(&mut current_elements),
                    print_content_scale: Default::default(),
                    document_svg_defs: Default::default(),
                    generated_content: current_generated_content.snapshot_and_advance(),
                    footnotes: std::mem::take(&mut current_footnotes),
                    geometry: Some(current_page_geometry),
                    page_name: active_page_type.name.clone(),
                    is_blank: false,
                });
                current_page_geometry =
                    active_page_type.geometry(&context.geometry, pages.len() + 1, false);
                content_height = current_page_geometry.content_height();
                repeated_decorations.append_to(&mut current_elements, current_page_geometry);
                y = 0.0;
                prev_margin_bottom = 0.0;
                first_on_page = true;
                on_first_page = false;
                left_floats.clear();
                right_floats.clear();
                advance_positioned_ancestors_after_page_break(
                    &mut positioned_y_by_depth,
                    consumed_height,
                );
                // Resume with the continuation on the next page.
                work.push_front(PendingElement::continuation(rest));
                continue;
            }
        }

        y += effective_margin_top;

        // Handle position: relative -- offset from normal position
        let effective_y = if elem_position.is_relative() {
            y + elem_offset_top
        } else {
            y
        };

        // Track positioned ancestor y for absolute children.
        if elem_positioned_depth > 0 && elem_position.is_positioned() {
            positioned_y_by_depth.insert(elem_positioned_depth, effective_y);
        }
        // A flex container records its PADDING-box top (border-box top + top
        // border) under its own depth so absolute children — whose `top`/resolved
        // `bottom` offsets are measured from the padding box — anchor correctly.
        if let Some((depth, border_top)) = flex_cb_depth {
            positioned_y_by_depth.insert(depth, effective_y + border_top);
        }

        apply_pending_target_anchors(
            &mut pending_target_anchors,
            &mut current_generated_content,
            first_on_page,
        );
        collect_footnotes_from_element(&element, &mut current_footnotes);
        current_elements.push((effective_y, element));
        y += content_h_val;
        prev_margin_bottom = margin_bottom_val;
        first_on_page = false;
    }

    // Finalize the pending page — but suppress a TRAILING BLANK page. A forced
    // break (`break-after: always` / `page-break-after: always`) on the last
    // in-flow box seeds a fresh page that ends up holding ONLY the duplicated
    // repeat-on-each-page backgrounds and no real content. Browsers drop such a
    // trailing empty page (Chrome emits one page for `…<div break-after:always>`,
    // not two), so only push the pending page if it carries real content — unless
    // it is the only page, so an otherwise-empty single-page document (e.g. an
    // empty body with a page background) still renders its one page.
    let has_real_content = current_elements
        .iter()
        .any(|(_, element)| element.page_content_role().retains_page());
    if !current_elements.is_empty() && (has_real_content || pages.is_empty()) {
        pages.push(Page {
            elements: current_elements,
            print_content_scale: Default::default(),
            document_svg_defs: Default::default(),
            generated_content: current_generated_content.clone(),
            footnotes: std::mem::take(&mut current_footnotes),
            geometry: Some(current_page_geometry),
            page_name: active_page_type.name.clone(),
            is_blank: false,
        });
    }

    if pages.is_empty() {
        let geometry = active_page_type.geometry(&context.geometry, 1, false);
        pages.push(Page {
            elements: Vec::new(),
            print_content_scale: Default::default(),
            document_svg_defs: Default::default(),
            generated_content: current_generated_content,
            footnotes: current_footnotes,
            geometry: Some(geometry),
            page_name: active_page_type.name,
            is_blank: false,
        });
    }

    pages
}

#[cfg(test)]
mod break_tests {
    use super::*;
    use crate::layout::cells::{
        CellBox, CellBoxModel, CellContent, CellFragmentation, GridCell, GridCellPlacement,
        TableCell,
    };
    use crate::layout::elements::{
        AvoidPageBreak, BoxFragmentation, BoxModel, FlexContent, FlexRow, GridContent, ImagePaint,
        ImageSampling, IntoLayoutNode, LayoutElementTestExt, LayoutElementTestMutExt, LayoutSize,
        LineFragmentation, PageBreak, ReplacedGeometry, SvgPaint,
    };

    fn narrow_footnote() -> FootnoteItem {
        FootnoteItem {
            marker: String::new(),
            text: "i i i i".to_string(),
            body: crate::layout::engine::FootnoteBodyStyle {
                font_size: 0.5,
                line_height_factor: 1.0,
                ..Default::default()
            },
            marker_color: crate::types::Color::BLACK,
            marker_prefix: String::new(),
            formatting: Default::default(),
        }
    }

    fn line_policy_call(marker: &str) -> TextRun {
        TextRun {
            text: marker.to_string(),
            font_size: 9.6,
            link_url: Some(crate::layout::engine::encode_footnote_link_data(
                &crate::layout::engine::FootnoteLinkData {
                    marker: marker.to_string(),
                    text: "note".to_string(),
                    marker_prefix: "{marker}. ".to_string(),
                    body: Default::default(),
                    marker_color: crate::types::Color::BLACK,
                    formatting: crate::style::computed::FootnoteFormatting {
                        policy: FootnotePolicy::Line,
                        ..Default::default()
                    },
                },
            )),
            ..Default::default()
        }
    }

    #[test]
    fn line_policy_breaks_at_the_call_that_overflows_the_footnote_area() {
        let fonts = HashMap::new();
        let first_line = crate::layout::engine::TextLine {
            runs: vec![line_policy_call("1")],
            ..Default::default()
        };
        let second_line = crate::layout::engine::TextLine {
            runs: vec![line_policy_call("2")],
            ..Default::default()
        };
        let mut element = TextBlock::empty_spacer().boxed();
        element
            .update_text(|text| text.lines = vec![first_line.clone(), second_line.clone()])
            .expect("empty spacer must be a text block");

        let area = FootnoteAreaLayout {
            content_width: 200.0,
            ..Default::default()
        };
        let mut one = Vec::new();
        let mut both = Vec::new();
        let mut seen = HashSet::new();
        collect_footnotes_from_runs(&first_line.runs, &mut seen, &mut one);
        collect_footnotes_from_runs(&second_line.runs, &mut seen, &mut both);
        both.splice(0..0, one.clone());
        let empty: &[FootnoteItem] = &[];
        let one_height = footnote_reserved_height(&[empty, one.as_slice()], area, &fonts);
        let both_height = footnote_reserved_height(&[empty, both.as_slice()], area, &fonts);
        let element_height = 50.0;
        let content_height = element_height + (one_height + both_height) / 2.0;

        assert_eq!(
            footnote_line_policy_break_index(
                &element,
                empty,
                0.0,
                element_height,
                content_height,
                area,
                &fonts,
            ),
            Some(1),
        );
    }

    #[test]
    fn block_policy_breaks_before_the_owning_block_only_when_its_body_overflows() {
        let fonts = HashMap::new();
        let area = FootnoteAreaLayout {
            content_width: 200.0,
            ..Default::default()
        };
        let element_y = 20.0;
        let element_height = 20.0;
        let mut block = narrow_footnote();
        block.formatting.policy = FootnotePolicy::Block;
        let body_height = footnote_reserved_height(&[std::slice::from_ref(&block)], area, &fonts);
        let content_height = element_y + element_height + body_height / 2.0;

        assert!(footnote_block_policy_requires_break(
            std::slice::from_ref(&block),
            &[],
            element_y,
            element_height,
            content_height,
            area,
            &fonts,
        ));

        block.formatting.policy = FootnotePolicy::Auto;
        assert!(!footnote_block_policy_requires_break(
            std::slice::from_ref(&block),
            &[],
            element_y,
            element_height,
            content_height,
            area,
            &fonts,
        ));
    }

    #[test]
    fn footnote_wrapping_preserves_half_point_and_thousandth_point_widths() {
        let fonts = HashMap::new();
        let footnote = narrow_footnote();
        let one_point = footnote_lines_height(std::slice::from_ref(&footnote), 1.0, &fonts);
        let half_point = footnote_lines_height(std::slice::from_ref(&footnote), 0.5, &fonts);
        let thousandth_point = footnote_lines_height(&[footnote], 0.001, &fonts);

        assert!(
            half_point > one_point,
            "0.5pt footnote width snapped to 1pt"
        );
        assert!(
            thousandth_point > half_point,
            "0.001pt footnote width snapped to the half-point or 1pt lane"
        );
    }

    #[test]
    fn asymmetric_footnote_padding_reduces_wrapping_width() {
        let fonts = HashMap::new();
        let footnote = narrow_footnote();
        let plain = FootnoteAreaLayout {
            content_width: 1.0,
            ..FootnoteAreaLayout::default()
        };
        let padded = FootnoteAreaLayout {
            style: ResolvedFootnoteAreaStyle {
                padding: EdgeSizes::new(0.0, 0.2, 0.0, 0.3),
                ..ResolvedFootnoteAreaStyle::default()
            },
            ..plain
        };

        assert!(
            footnote_content_height(std::slice::from_ref(&footnote), padded, &fonts)
                > footnote_content_height(std::slice::from_ref(&footnote), plain, &fonts),
            "left and right footnote padding must narrow the wrapping lane"
        );
    }

    #[test]
    fn footnote_area_reserves_vertical_geometry_once_for_all_groups() {
        let fonts = HashMap::new();
        let first = narrow_footnote();
        let second = narrow_footnote();
        let area = FootnoteAreaLayout {
            content_width: 1.0,
            style: ResolvedFootnoteAreaStyle {
                padding: EdgeSizes::new(2.0, 0.0, 3.0, 0.0),
                separator: FootnoteSeparator {
                    width: 4.0,
                    ..FootnoteSeparator::default()
                },
            },
            ..FootnoteAreaLayout::default()
        };
        let first_slice = std::slice::from_ref(&first);
        let second_slice = std::slice::from_ref(&second);
        let content_height = footnote_content_height(first_slice, area, &fonts)
            + footnote_content_height(second_slice, area, &fonts);
        let reserved = footnote_reserved_height(&[first_slice, second_slice], area, &fonts);

        assert!((reserved - (content_height + 2.0 + 3.0 + 4.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn footnote_max_height_constrains_content_box_not_padding_and_separator() {
        let fonts = HashMap::new();
        let footnote = narrow_footnote();
        let footnotes = std::slice::from_ref(&footnote);
        let mut area = FootnoteAreaLayout {
            content_width: 1.0,
            style: ResolvedFootnoteAreaStyle {
                padding: EdgeSizes::new(2.0, 0.0, 3.0, 0.0),
                separator: FootnoteSeparator {
                    width: 4.0,
                    ..FootnoteSeparator::default()
                },
            },
            ..FootnoteAreaLayout::default()
        };
        let content_height = footnote_content_height(footnotes, area, &fonts);
        area.max_height = Some(content_height);

        // `max-height` uses the initial `box-sizing: content-box`; padding and
        // the separator remain outside that limit but inside page reservation.
        assert_eq!(
            footnote_reserved_height(&[footnotes], area, &fonts),
            content_height + 9.0
        );
    }

    /// A fixed-height, in-flow content block (counts as "real content" for the
    /// leading-blank-page suppression).
    fn block(h: f32) -> LayoutNode {
        TextBlock {
            box_model: BoxModel {
                size: crate::layout::elements::LayoutSize {
                    height: BlockSize::definite(h),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
        .boxed()
    }

    fn avoiding_block(h: f32) -> LayoutNode {
        let mut element = block(h);
        element
            .update_text(|text| {
                text.fragmentation.box_fragmentation.inside =
                    crate::layout::elements::FragmentBreakAvoidance::Avoid;
            })
            .expect("test block must expose text fragmentation");
        element
    }

    fn text_block(h: f32) -> LayoutNode {
        let mut element = block(h);
        element.update_text(|text| {
            text.lines.push(crate::layout::engine::TextLine {
                height: 12.0,
                ..Default::default()
            });
        });
        element
    }

    fn flow_container(children: Vec<LayoutNode>, padding: crate::types::EdgeSizes) -> LayoutNode {
        Container {
            children,
            box_model: BoxModel {
                padding,
                ..Default::default()
            },
            ..Default::default()
        }
        .boxed()
    }

    fn nested_flow_containers(
        depth: usize,
        leaf_height: f32,
        padding: crate::types::EdgeSizes,
    ) -> LayoutNode {
        (0..depth).fold(block(leaf_height), |child, _| {
            flow_container(vec![child], padding)
        })
    }

    fn brk(side: PageBreakSide) -> LayoutNode {
        PageBreak {
            side,
            page_name: None,
        }
        .boxed()
    }

    fn grid_cell_with_nested(nested: LayoutNode, height: f32) -> GridCell {
        GridCell {
            layout: CellBox {
                content: CellContent {
                    children: vec![nested],
                    ..Default::default()
                },
                box_model: CellBoxModel {
                    minimum_block_size: height,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn grid_row(height: f32) -> LayoutNode {
        GridRow {
            content: GridContent {
                cells: vec![GridCell {
                    layout: CellBox {
                        box_model: CellBoxModel {
                            minimum_block_size: height,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                column_widths: vec![10.0],
                ..Default::default()
            },
            ..Default::default()
        }
        .boxed()
    }

    fn svg(height: f32) -> LayoutNode {
        Svg {
            tree: crate::parser::svg::SvgTree {
                width: 1.0,
                height,
                width_attr: None,
                height_attr: None,
                preserve_aspect_ratio: Default::default(),
                view_box: Some(crate::parser::svg::ViewBox {
                    min_x: 0.0,
                    min_y: 0.0,
                    width: 1.0,
                    height,
                }),
                defs: Default::default(),
                children: Vec::new(),
                text_ctx: Default::default(),
                source_markup: None,
            },
            geometry: ReplacedGeometry::new(
                Size::new(1.0, height),
                BlockMargins::default(),
                Default::default(),
            ),
            positioning: Default::default(),
            paint: SvgPaint::default(),
            replaced: Default::default(),
        }
        .boxed()
    }

    fn raster_image(height: f32) -> LayoutNode {
        Image {
            source: crate::layout::engine::RasterImageAsset::source(
                Vec::new(),
                1,
                4,
                crate::layout::engine::ImageFormat::Png,
                None,
            ),
            geometry: ReplacedGeometry::new(
                Size::new(1.0, height),
                BlockMargins::default(),
                Default::default(),
            ),
            positioning: Default::default(),
            sampling: ImageSampling {
                replaced: crate::layout::engine::ReplacedContent {
                    object_fit: ObjectFit::Fill,
                    ..Default::default()
                },
                ..Default::default()
            },
            paint: ImagePaint::default(),
        }
        .boxed()
    }

    #[test]
    fn height_estimation_preserves_the_former_depth_boundary() {
        let padding = crate::types::EdgeSizes::new(1.0, 0.0, 1.0, 0.0);

        for depth in [49, 50, 51] {
            let element = nested_flow_containers(depth, 7.0, padding);
            assert_eq!(
                estimate_element_height(&element),
                7.0 + depth as f32 * padding.vertical(),
                "all wrapper and leaf geometry must survive at depth {depth}",
            );
        }
    }

    #[test]
    fn height_estimation_uses_heap_work_stack_for_deep_nesting() {
        let depth = 2048;
        let element = nested_flow_containers(depth, 11.0, crate::types::EdgeSizes::ZERO);

        assert_eq!(estimate_element_height(&element), 11.0);
    }

    #[test]
    fn pagination_sees_content_across_the_former_depth_boundary() {
        for depth in [49, 50, 51] {
            let nested = nested_flow_containers(depth, 10.0, crate::types::EdgeSizes::ZERO);
            let pages = paginate(vec![block(1.0), nested], 10.0, 0.0);

            assert_eq!(pages.len(), 2, "depth {depth} must force the real break");
            assert_eq!(pages[0].elements.len(), 1);
            assert_eq!(pages[1].elements.len(), 1);
            assert_eq!(estimate_element_height(&pages[1].elements[0].1), 10.0);
        }
    }

    #[test]
    fn grid_fragment_rest_height_is_its_measured_content_not_a_scaled_guess() {
        let cell = grid_cell_with_nested(block(70.0), 70.0);
        let (_first, rest) = split_grid_cell(&cell, 20.0, 50.0, FragmentBreakRule::Normal);

        assert_eq!(rest.layout.content.children.len(), 1);
        assert_eq!(
            estimate_element_height(&rest.layout.content.children[0]),
            50.0
        );
        assert_eq!(rest.layout.box_model.minimum_block_size, 50.0);
    }

    #[test]
    fn grid_fragment_continuation_expands_to_its_trailing_nested_content() {
        let cell = GridCell {
            layout: CellBox {
                content: CellContent {
                    children: vec![block(70.0), block(15.0)],
                    ..Default::default()
                },
                box_model: CellBoxModel {
                    minimum_block_size: 85.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let (_first, rest) = split_grid_cell(&cell, 70.0, 5.0, FragmentBreakRule::Normal);

        assert_eq!(rest.layout.content.children.len(), 1);
        assert_eq!(
            estimate_element_height(&rest.layout.content.children[0]),
            15.0
        );
        assert_eq!(rest.layout.box_model.minimum_block_size, 15.0);
    }

    #[test]
    fn spanning_grid_fragment_keeps_the_resolved_track_remainder() {
        let mut cell = grid_cell_with_nested(block(70.0), 85.0);
        cell.layout.content.children.push(block(15.0));
        cell.placement = GridCellPlacement {
            row_span: 3,
            ..Default::default()
        };
        let (_first, rest) = split_grid_cell(&cell, 70.0, 5.0, FragmentBreakRule::Normal);

        assert_eq!(rest.layout.box_model.minimum_block_size, 5.0);
    }

    #[test]
    fn grid_cell_keeps_shared_row_geometry_after_its_content_is_consumed() {
        let cell = GridCell {
            layout: CellBox {
                content: CellContent {
                    lines: vec![TextLine {
                        height: 10.0,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                box_model: CellBoxModel {
                    minimum_block_size: 50.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let (_, rest) = split_grid_cell(&cell, 45.0, 5.0, FragmentBreakRule::Normal);

        assert!(rest.layout.content.lines.is_empty());
        assert_eq!(rest.layout.box_model.minimum_block_size, 5.0);
    }

    #[test]
    fn fixed_height_text_with_inline_content_keeps_a_subpoint_continuation() {
        let (first, rest) = split_fixed_height_text_block(&text_block(100.5), 100.0)
            .expect("both positive fragments must survive");

        let first_height = first
            .inspect_text(|text| text.box_model.size.height.used())
            .flatten()
            .expect("expected a text fragment");
        let rest_height = rest
            .inspect_text(|text| text.box_model.size.height.used())
            .flatten()
            .expect("expected a text continuation");
        assert_eq!(first_height, 100.0);
        assert_eq!(rest_height, 0.5);
    }

    #[test]
    fn fixed_height_container_clone_keeps_complete_fragment_decoration() {
        let mut element = Container {
            box_model: BoxModel {
                size: LayoutSize::fixed_inline(80.0, BlockSize::definite(120.0)),
                margins: BlockMargins::new(3.0, 5.0),
                padding: EdgeSizes::new(7.0, 11.0, 13.0, 17.0),
                ..Default::default()
            },
            fragmentation: BoxFragmentation {
                decoration: BoxDecorationBreak::Clone,
                ..Default::default()
            },
            ..Default::default()
        };
        element.box_model.border.top.width = 2.0;
        element.box_model.border.bottom.width = 4.0;

        let (first, rest) =
            split_empty_sized_container_node(&element, 70.0).expect("the definite box must split");
        for fragment in [&first, &rest] {
            let decoration = fragment
                .inspect_container(|container| {
                    (
                        container.box_model.margins,
                        container.box_model.padding,
                        container.box_model.border.top.width,
                        container.box_model.border.bottom.width,
                        container.fragmentation.reference_slice,
                    )
                })
                .expect("expected a container fragment");
            assert_eq!(decoration.0, element.box_model.margins);
            assert_eq!(decoration.1, element.box_model.padding);
            assert_eq!(decoration.2, 2.0);
            assert_eq!(decoration.3, 4.0);
            assert_eq!(decoration.4, None);
        }
    }

    #[test]
    fn empty_min_height_container_fragments_its_single_composite_floor() {
        let container = Container {
            box_model: BoxModel {
                size: LayoutSize {
                    height: BlockSize::minimum(220.0),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
        .boxed();

        let pages = paginate(vec![container], 100.0, 0.0);

        assert_eq!(pages.len(), 3);
        assert_eq!(
            pages
                .iter()
                .map(|page| estimate_element_height(page.elements[0].1.as_ref()))
                .collect::<Vec<_>>(),
            [100.0, 100.0, 20.0]
        );
    }

    #[test]
    fn normal_container_fragmentation_moves_an_avoided_child_intact() {
        let element = Container {
            children: vec![block(48.0), block(48.0), block(48.0), avoiding_block(48.0)],
            box_model: BoxModel {
                padding: EdgeSizes::new(18.0, 0.0, 0.0, 0.0),
                ..Default::default()
            },
            ..Default::default()
        };

        let (first, rest) = split_container_node(&element, 180.0, FragmentBreakRule::Normal)
            .expect("the final child must continue on the next fragmentainer");

        assert_eq!(first.children.len(), 3);
        assert_eq!(rest.children.len(), 1);
        assert_eq!(estimate_element_height(rest.children[0].as_ref()), 48.0);
    }

    #[test]
    fn emergency_fragmentation_can_split_an_over_tall_avoided_box() {
        let element = avoiding_block(120.0);

        assert!(split_element(element.as_ref(), 100.0, FragmentBreakRule::Normal).is_none());
        assert!(split_element(element.as_ref(), 100.0, FragmentBreakRule::Emergency).is_some());
    }

    #[test]
    fn empty_fixed_height_blocks_can_extend_into_page_margins() {
        let pages = paginate_with_context(
            vec![block(100.0), block(100.0)],
            PaginationContext::new(
                super::super::page_context::PageGeometryContext::uniform(
                    PageSize::new(100.0, 100.0),
                    Margin::new(1.0, 0.0, 1.0, 0.0),
                ),
                FootnoteAreaLayout::default(),
                0.0,
            ),
            &HashMap::new(),
        );

        assert_eq!(pages.len(), 2);
        assert!(pages.iter().all(|page| page.elements.len() == 1));
    }

    #[test]
    fn empty_fixed_height_continuations_keep_fragmenting_to_the_content_box() {
        let pages = paginate_with_context(
            vec![block(220.0)],
            PaginationContext::new(
                super::super::page_context::PageGeometryContext::uniform(
                    PageSize::new(104.0, 104.0),
                    Margin::new(20.0, 0.0, 20.0, 0.0),
                ),
                FootnoteAreaLayout::default(),
                0.0,
            ),
            &HashMap::new(),
        );

        assert_eq!(pages.len(), 4);
        assert_eq!(estimate_element_height(&pages[3].elements[0].1), 28.0);
    }

    #[test]
    fn raster_image_keeps_a_subpoint_continuation() {
        let (first, rest) =
            split_image_block(&raster_image(100.5), 100.0).expect("positive slices must survive");

        let (first_height, first_crop) = first
            .inspect_image(|image| (image.geometry.size.height, image.sampling.replaced.fragment))
            .and_then(|(height, fragment)| fragment.map(|fragment| (height, fragment)))
            .expect("expected an image fragment");
        let (rest_height, rest_crop) = rest
            .inspect_image(|image| (image.geometry.size.height, image.sampling.replaced.fragment))
            .and_then(|(height, fragment)| fragment.map(|fragment| (height, fragment)))
            .expect("expected an image continuation");
        assert_eq!(first_height, 100.0);
        assert_eq!(rest_height, 0.5);
        assert_eq!(first_crop.source_content_size.height, 100.5);
        assert_eq!(first_crop.content_offset.y, 0.0);
        assert_eq!(rest_crop.source_content_size.height, 100.5);
        assert_eq!(rest_crop.content_offset.y, 100.0);
    }

    #[test]
    fn subpoint_svg_splits_into_two_valid_fragments() {
        let (first, rest) =
            split_svg_block(&svg(0.75), 0.375).expect("positive SVG slices must survive");

        let (first_height, first_source_height, first_offset) = first
            .inspect_svg(|svg| {
                (
                    svg.geometry.size.height,
                    svg.tree.view_box.map(|view_box| view_box.height),
                    svg.replaced
                        .fragment
                        .map(|fragment| fragment.content_offset.y),
                )
            })
            .expect("expected an SVG fragment");
        let (rest_height, rest_source_height, rest_offset) = rest
            .inspect_svg(|svg| {
                (
                    svg.geometry.size.height,
                    svg.tree.view_box.map(|view_box| view_box.height),
                    svg.replaced
                        .fragment
                        .map(|fragment| fragment.content_offset.y),
                )
            })
            .expect("expected an SVG continuation");
        assert_eq!(first_height, 0.375);
        assert_eq!(rest_height, 0.375);
        assert_eq!(first_source_height, Some(0.75));
        assert_eq!(rest_source_height, Some(0.75));
        assert_eq!(first_offset, Some(0.0));
        assert_eq!(rest_offset, Some(0.375));
    }

    #[test]
    fn subpoint_space_is_offered_to_a_grid_row_splitter() {
        let container = flow_container(
            vec![block(10.0), grid_row(2.0)],
            crate::types::EdgeSizes::ZERO,
        );
        let (first, rest) = split_container(&container, 10.5, FragmentBreakRule::Normal)
            .expect("the grid row has 0.5pt to fragment into");

        let first_grid_height = first
            .inspect_container(|container| {
                container.children.get(1).and_then(|child| {
                    child.inspect_grid(|grid| {
                        grid.content.cells[0].layout.box_model.minimum_block_size
                    })
                })
            })
            .flatten()
            .expect("the grid fragment must remain on the first page");
        assert_eq!(first_grid_height, 0.5);

        let rest_grid_height = rest
            .inspect_container(|container| {
                container.children.first().and_then(|child| {
                    child.inspect_grid(|grid| {
                        grid.content.cells[0].layout.box_model.minimum_block_size
                    })
                })
            })
            .flatten()
            .expect("the grid continuation must move to the next page");
        assert_eq!(rest_grid_height, 1.5);
    }

    #[test]
    fn partially_fitting_nested_flow_uses_the_shared_recursive_splitter() {
        let nested = flow_container(
            vec![block(30.0), block(30.0)],
            crate::types::EdgeSizes::ZERO,
        );
        let outer = flow_container(vec![block(40.0), nested], crate::types::EdgeSizes::ZERO);

        let (first, rest) = split_container(&outer, 60.0, FragmentBreakRule::Normal)
            .expect("the nested flow has 20pt available in the first fragment");
        let nested_fragments = |fragment: &LayoutNode| {
            fragment
                .inspect_container(|outer| {
                    outer
                        .children
                        .iter()
                        .filter_map(|child| {
                            child.inspect_container(|inner| {
                                inner
                                    .children
                                    .iter()
                                    .map(|child| estimate_element_height(child.as_ref()))
                                    .collect::<Vec<_>>()
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .expect("expected the outer flow fragment")
        };

        assert_eq!(nested_fragments(&first), [vec![20.0]]);
        assert_eq!(nested_fragments(&rest), [vec![10.0, 30.0]]);
    }

    #[test]
    fn fragmented_container_preserves_its_composite_minimum_height() {
        let container = Container {
            children: (0..6).map(|_| block(100.0)).collect(),
            box_model: BoxModel {
                size: LayoutSize {
                    height: BlockSize::minimum(680.0),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
        .boxed();

        let pages = paginate(vec![container], 312.0, 0.0);

        assert_eq!(pages.len(), 3);
        assert_eq!(
            pages
                .iter()
                .map(|page| estimate_element_height(page.elements[0].1.as_ref()))
                .collect::<Vec<_>>(),
            [312.0, 312.0, 56.0]
        );
    }

    #[test]
    fn grid_cell_fragment_reserves_both_principal_box_edges_before_line_break() {
        let mut cell = GridCell::default();
        cell.layout.box_model.border_insets = EdgeSizes::new(2.0, 0.0, 2.0, 0.0);
        cell.layout.box_model.content_insets = EdgeSizes::new(5.0, 0.0, 5.0, 0.0);
        cell.layout.box_model.border.top.width = 2.0;
        cell.layout.box_model.border.bottom.width = 2.0;
        cell.layout.fragmentation = CellFragmentation {
            lines: LineFragmentation::new(1, 1),
        };
        cell.layout.content.lines = (0..3)
            .map(|_| TextLine {
                height: 10.0,
                ..Default::default()
            })
            .collect();

        let (first, rest) = split_grid_cell(&cell, 35.0, 5.0, FragmentBreakRule::Normal);

        assert_eq!(first.layout.content.lines.len(), 2);
        assert_eq!(rest.layout.content.lines.len(), 1);
        assert_eq!(first.layout.box_model.border_insets.bottom, 0.0);
        assert_eq!(rest.layout.box_model.border_insets.top, 0.0);
        assert_eq!(first.layout.box_model.padding().bottom, 0.0);
        assert_eq!(rest.layout.box_model.padding().top, 0.0);
    }

    #[test]
    fn grid_row_moves_intact_when_only_one_orphan_would_fit() {
        let lines = (0..4)
            .map(|_| TextLine {
                height: 10.0,
                ..Default::default()
            })
            .collect();
        let row = GridRow {
            content: GridContent {
                cells: vec![GridCell {
                    layout: CellBox {
                        content: CellContent {
                            lines,
                            ..Default::default()
                        },
                        box_model: CellBoxModel {
                            minimum_block_size: 40.0,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(
            split_grid_row_node(&row, 10.0, FragmentBreakRule::Normal).is_none(),
            "the default orphans:2 must move a one-line fragment"
        );
        let (first, rest) = split_grid_row_node(&row, 20.0, FragmentBreakRule::Normal)
            .expect("two orphans and two widows form a legal break");
        assert_eq!(
            first.inspect_grid(|row| row.content.cells[0].layout.content.lines.len()),
            Some(2)
        );
        assert_eq!(
            rest.inspect_grid(|row| row.content.cells[0].layout.content.lines.len()),
            Some(2)
        );

        let (first, rest) = split_grid_row_node(&row, 10.0, FragmentBreakRule::Emergency)
            .expect("the emergency rule must fragment an over-tall grid flow");
        assert_eq!(
            first.inspect_grid(|row| row.content.cells[0].layout.content.lines.len()),
            Some(1)
        );
        assert_eq!(
            rest.inspect_grid(|row| row.content.cells[0].layout.content.lines.len()),
            Some(3)
        );
    }

    #[test]
    fn grid_row_fragments_parallel_cells_at_their_independent_legal_breaks() {
        let cell = |line_count| GridCell {
            layout: CellBox {
                content: CellContent {
                    lines: (0..line_count)
                        .map(|_| TextLine {
                            height: 10.0,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
                box_model: CellBoxModel {
                    minimum_block_size: 30.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let row = GridRow {
            content: GridContent {
                cells: vec![cell(3), cell(1)],
                ..Default::default()
            },
            ..Default::default()
        };

        let (first, rest) = split_grid_row_node(&row, 10.0, FragmentBreakRule::Normal)
            .expect("the one-line cell advances while the three-line cell observes orphans");
        let line_counts = |fragment: &LayoutNode| {
            fragment
                .inspect_grid(|row| {
                    row.content
                        .cells
                        .iter()
                        .map(|cell| cell.layout.content.lines.len())
                        .collect::<Vec<_>>()
                })
                .expect("expected a grid-row fragment")
        };
        assert_eq!(line_counts(&first), [0, 1]);
        assert_eq!(line_counts(&rest), [3, 0]);
        assert!(
            split_grid_row_node(&row, 10.0, FragmentBreakRule::Emergency).is_some(),
            "an over-tall row must still make emergency progress in every cell"
        );
    }

    #[test]
    fn grid_row_does_not_emit_a_decoration_only_fragment_when_no_cell_can_break() {
        let cell = || GridCell {
            layout: CellBox {
                content: CellContent {
                    lines: (0..2)
                        .map(|_| TextLine {
                            height: 10.0,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
                box_model: CellBoxModel {
                    minimum_block_size: 20.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let row = GridRow {
            content: GridContent {
                cells: vec![cell(), cell()],
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(
            split_grid_row_node(&row, 10.0, FragmentBreakRule::Normal).is_none(),
            "two-line cells must move together when only one line could fit"
        );
    }

    #[test]
    fn incoming_single_line_grid_row_moves_instead_of_clipping_the_line() {
        let row = GridRow {
            content: GridContent {
                cells: vec![GridCell {
                    layout: CellBox {
                        content: CellContent {
                            lines: vec![TextLine {
                                height: 20.0,
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                        box_model: CellBoxModel {
                            minimum_block_size: 30.0,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(split_grid_row_node(&row, 10.0, FragmentBreakRule::Normal).is_none());
        assert!(split_grid_row_node(&row, 10.0, FragmentBreakRule::Emergency).is_some());
    }

    #[test]
    fn table_row_moves_intact_when_only_one_orphan_would_fit() {
        let lines = (0..4)
            .map(|_| TextLine {
                height: 10.0,
                ..Default::default()
            })
            .collect();
        let row = TableRow {
            content: crate::layout::elements::TableCells {
                cells: vec![TableCell {
                    layout: CellBox {
                        content: CellContent {
                            lines,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(
            split_table_row_node(&row, 10.0, FragmentBreakRule::Normal).is_none(),
            "the default orphans:2 must move a one-line table fragment"
        );
        let (first, rest) = split_table_row_node(&row, 20.0, FragmentBreakRule::Normal)
            .expect("two table-cell orphans and widows form a legal break");
        assert_eq!(
            first.inspect_table(|row| row.content.cells[0].layout.content.lines.len()),
            Some(2)
        );
        assert_eq!(
            rest.inspect_table(|row| row.content.cells[0].layout.content.lines.len()),
            Some(2)
        );

        let (first, rest) = split_table_row_node(&row, 10.0, FragmentBreakRule::Emergency)
            .expect("the emergency rule must fragment an over-tall table-cell flow");
        assert_eq!(
            first.inspect_table(|row| row.content.cells[0].layout.content.lines.len()),
            Some(1)
        );
        assert_eq!(
            rest.inspect_table(|row| row.content.cells[0].layout.content.lines.len()),
            Some(3)
        );
    }

    #[test]
    fn avoided_table_row_moves_before_repeated_header_instead_of_being_sliced() {
        let row = |height: f32, repeats_as_header: bool| {
            TableRow {
                content: crate::layout::elements::TableCells {
                    cells: vec![TableCell {
                        layout: CellBox {
                            box_model: CellBoxModel {
                                minimum_block_size: height,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                fragmentation: crate::layout::elements::TableFragmentation {
                    repeats_as_header,
                    avoid_inside: true,
                    ..Default::default()
                },
                ..Default::default()
            }
            .boxed()
        };
        let pages = paginate(
            vec![
                row(30.0, true),
                row(30.0, false),
                row(44.0, false),
                row(44.0, false),
                row(44.0, false),
            ],
            160.0,
            0.0,
        );
        let row_heights = |page: &Page| {
            page.elements
                .iter()
                .filter_map(|(_, element)| {
                    element.inspect_table(|row| row.content.cells[0].row_block_extent())
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(pages.len(), 2);
        assert_eq!(row_heights(&pages[0]), [30.0, 30.0, 44.0, 44.0]);
        assert_eq!(row_heights(&pages[1]), [30.0, 44.0]);
    }

    #[test]
    fn table_cell_nested_flow_fragments_without_losing_descendants() {
        let row = TableRow {
            content: crate::layout::elements::TableCells {
                cells: vec![TableCell {
                    layout: CellBox {
                        content: CellContent {
                            children: vec![block(220.0)],
                            ..Default::default()
                        },
                        box_model: CellBoxModel {
                            minimum_block_size: 220.0,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        let (first, rest) = split_table_row_node(&row, 100.0, FragmentBreakRule::Normal)
            .expect("nested table-cell flow must fragment through the shared cell path");
        let fragment_heights = |fragment: &LayoutNode| {
            fragment
                .inspect_table(|row| {
                    row.content.cells[0]
                        .layout
                        .content
                        .children
                        .iter()
                        .map(|child| estimate_element_height(child.as_ref()))
                        .collect::<Vec<_>>()
                })
                .expect("expected a table row fragment")
        };
        assert_eq!(fragment_heights(&first), [100.0]);
        assert_eq!(fragment_heights(&rest), [120.0]);
    }

    #[test]
    fn text_line_constraints_relax_only_under_the_emergency_rule() {
        let block = TextBlock::plain(
            (0..4)
                .map(|_| TextLine {
                    height: 10.0,
                    ..Default::default()
                })
                .collect(),
        );

        assert!(
            split_text_block_node(&block, 10.0, FragmentBreakRule::Normal).is_none(),
            "a normal break must not leave one orphan"
        );
        let (first, rest) = split_text_block_node(&block, 10.0, FragmentBreakRule::Emergency)
            .expect("an over-tall text flow must still make progress");
        assert_eq!(first.inspect_text(|text| text.lines.len()), Some(1));
        assert_eq!(rest.inspect_text(|text| text.lines.len()), Some(3));
    }

    #[test]
    fn ordinary_sibling_is_not_backtracked_without_break_avoidance() {
        let pages = paginate(vec![block(20.0), block(10.0), block(80.0)], 100.0, 0.0);

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 2);
        assert_eq!(pages[1].elements.len(), 1);
    }

    #[test]
    fn root_gutter_does_not_create_a_blank_page_before_first_fragmentable_box() {
        let root = flow_container(vec![block(45.0), block(45.0)], EdgeSizes::ZERO);
        let pages = paginate(vec![root], 100.0, 20.0);

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 1);
        assert_eq!(pages[0].elements[0].0, 20.0);
        assert_eq!(pages[1].elements.len(), 1);
        assert_eq!(pages[1].elements[0].0, 0.0);
    }

    #[test]
    fn avoid_page_break_keeps_adjacent_fitting_boxes_together() {
        let pages = paginate(
            vec![
                block(55.0),
                block(20.0),
                AvoidPageBreak.boxed(),
                block(30.0),
            ],
            100.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 1);
        assert_eq!(pages[1].elements.len(), 2);
        assert_eq!(pages[1].elements[0].0, 0.0);
        assert_eq!(pages[1].elements[1].0, 20.0);
    }

    #[test]
    fn avoid_page_break_crosses_anchor_metadata() {
        let pages = paginate(
            vec![
                block(25.0),
                block(50.0),
                NamedString {
                    name: "target-anchor".into(),
                    value: String::new(),
                }
                .boxed(),
                AvoidPageBreak.boxed(),
                block(30.0),
            ],
            100.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 1);
        assert_eq!(pages[1].elements.len(), 2);
    }

    #[test]
    fn avoid_page_break_uses_collapsed_group_margins() {
        let mut first = block(20.0);
        let mut second = block(20.0);
        first.update_text(|text| text.box_model.margins.end = 50.0);
        second.update_text(|text| text.box_model.margins.start = 50.0);

        let pages = paginate(
            vec![block(20.0), first, AvoidPageBreak.boxed(), second],
            100.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 1);
        assert_eq!(pages[1].elements.len(), 2);
    }

    #[test]
    fn avoid_page_break_yields_when_the_adjacent_group_cannot_fit_a_page() {
        let pages = paginate(
            vec![
                block(40.0),
                block(60.0),
                AvoidPageBreak.boxed(),
                block(50.0),
            ],
            100.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 2);
        assert_eq!(pages[1].elements.len(), 1);
    }

    #[test]
    fn forced_break_overrides_adjacent_avoidance() {
        let pages = paginate(
            vec![
                block(20.0),
                AvoidPageBreak.boxed(),
                brk(PageBreakSide::Any),
                block(20.0),
            ],
            100.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 1);
        assert_eq!(pages[1].elements.len(), 1);
    }

    #[test]
    fn visible_subpoint_overflow_is_not_hidden_by_a_page_break_epsilon() {
        let pages = paginate(vec![block(50.0), block(50.01)], 100.0, 0.0);

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 1);
        assert_eq!(pages[1].elements.len(), 1);
    }

    #[test]
    fn forced_break_page_paginates() {
        // Two blocks split by a plain forced break => two pages, one block each.
        let pages = paginate(
            vec![block(100.0), brk(PageBreakSide::Any), block(100.0)],
            1000.0,
            0.0,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].elements.len(), 1);
        assert_eq!(pages[1].elements.len(), 1);
    }

    fn overflow_continuation() -> LayoutNode {
        FlexRow {
            content: FlexContent {
                cells: vec![crate::layout::engine::FlexCell {
                    nested_elements: vec![block(10.0)],
                    ..Default::default()
                }],
                fragment_role:
                    crate::layout::engine::FlexFragmentRole::ParallelOverflowContinuation,
                ..Default::default()
            },
            ..Default::default()
        }
        .boxed()
    }

    #[test]
    fn painted_overflow_continuation_retains_the_final_page() {
        let pages = paginate(
            vec![
                block(10.0),
                brk(PageBreakSide::Any),
                overflow_continuation(),
            ],
            100.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].elements.len(), 1);
    }

    #[test]
    fn overflow_continuation_does_not_interrupt_forced_break_sequence() {
        let pages = paginate(
            vec![
                block(10.0),
                brk(PageBreakSide::Any),
                overflow_continuation(),
                brk(PageBreakSide::Any),
                block(10.0),
            ],
            100.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].elements.len(), 2);
    }

    #[test]
    fn leading_forced_break_emits_no_blank_page() {
        // A forced break before any real content is ignored (no leading blank).
        let pages = paginate(vec![brk(PageBreakSide::Any), block(100.0)], 1000.0, 0.0);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].elements.len(), 1);
    }

    #[test]
    fn named_page_break_applies_named_margin_to_started_page() {
        // CSS Paged Media 3 §3.4: a `page: <name>` break starts a page that
        // adopts the matching `@page <name>` margin; the page before it keeps the
        // default geometry.
        let named_margin = crate::types::Margin::uniform(5.0);
        let rules = crate::parser::css::parse_page_rules("@page wide { margin: 5pt; }");
        let pages = paginate_with_context(
            vec![
                block(100.0),
                PageBreak {
                    side: PageBreakSide::Any,
                    page_name: Some("wide".to_string()),
                }
                .boxed(),
                block(100.0),
            ],
            PaginationContext::new(
                super::super::page_context::PageGeometryContext::from_rules(
                    PageSize::new(1000.0, 1000.0),
                    Margin::uniform(0.0),
                    &rules,
                ),
                FootnoteAreaLayout::default(),
                0.0,
            ),
            &HashMap::new(),
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages[0].geometry.map(|geometry| geometry.margin),
            Some(Margin::uniform(0.0)),
            "page 1 keeps default margin"
        );
        assert_eq!(
            pages[1].geometry.map(|geometry| geometry.margin),
            Some(named_margin),
            "page 2 adopts the @page wide margin"
        );
    }

    #[test]
    fn named_page_break_without_geometry_retains_page_identity() {
        // A page type exists independently from an @page geometry rule. Its
        // identity must remain available to compound selectors such as
        // `@page ghost:right`, while its geometry stays at the default.
        let pages = paginate_with_context(
            vec![
                block(100.0),
                PageBreak {
                    side: PageBreakSide::Any,
                    page_name: Some("ghost".to_string()),
                }
                .boxed(),
                block(100.0),
            ],
            PaginationContext::new(
                super::super::page_context::PageGeometryContext::uniform(
                    PageSize::new(1000.0, 1000.0),
                    Margin::uniform(0.0),
                ),
                FootnoteAreaLayout::default(),
                0.0,
            ),
            &HashMap::new(),
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages[1].geometry.map(|geometry| geometry.margin),
            Some(Margin::uniform(0.0))
        );
        assert_eq!(pages[1].page_name.as_deref(), Some("ghost"));
    }

    #[test]
    fn consecutive_forced_breaks_collapse() {
        // Two adjacent breaks between two blocks still yield exactly two pages.
        let pages = paginate(
            vec![
                block(100.0),
                brk(PageBreakSide::Any),
                brk(PageBreakSide::Any),
                block(100.0),
            ],
            1000.0,
            0.0,
        );
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn sided_break_right_inserts_blank_parity_page() {
        // Content on page 1 (a right/recto page). `break-*: right` then forces the
        // next content onto the next right page — page 2 would be a LEFT page, so a
        // blank page is inserted and the content lands on page 3.
        let pages = paginate(
            vec![block(100.0), brk(PageBreakSide::Right), block(100.0)],
            1000.0,
            0.0,
        );
        assert_eq!(pages.len(), 3, "expected blank parity page");
        assert!(
            pages[1].elements.is_empty(),
            "middle page should be the inserted blank"
        );
        assert_eq!(pages[2].elements.len(), 1);
    }

    #[test]
    fn sided_break_left_needs_no_blank_when_next_is_left() {
        // Content on page 1 (right). `break-*: left` wants a LEFT page; page 2 is
        // already a left page, so no blank is inserted (2 pages total).
        let pages = paginate(
            vec![block(100.0), brk(PageBreakSide::Left), block(100.0)],
            1000.0,
            0.0,
        );
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].elements.len(), 1);
    }
}
