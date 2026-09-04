use crate::parser::css::{AncestorInfo, CssRule, PseudoElement, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    ComputedStyle, Display, compute_pseudo_element_style_with_font_metrics,
    compute_style_with_context_with_font_metrics,
};
use crate::style::font_metrics::FontMetrics;
use std::collections::HashMap;

use super::elements::{BoxModel, IntoLayoutNode, LayoutNode, TextBlock};
use super::engine::{
    CounterState, ElementSiblingPosition, TextRun, element_is_empty, element_sibling_list,
    forward_siblings,
};
use super::helpers::{append_pseudo_inline_run, build_pseudo_inline_run};
use super::text::{
    InlineTextSequence, TextWrapOptions, parent_line_strut, text_run_line_height_factor,
    used_font_size, wrap_text_runs,
};

mod flow;
pub(crate) use flow::{IndependentFlowLayout, layout_mixed_flow_children};

/// Source-order selector position for one inline sibling sequence.
///
/// Every inline analysis and layout pass advances this same cursor. This keeps
/// `:first-of-type`, `:last-of-type`, sibling combinators, `:empty`, and the
/// ancestor metadata used by descendant selectors identical at every depth.
pub(crate) struct InlineSiblingCursor {
    siblings: Vec<(String, Vec<String>)>,
    next_index: usize,
}

impl InlineSiblingCursor {
    pub(crate) fn starting_at(nodes: &[DomNode], next_index: usize) -> Self {
        let siblings = element_sibling_list(nodes);
        Self {
            next_index: next_index.min(siblings.len()),
            siblings,
        }
    }

    pub(crate) fn next_context<'a>(
        &mut self,
        element: &'a ElementNode,
        ancestors: &[AncestorInfo<'a>],
    ) -> SelectorContext<'a> {
        let child_index = self.next_index;
        self.next_index = self.next_index.saturating_add(1);
        SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index,
            sibling_count: self.siblings.len(),
            preceding_siblings: self
                .siblings
                .get(..child_index)
                .unwrap_or_default()
                .to_vec(),
            following_siblings: forward_siblings(&self.siblings, child_index).to_vec(),
            is_empty: element_is_empty(element),
        }
    }
}

/// One generated box attached to an originating element.
///
/// Keeping the originating element and computed pseudo style inseparable lets
/// every formatting context resolve attributes, counters, and quotes through
/// the same representation. The box may later participate as inline content or
/// be wrapped by table/block fixup; it is not a synthetic DOM element.
#[derive(Clone, Copy)]
pub(crate) struct GeneratedBox<'a> {
    originating_element: &'a ElementNode,
    style: &'a ComputedStyle,
}

impl<'a> GeneratedBox<'a> {
    pub(crate) const fn new(
        originating_element: &'a ElementNode,
        style: &'a ComputedStyle,
    ) -> Self {
        Self {
            originating_element,
            style,
        }
    }

    pub(crate) const fn style(self) -> &'a ComputedStyle {
        self.style
    }

    pub(crate) const fn originating_element(self) -> &'a ElementNode {
        self.originating_element
    }

    pub(crate) fn append_inline(
        self,
        runs: &mut Vec<TextRun>,
        fonts: &HashMap<String, TtfFont>,
        counter_state: &mut CounterState,
        resources: &mut crate::security::resources::ResourceLoader,
    ) {
        append_pseudo_inline_run(
            runs,
            Some(self.style),
            self.originating_element,
            fonts,
            counter_state,
            resources,
        );
    }

    /// Append the generated value as measurable text regardless of its outer
    /// display role. Table intrinsic sizing measures the contents of block and
    /// inline generated boxes alike; table fixup decides their final box role.
    pub(crate) fn append_measurement_run(
        self,
        runs: &mut Vec<TextRun>,
        fonts: &HashMap<String, TtfFont>,
        counter_state: &mut CounterState,
        resources: &mut crate::security::resources::ResourceLoader,
    ) {
        runs.push(build_pseudo_inline_run(
            self.style,
            self.originating_element,
            fonts,
            counter_state,
            resources,
        ));
    }
}

/// Owned computed styles for an originating element's generated children.
///
/// Style resolution produces this value once. Formatting contexts then borrow
/// [`GeneratedInlineContent`] from it instead of inventing their own
/// before/after pairs and silently diverging in generated-content behavior.
#[derive(Debug, Clone, Default)]
pub(crate) struct GeneratedContentStyles {
    before: Option<ComputedStyle>,
    after: Option<ComputedStyle>,
}

impl GeneratedContentStyles {
    pub(crate) fn resolve(
        element: &ElementNode,
        principal: &ComputedStyle,
        rules: &[CssRule],
        selector: &SelectorContext<'_>,
        fonts: &HashMap<String, TtfFont>,
    ) -> Self {
        let classes = element.class_list();
        let resolve = |pseudo| {
            compute_pseudo_element_style_with_font_metrics(
                principal,
                rules,
                element.tag_name(),
                &classes,
                element.id(),
                &element.attributes,
                selector,
                pseudo,
                FontMetrics::new(fonts),
            )
        };
        Self {
            before: resolve(PseudoElement::Before),
            after: resolve(PseudoElement::After),
        }
    }

    pub(crate) const fn boxes<'a>(
        &'a self,
        element: &'a ElementNode,
    ) -> GeneratedInlineContent<'a> {
        GeneratedInlineContent::new(element, self.before.as_ref(), self.after.as_ref())
    }

    pub(crate) const fn before(&self) -> Option<&ComputedStyle> {
        self.before.as_ref()
    }

    pub(crate) const fn after(&self) -> Option<&ComputedStyle> {
        self.after.as_ref()
    }

    /// Whether either generated child needs box-tree layout instead of the
    /// inline-run path.
    pub(crate) fn requires_box_layout(&self) -> bool {
        [self.before(), self.after()]
            .into_iter()
            .flatten()
            .any(|style| {
                style.position.is_absolute()
                    || matches!(style.display, Display::Block | Display::ListItem)
            })
    }
}

/// Pseudo styles attached to one principal box.
///
/// The generated children and typographic first-fragment styles are resolved
/// together at the element boundary. Downstream formatting contexts borrow
/// this proof instead of independently re-running part of the pseudo cascade.
#[derive(Debug, Clone, Default)]
pub(crate) struct PrincipalPseudoStyles {
    generated: GeneratedContentStyles,
    first_line: Option<ComputedStyle>,
    first_letter: Option<ComputedStyle>,
}

impl PrincipalPseudoStyles {
    pub(crate) fn resolve(
        element: &ElementNode,
        principal: &ComputedStyle,
        rules: &[CssRule],
        selector: &SelectorContext<'_>,
        fonts: &HashMap<String, TtfFont>,
    ) -> Self {
        let classes = element.class_list();
        let resolve = |pseudo| {
            compute_pseudo_element_style_with_font_metrics(
                principal,
                rules,
                element.tag_name(),
                &classes,
                element.id(),
                &element.attributes,
                selector,
                pseudo,
                FontMetrics::new(fonts),
            )
        };
        Self {
            generated: GeneratedContentStyles::resolve(element, principal, rules, selector, fonts),
            first_line: resolve(PseudoElement::FirstLine),
            first_letter: resolve(PseudoElement::FirstLetter),
        }
    }

    pub(crate) const fn generated(&self) -> &GeneratedContentStyles {
        &self.generated
    }

    pub(crate) const fn first_line(&self) -> Option<&ComputedStyle> {
        self.first_line.as_ref()
    }

    pub(crate) const fn first_letter(&self) -> Option<&ComputedStyle> {
        self.first_letter.as_ref()
    }
}

/// Generated content at the two boundaries of an originating element's
/// principal box.
///
/// CSS generated content participates in the same formatting sequence as the
/// originating element's real children. Keeping both boundaries together
/// prevents a layout path from treating `::before` or `::after` as unrelated
/// sibling blocks merely because the sequence also contains an atomic inline.
#[derive(Clone, Copy, Default)]
pub(crate) struct GeneratedInlineContent<'a> {
    before: Option<GeneratedBox<'a>>,
    after: Option<GeneratedBox<'a>>,
}

impl<'a> GeneratedInlineContent<'a> {
    pub(crate) const fn new(
        originating_element: &'a ElementNode,
        before: Option<&'a ComputedStyle>,
        after: Option<&'a ComputedStyle>,
    ) -> Self {
        Self {
            before: match before {
                Some(style) => Some(GeneratedBox::new(originating_element, style)),
                None => None,
            },
            after: match after {
                Some(style) => Some(GeneratedBox::new(originating_element, style)),
                None => None,
            },
        }
    }

    pub(crate) const fn from_boxes(
        before: Option<GeneratedBox<'a>>,
        after: Option<GeneratedBox<'a>>,
    ) -> Self {
        Self { before, after }
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.before.is_none() && self.after.is_none()
    }

    pub(crate) const fn before(self) -> Option<GeneratedBox<'a>> {
        self.before
    }

    pub(crate) const fn after(self) -> Option<GeneratedBox<'a>> {
        self.after
    }

    pub(crate) fn append_before(
        self,
        runs: &mut Vec<TextRun>,
        fonts: &HashMap<String, TtfFont>,
        counter_state: &mut CounterState,
        resources: &mut crate::security::resources::ResourceLoader,
    ) {
        if let Some(before) = self.before {
            before.append_inline(runs, fonts, counter_state, resources);
        }
    }

    pub(crate) fn append_after(
        self,
        runs: &mut Vec<TextRun>,
        fonts: &HashMap<String, TtfFont>,
        counter_state: &mut CounterState,
        resources: &mut crate::security::resources::ResourceLoader,
    ) {
        if let Some(after) = self.after {
            after.append_inline(runs, fonts, counter_state, resources);
        }
    }

    pub(crate) fn append_before_measurement(
        self,
        runs: &mut Vec<TextRun>,
        fonts: &HashMap<String, TtfFont>,
        counter_state: &mut CounterState,
        resources: &mut crate::security::resources::ResourceLoader,
    ) {
        if let Some(before) = self.before {
            before.append_measurement_run(runs, fonts, counter_state, resources);
        }
    }

    pub(crate) fn append_after_measurement(
        self,
        runs: &mut Vec<TextRun>,
        fonts: &HashMap<String, TtfFont>,
        counter_state: &mut CounterState,
        resources: &mut crate::security::resources::ResourceLoader,
    ) {
        if let Some(after) = self.after {
            after.append_measurement_run(runs, fonts, counter_state, resources);
        }
    }
}

/// The complete source-order content of one inline formatting context.
///
/// DOM children alone are insufficient: generated content forms the first and
/// last inline items of the originating element. This value is shared by the
/// analysis and execution passes so neither pass can silently omit a boundary.
#[derive(Clone, Copy)]
pub(crate) struct InlineContentSequence<'a> {
    source_nodes: &'a [DomNode],
    start: usize,
    end: usize,
    generated: Option<GeneratedInlineContent<'a>>,
}

impl<'a> InlineContentSequence<'a> {
    pub(crate) const fn new(nodes: &'a [DomNode]) -> Self {
        Self {
            source_nodes: nodes,
            start: 0,
            end: nodes.len(),
            generated: None,
        }
    }

    pub(crate) fn segment(nodes: &'a [DomNode], start: usize, end: usize) -> Self {
        let start = start.min(nodes.len());
        let end = end.clamp(start, nodes.len());
        Self {
            source_nodes: nodes,
            start,
            end,
            generated: None,
        }
    }

    /// Select one item while retaining the complete source sibling list.
    ///
    /// Selector matching depends on siblings outside the selected range, so a
    /// one-item sequence must never be rebuilt from a one-node slice.
    pub(crate) fn item(self, offset: usize) -> Self {
        let start = self.start.saturating_add(offset).min(self.end);
        Self::segment(
            self.source_nodes,
            start,
            start.saturating_add(1).min(self.end),
        )
    }

    pub(crate) const fn with_generated(
        nodes: &'a [DomNode],
        generated: GeneratedInlineContent<'a>,
    ) -> Self {
        Self {
            source_nodes: nodes,
            start: 0,
            end: nodes.len(),
            generated: Some(generated),
        }
    }

    pub(crate) fn nodes(self) -> &'a [DomNode] {
        self.source_nodes
            .get(self.start..self.end)
            .unwrap_or_default()
    }

    pub(crate) const fn source_nodes(self) -> &'a [DomNode] {
        self.source_nodes
    }

    pub(crate) fn starting_element_index(self) -> usize {
        self.source_nodes
            .get(..self.start)
            .unwrap_or_default()
            .iter()
            .filter(|node| matches!(node, DomNode::Element(_)))
            .count()
    }

    pub(crate) const fn start(self) -> usize {
        self.start
    }

    pub(crate) const fn end(self) -> usize {
        self.end
    }

    pub(crate) fn append_before(
        self,
        runs: &mut Vec<TextRun>,
        fonts: &HashMap<String, TtfFont>,
        counter_state: &mut CounterState,
        resources: &mut crate::security::resources::ResourceLoader,
    ) {
        if let Some(generated) = self.generated {
            generated.append_before(runs, fonts, counter_state, resources);
        }
    }

    pub(crate) fn append_after(
        self,
        runs: &mut Vec<TextRun>,
        fonts: &HashMap<String, TtfFont>,
        counter_state: &mut CounterState,
        resources: &mut crate::security::resources::ResourceLoader,
    ) {
        if let Some(generated) = self.generated {
            generated.append_after(runs, fonts, counter_state, resources);
        }
    }
}

/// Builds anonymous block boxes for contiguous inline fragments separated by
/// block-level siblings.
///
/// The anonymous box owns only the line formatting. The originating block owns
/// its definite size, margins, padding, border, paint and positioning, so none
/// of those properties may leak into this child.
pub(crate) struct AnonymousInlineFormattingContext<'a> {
    parent_style: &'a ComputedStyle,
    available_width: f32,
    fonts: &'a HashMap<String, TtfFont>,
}

impl<'a> AnonymousInlineFormattingContext<'a> {
    pub(crate) const fn new(
        parent_style: &'a ComputedStyle,
        available_width: f32,
        fonts: &'a HashMap<String, TtfFont>,
    ) -> Self {
        Self {
            parent_style,
            available_width,
            fonts,
        }
    }

    pub(crate) fn layout_runs(&self, mut runs: Vec<TextRun>) -> Option<LayoutNode> {
        if runs.is_empty() {
            return None;
        }
        runs.as_mut_slice()
            .resolve_unclaimed_boundaries(super::elements::TextSpacing::from_style(
                self.parent_style,
            ));
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(
                self.available_width,
                used_font_size(self.parent_style, self.fonts),
                text_run_line_height_factor(self.parent_style, self.fonts),
                self.parent_style.overflow_wrap,
            )
            .with_white_space(self.parent_style.white_space)
            .with_parent_strut(parent_line_strut(self.parent_style, self.fonts))
            .with_rtl(self.parent_style.direction_rtl)
            .with_bidi_override(self.parent_style.bidi_override)
            .with_bidi_plaintext(self.parent_style.bidi_plaintext),
            self.fonts,
        );
        if lines.is_empty() {
            return None;
        }

        let mut block = TextBlock::from_style(lines, self.parent_style, BoxModel::default());
        block.paint = Default::default();
        block.flow = Default::default();
        block.positioning = Default::default();
        block.fragmentation = Default::default();
        block.clipping = Default::default();
        Some(block.boxed())
    }
}

/// How an element participates in its parent's inline formatting context.
///
/// This is the shared contract between the analysis pass that selects the
/// mixed-inline layout path and the layout pass that builds its atomic cells.
/// Keeping the decision here prevents replaced elements from being accepted as
/// inline content by one pass and then silently discarded by text collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineFormattingRole {
    Hidden,
    Text,
    Atomic(AtomicInlineKind),
    OutOfFlow,
    Outside,
}

/// Which parent inline-layout path owns the paint for atomic children.
///
/// Child traversal must not independently lay out a box already embedded in a
/// text run or mixed inline row. Keeping that ownership explicit avoids
/// tag-specific duplicate-suppression rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum AtomicInlineEmission {
    #[default]
    Independent,
    InlineBlockRuns,
    MixedRow,
}

impl AtomicInlineEmission {
    const fn owns(self, role: InlineFormattingRole) -> bool {
        match self {
            Self::Independent => false,
            Self::InlineBlockRuns => matches!(
                role,
                InlineFormattingRole::Atomic(AtomicInlineKind::InlineBlock)
            ),
            Self::MixedRow => matches!(role, InlineFormattingRole::Atomic(_)),
        }
    }
}

/// Computed participation of one element child in an inline formatting context.
pub(crate) struct InlineFormattingChild {
    pub(crate) style: ComputedStyle,
    pub(crate) role: InlineFormattingRole,
    source: ElementSiblingPosition,
}

impl InlineFormattingChild {
    pub(crate) const fn source(&self) -> &ElementSiblingPosition {
        &self.source
    }
}

/// One authoritative style/classification pass for an inline sibling sequence.
/// Indices are element-sibling indices, never raw DOM-node indices.
pub(crate) struct InlineFormattingChildren(Vec<InlineFormattingChild>);

impl InlineFormattingChildren {
    pub(crate) fn get(&self, element_index: usize) -> Option<&InlineFormattingChild> {
        self.0.get(element_index)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &InlineFormattingChild> {
        self.0.iter()
    }

    pub(crate) fn has_out_of_flow(&self) -> bool {
        self.0
            .iter()
            .any(|child| child.role == InlineFormattingRole::OutOfFlow)
    }

    pub(crate) fn has_outside(&self) -> bool {
        self.0
            .iter()
            .any(|child| child.role == InlineFormattingRole::Outside)
    }

    pub(crate) fn requires_independent_layout(&self, element_index: usize) -> bool {
        self.0.get(element_index).is_some_and(|child| {
            matches!(
                child.role,
                InlineFormattingRole::OutOfFlow | InlineFormattingRole::Outside
            )
        })
    }

    pub(crate) fn is_grouped_atomic(&self, element_index: usize) -> bool {
        self.0.get(element_index).is_some_and(|child| {
            matches!(
                child.role,
                InlineFormattingRole::Atomic(
                    AtomicInlineKind::InlineBlock
                        | AtomicInlineKind::InlineFlex
                        | AtomicInlineKind::InlineGrid
                        | AtomicInlineKind::InlineTable
                )
            )
        })
    }

    pub(crate) fn atomic_is_emitted(
        &self,
        element_index: usize,
        emission: AtomicInlineEmission,
    ) -> bool {
        self.0
            .get(element_index)
            .is_some_and(|child| emission.owns(child.role))
    }

    pub(crate) fn is_inline_text(&self, element_index: usize) -> bool {
        self.0.get(element_index).map(|child| child.role) == Some(InlineFormattingRole::Text)
    }
}

/// The layout mechanism required by an atomic inline-level element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomicInlineKind {
    ReplacedImage,
    InlineBlock,
    InlineFlex,
    InlineGrid,
    InlineTable,
}

impl AtomicInlineKind {
    const fn requires_environment_aware_layout(self) -> bool {
        !matches!(self, Self::InlineBlock)
    }
}

impl InlineFormattingRole {
    /// Whether this role participates in a table cell's text flow.
    pub(crate) const fn participates_in_table_cell_text_flow(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Atomic(AtomicInlineKind::InlineBlock)
        )
    }

    /// Text and the established embedded inline-block path are collected as
    /// runs. Other atomic boxes are owned by their source-ordered row segment.
    pub(crate) fn uses_text_run_layout(self, element: &ElementNode) -> bool {
        matches!(self, Self::Text)
            || matches!(self, Self::Atomic(AtomicInlineKind::InlineBlock))
                && element.tag.is_inline()
    }

    pub(crate) fn of(element: &ElementNode, style: &ComputedStyle) -> Self {
        if style.display == Display::None {
            return Self::Hidden;
        }
        if style.position.is_absolute() {
            return Self::OutOfFlow;
        }
        if element.tag == HtmlTag::Svg {
            return Self::Outside;
        }

        let inline_level = matches!(
            style.display,
            Display::Inline
                | Display::InlineBlock
                | Display::InlineFlex
                | Display::InlineGrid
                | Display::InlineTable
        );
        if element.tag == HtmlTag::Img && inline_level {
            return Self::Atomic(AtomicInlineKind::ReplacedImage);
        }

        match style.display {
            Display::InlineBlock => Self::Atomic(AtomicInlineKind::InlineBlock),
            Display::InlineFlex => Self::Atomic(AtomicInlineKind::InlineFlex),
            Display::InlineGrid => Self::Atomic(AtomicInlineKind::InlineGrid),
            Display::InlineTable => Self::Atomic(AtomicInlineKind::InlineTable),
            Display::Inline => Self::Text,
            _ => Self::Outside,
        }
    }
}

/// Immutable style context used to decide whether a sibling sequence needs the
/// environment-aware atomic-inline layout path.
pub(crate) struct InlineFormattingContext<'a> {
    parent_style: &'a ComputedStyle,
    rules: &'a [CssRule],
    ancestors: &'a [AncestorInfo<'a>],
    font_metrics: FontMetrics<'a>,
}

impl<'a> InlineFormattingContext<'a> {
    pub(crate) const fn new(
        parent_style: &'a ComputedStyle,
        rules: &'a [CssRule],
        ancestors: &'a [AncestorInfo<'a>],
        font_metrics: FontMetrics<'a>,
    ) -> Self {
        Self {
            parent_style,
            rules,
            ancestors,
            font_metrics,
        }
    }

    pub(crate) fn children(&self, sequence: InlineContentSequence<'_>) -> InlineFormattingChildren {
        let mut siblings = InlineSiblingCursor::starting_at(
            sequence.source_nodes(),
            sequence.starting_element_index(),
        );
        let roles = sequence
            .nodes()
            .iter()
            .filter_map(|node| {
                let DomNode::Element(element) = node else {
                    return None;
                };
                let selector_context = siblings.next_context(element, self.ancestors);
                let classes = element.class_list();
                let style = compute_style_with_context_with_font_metrics(
                    element.tag,
                    element.style_attr(),
                    self.parent_style,
                    self.rules,
                    element.tag_name(),
                    &classes,
                    element.id(),
                    &element.attributes,
                    &selector_context,
                    self.font_metrics,
                );
                Some(InlineFormattingChild {
                    role: InlineFormattingRole::of(element, &style),
                    style,
                    source: ElementSiblingPosition::from_selector_context(&selector_context),
                })
            })
            .collect();
        InlineFormattingChildren(roles)
    }

    /// Returns true only when every visible child can share one inline
    /// formatting context and at least one child requires an atomic cell.
    pub(crate) fn requires_atomic_layout(&self, sequence: InlineContentSequence<'_>) -> bool {
        let nodes = sequence.nodes();
        let mut siblings = InlineSiblingCursor::starting_at(
            sequence.source_nodes(),
            sequence.starting_element_index(),
        );
        let mut saw_atomic = false;

        for node in nodes {
            let DomNode::Element(element) = node else {
                continue;
            };
            let classes = element.class_list();
            let selector_context = siblings.next_context(element, self.ancestors);
            let style = compute_style_with_context_with_font_metrics(
                element.tag,
                element.style_attr(),
                self.parent_style,
                self.rules,
                element.tag_name(),
                &classes,
                element.id(),
                &element.attributes,
                &selector_context,
                self.font_metrics,
            );

            match InlineFormattingRole::of(element, &style) {
                InlineFormattingRole::Atomic(kind) => {
                    saw_atomic |= kind.requires_environment_aware_layout();
                }
                // Out-of-flow inline children are laid out independently, but
                // their static source position does not split the surrounding
                // inline formatting context.
                InlineFormattingRole::OutOfFlow
                | InlineFormattingRole::Hidden
                | InlineFormattingRole::Text => {}
                InlineFormattingRole::Outside => return false,
            }
        }

        saw_atomic
    }

    /// Maximal source-order ranges that contain any atomic inline and remain
    /// inside one inline formatting context.
    pub(crate) fn atomic_layout_segments<'b>(
        &self,
        sequence: InlineContentSequence<'b>,
    ) -> Vec<InlineContentSequence<'b>> {
        self.atomic_layout_segments_requiring(sequence, |_| true)
    }

    /// Maximal source-order ranges that contain an atomic inline requiring
    /// access to the layout environment.
    pub(crate) fn environment_aware_atomic_layout_segments<'b>(
        &self,
        sequence: InlineContentSequence<'b>,
    ) -> Vec<InlineContentSequence<'b>> {
        self.atomic_layout_segments_requiring(sequence, |kind| {
            kind.requires_environment_aware_layout()
        })
    }

    /// Block/table participants split ranges without forcing later inline
    /// content onto the legacy per-element path. Each returned sequence keeps
    /// the complete sibling list for selector matching.
    fn atomic_layout_segments_requiring<'b>(
        &self,
        sequence: InlineContentSequence<'b>,
        requires_segment: impl Fn(AtomicInlineKind) -> bool,
    ) -> Vec<InlineContentSequence<'b>> {
        let source = sequence.source_nodes();
        let mut siblings =
            InlineSiblingCursor::starting_at(source, sequence.starting_element_index());
        let mut segment_start = sequence.start();
        let mut segment_has_atomic = false;
        let mut segments = Vec::new();

        for node_index in sequence.start()..sequence.end() {
            let Some(DomNode::Element(element)) = source.get(node_index) else {
                continue;
            };
            let classes = element.class_list();
            let selector_context = siblings.next_context(element, self.ancestors);
            let style = compute_style_with_context_with_font_metrics(
                element.tag,
                element.style_attr(),
                self.parent_style,
                self.rules,
                element.tag_name(),
                &classes,
                element.id(),
                &element.attributes,
                &selector_context,
                self.font_metrics,
            );

            match InlineFormattingRole::of(element, &style) {
                InlineFormattingRole::Atomic(kind) => {
                    segment_has_atomic |= requires_segment(kind);
                }
                InlineFormattingRole::OutOfFlow => {}
                InlineFormattingRole::Outside => {
                    if segment_has_atomic && segment_start < node_index {
                        segments.push(InlineContentSequence::segment(
                            source,
                            segment_start,
                            node_index,
                        ));
                    }
                    segment_start = node_index + 1;
                    segment_has_atomic = false;
                }
                InlineFormattingRole::Hidden | InlineFormattingRole::Text => {}
            }
        }

        if segment_has_atomic && segment_start < sequence.end() {
            segments.push(InlineContentSequence::segment(
                source,
                segment_start,
                sequence.end(),
            ));
        }
        segments
    }
}
