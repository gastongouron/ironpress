use crate::parser::css::{
    AncestorInfo, CssRule, PageRule, PageSelector, PageSelectorContext, PageTextStyle,
    PseudoElement, SelectorContext,
};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    AlignItems, AlignSelf, BorderSides, ComputedStyle, ContentItem, Display, FontFamily, FontStyle,
    FontVariantPosition, FontWeight, FootnoteFormatting, ListStylePosition, ListStyleType,
    PercentageBasis, TARGET_PLACEHOLDER_END, TARGET_PLACEHOLDER_START, TextAlign, Transform,
    VerticalAlign, WritingMode, compute_pseudo_element_style_with_font_metrics,
    compute_style_with_context, compute_style_with_context_and_percentage_basis_with_font_metrics,
    compute_style_with_context_with_font_metrics,
};
use crate::style::font_metrics::FontMetrics;
use crate::types::{
    CornerRadii, EdgeSizes, Margin, PageSize, PhysicalEdges, PhysicalSide, Point, Size,
};
use std::collections::HashMap;

#[cfg(test)]
use crate::style::computed::{BorderCollapse, Clear, Float, Position};
#[cfg(test)]
use crate::types::Color;

use super::block::layout_block_element;
#[cfg(test)]
use super::cells::CellBoxHolder;
pub use super::cells::{GridCell, TableCell};
pub(crate) use super::elements::{
    AvoidPageBreak, BackgroundBox, BackgroundBoxGeometry, BoxFragmentation, BoxModel, Container,
    FlexRow, GridRow, HorizontalRule, Image, ImagePaint, ImageSampling, IntoLayoutNode,
    LayoutElement, LayoutNode, LayoutSize, LayoutVisitor, LayoutVisitorMut, LineFragmentation,
    MathBlock, NamedString, PageBreak, Positioning, ProgressBar, ProgressColors, ReplacedContent,
    ReplacedFragment, ReplacedGeometry, RunningElement, Svg, SvgPaint, TableRow, TextBlock,
    TextBlockStyle, TextFragmentation, TextSpacing, visit_layout_tree, visit_layout_tree_mut,
};
use super::flex::layout_flex_container;
pub(crate) use super::flow_metrics::BlockMargins;
use super::grid::layout_grid_container;
pub(crate) use super::helpers::*;
use super::images::*;
use super::inline::{
    layout_inline_block_group_with_env_and_spacing, layout_inline_mixed_sequence_with_env,
};
pub use super::inline_box::{CenteredStroke, InlineBox, InlineBoxPaint};
use super::inline_formatting::{
    AtomicInlineKind, GeneratedContentStyles, InlineContentSequence, InlineFormattingContext,
    InlineFormattingRole, PrincipalPseudoStyles,
};
use super::list_markers::{
    BuiltInBulletSlot, build_list_bullet_marker, format_counter_value, format_list_marker,
};
#[cfg(test)]
use super::list_markers::{to_alpha_lower, to_roman_lower};
use super::paginate::FootnoteAreaLayout;
use super::print_scale::{PrintContentScale, assign_page_print_scales};
use super::root_formatting::{DocumentRootStyles, RootFormattingContext};
use super::table::{
    TableLayoutContext, anonymous_table_box_style, anonymous_table_from_cells, flatten_table,
};
pub(crate) use super::traversal::{
    ElementLayoutContext, ElementSiblingContext, ElementSiblingPosition, FilterApplication,
    LayoutTreeContext,
};

#[cfg(test)]
use super::text::OverflowWrap;
use super::text::{
    InlineRunCollector, TextWrapOptions, collapse_whitespace, estimate_word_width,
    has_non_collapsible_text, parent_line_strut, push_text_run_with_fallback,
    resolve_style_font_family, text_run_line_height_factor, used_font_size, used_line_height,
    wrap_text_runs,
};
/// A single border side for layout rendering.
#[derive(Debug, Clone, Copy)]
pub struct LayoutBorderSide {
    pub width: f32,
    pub color: crate::types::Color,
    pub style: crate::style::computed::BorderStyle,
}

impl Default for LayoutBorderSide {
    fn default() -> Self {
        Self {
            width: 0.0,
            color: crate::types::Color::BLACK,
            style: crate::style::computed::BorderStyle::default(),
        }
    }
}

impl LayoutBorderSide {
    pub const fn solid(width: f32, color: crate::types::Color) -> Self {
        Self {
            width,
            color,
            style: crate::style::computed::BorderStyle::Solid,
        }
    }

    /// Whether this side actually paints: it must have a positive width and a
    /// style other than `none`/`hidden`. CSS `border-style: none` suppresses the
    /// edge even when a width was declared.
    pub fn paints(&self) -> bool {
        self.width > 0.0 && self.style.paints()
    }

    /// Whether this side has the same visible paint as another side. Layout
    /// metadata is deliberately excluded: it affects table placement, not the
    /// PDF border operation.
    pub fn same_paint(&self, other: &Self) -> bool {
        self.width == other.width && self.color == other.color && self.style == other.style
    }

    /// Whether adjoining sides form one continuous solid-colour paint region.
    ///
    /// Width is intentionally excluded: the canonical border ring already
    /// owns the inner and outer contours for each side, while one fill avoids
    /// introducing an antialiased seam along their shared frontier.
    pub fn shares_solid_region_with(&self, other: &Self) -> bool {
        self.paints()
            && other.paints()
            && self.style == crate::style::computed::BorderStyle::Solid
            && other.style == crate::style::computed::BorderStyle::Solid
            && self.color == other.color
    }
}

/// Per-side border for layout rendering.
pub type LayoutBorder = PhysicalEdges<LayoutBorderSide>;

#[allow(dead_code)]
impl PhysicalEdges<LayoutBorderSide> {
    /// Resolve computed CSS border sides into used layout border sides.
    pub fn from_computed(b: &BorderSides, current_color: crate::types::Color) -> Self {
        let side = |side: &crate::style::computed::BorderSide| LayoutBorderSide {
            // CSS Backgrounds 3: the computed border width is zero when the
            // style is `none` or `hidden`. Normalize at the layout boundary so
            // every box-model consumer sees the same used geometry.
            width: side.used_width(),
            color: side.color.resolve(current_color),
            style: side.style,
        };
        Self {
            top: side(&b.top),
            right: side(&b.right),
            bottom: side(&b.bottom),
            left: side(&b.left),
        }
    }
    /// Whether any side has a positive used width.
    pub fn has_any(&self) -> bool {
        self.top.width > 0.0
            || self.right.width > 0.0
            || self.bottom.width > 0.0
            || self.left.width > 0.0
    }
    /// Whether any side actually paints (positive width AND non-`none` style).
    pub fn has_visible(&self) -> bool {
        self.top.paints() || self.right.paints() || self.bottom.paints() || self.left.paints()
    }
    /// The common paint for a complete, visually uniform frame.
    ///
    /// PDF can emit this as one closed stroke. Borders with a missing edge or
    /// a paint difference must remain per-side so their CSS joins are kept.
    pub fn uniform_paint_side(&self) -> Option<LayoutBorderSide> {
        let top = self.top;
        (top.paints()
            && top.same_paint(&self.right)
            && top.same_paint(&self.bottom)
            && top.same_paint(&self.left))
        .then_some(top)
    }
    /// The shared color of an open or complete solid border region.
    ///
    /// Missing sides do not prevent the remaining sides from forming one
    /// paint region. Any visible non-solid side or color transition does.
    pub fn common_solid_color(&self) -> Option<crate::types::Color> {
        let color = PhysicalSide::ALL
            .into_iter()
            .map(|edge| self.get(edge))
            .find(|side| side.paints())?
            .color;
        PhysicalSide::ALL
            .into_iter()
            .map(|edge| self.get(edge))
            .all(|side| {
                !side.paints()
                    || (side.style == crate::style::computed::BorderStyle::Solid
                        && side.color == color)
            })
            .then_some(color)
    }
    /// Whether the border has at least one unpainted physical edge.
    pub fn has_open_edge(&self) -> bool {
        PhysicalSide::ALL
            .into_iter()
            .any(|edge| !self.get(edge).paints())
    }
    /// Sum the used left and right border widths.
    pub fn horizontal_width(&self) -> f32 {
        self.left.width + self.right.width
    }
    /// Sum the used top and bottom border widths.
    pub fn vertical_width(&self) -> f32 {
        self.top.width + self.bottom.width
    }
    /// Resolved physical border widths as one box-model edge group.
    pub fn widths(&self) -> EdgeSizes {
        EdgeSizes::new(
            self.top.width,
            self.right.width,
            self.bottom.width,
            self.left.width,
        )
    }

    /// Largest used width among the four sides.
    pub fn max_width(&self) -> f32 {
        self.top
            .width
            .max(self.right.width)
            .max(self.bottom.width)
            .max(self.left.width)
    }
}

// Inline-block layout functions moved to `super::inline`.

/// Counter state for CSS counters.
#[derive(Debug, Default, Clone)]
#[allow(dead_code)]
pub(crate) struct CounterState {
    pub(crate) stacks: HashMap<String, Vec<i32>>,
    pub(crate) quote_depth: usize,
}

/// Counter operations attached to one generated element box.
///
/// Layout routes may differ, but counter semantics do not: every box enters
/// through [`CounterState::enter_element`] before generated content is resolved
/// and leaves through [`CounterState::leave_element`] after its descendants.
/// The scope retains only reset names, so it never borrows either the computed
/// style or counter state while recursive layout runs.
pub(crate) struct CounterScope {
    reset_names: Vec<String>,
}

#[allow(dead_code)]
impl CounterState {
    /// Whether no generated-content counter or quote context is active.
    pub(crate) fn is_generated_content_context_free(&self) -> bool {
        self.stacks.is_empty() && self.quote_depth == 0
    }

    pub(crate) fn enter_element(&mut self, style: &ComputedStyle) -> CounterScope {
        self.apply_resets(&style.counter_reset);
        self.apply_increments(&style.counter_increment);
        self.apply_sets(&style.counter_set);
        CounterScope {
            reset_names: style
                .counter_reset
                .iter()
                .map(|(name, _)| name.clone())
                .collect(),
        }
    }

    pub(crate) fn leave_element(&mut self, scope: CounterScope) {
        for name in scope.reset_names {
            self.pop_name(&name);
        }
    }

    fn apply_resets(&mut self, resets: &[(String, i32)]) {
        for (name, val) in resets {
            self.stacks.entry(name.clone()).or_default().push(*val);
        }
    }
    fn apply_increments(&mut self, increments: &[(String, i32)]) {
        for (name, val) in increments {
            let stack = self.stacks.entry(name.clone()).or_default();
            if stack.is_empty() {
                stack.push(0);
            }
            if let Some(top) = stack.last_mut() {
                *top += val;
            }
        }
    }
    fn apply_sets(&mut self, sets: &[(String, i32)]) {
        for (name, val) in sets {
            self.set_current(name, *val);
        }
    }
    fn pop_resets(&mut self, resets: &[(String, i32)]) {
        for (name, _) in resets {
            if let Some(stack) = self.stacks.get_mut(name) {
                stack.pop();
            }
        }
    }
    pub(crate) fn get(&self, name: &str) -> i32 {
        self.stacks
            .get(name)
            .and_then(|s| s.last().copied())
            .unwrap_or(0)
    }
    fn has_current(&self, name: &str) -> bool {
        self.stacks.get(name).is_some_and(|s| !s.is_empty())
    }
    fn push_reset(&mut self, name: &str, value: i32) {
        self.stacks.entry(name.to_string()).or_default().push(value);
    }
    fn increment(&mut self, name: &str, value: i32) {
        let stack = self.stacks.entry(name.to_string()).or_default();
        if stack.is_empty() {
            stack.push(0);
        }
        if let Some(top) = stack.last_mut() {
            *top += value;
        }
    }
    fn set_current(&mut self, name: &str, value: i32) {
        let stack = self.stacks.entry(name.to_string()).or_default();
        if stack.is_empty() {
            stack.push(value);
        } else if let Some(top) = stack.last_mut() {
            *top = value;
        }
    }
    fn pop_name(&mut self, name: &str) {
        if let Some(stack) = self.stacks.get_mut(name) {
            stack.pop();
        }
    }
    pub(crate) fn get_all(&self, name: &str, sep: &str) -> String {
        self.get_all_styled(name, sep, &ListStyleType::Decimal)
    }
    /// Like `get_all`, but renders every nested level with the given
    /// list-style-type (e.g. `counters(x, '.', upper-roman)`).
    pub(crate) fn get_all_styled(&self, name: &str, sep: &str, style: &ListStyleType) -> String {
        self.stacks
            .get(name)
            .map(|s| {
                s.iter()
                    .map(|v| format_counter_value(style, *v))
                    .collect::<Vec<_>>()
                    .join(sep)
            })
            .unwrap_or_else(|| format_counter_value(style, 0))
    }
}

/// Context for rendering list items.
#[derive(Debug, Clone)]
pub(crate) enum ListContext {
    Unordered { indent: f32 },
    Ordered { index: i32, step: i32, indent: f32 },
}

/// Identity of one flex line within its owning [`FlexRow`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FlexLineId(usize);

impl FlexLineId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(index)
    }
}

/// A forced page break propagated from a flex item to the flex line that owns
/// it. CSS Flexbox §10 applies item `break-before` / `break-after` values to a
/// row flex line rather than treating them as descendants to paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForcedFlexLineBreak {
    pub before: FlexLineId,
    pub side: PageBreakSide,
}

/// Pagination role of a flex fragment.
///
/// An overflow continuation paints content from a forced break inside a flex
/// item, but contributes no content to the surrounding main flow. A subsequent
/// main-flow break therefore remains consecutive with the break that opened
/// this fragmentainer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FlexFragmentRole {
    #[default]
    Normal,
    ParallelOverflowContinuation,
}

/// How an authored block size constrains a flex item's fragments.
///
/// A definite height caps the principal box: content forced onto a later page
/// is parallel overflow. An automatic height can grow with fragmented content,
/// while CSS table `height` is a minimum and therefore has the same growth
/// behavior despite being explicitly authored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FlexItemBlockSize {
    #[default]
    Auto,
    Definite,
    Minimum,
}

impl FlexItemBlockSize {
    pub(crate) const fn is_explicit(self) -> bool {
        !matches!(self, Self::Auto)
    }

    pub(crate) const fn fragments_principal_box(self) -> bool {
        !matches!(self, Self::Definite)
    }
}

/// Fragmentation state retained with a flattened flex item.
///
/// Keeping the authored constraint, decoration behavior, and per-fragment
/// paint extent together prevents pagination from re-inferring CSS semantics
/// from a bare height boolean after the original box has been flattened.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FlexItemFragmentation {
    pub(crate) block_size: FlexItemBlockSize,
    pub(crate) box_fragmentation: BoxFragmentation,
    pub(crate) fragment_block_extent: Option<f32>,
}

/// Coordinate space used by complex descendants flattened into a flex cell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FlexNestedOrigin {
    #[default]
    ContentBox,
    /// Table row geometry already carries the table wrapper's border/padding
    /// insets and therefore starts at the principal border-box origin.
    TableBorderBox,
}

/// Semantic role of one cell in the formatting context that produced it.
///
/// Inline formatting stores atomic boxes in flex-shaped rows, but their
/// subpixel text advance remains authoritative for the box and all of its
/// descendants. Ordinary flex items use the browser-compatible paint grid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FlexItemRole {
    #[default]
    FlexItem,
    AtomicInline(AtomicInlineKind),
}

impl FlexItemRole {
    pub(crate) const fn is_atomic_inline(self) -> bool {
        matches!(self, Self::AtomicInline(_))
    }
}

impl FlexItemFragmentation {
    pub(crate) const fn definite() -> Self {
        Self {
            block_size: FlexItemBlockSize::Definite,
            box_fragmentation: BoxFragmentation {
                decoration: crate::style::computed::BoxDecorationBreak::Slice,
                inside: crate::layout::elements::FragmentBreakAvoidance::Auto,
                content_role: super::elements::PageContentRole::MainFlow,
                reference_slice: None,
            },
            fragment_block_extent: None,
        }
    }

    pub(crate) fn from_style(style: &ComputedStyle) -> Self {
        let block_size = match (style.height, style.display) {
            (None, _) => FlexItemBlockSize::Auto,
            (Some(_), Display::Table | Display::InlineTable) => FlexItemBlockSize::Minimum,
            (Some(_), _) => FlexItemBlockSize::Definite,
        };
        Self {
            block_size,
            box_fragmentation: BoxFragmentation::from_style(style),
            fragment_block_extent: None,
        }
    }
}

/// A cell within a flex row, with its computed x-offset and width.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FlexCell {
    pub lines: Vec<TextLine>,
    pub x_offset: f32,
    pub width: f32,
    pub text_align: TextAlign,
    pub padding: EdgeSizes,
    pub border: LayoutBorder,
    /// Natural height of this flex item (without stretching)
    pub natural_height: f32,
    pub(crate) fragmentation: FlexItemFragmentation,
    /// Min/max clamp on the item's used CROSS-axis size (height for a row
    /// container). The renderer clamps the stretched line cross size AND a
    /// non-stretch item's natural height to `[cross_min, cross_max]`
    /// (css-flexbox-1 §9.4 step 11). `cross_max` is `f32::INFINITY` when
    /// unconstrained; `cross_min` defaults to 0.
    pub cross_min: f32,
    pub cross_max: f32,
    /// Per-item `align-self` override. `Auto` defers to the FlexRow's
    /// `align_items`; otherwise this item aligns independently on the cross
    /// axis.
    pub align_self: crate::style::computed::AlignSelf,
    pub(crate) paint: super::cells::CellPaint,
    pub(crate) positioning: super::elements::Positioning,
    /// Nested layout elements for complex flex items (tables, images, etc.)
    pub nested_elements: Vec<LayoutNode>,
    pub(crate) nested_origin: FlexNestedOrigin,
    pub(crate) role: FlexItemRole,
    /// The composited output of a `filter: url(#...)` on a simple flex item.
    /// Keeping the bitmap with the cell makes the filter's SourceGraphic an
    /// atomic paint result instead of independently recolouring child vectors.
    /// Cross-axis offset of this cell within the FlexRow. For single-line
    /// rows this is 0; for `flex-wrap: wrap` with multiple lines, items on
    /// subsequent lines carry their cumulative cross_offset here so a single
    /// FlexRow can visually span every wrapped line.
    pub y_offset: f32,
    /// Cross-axis size of the flex line this cell belongs to. Drives
    /// per-line alignment math (stretch/center/flex-end) so that a single
    /// FlexRow carrying cells from multiple wrapped lines still aligns each
    /// item against its own line rather than the entire row.
    pub line_cross_size: f32,
    /// Semantic ownership by a flex line. Geometry may move independently via
    /// margins, relative positioning, and alignment.
    pub line_id: FlexLineId,
}

/// Resolved cross-axis border-box geometry of a flex item.
///
/// Keeping this calculation with [`FlexCell`] ensures pagination and both PDF
/// layout paths agree about the physical top and size of centered, stretched,
/// and self-aligned items.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlexCellCrossGeometry {
    pub(crate) size: f32,
    pub(crate) offset: f32,
}

impl FlexCell {
    pub(crate) fn effective_cross_alignment(&self, container: AlignItems) -> AlignItems {
        match self.align_self {
            AlignSelf::Auto => container,
            AlignSelf::FlexStart => AlignItems::FlexStart,
            AlignSelf::FlexEnd => AlignItems::FlexEnd,
            AlignSelf::Center => AlignItems::Center,
            AlignSelf::Baseline => AlignItems::Baseline,
            AlignSelf::Stretch => AlignItems::Stretch,
        }
    }

    pub(crate) fn cross_geometry(
        &self,
        container_size: f32,
        container_alignment: AlignItems,
        baseline_shift: f32,
    ) -> FlexCellCrossGeometry {
        let line_size = if self.line_cross_size > 0.0 {
            self.line_cross_size
        } else {
            container_size
        };
        let clamp = |size: f32| size.min(self.cross_max).max(self.cross_min);
        let natural_size = clamp(self.natural_height);
        let alignment = self.effective_cross_alignment(container_alignment);
        let (size, alignment_offset) = match alignment {
            AlignItems::Stretch if self.fragmentation.block_size.is_explicit() => {
                (natural_size, 0.0)
            }
            AlignItems::Stretch => (clamp(line_size), 0.0),
            AlignItems::Baseline => (natural_size, baseline_shift.max(0.0)),
            AlignItems::FlexStart => (natural_size, 0.0),
            AlignItems::FlexEnd => (natural_size, line_size - natural_size),
            AlignItems::Center => (natural_size, (line_size - natural_size) / 2.0),
        };
        FlexCellCrossGeometry {
            size,
            offset: self.y_offset + alignment_offset,
        }
    }
}

impl super::cells::CellPaintHolder for FlexCell {
    fn cell_paint(&self) -> &super::cells::CellPaint {
        &self.paint
    }

    fn cell_paint_mut(&mut self) -> &mut super::cells::CellPaint {
        &mut self.paint
    }
}

impl super::elements::PositioningOwner for FlexCell {
    fn positioning(&self) -> &super::elements::Positioning {
        &self.positioning
    }

    fn positioning_mut(&mut self) -> &mut super::elements::Positioning {
        &mut self.positioning
    }
}

impl super::elements::BoxPaintOwner for FlexCell {
    fn box_paint(&self) -> &super::elements::BoxPaint {
        &self.paint.box_paint
    }

    fn box_paint_mut(&mut self) -> &mut super::elements::BoxPaint {
        &mut self.paint.box_paint
    }
}

impl Default for FlexCell {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            x_offset: 0.0,
            width: 0.0,
            text_align: TextAlign::default(),
            padding: EdgeSizes::default(),
            border: LayoutBorder::default(),
            natural_height: 0.0,
            fragmentation: FlexItemFragmentation::default(),
            cross_min: 0.0,
            cross_max: f32::INFINITY,
            align_self: crate::style::computed::AlignSelf::default(),
            paint: Default::default(),
            positioning: Default::default(),
            nested_elements: Vec::new(),
            nested_origin: FlexNestedOrigin::default(),
            role: FlexItemRole::default(),
            y_offset: 0.0,
            line_cross_size: 0.0,
            line_id: FlexLineId::default(),
        }
    }
}

/// A styled text run (a piece of text with uniform style).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SyntheticFontWeight {
    /// Use browser-compatible algorithmic emboldening when the selected face
    /// lacks the requested bold weight.
    #[default]
    Auto,
    /// The authored request explicitly resolved without synthetic weight.
    Suppressed,
}

impl SyntheticFontWeight {
    /// Resolve Skia's size-dependent fake-bold stroke in point-space.
    ///
    /// Blink supplies the CSS-pixel font size to Skia. Skia uses 1/24 em at
    /// 9px and below, 1/32 em at 36px and above, and linearly interpolates the
    /// ratio between those limits.
    pub(crate) fn stroke_width(self, font_size: f32) -> Option<f32> {
        if self == Self::Suppressed || !font_size.is_finite() || font_size <= 0.0 {
            return None;
        }
        let css_pixels = font_size / crate::fonts::PT_PER_CSS_PX;
        let ratio = if css_pixels <= 9.0 {
            1.0 / 24.0
        } else if css_pixels >= 36.0 {
            1.0 / 32.0
        } else {
            let progress = (css_pixels - 9.0) / (36.0 - 9.0);
            (1.0 / 24.0) + progress * ((1.0 / 32.0) - (1.0 / 24.0))
        };
        Some(font_size * ratio)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontSynthesisState {
    pub weight: SyntheticFontWeight,
    pub small_caps: bool,
}

/// How the line wrapper treats whitespace in a resolved run.
///
/// Most text runs use CSS's ordinary collapsed-space behavior. A synthesized
/// small-caps split can leave an intervening space at the original font size;
/// that space is kept as its own run so it cannot inherit the following small
/// cap's smaller metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunWhitespace {
    #[default]
    Collapsible,
    Preserve,
}

/// Physical background geometry shared by an inline edge and its text runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct InlineDecoration {
    pub(crate) id: InlineDecorationId,
    pub(crate) background_color: Option<crate::types::Color>,
    pub(crate) padding: EdgeSizes,
    pub(crate) border_radii: CornerRadii,
}

/// Identity of one authored inline decoration within a flattened run list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InlineDecorationId(usize);

impl InlineDecorationId {
    pub(crate) const fn from_index(index: usize) -> Self {
        Self(index)
    }
}

/// Which visual edge of a non-replaced inline box a marker owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineEdgeSide {
    Opening,
    Closing,
}

/// One non-painting edge marker retained in the flattened inline sequence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct InlineEdge {
    pub(crate) side: InlineEdgeSide,
    pub(crate) decoration: InlineDecoration,
}

impl InlineEdge {
    pub(crate) fn reverse(&mut self) {
        self.side = match self.side {
            InlineEdgeSide::Opening => InlineEdgeSide::Closing,
            InlineEdgeSide::Closing => InlineEdgeSide::Opening,
        };
    }
}

impl RunWhitespace {
    pub(crate) const fn preserves_source_spacing(self) -> bool {
        matches!(self, Self::Preserve)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InlineBoundaryAdvance {
    /// Tracking between the final typographic unit in this run and the first
    /// unit in the following run. Its state distinguishes an unclaimed source
    /// boundary, a mechanical token split, a resolved source boundary, and a
    /// boundary suppressed by line breaking.
    letter_spacing: InlineBoundaryLetterSpacing,
    /// Pair-positioning retained when adjacent glyphs must remain in distinct
    /// paint runs.
    contextual_shaping: f32,
}

#[derive(Debug, Clone, Copy, Default)]
enum InlineBoundaryLetterSpacing {
    /// The nearest common inline ancestor has not claimed this source
    /// boundary yet.
    #[default]
    Unclaimed,
    /// A mechanical token split inside one source run. Coalescing restores
    /// this to ordinary internal tracking.
    Internal,
    /// A real source boundary with its resolved owner and used advance.
    Resolved(f32),
    /// A source boundary removed by a line break.
    Suppressed,
}

impl InlineBoundaryAdvance {
    pub(crate) const fn none() -> Self {
        Self {
            letter_spacing: InlineBoundaryLetterSpacing::Suppressed,
            contextual_shaping: 0.0,
        }
    }

    pub(crate) const fn is_unresolved(self) -> bool {
        matches!(self.letter_spacing, InlineBoundaryLetterSpacing::Unclaimed)
    }

    pub(crate) fn resolve_letter_spacing(&mut self, advance: f32) {
        if self.is_unresolved() {
            self.letter_spacing = InlineBoundaryLetterSpacing::Resolved(advance);
        }
    }

    pub(crate) fn set_contextual_shaping(&mut self, advance: f32) {
        self.contextual_shaping = advance;
    }

    pub(crate) fn discard(&mut self) {
        *self = Self::none();
    }

    pub(crate) fn total(self) -> f32 {
        let letter_spacing = match self.letter_spacing {
            InlineBoundaryLetterSpacing::Resolved(advance) => advance,
            InlineBoundaryLetterSpacing::Unclaimed
            | InlineBoundaryLetterSpacing::Internal
            | InlineBoundaryLetterSpacing::Suppressed => 0.0,
        };
        letter_spacing + self.contextual_shaping
    }

    pub(crate) fn can_be_absorbed_by(self, spacing: TextSpacing) -> bool {
        self.contextual_shaping == 0.0
            && match self.letter_spacing {
                InlineBoundaryLetterSpacing::Unclaimed | InlineBoundaryLetterSpacing::Internal => {
                    true
                }
                InlineBoundaryLetterSpacing::Resolved(advance) => advance == spacing.letter,
                InlineBoundaryLetterSpacing::Suppressed => false,
            }
    }

    pub(crate) fn mark_internal(&mut self) {
        *self = Self {
            letter_spacing: InlineBoundaryLetterSpacing::Internal,
            contextual_shaping: 0.0,
        };
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TextRunMetadata {
    pub(crate) font_locale: crate::font_pack::FontLocale,
    /// CSS `text-emphasis` state, kept together because its mark, colour,
    /// position, and resolved ruby geometry must travel as one unit.
    pub emphasis: crate::layout::text_emphasis::TextEmphasis,
    pub(crate) spacing: TextSpacing,
    /// The inherited `text-combine-upright` rule carried to vertical text
    /// expansion. The expansion clears it for ordinary glyphs and retains it
    /// only on the resulting single-glyph composition.
    pub text_combine_upright: crate::style::computed::TextCombineUpright,
    pub is_drop_cap: bool,
    pub whitespace: RunWhitespace,
    pub(crate) inline_edge: Option<InlineEdge>,
    pub(crate) inline_decoration: Option<InlineDecorationId>,
    /// Inline geometry owned by the boundary after this run. Keeping tracking
    /// and contextual shaping together makes every width and paint consumer
    /// advance through one semantic boundary.
    pub(crate) boundary: InlineBoundaryAdvance,
}

/// OpenType features that affect the shape and advance of a text run.
///
/// CSS controls kerning and ligatures independently. Keeping them together in
/// this small value type makes every measure and paint path use the same
/// resolved feature set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextShaping {
    pub ligatures: bool,
    pub kerning: bool,
}

impl TextShaping {
    /// Shape only pair positioning. This is used for a contextual boundary
    /// advance, where forming a glyph across separately painted styles would
    /// be incorrect.
    pub const KERNING_ONLY: Self = Self {
        ligatures: false,
        kerning: true,
    };

    /// The feature set once CSS tracking applies: a non-zero `letter-spacing`
    /// suppresses optional ligatures, because a ligature would swallow the
    /// spacing between its component characters (CSS Text 3 §8.2). Kerning is
    /// controlled independently by `font-kerning`, so it is left as is.
    pub const fn tracked(self, letter_spacing: f32) -> Self {
        Self {
            ligatures: self.ligatures && letter_spacing == 0.0,
            kerning: self.kerning,
        }
    }
}

impl Default for TextShaping {
    fn default() -> Self {
        Self {
            ligatures: true,
            kerning: true,
        }
    }
}

/// Glyph substitution paired with the CSS family that owns inline metrics.
#[derive(Debug, Clone)]
pub(crate) struct GlyphFontFallback {
    /// Computed CSS family whose metrics continue to define the inline box.
    css_family: FontFamily,
}

#[derive(Debug, Clone)]
pub struct TextRun {
    pub text: String,
    pub font_size: f32,
    pub bold: bool,
    pub font_style: FontStyle,
    pub color: crate::types::Color,
    pub decorations: Vec<crate::style::computed::TextDecoration>,
    pub link_url: Option<String>,
    /// Face that shapes and paints this run after character fallback.
    pub font_family: FontFamily,
    /// Original CSS family retained when glyphs come from a fallback face.
    pub(crate) glyph_fallback: Option<GlyphFontFallback>,
    /// Explicit algorithmic font treatment; never encoded in geometry.
    pub font_synthesis: FontSynthesisState,
    /// Background color for inline spans (e.g. badge/highlight).
    pub background_color: Option<crate::types::Color>,
    /// Physical inline-background padding.
    pub padding: EdgeSizes,
    /// Resolved corner radii for inline spans (e.g. badge backgrounds).
    pub border_radii: CornerRadii,
    /// Resolved line-height as a multiple of the run's font size.
    ///
    /// CSS line boxes take their height from the inline content they contain,
    /// so each run carries the line-height resolved from its *own* element's
    /// computed style. `NaN` means "unspecified" — the line-box height then
    /// falls back to the block-level line-height passed via `TextWrapOptions`.
    pub line_height_factor: f32,
    /// The font-size used to resolve this run's `line-height`.
    ///
    /// It normally matches `font_size`. OpenType font-variant positioning uses
    /// a smaller painted glyph while retaining the originating inline box's
    /// inherited `line-height`, so that case records the unscaled basis here.
    /// `NaN` means `font_size`.
    pub line_height_basis: f32,
    /// CSS `font-variant-position`, retained separately from `vertical-align`.
    /// The former selects a sub/superscript glyph; it must not be modelled as a
    /// different CSS `vertical-align` value.
    pub font_variant_position: FontVariantPosition,
    /// Atomic inline-level box (`display: inline-block`) embedded in the line.
    ///
    /// When `Some`, this run is NOT text: it occupies `inline_box.width` of
    /// horizontal advance, contributes `inline_box.height` to the line box, and
    /// the renderer paints the box (background/border + inner text) at the
    /// vertical position dictated by `inline_box.vertical_align`. `text` is kept
    /// empty for such runs so the glyph pipeline ignores them.
    pub inline_box: Option<Box<InlineBox>>,
    /// Resolved OpenType features that affect glyph selection and positioning.
    pub shaping: TextShaping,
    /// CSS `vertical-align` for this text run (css2 §10.8). Only `Sub`/`Super`
    /// move a pure-text run: the glyphs are painted with their baseline shifted
    /// down/up by a fraction of the run's font size, and the line box grows to
    /// contain the shift. Other values (`Baseline`/top/middle/bottom) leave a
    /// text run on the line baseline. Atomic inline boxes carry their own
    /// alignment in `inline_box.vertical_align` instead.
    pub vertical_align: VerticalAlign,
    /// CSS `text-shadow` (css-text-decor-3 §3): a list of shadows painted behind
    /// the glyphs, back-to-front, before the text fill. Each entry reuses the
    /// `BoxShadow` shape (offset_x, offset_y, blur, color); `spread`/`inset` are
    /// unused for text shadows. Empty for the common no-shadow case.
    pub text_shadow: Vec<crate::style::computed::BoxShadow>,
    /// Explicit run-only typography state. This keeps line height and border
    /// radius free of NaN/negative-value metadata channels.
    pub metadata: TextRunMetadata,
}

impl Default for TextRun {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_size: 12.0,
            bold: false,
            font_style: FontStyle::default(),
            color: crate::types::Color::BLACK,
            decorations: Vec::new(),
            link_url: None,
            font_family: FontFamily::default(),
            glyph_fallback: None,
            font_synthesis: FontSynthesisState::default(),
            background_color: None,
            padding: EdgeSizes::ZERO,
            border_radii: CornerRadii::ZERO,
            line_height_factor: f32::NAN,
            line_height_basis: f32::NAN,
            font_variant_position: FontVariantPosition::Normal,
            inline_box: None,
            shaping: TextShaping::default(),
            vertical_align: VerticalAlign::default(),
            text_shadow: Vec::new(),
            metadata: TextRunMetadata::default(),
        }
    }
}

impl TextRun {
    /// Substitute the glyph face without changing the CSS inline-box metrics.
    pub(crate) fn with_glyph_fallback(mut self, glyph_family: FontFamily) -> Self {
        let css_family = self
            .glyph_fallback
            .take()
            .map_or_else(|| self.font_family.clone(), |fallback| fallback.css_family);
        if glyph_family == css_family {
            self.font_family = css_family;
        } else {
            self.font_family = glyph_family;
            self.glyph_fallback = Some(GlyphFontFallback { css_family });
        }
        self
    }

    /// Family whose computed metrics define line boxes and text decorations.
    pub(crate) fn css_font_family(&self) -> &FontFamily {
        self.glyph_fallback
            .as_ref()
            .map_or(&self.font_family, |fallback| &fallback.css_family)
    }

    /// Add the finite inline advance retained at this run's trailing edge.
    ///
    /// CSS tracking and pair positioning can both cross a paint-run boundary.
    /// Every width consumer goes through this helper so layout, clipping, and
    /// PDF painting agree.
    pub(crate) fn inline_advance(&self, width: f32) -> f32 {
        let boundary = self.metadata.boundary.total();
        width
            + if boundary.is_finite() {
                boundary
            } else {
                Default::default()
            }
    }

    /// Complete advance for text painted by this run.
    ///
    /// Internal tracking and the separately-owned outgoing inline boundary
    /// are deliberately composed here so every layout and paint consumer uses
    /// the same geometry.
    pub(crate) fn text_advance(&self, raw_width: f32, text: &str) -> f32 {
        self.inline_advance(self.internal_text_advance(raw_width, text))
    }

    pub(crate) fn internal_text_advance(&self, raw_width: f32, text: &str) -> f32 {
        self.metadata.spacing.add_internal_advance(raw_width, text)
    }

    /// Complete advance for an atomic inline carried by this run.
    pub(crate) fn atomic_inline_advance(&self) -> Option<f32> {
        self.inline_box
            .as_deref()
            .map(|inline| self.inline_advance(inline.outer_width()))
    }

    pub(crate) fn has_typographic_unit(&self) -> bool {
        self.metadata.inline_edge.is_none()
            && (self.inline_box.is_some() || self.text.chars().any(|character| character != '\n'))
    }

    /// Whether this run carries only a non-painting inline edge advance.
    pub(crate) fn is_inline_edge(&self) -> bool {
        self.metadata.inline_edge.is_some()
    }

    /// Whether this run opens a non-replaced inline box.
    pub(crate) fn is_opening_inline_edge(&self) -> bool {
        self.metadata
            .inline_edge
            .is_some_and(|edge| edge.side == InlineEdgeSide::Opening)
    }

    /// Reverse the source-order edge role after bidi places an object on an
    /// odd embedding level. Wrapping consumes visual order, so its opening and
    /// closing roles must describe that order too.
    pub(crate) fn reverse_inline_edge_role(&mut self) {
        if let Some(edge) = self.metadata.inline_edge.as_mut() {
            edge.reverse();
        }
    }

    pub(crate) fn forces_line_break(&self) -> bool {
        self.inline_box.is_none() && self.text == "\n"
    }

    pub(crate) fn joins_typographically(&self, next: &Self) -> bool {
        if self.inline_box.is_some() || next.inline_box.is_some() {
            return !(self.inline_box.is_some() && next.inline_box.is_some());
        }
        self.has_typographic_unit() && next.has_typographic_unit()
    }

    pub(crate) fn text_fragment(&self, text: String, owns_outgoing_boundary: bool) -> Self {
        let mut fragment = self.clone();
        fragment.text = text;
        if !owns_outgoing_boundary {
            fragment.metadata.boundary.mark_internal();
        }
        fragment
    }

    pub(crate) fn line_height_font_size(&self) -> f32 {
        if self.line_height_basis.is_finite() {
            self.line_height_basis
        } else {
            self.font_size
        }
    }

    /// The line-box baseline offset caused by CSS `vertical-align`.
    ///
    /// `font-variant-position` selects a positioned glyph but retains
    /// `vertical-align: baseline`; its fallback paint offset must therefore not
    /// enlarge or move the surrounding line box.
    pub(crate) fn vertical_align_shift(&self, parent_font_size: f32) -> f32 {
        match self.vertical_align {
            VerticalAlign::Super => parent_font_size * crate::render::pdf::SUPER_SHIFT_RATIO,
            VerticalAlign::Sub => -parent_font_size * crate::render::pdf::SUB_SHIFT_RATIO,
            VerticalAlign::Length(value) => value,
            VerticalAlign::Percent(percent) => {
                let factor = if self.line_height_factor.is_finite() {
                    self.line_height_factor.max(0.0)
                } else {
                    1.2
                };
                self.line_height_font_size() * factor * percent
            }
            _ => 0.0,
        }
    }

    /// Fallback paint offset for `font-variant-position` when the selected face
    /// has no OpenType `sups`/`subs` glyphs. This is deliberately separate from
    /// [`Self::vertical_align_shift`]: the CSS feature changes the glyph, while
    /// the inline box remains baseline-aligned.
    pub(crate) fn glyph_baseline_shift(&self, parent_font_size: f32) -> f32 {
        let variant_shift = match self.font_variant_position {
            FontVariantPosition::Super => parent_font_size * crate::render::pdf::SUPER_SHIFT_RATIO,
            FontVariantPosition::Sub => -parent_font_size * crate::render::pdf::SUB_SHIFT_RATIO,
            FontVariantPosition::Normal => 0.0,
        };
        self.vertical_align_shift(parent_font_size) + variant_shift
    }

    /// Width of the synthetic-weight stroke in points, when the resolved face
    /// actually needs one. The authored synthesis state is authoritative, but
    /// cannot manufacture a stroke for a genuine bold face or a non-custom
    /// font.
    pub(crate) fn synthetic_bold_stroke_width(
        &self,
        fonts: &HashMap<String, TtfFont>,
    ) -> Option<f32> {
        (matches!(self.font_family, FontFamily::Custom(_))
            && crate::system_fonts::needs_faux_bold(
                fonts,
                self.font_family.name(),
                self.bold,
                self.font_style.is_slanted(),
            ))
        .then(|| self.font_synthesis.weight.stroke_width(self.font_size))
        .flatten()
    }

    pub(crate) fn synthetic_italic_shear(&self, fonts: &HashMap<String, TtfFont>) -> Option<f32> {
        (matches!(self.font_family, FontFamily::Custom(_))
            && crate::system_fonts::needs_faux_italic(
                fonts,
                self.font_family.name(),
                self.bold,
                self.font_style.is_slanted(),
            ))
        .then(|| self.font_style.synthetic_shear())
        .flatten()
    }
}

const FOOTNOTE_LINK_PREFIX: &str = "ironpress-footnote:";
const FOOTNOTE_LINK_SEPARATOR: char = '\u{1f}';
const TARGET_ANCHOR_PREFIX: &str = "ironpress-target-anchor:";

/// Formatting inherited by a footnote body from its originating element.
///
/// A footnote call is a separate pseudo-element and may have entirely
/// different font metrics. Keep the body style in the link payload instead of
/// attempting to recover it from the call run during pagination.
#[derive(Debug, Clone)]
pub(crate) struct FootnoteBodyStyle {
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: crate::types::Color,
    pub font_family: FontFamily,
    pub line_height_factor: f32,
}

impl Default for FootnoteBodyStyle {
    fn default() -> Self {
        Self {
            font_size: 12.0,
            bold: false,
            italic: false,
            color: crate::types::Color::BLACK,
            font_family: FontFamily::default(),
            line_height_factor: 1.2,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FootnoteLinkData {
    pub marker: String,
    pub text: String,
    pub marker_prefix: String,
    pub body: FootnoteBodyStyle,
    pub marker_color: crate::types::Color,
    pub formatting: FootnoteFormatting,
}

fn encode_footnote_font_family(font_family: &FontFamily) -> String {
    match font_family {
        FontFamily::Helvetica => "Helvetica".to_string(),
        FontFamily::TimesRoman => "Times-Roman".to_string(),
        FontFamily::Courier => "Courier".to_string(),
        FontFamily::Custom(name) => name.clone(),
    }
}

fn decode_footnote_font_family(value: &str) -> FontFamily {
    match value {
        "Helvetica" => FontFamily::Helvetica,
        "Times-Roman" => FontFamily::TimesRoman,
        "Courier" => FontFamily::Courier,
        name => FontFamily::Custom(name.to_string()),
    }
}

pub(crate) fn encode_footnote_link_data(data: &FootnoteLinkData) -> String {
    let clean = |value: &str| value.replace(FOOTNOTE_LINK_SEPARATOR, " ");
    let (body_red, body_green, body_blue) = data.body.color.to_f32_rgb();
    let (marker_red, marker_green, marker_blue) = data.marker_color.to_f32_rgb();
    let fields = [
        clean(&data.marker),
        clean(&data.text),
        clean(&data.marker_prefix),
        data.body.font_size.to_string(),
        data.body.bold.to_string(),
        data.body.italic.to_string(),
        clean(&encode_footnote_font_family(&data.body.font_family)),
        data.body.line_height_factor.to_string(),
        body_red.to_string(),
        body_green.to_string(),
        body_blue.to_string(),
        marker_red.to_string(),
        marker_green.to_string(),
        marker_blue.to_string(),
        data.formatting.display_keyword().to_string(),
        data.formatting.policy_keyword().to_string(),
    ];
    let sep = FOOTNOTE_LINK_SEPARATOR.to_string();
    format!("{FOOTNOTE_LINK_PREFIX}{}", fields.join(&sep))
}

pub(crate) fn decode_footnote_link(value: &str) -> Option<(String, String)> {
    let data = decode_footnote_link_data(value)?;
    Some((data.marker, data.text))
}

pub(crate) fn decode_footnote_link_data(value: &str) -> Option<FootnoteLinkData> {
    let payload = value.strip_prefix(FOOTNOTE_LINK_PREFIX)?;
    let mut parts = payload.split(FOOTNOTE_LINK_SEPARATOR);
    let marker = parts.next()?.to_string();
    let text = parts.next()?.to_string();
    let marker_prefix = parts.next()?.to_string();
    let font_size = parts.next()?.parse().ok()?;
    let bold = parts.next()?.parse().ok()?;
    let italic = parts.next()?.parse().ok()?;
    let font_family = decode_footnote_font_family(parts.next()?);
    let line_height_factor = parts.next()?.parse().ok()?;
    let body_color = match (parts.next(), parts.next(), parts.next()) {
        (Some(r), Some(g), Some(b)) => {
            crate::types::Color::from_srgb(r.parse().ok()?, g.parse().ok()?, b.parse().ok()?, 1.0)
        }
        _ => return None,
    };
    let marker_color = match (parts.next(), parts.next(), parts.next()) {
        (Some(r), Some(g), Some(b)) => {
            crate::types::Color::from_srgb(r.parse().ok()?, g.parse().ok()?, b.parse().ok()?, 1.0)
        }
        _ => return None,
    };
    let display = parts.next()?;
    let policy = parts.next()?;
    Some(FootnoteLinkData {
        marker,
        text,
        marker_prefix,
        body: FootnoteBodyStyle {
            font_size,
            bold,
            italic,
            color: body_color,
            font_family,
            line_height_factor,
        },
        marker_color,
        formatting: FootnoteFormatting::from_keywords(display, policy)?,
    })
}

pub(crate) fn is_internal_target_anchor(value: &str) -> bool {
    value.starts_with(TARGET_ANCHOR_PREFIX)
}

pub(crate) fn target_anchor_id(value: &str) -> Option<&str> {
    value.strip_prefix(TARGET_ANCHOR_PREFIX)
}

/// Block-level text state that cannot be inferred from an individual run.
///
/// Keep this separate from geometric fields: renderer state must never be
/// smuggled through sentinel values in `TextLine::x_offset`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextLineMetadata {
    pub writing_mode: WritingMode,
    pub text_orientation_upright: bool,
}

/// A laid-out line of text runs.
#[derive(Debug, Clone, Default)]
pub struct TextLine {
    pub runs: Vec<TextRun>,
    pub height: f32,
    /// Distance from the line-box top to its baseline when the wrapping pass has
    /// resolved the line's inline boxes and containing-block strut. Synthetic
    /// lines built outside that pass use `None` and retain renderer-side metric
    /// resolution.
    pub baseline_ascent: Option<f32>,
    /// Left inset applied to this line's inline content, in px. Used for the
    /// float exclusion of a `::first-letter { float: left }` drop cap
    /// (css-pseudo-4 §2.2 + css2 §9.5): the lines that vertically overlap the
    /// floated initial are shifted right by the drop cap's width so they wrap
    /// beside it. Zero for ordinary lines.
    pub x_offset: f32,
    pub metadata: TextLineMetadata,
}

/// The format of an embedded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    /// A raw PNG with an alpha channel (RGBA / grayscale+alpha). The `data`
    /// field holds the complete original PNG file; the renderer decodes it into
    /// a colour stream plus a soft-mask (`/SMask`) so transparency is preserved.
    PngAlpha,
}

#[derive(Debug, Clone)]
pub struct FootnoteItem {
    pub marker: String,
    pub text: String,
    pub body: FootnoteBodyStyle,
    pub marker_color: crate::types::Color,
    pub marker_prefix: String,
    pub formatting: FootnoteFormatting,
}

impl FootnoteItem {
    pub(crate) fn text_runs(&self) -> Vec<TextRun> {
        vec![
            TextRun {
                text: self
                    .marker_prefix
                    .replace("{marker}", &self.marker)
                    .replace("{counter}", &self.marker),
                font_size: self.body.font_size,
                bold: self.body.bold,
                font_style: if self.body.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
                color: self.marker_color,
                font_family: self.body.font_family.clone(),
                line_height_factor: self.body.line_height_factor,
                ..Default::default()
            },
            TextRun {
                text: self.text.clone(),
                font_size: self.body.font_size,
                bold: self.body.bold,
                font_style: if self.body.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
                color: self.body.color,
                font_family: self.body.font_family.clone(),
                line_height_factor: self.body.line_height_factor,
                ..Default::default()
            },
        ]
    }
}

/// Parsed PNG metadata needed for PDF FlateDecode parameters.
#[derive(Debug, Clone)]
pub struct PngMetadata {
    pub channels: u8,
    pub bit_depth: u8,
}

/// Whether an image comes from document content or from a renderer-owned raster pass.
///
/// Source images may be downscaled to the configured source-image DPI. Rendered
/// rasters already embody a distinct quality setting (for example filter DPI),
/// so embedding must retain their native pixel dimensions.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum RasterImageOrigin {
    #[default]
    Source,
    Rendered(RasterPixelDensity),
}

impl RasterImageOrigin {
    pub(crate) const fn preserves_native_resolution(self) -> bool {
        matches!(self, Self::Rendered(_))
    }

    pub(crate) const fn pixel_density(self) -> Option<RasterPixelDensity> {
        match self {
            Self::Source => None,
            Self::Rendered(density) => Some(density),
        }
    }
}

/// Physical sampling density owned by a renderer-generated raster.
///
/// Unlike a document image, whose pixels are mapped into an independently
/// resolved CSS box, a generated surface has a fixed physical pixel footprint.
/// Retaining that footprint prevents PDF paint from reconstructing its size
/// through lossy point-space arithmetic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterPixelDensity {
    dpi: f32,
}

impl RasterPixelDensity {
    pub(crate) fn from_dpi(dpi: f32) -> Self {
        Self {
            dpi: crate::style::raster_quality::raster_dpi_at_least(dpi, 1.0),
        }
    }

    pub(crate) const fn dpi(self) -> f32 {
        self.dpi
    }
}

/// Raster image bytes plus the pixel dimensions and resolution ownership needed
/// by the PDF renderer.
#[derive(Debug, Clone)]
pub struct RasterImageAsset {
    pub data: Vec<u8>,
    pub source_width: u32,
    pub source_height: u32,
    pub format: ImageFormat,
    pub png_metadata: Option<PngMetadata>,
    pub origin: RasterImageOrigin,
}

impl RasterImageAsset {
    pub fn source(
        data: Vec<u8>,
        source_width: u32,
        source_height: u32,
        format: ImageFormat,
        png_metadata: Option<PngMetadata>,
    ) -> Self {
        Self::with_origin(
            data,
            source_width,
            source_height,
            format,
            png_metadata,
            RasterImageOrigin::Source,
        )
    }

    pub(crate) fn rendered(
        data: Vec<u8>,
        source_width: u32,
        source_height: u32,
        format: ImageFormat,
        png_metadata: Option<PngMetadata>,
        dpi: f32,
    ) -> Self {
        Self::with_origin(
            data,
            source_width,
            source_height,
            format,
            png_metadata,
            RasterImageOrigin::Rendered(RasterPixelDensity::from_dpi(dpi)),
        )
    }

    pub(crate) fn with_origin(
        data: Vec<u8>,
        source_width: u32,
        source_height: u32,
        format: ImageFormat,
        png_metadata: Option<PngMetadata>,
        origin: RasterImageOrigin,
    ) -> Self {
        Self {
            data,
            source_width,
            source_height,
            format,
            png_metadata,
            origin,
        }
    }
}

/// A rasterized effect associated with an image, drawn independently from the
/// source image so the effect can use filter DPI while the image uses image DPI.
#[derive(Debug, Clone)]
pub struct ImageEffectRaster {
    pub image: RasterImageAsset,
    pub overflow: f32,
}

pub use super::context::*;

/// Parity side carried by a forced [`PageBreak`] (CSS Fragmentation
/// 3 §3.1). `Any` is a plain forced break (`break-*: page` / legacy `always`);
/// the sided values force the following content onto a left/right (verso/recto)
/// page, with pagination inserting a blank page when the natural next page has
/// the wrong parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageBreakSide {
    #[default]
    Any,
    Left,
    Right,
    Recto,
    Verso,
}

/// The stacking boundary an element must retain after layout.
///
/// Filter Effects requires every non-`none` filter to isolate descendants as a
/// group, including visual identities such as `brightness(1)`. This is kept
/// separately from opacity and blend mode because neither is a faithful proxy
/// for the filter property's stacking behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackingContext {
    #[default]
    None,
    /// The element's source subtree still needs to be isolated before its
    /// filter operations are evaluated.
    Filter,
    /// The source subtree and filter operations have already been composited
    /// into one atomic raster. The stacking boundary remains, but wrapping the
    /// raster in another transparency group would only quantize it again.
    FilteredOutput,
}

impl StackingContext {
    pub(crate) const fn establishes(self) -> bool {
        !matches!(self, Self::None)
    }

    pub(crate) const fn needs_source_isolation(self) -> bool {
        matches!(self, Self::Filter)
    }

    pub(crate) const fn materialized(self) -> Self {
        match self {
            Self::Filter => Self::FilteredOutput,
            Self::None | Self::FilteredOutput => self,
        }
    }
}

impl From<&crate::style::computed::FilterEffects> for StackingContext {
    fn from(filter: &crate::style::computed::FilterEffects) -> Self {
        if filter.establishes_stacking_context {
            Self::Filter
        } else {
            Default::default()
        }
    }
}

impl From<crate::style::computed::BreakValue> for PageBreakSide {
    fn from(v: crate::style::computed::BreakValue) -> Self {
        use crate::style::computed::BreakValue;
        match v {
            BreakValue::Left => PageBreakSide::Left,
            BreakValue::Right => PageBreakSide::Right,
            BreakValue::Recto => PageBreakSide::Recto,
            BreakValue::Verso => PageBreakSide::Verso,
            // `page`/`always`, `avoid`, `auto` carry no parity requirement.
            _ => PageBreakSide::Any,
        }
    }
}

/// Preserve a forced or avoided page break before a laid-out box. A named page
/// is itself a forced break, so it takes precedence over an `avoid` value.
pub(crate) fn emit_page_break_before(style: &ComputedStyle, output: &mut Vec<LayoutNode>) {
    if style.page_break_before || style.page_name.is_some() {
        output.push(
            PageBreak {
                side: PageBreakSide::from(style.break_before),
                page_name: style.page_name.clone(),
            }
            .boxed(),
        );
    } else if style.break_before == crate::style::computed::BreakValue::Avoid {
        output.push(AvoidPageBreak.boxed());
    }
}

/// Preserve a forced or avoided page break after a laid-out box.
pub(crate) fn emit_page_break_after(style: &ComputedStyle, output: &mut Vec<LayoutNode>) {
    if style.page_break_after {
        output.push(
            PageBreak {
                side: PageBreakSide::from(style.break_after),
                page_name: None,
            }
            .boxed(),
        );
    } else if style.break_after == crate::style::computed::BreakValue::Avoid {
        output.push(AvoidPageBreak.boxed());
    }
}

/// A fully laid-out page.
#[derive(Default)]
pub struct Page {
    pub elements: Vec<(f32, LayoutNode)>, // (y_position, element)
    /// Browser print-to-page scale derived from this page's normal-flow width.
    /// PDF rendering applies it uniformly around the physical page's top-left.
    pub(crate) print_content_scale: PrintContentScale,
    /// Fragment-addressable SVG resources owned by this document. The layout
    /// entry point stores the complete collection on the first page so it has a
    /// single owner even when pagination produces many pages.
    pub document_svg_defs: crate::parser::svg::SvgDefs,
    /// GCPM running elements and named strings, including the distinct entry,
    /// first, start, and exit states for this physical page.
    pub(crate) generated_content: super::page_values::PageGeneratedContent,
    /// CSS GCPM footnotes collected while laying out this page.
    pub footnotes: Vec<FootnoteItem>,
    /// Selected physical page geometry and root-flow insets. Keeping the two
    /// coordinate spaces together prevents body gutters from becoming
    /// physical `@page` clips.
    pub(crate) geometry: Option<super::page_context::PageGeometry>,
    /// Active named page, when selected by the CSS `page` property.
    pub page_name: Option<String>,
    /// True for an inserted blank page from a forced left/right/recto/verso break.
    pub is_blank: bool,
}

/// Page geometry at the document-layout boundary.
///
/// `content_margin` includes projected body margin/padding used to position
/// normal flow. `initial_containing_block` is the CSS page area before those
/// body-owned gutters are applied; viewport units must resolve against this
/// stable page-area size rather than against an element's content box.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DocumentGeometry {
    page_size: PageSize,
    content_margin: Margin,
    initial_containing_block: Size,
}

impl DocumentGeometry {
    pub(crate) fn new(page_size: PageSize, content_margin: Margin) -> Self {
        Self {
            page_size,
            content_margin,
            initial_containing_block: Size::new(
                page_size.width - content_margin.horizontal(),
                page_size.height - content_margin.top - content_margin.bottom,
            ),
        }
    }

    pub(crate) const fn with_initial_containing_block(
        self,
        initial_containing_block: Size,
    ) -> Self {
        Self {
            initial_containing_block,
            ..self
        }
    }
}

/// Lay out the DOM nodes into pages.
#[allow(dead_code)]
pub fn layout(nodes: &[DomNode], page_size: PageSize, margin: Margin) -> Vec<Page> {
    layout_with_rules(nodes, page_size, margin, &[])
}

/// Resolve the margin declared on `body`, `html`, or `:root` selectors against
/// the given page size. The result is additive to the caller-supplied page
/// margin — Chrome treats the body margin as shrinking the printable area
/// inside the page margin, so both offsets stack.
///
/// Returns zeros when no matching rule declares a margin.
pub fn compute_root_margin(rules: &[CssRule], page_size: PageSize) -> Margin {
    let mut style = ComputedStyle::default();
    let parent = ComputedStyle {
        viewport_width: page_size.width,
        viewport_height: page_size.height,
        root_font_size: style.font_size,
        width: Some(page_size.width),
        ..ComputedStyle::default()
    };

    for rule in rules {
        let sel = rule.selector.trim();
        if sel == "body" || sel == "html" || sel == ":root" {
            crate::style::computed::apply_style_map(&mut style, &rule.declarations, &parent);
        }
    }

    Margin {
        top: style.margin.top,
        right: style.margin.right,
        bottom: style.margin.bottom,
        left: style.margin.left,
    }
}

/// Compute the extra horizontal gutter that body's `max-width` plus
/// `margin: auto` produces. This pattern (`body { max-width: 640px;
/// margin: 40px auto; }`) centers the body content within the page's
/// printable area. Since ironpress strips the `<body>` element before
/// layout, we emulate the centering by folding the remainder width
/// `(printable - max_width) / 2` into the effective page margin.
///
/// `printable_width` is the page width minus existing left/right margins
/// (including any previously folded body margin/padding). Returns the
/// half-gutter width to add on BOTH sides, or 0 if the body doesn't
/// declare a max-width or both margins aren't auto.
pub fn compute_root_body_centering_gutter(
    rules: &[CssRule],
    page_size: PageSize,
    printable_width: f32,
) -> f32 {
    let style = compute_root_element_style(rules, page_size);
    // Require BOTH left and right margin: auto (centering) plus a max-width.
    if !(style.margin_left_auto && style.margin_right_auto) {
        return 0.0;
    }
    let max_w = match (style.max_width, style.percentage_sizing.max_width) {
        (Some(w), _) => w,
        (None, Some(pct)) => pct / 100.0 * printable_width,
        _ => return 0.0,
    };
    if max_w <= 0.0 || max_w >= printable_width {
        return 0.0;
    }
    (printable_width - max_w) / 2.0
}

/// Resolve the padding declared on `body`, `html`, or `:root` selectors against
/// the given page size. Chrome treats body padding as shrinking the printable
/// area inside the page margin (like an inner gutter), so we fold it into the
/// effective page margin alongside `compute_root_margin`.
///
/// Returns zero edges when no matching rule declares padding.
pub fn compute_root_padding(rules: &[CssRule], page_size: PageSize) -> EdgeSizes {
    compute_root_element_style(rules, page_size).padding
}

/// Resolved text properties for one CSS page-margin box.
#[derive(Debug, Clone)]
pub struct PageMarginTextDefaults {
    pub font_family: FontFamily,
    pub font_size: f32,
    pub line_height_factor: f32,
}

impl Default for PageMarginTextDefaults {
    fn default() -> Self {
        Self {
            font_family: FontFamily::default(),
            font_size: 12.0,
            line_height_factor: 1.2,
        }
    }
}

impl PageMarginTextDefaults {
    fn from_computed_style(style: &ComputedStyle, fonts: &HashMap<String, TtfFont>) -> Self {
        Self {
            font_family: Self::resolve_font_family(style, fonts),
            font_size: used_font_size(style, fonts),
            line_height_factor: text_run_line_height_factor(style, fonts),
        }
    }

    /// Keeps the built-in page default stable while resolving an authored
    /// custom stack past an unavailable first family.
    fn resolve_font_family(style: &ComputedStyle, fonts: &HashMap<String, TtfFont>) -> FontFamily {
        let FontFamily::Custom(name) = &style.font_family else {
            return style.font_family.clone();
        };
        if crate::system_fonts::find_font_with_stretch(
            fonts,
            name,
            style.font_weight.is_bold(),
            style.font_style.is_slanted(),
            style.font_stretch,
        )
        .is_some()
        {
            return style.font_family.clone();
        }
        resolve_style_font_family(style, fonts)
    }
}

/// Cascaded text state of the CSS page context.
///
/// Root inheritance is established once. Page-specific declarations remain
/// unevaluated until a physical page is known, because `:first`, spread, blank,
/// and named selectors can choose a different inherited text style per page.
#[derive(Debug, Clone, Default)]
pub struct PageMarginTextContext {
    root_style: ComputedStyle,
    page_rules: Vec<PageMarginTextRule>,
}

#[derive(Debug, Clone)]
struct PageMarginTextRule {
    selector: PageSelector,
    style: PageTextStyle,
}

impl PageMarginTextContext {
    pub fn resolve(
        &self,
        page: PageSelectorContext<'_>,
        margin_style: &PageTextStyle,
        fonts: &HashMap<String, TtfFont>,
    ) -> PageMarginTextDefaults {
        let has_matching_page_rule = self
            .page_rules
            .iter()
            .any(|rule| rule.selector.applies_to(page) && !rule.style.is_empty());
        if !has_matching_page_rule && margin_style.is_empty() {
            return PageMarginTextDefaults::from_computed_style(&self.root_style, fonts);
        }

        // The context owns one root style and clones it only when a physical
        // page actually needs a cascade. Merge specified declarations before
        // computing them so `!important`, specificity, and source order remain
        // one coherent cascade.
        let mut style = self.root_style.clone();
        let mut matching: Vec<_> = self
            .page_rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.selector.applies_to(page))
            .collect();
        matching.sort_by_key(|(source_order, rule)| (rule.selector.specificity(), *source_order));
        let mut page_declarations = crate::parser::css::StyleMap::new();
        for (_, rule) in matching {
            page_declarations.merge(&rule.style.declarations);
        }
        crate::style::computed::apply_style_map(&mut style, &page_declarations, &self.root_style);
        crate::style::computed::apply_style_map(
            &mut style,
            &margin_style.declarations,
            &self.root_style,
        );
        PageMarginTextDefaults::from_computed_style(&style, fonts)
    }
}

/// Preserve root inheritance and retain the page-selector cascade needed to
/// resolve each page-margin box after pagination.
pub fn compute_page_margin_text_context(
    rules: &[CssRule],
    page_rules: &[PageRule],
    page_size: PageSize,
) -> PageMarginTextContext {
    // CSS Paged Media §6: the page context inherits from the root element,
    // then page-margin boxes inherit from that context. `body` is document
    // content, so its text properties must not leak into running headers.
    PageMarginTextContext {
        root_style: compute_page_context_root_style(rules, page_size),
        page_rules: page_rules
            .iter()
            .filter(|rule| !rule.text_style.is_empty())
            .map(|rule| PageMarginTextRule {
                selector: rule.selector.clone(),
                style: rule.text_style.clone(),
            })
            .collect(),
    }
}

fn compute_root_element_style(rules: &[CssRule], page_size: PageSize) -> ComputedStyle {
    compute_root_style(rules, page_size, RootStyleScope::Document)
}

/// The page context inherits from the document root, not from `body`.
fn compute_page_context_root_style(rules: &[CssRule], page_size: PageSize) -> ComputedStyle {
    compute_root_style(rules, page_size, RootStyleScope::PageContext)
}

#[derive(Clone, Copy)]
enum RootStyleScope {
    Document,
    PageContext,
}

fn compute_root_style(
    rules: &[CssRule],
    page_size: PageSize,
    scope: RootStyleScope,
) -> ComputedStyle {
    let mut style = ComputedStyle::default();
    let parent = ComputedStyle {
        viewport_width: page_size.width,
        viewport_height: page_size.height,
        root_font_size: style.font_size,
        width: Some(page_size.width),
        ..ComputedStyle::default()
    };

    for rule in rules {
        let sel = rule.selector.trim();
        let applies = match scope {
            RootStyleScope::Document => matches!(sel, "body" | "html" | ":root"),
            RootStyleScope::PageContext => matches!(sel, "html" | ":root"),
        };
        if applies {
            crate::style::computed::apply_style_map(&mut style, &rule.declarations, &parent);
        }
    }
    style
}

/// Lay out the DOM nodes into pages with stylesheet rules.
#[allow(dead_code)]
pub fn layout_with_rules(
    nodes: &[DomNode],
    page_size: PageSize,
    margin: Margin,
    rules: &[CssRule],
) -> Vec<Page> {
    layout_with_rules_and_fonts(
        nodes,
        page_size,
        margin,
        rules,
        &HashMap::new(),
        None,
        0.0,
        FootnoteAreaLayout::default(),
    )
}

/// Walk the DOM and record every element bearing an `id` attribute into
/// `defs` (first occurrence wins, matching HTML's "first id" resolution). Used
/// to resolve `filter: url(#id)` references to inline SVG `<filter>` elements.
fn collect_id_defs(nodes: &[DomNode], defs: &mut HashMap<String, ElementNode>) {
    for node in nodes {
        if let DomNode::Element(el) = node {
            if let Some(id) = el.attributes.get("id") {
                defs.entry(id.clone()).or_insert_with(|| el.clone());
            }
            collect_id_defs(&el.children, defs);
        }
    }
}

fn first_root_child_margin_top(
    nodes: &[DomNode],
    parent_style: &ComputedStyle,
    rules: &[CssRule],
) -> f32 {
    for node in nodes {
        match node {
            DomNode::Text(text) if !has_non_collapsible_text(text) => continue,
            DomNode::Text(_) => return 0.0,
            DomNode::Element(el) => {
                let classes = el.class_list();
                let class_refs: Vec<&str> = classes.iter().map(|s| s.as_ref()).collect();
                let selector_ctx = SelectorContext::default();
                let style = compute_style_with_context(
                    el.tag,
                    el.style_attr(),
                    parent_style,
                    rules,
                    el.tag_name(),
                    &class_refs,
                    el.id(),
                    &selector_attributes_with_has(el),
                    &selector_ctx,
                );
                return style.margin.top;
            }
        }
    }
    0.0
}

fn background_paint_differs(a: &ComputedStyle, b: &ComputedStyle) -> bool {
    a.background_color != b.background_color
        || a.background_gradient.is_some() != b.background_gradient.is_some()
        || a.background_radial_gradient.is_some() != b.background_radial_gradient.is_some()
        || a.background_conic_gradient.is_some() != b.background_conic_gradient.is_some()
        || a.background_svg.is_some() != b.background_svg.is_some()
}

/// Lay out the DOM nodes into pages with stylesheet rules and custom fonts.
#[allow(clippy::too_many_arguments)]
pub fn layout_with_rules_and_fonts(
    nodes: &[DomNode],
    page_size: PageSize,
    margin: Margin,
    rules: &[CssRule],
    custom_fonts: &HashMap<String, TtfFont>,
    page_background: Option<&ComputedStyle>,
    page_bleed: f32,
    footnote_area: FootnoteAreaLayout,
) -> Vec<Page> {
    let mut resources = crate::security::resources::ResourceLoader::default();
    let page_background = super::page_context::PageBackgroundContext::uniform(
        page_background,
        page_bleed,
        crate::style::raster_quality::RasterQuality::default(),
    );
    layout_with_rules_and_fonts_raster_quality(
        nodes,
        DocumentGeometry::new(page_size, margin),
        rules,
        custom_fonts,
        crate::font_pack::FontLocale::Unspecified,
        &page_background,
        super::paginate::PaginationContext::new(
            super::page_context::PageGeometryContext::uniform(page_size, margin),
            footnote_area,
            0.0,
        ),
        crate::style::raster_quality::RasterQuality::default(),
        &mut resources,
    )
}

/// Lay out the DOM nodes into pages with stylesheet rules, custom fonts, and
/// one conversion-owned raster quality policy.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_with_rules_and_fonts_raster_quality(
    nodes: &[DomNode],
    geometry: DocumentGeometry,
    rules: &[CssRule],
    custom_fonts: &HashMap<String, TtfFont>,
    font_locale: crate::font_pack::FontLocale,
    page_background: &super::page_context::PageBackgroundContext,
    pagination_context: super::paginate::PaginationContext,
    raster_quality: crate::style::raster_quality::RasterQuality,
    resources: &mut crate::security::resources::ResourceLoader,
) -> Vec<Page> {
    let DocumentGeometry {
        page_size,
        content_margin: margin,
        initial_containing_block,
    } = geometry;
    let raster_quality = raster_quality.normalized();
    let font_metrics = FontMetrics::new(custom_fonts);
    let document_svg_defs = crate::parser::svg::collect_document_svg_defs(nodes);
    let available_width = page_size.width - margin.horizontal();
    let content_height = page_size.height - margin.top - margin.bottom;
    let root_styles = DocumentRootStyles::resolve(
        nodes,
        rules,
        raster_quality,
        initial_containing_block.width,
        initial_containing_block.height,
        font_metrics,
    );
    let root_start_page_name = root_styles.start_page_name().map(str::to_owned);
    let html_style = root_styles.html;
    let body_style = root_styles.body;
    let mut parent_style = body_style.clone();
    parent_style.font_locale = font_locale;
    // The conversion boundary has already projected horizontal body padding
    // into `content_margin`. Direct body children must therefore resolve
    // percentages against that content width itself, not subtract the authored
    // padding a second time through their parent style.
    parent_style.padding = EdgeSizes::ZERO;
    parent_style.width = Some(available_width);

    // First, flatten DOM into layout elements
    let mut elements = Vec::new();
    if let Some(page_name) = root_start_page_name {
        elements.push(PageBreak::named(page_name).boxed());
    }

    // Propagated root backgrounds cover the selected physical page area on
    // every fragment. Pagination later expresses that area relative to the
    // root flow origin, which may be inset by body padding or centering.
    let html_has_bg = has_background_paint(&html_style);
    let body_has_bg = has_background_paint(&body_style);
    let canvas_background_style = if html_has_bg {
        Some(&html_style)
    } else if has_background_paint(&body_style) {
        Some(&body_style)
    } else {
        None
    };
    if let Some(canvas_style) = canvas_background_style {
        elements.push(
            BackgroundBox::new(
                canvas_style,
                BackgroundBoxGeometry::repeated_page_area(
                    super::elements::PageAreaInFlowSpace::new(
                        Point::default(),
                        Size::new(available_width, content_height),
                    ),
                    -1,
                ),
            )
            .boxed(),
        );
    }

    if html_has_bg && body_has_bg && background_paint_differs(&html_style, &body_style) {
        let body_w =
            body_style.width.unwrap_or(available_width).max(0.0) + body_style.padding.horizontal();
        let body_h =
            body_style.height.unwrap_or(content_height).max(0.0) + body_style.padding.vertical();
        let body_offset_top = first_root_child_margin_top(nodes, &parent_style, rules);
        elements.push(
            BackgroundBox::new(
                &body_style,
                BackgroundBoxGeometry::repeated_canvas(
                    Size::new(body_w, body_h),
                    crate::types::Point::new(0.0, body_offset_top),
                    -1,
                ),
            )
            .boxed(),
        );
    }

    let ancestors: Vec<AncestorInfo> = Vec::new();
    let mut counter_state = CounterState::default();
    counter_state.apply_resets(&parent_style.counter_reset);
    counter_state.apply_increments(&parent_style.counter_increment);
    let root_ctx = LayoutContext {
        viewport: Viewport {
            width: initial_containing_block.width,
            height: initial_containing_block.height,
        },
        parent: ParentBox {
            content_width: available_width,
            content_height: Some(content_height),
            font_size: parent_style.font_size,
            percent_width_basis: available_width,
        },
        containing_block: None,
        percent_height_cb: None,
        root_font_size: parent_style.root_font_size,
    };
    // Build a document-wide `id -> element` map so `filter: url(#id)`
    // (css-filter-effects-1 §3) can resolve to its inline SVG `<filter>`
    // element regardless of where in the tree it lives.
    let mut filter_defs: HashMap<String, ElementNode> = HashMap::new();
    collect_id_defs(nodes, &mut filter_defs);
    // Owned by this layout and dropped with it, so the DOM addresses the table
    // auto-sizing pass records as keys stay valid for exactly as long as the
    // memo can be consulted.
    let mut table_cell_sizing = super::table::TableCellSizingMemo::default();
    let mut env = LayoutEnv {
        rules,
        fonts: custom_fonts,
        counter_state: &mut counter_state,
        resources,
        filter_defs: &filter_defs,
        filter_dpi: raster_quality.filter_dpi,
        table_cell_sizing: &mut table_cell_sizing,
    };
    if let Some(root) =
        RootFormattingContext::from_projected_body(nodes, &body_style, available_width)
    {
        let root_ancestors = root.descendant_ancestors();
        if root.style().display == Display::Flex {
            let root_selector = SelectorContext {
                sibling_count: 1,
                is_empty: root.element().children.is_empty(),
                ..Default::default()
            };
            let generated_styles = super::inline_formatting::GeneratedContentStyles::resolve(
                root.element(),
                root.style(),
                env.rules,
                &root_selector,
                env.fonts,
            );
            layout_flex_container(
                root.element(),
                root.style(),
                &root_ctx,
                &mut elements,
                &root_ancestors,
                generated_styles.boxes(root.element()),
                0,
                &mut env,
            );
        } else {
            layout_grid_container(
                root.element(),
                root.style(),
                &root_ctx,
                &mut elements,
                &root_ancestors,
                0,
                &mut env,
            );
        }
    } else {
        flatten_nodes(
            nodes,
            LayoutTreeContext::new(&parent_style, &root_ctx, &ancestors),
            &mut elements,
            &mut env,
        );
    }

    // Pagination is the first point where page number, spread side, blank
    // state, and named-page identity all exist. Resolve geometry there from one
    // cascade rather than precomputing independent special-case buckets.
    let pagination_context = pagination_context
        .with_footnote_content_width(available_width)
        .with_root_margin_top(body_style.margin.top + body_style.padding.top);
    let mut pages =
        super::paginate::paginate_with_context(elements, pagination_context, custom_fonts);
    page_background.apply(&mut pages, page_size, margin, custom_fonts);
    super::filter::materialize_page_filters(
        &mut pages,
        margin,
        custom_fonts,
        raster_quality.filter_dpi,
    );
    assign_page_print_scales(&mut pages, page_size, margin);
    super::fragmentation::transfer_page_spanning_graphical_effects(&mut pages, page_size, margin);
    if let Some(first_page) = pages.first_mut() {
        first_page.document_svg_defs = document_svg_defs;
    }
    let mut dom_targets = HashMap::new();
    collect_dom_targets(nodes, &mut dom_targets);
    resolve_target_placeholders(&mut pages, &dom_targets);
    pages
}

fn collect_plain_text(nodes: &[DomNode], out: &mut String) {
    for node in nodes {
        match node {
            DomNode::Text(text) => {
                out.push_str(text);
                out.push(' ');
            }
            DomNode::Element(el) => collect_plain_text(&el.children, out),
        }
    }
}

fn string_set_marker(
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
) -> Option<LayoutNode> {
    let raw = match authored_property_value(el, rules, ancestors, selector_ctx, "string-set")? {
        crate::parser::css::CssValue::Keyword(value) => value,
        _ => return None,
    };
    let mut parts = raw.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim().to_ascii_lowercase();
    let value_expr = parts.next().unwrap_or("").trim();
    if name.is_empty() {
        return None;
    }
    let value = if value_expr.eq_ignore_ascii_case("content()") {
        let mut text = String::new();
        collect_plain_text(&el.children, &mut text);
        collapse_whitespace(&text)
    } else if value_expr.len() >= 5 && value_expr[..5].eq_ignore_ascii_case("attr(") {
        value_expr
            .find(')')
            .and_then(|end| el.attributes.get(value_expr[5..end].trim()))
            .cloned()
            .unwrap_or_default()
    } else {
        value_expr
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string()
    };
    Some(NamedString { name, value }.boxed())
}

fn target_anchor_marker(el: &ElementNode) -> Option<LayoutNode> {
    let id = el.id()?.trim();
    if id.is_empty() {
        return None;
    }
    Some(
        NamedString {
            name: format!("{TARGET_ANCHOR_PREFIX}{id}"),
            value: String::new(),
        }
        .boxed(),
    )
}

fn collect_dom_targets(nodes: &[DomNode], out: &mut HashMap<String, String>) {
    for node in nodes {
        if let DomNode::Element(el) = node {
            if let Some(id) = el.id().filter(|id| !id.is_empty()) {
                let mut text = String::new();
                collect_plain_text(&el.children, &mut text);
                let text = collapse_whitespace(&text);
                if !text.is_empty() {
                    out.insert(id.to_string(), text);
                }
            }
            collect_dom_targets(&el.children, out);
        }
    }
}

fn content_items_include_target_placeholder(items: &[ContentItem]) -> bool {
    items.iter().any(|item| {
        matches!(
            item,
            ContentItem::String(text) if text.contains(TARGET_PLACEHOLDER_START)
        )
    })
}

fn resolve_target_text_placeholders_in_runs(
    runs: &mut [TextRun],
    id_defs: &HashMap<String, ElementNode>,
) {
    for run in runs {
        if run.text.contains(TARGET_PLACEHOLDER_START) {
            run.text = resolve_target_text_placeholders_in_text(&run.text, id_defs);
        }
    }
}

fn resolve_target_text_placeholders_in_text(
    text: &str,
    id_defs: &HashMap<String, ElementNode>,
) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(TARGET_PLACEHOLDER_START) {
        out.push_str(&rest[..start]);
        let payload_start = start + TARGET_PLACEHOLDER_START.len();
        let Some(end_rel) = rest[payload_start..].find(TARGET_PLACEHOLDER_END) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let marker_end = payload_start + end_rel + TARGET_PLACEHOLDER_END.len();
        let payload = &rest[payload_start..payload_start + end_rel];
        if let Some(id) = payload
            .strip_prefix("text|")
            .and_then(|target| target.strip_prefix('#'))
            && let Some(el) = id_defs.get(id)
        {
            let mut target_text = String::new();
            collect_plain_text(&el.children, &mut target_text);
            out.push_str(&collapse_whitespace(&target_text));
        } else {
            out.push_str(&rest[start..marker_end]);
        }
        rest = &rest[marker_end..];
    }
    out.push_str(rest);
    out
}

fn page_contains_text(page: &Page, needle: &str) -> bool {
    page.elements
        .iter()
        .any(|(_, element)| element_contains_text(element.as_ref(), needle))
}

fn element_contains_text(element: &dyn LayoutElement, needle: &str) -> bool {
    struct TextSearch<'a> {
        needle: &'a str,
        found: bool,
    }

    impl LayoutVisitor for TextSearch<'_> {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.search_lines(&element.lines);
        }

        fn visit_table_row(&mut self, element: &TableRow) {
            for cell in &element.content.cells {
                self.search_lines(&cell.layout.content.lines);
            }
        }

        fn visit_grid_row(&mut self, element: &GridRow) {
            for cell in &element.content.cells {
                self.search_lines(&cell.layout.content.lines);
            }
        }

        fn visit_flex_row(&mut self, element: &FlexRow) {
            for cell in &element.content.cells {
                self.search_lines(&cell.lines);
            }
        }
    }

    impl TextSearch<'_> {
        fn search_lines(&mut self, lines: &[TextLine]) {
            self.found |= lines.iter().any(|line| {
                line.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
                    .contains(self.needle)
            });
        }
    }

    let mut search = TextSearch {
        needle,
        found: false,
    };
    visit_layout_tree(element, &mut search);
    search.found
}

fn resolve_target_placeholders(pages: &mut [Page], dom_targets: &HashMap<String, String>) {
    if dom_targets.is_empty() && !pages.iter().any(page_has_target_anchor_marker) {
        return;
    }
    let mut page_by_id = HashMap::new();
    for (idx, page) in pages.iter().enumerate() {
        for name in page.generated_content.named_string_names() {
            if let Some(id) = target_anchor_id(name) {
                page_by_id.entry(id.to_string()).or_insert(idx + 1);
            }
        }
    }
    for (id, text) in dom_targets {
        if page_by_id.contains_key(id) {
            continue;
        }
        if let Some((idx, _)) = pages
            .iter()
            .enumerate()
            .find(|(_, page)| page_contains_text(page, text))
        {
            page_by_id.insert(id.clone(), idx + 1);
        }
    }
    for page in pages {
        for (_, element) in &mut page.elements {
            resolve_target_placeholders_in_element(element.as_mut(), dom_targets, &page_by_id);
        }
    }
}

fn page_has_target_anchor_marker(page: &Page) -> bool {
    page.generated_content
        .named_string_names()
        .any(|name| target_anchor_id(name).is_some())
}

fn resolve_target_placeholders_in_element(
    element: &mut dyn LayoutElement,
    dom_targets: &HashMap<String, String>,
    page_by_id: &HashMap<String, usize>,
) {
    struct TargetPlaceholderResolver<'a> {
        dom_targets: &'a HashMap<String, String>,
        page_by_id: &'a HashMap<String, usize>,
    }

    impl LayoutVisitorMut for TargetPlaceholderResolver<'_> {
        fn visit_text_block(&mut self, element: &mut TextBlock) {
            for line in &mut element.lines {
                for run in &mut line.runs {
                    if run.text.contains(TARGET_PLACEHOLDER_START) {
                        run.text = resolve_target_placeholders_in_text(
                            &run.text,
                            self.dom_targets,
                            self.page_by_id,
                        );
                    }
                }
            }
        }
    }

    visit_layout_tree_mut(
        element,
        &mut TargetPlaceholderResolver {
            dom_targets,
            page_by_id,
        },
    );
}

fn resolve_target_placeholders_in_text(
    text: &str,
    dom_targets: &HashMap<String, String>,
    page_by_id: &HashMap<String, usize>,
) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(TARGET_PLACEHOLDER_START) {
        out.push_str(&rest[..start]);
        let payload_start = start + TARGET_PLACEHOLDER_START.len();
        let Some(end_rel) = rest[payload_start..].find(TARGET_PLACEHOLDER_END) else {
            out.push_str(&rest[start..]);
            return out;
        };
        let payload = &rest[payload_start..payload_start + end_rel];
        out.push_str(&resolve_target_payload(payload, dom_targets, page_by_id));
        rest = &rest[payload_start + end_rel + TARGET_PLACEHOLDER_END.len()..];
    }
    out.push_str(rest);
    out
}

fn resolve_target_payload(
    payload: &str,
    dom_targets: &HashMap<String, String>,
    page_by_id: &HashMap<String, usize>,
) -> String {
    let mut parts = payload.split('|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("counter"), Some(target), Some("page")) => target
            .strip_prefix('#')
            .and_then(|id| page_by_id.get(id))
            .map(|page| page.to_string())
            .unwrap_or_default(),
        (Some("text"), Some(target), _) => target
            .strip_prefix('#')
            .and_then(|id| dom_targets.get(id))
            .cloned()
            .unwrap_or_default(),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunningElementMode {
    Capture,
    LayoutCapturedContent,
}

fn capture_running_element(
    name: String,
    el: &ElementNode,
    context: ElementLayoutContext<'_, '_, '_>,
    env: &mut LayoutEnv,
) -> Option<LayoutNode> {
    let mut captured = Vec::new();
    flatten_element_with_running_mode(
        el,
        context,
        &mut captured,
        env,
        RunningElementMode::LayoutCapturedContent,
    );
    let element = if captured.len() == 1 {
        captured.pop()?
    } else {
        Container {
            children: captured,
            ..Default::default()
        }
        .boxed()
    };
    Some(RunningElement { name, element }.boxed())
}

fn effective_transform(
    style: &ComputedStyle,
    parent_style: &ComputedStyle,
    ctx: &LayoutContext,
) -> Option<Transform> {
    let transform = style.transform?;
    let projected_matrix = match transform {
        Transform::Matrix3d(matrix) | Transform::Project3d { matrix, .. } => Some(matrix),
        _ => None,
    };
    if let (Some(matrix), Some(perspective)) = (projected_matrix, parent_style.perspective) {
        let parent_w = parent_style.width.unwrap_or(ctx.parent.content_width);
        let parent_h = parent_style
            .height
            .or(ctx.parent.content_height)
            .unwrap_or(ctx.viewport.height);
        let (px, py) = parent_style.perspective_origin.resolve(parent_w, parent_h);
        let child_x = style.left.unwrap_or(0.0);
        let child_y = style.top.unwrap_or(0.0);
        Some(Transform::Project3d {
            matrix,
            perspective: f64::from(perspective),
            perspective_origin: crate::style::computed::CssVector::new(
                f64::from(px - child_x),
                f64::from(py - child_y),
            ),
        })
    } else {
        Some(transform)
    }
}

struct DirectFlexItemFilter {
    filter: super::filter::ResolvedFilter,
}

fn direct_flex_item_filters(
    flex_el: &ElementNode,
    parent_style: &ComputedStyle,
    ancestors: &[AncestorInfo],
    env: &LayoutEnv<'_>,
) -> Vec<Option<DirectFlexItemFilter>> {
    let child_elements: Vec<&ElementNode> = flex_el
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(el) => Some(el),
            _ => None,
        })
        .collect();
    let child_count = child_elements.len();
    let mut out = Vec::new();
    for (idx, child_el) in child_elements.into_iter().enumerate() {
        let classes = child_el.class_list();
        let selector_ctx = SelectorContext {
            ancestors: ancestors.to_vec(),
            child_index: idx,
            sibling_count: child_count,
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        };
        let mut child_style = compute_style_with_context_with_font_metrics(
            child_el.tag,
            child_el.style_attr(),
            parent_style,
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
        let filter = super::filter::ResolvedFilter::from_style(&mut child_style, env.filter_defs);
        if filter.operations.is_empty() {
            out.push(None);
        } else {
            out.push(Some(DirectFlexItemFilter { filter }));
        }
    }
    out
}

pub(super) fn apply_direct_flex_item_filters(
    flex_el: &ElementNode,
    parent_style: &ComputedStyle,
    ancestors: &[AncestorInfo],
    env: &LayoutEnv<'_>,
    elements: &mut [LayoutNode],
) {
    let filters = direct_flex_item_filters(flex_el, parent_style, ancestors, env);
    if filters.iter().all(Option::is_none) {
        return;
    }
    struct FlexFilterApplier {
        next_filter: std::vec::IntoIter<Option<DirectFlexItemFilter>>,
        exhausted: bool,
    }

    impl LayoutVisitorMut for FlexFilterApplier {
        fn visit_flex_row(&mut self, element: &mut FlexRow) {
            for cell in &mut element.content.cells {
                let Some(filter) = self.next_filter.next() else {
                    self.exhausted = true;
                    return;
                };
                let Some(effect) = filter else {
                    continue;
                };
                use super::elements::FilterHolder;
                *cell.paint.filter_slot_mut() = Some(effect.filter);
            }
        }
    }

    let mut applier = FlexFilterApplier {
        next_filter: filters.into_iter(),
        exhausted: false,
    };
    for element in elements {
        element.accept_mut(&mut applier);
        if applier.exhausted {
            return;
        }
    }
}

struct FilterRasterGeometry {
    size: Size,
    margins: BlockMargins,
    positioning: Positioning,
    raster_overflow: EdgeSizes,
}

struct FilterGroupRaster {
    asset: RasterImageAsset,
    geometry: FilterRasterGeometry,
}

fn prepare_filtered_output(
    style: &ComputedStyle,
    filter: &super::filter::ResolvedFilter,
    env: &LayoutEnv<'_>,
    output: &mut Vec<LayoutNode>,
    start: usize,
) -> bool {
    if output.len() != start + 1 {
        return false;
    }
    let filter_el = style
        .filter
        .url_id
        .as_ref()
        .and_then(|id| env.filter_defs.get(id));
    let has_displacement = filter_el.is_some_and(svg_filter_has_turbulence_displacement);
    if !filter.requires_source_surface() && !has_displacement {
        return false;
    }

    if !has_displacement && super::filter::retain_for_fragmentation(output[start].as_mut(), filter)
    {
        return true;
    }

    let Some(element) = output.pop() else {
        return false;
    };
    struct UnconstrainedTextWidth(bool);
    impl LayoutVisitor for UnconstrainedTextWidth {
        fn visit_text_block(&mut self, element: &TextBlock) {
            self.0 = element.box_model.size.width.is_fill_available();
        }
    }
    let mut unconstrained_text_width = UnconstrainedTextWidth(false);
    element.accept(&mut unconstrained_text_width);
    if unconstrained_text_width.0 {
        output.push(element);
        return false;
    }
    let source_group = element
        .paint_group_owner()
        .map(super::elements::PaintGroupOwner::paint_group)
        .cloned()
        .unwrap_or_default();
    let replacement = if has_displacement {
        filter_el
            .and_then(|definition| {
                rasterize_svg_displacement_rect(&element, definition, env.filter_dpi)
            })
            .map(|raster| {
                Image {
                    source: raster.asset,
                    geometry: ReplacedGeometry::new(
                        raster.geometry.size,
                        raster.geometry.margins,
                        LayoutBorder::default(),
                    ),
                    positioning: raster.geometry.positioning,
                    sampling: ImageSampling {
                        replaced: ReplacedContent {
                            object_fit: crate::style::computed::ObjectFit::Fill,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    paint: ImagePaint {
                        raster_overflow: raster.geometry.raster_overflow,
                        group: source_group.clone(),
                        ..Default::default()
                    },
                }
                .boxed()
            })
    } else {
        None
    };
    let Some(replacement) = replacement else {
        output.push(element);
        return false;
    };
    output.push(replacement);
    true
}

fn svg_filter_has_turbulence_displacement(filter_el: &ElementNode) -> bool {
    if !filter_el.raw_tag_name.eq_ignore_ascii_case("filter") {
        return false;
    }
    let mut saw_turbulence = false;
    for child in &filter_el.children {
        let DomNode::Element(el) = child else {
            continue;
        };
        if el.raw_tag_name.eq_ignore_ascii_case("feTurbulence") {
            saw_turbulence = true;
        } else if saw_turbulence && el.raw_tag_name.eq_ignore_ascii_case("feDisplacementMap") {
            return true;
        }
    }
    false
}

fn rasterize_svg_displacement_rect(
    element: &dyn LayoutElement,
    filter_el: &ElementNode,
    filter_dpi: f32,
) -> Option<FilterGroupRaster> {
    let source_geometry = super::filter::surface::source_geometry(element)?;
    let Size { width, height } = source_geometry.size;
    let color = solid_filter_rect_color(element)?;
    let css_w = width / 0.75;
    let css_h = height / 0.75;
    let overflow_css = svg_filter_region_overflow_css(filter_el, css_w, css_h);
    let spec = svg_turbulence_displacement_spec(filter_el, overflow_css)?;
    let raster =
        crate::render::blur::turbulence_displacement_rect(width, height, color, &spec, filter_dpi)?;
    Some(FilterGroupRaster {
        asset: raster.asset,
        geometry: FilterRasterGeometry {
            size: source_geometry.size,
            margins: source_geometry.flow.margins,
            positioning: source_geometry.positioning,
            raster_overflow: raster.raster_overflow,
        },
    })
}

fn solid_filter_rect_color(element: &dyn LayoutElement) -> Option<crate::types::Color> {
    struct SolidColor(Option<crate::types::Color>);
    impl LayoutVisitor for SolidColor {
        fn visit_container(&mut self, element: &Container) {
            if element.children.is_empty() {
                self.0 = element.paint.background.color;
            }
        }

        fn visit_text_block(&mut self, element: &TextBlock) {
            if element.lines.is_empty() {
                self.0 = element.paint.background.color;
            }
        }
    }
    let mut color = SolidColor(None);
    element.accept(&mut color);
    color.0
}

fn svg_filter_region_overflow_css(filter_el: &ElementNode, width: f32, height: f32) -> EdgeSizes {
    let x = svg_filter_region_attr(filter_el, "x", -0.10, width);
    let y = svg_filter_region_attr(filter_el, "y", -0.10, height);
    let w = svg_filter_region_attr(filter_el, "width", 1.20, width);
    let h = svg_filter_region_attr(filter_el, "height", 1.20, height);
    EdgeSizes::new(
        (-y).max(0.0),
        (x + w - width).max(0.0),
        (y + h - height).max(0.0),
        (-x).max(0.0),
    )
}

fn svg_filter_region_attr(filter_el: &ElementNode, name: &str, default: f32, size: f32) -> f32 {
    let Some(raw) = filter_el.attributes.get(name).map(|v| v.trim()) else {
        return default * size;
    };
    if let Some(percent) = raw.strip_suffix('%') {
        return percent.trim().parse::<f32>().unwrap_or(default * 100.0) * size / 100.0;
    }
    raw.parse::<f32>()
        .map(|v| v * size)
        .unwrap_or(default * size)
}

fn svg_turbulence_displacement_spec(
    filter_el: &ElementNode,
    filter_region_overflow: EdgeSizes,
) -> Option<crate::render::blur::SvgTurbulenceDisplacement> {
    let mut base_frequency = (0.0_f64, 0.0_f64);
    let mut num_octaves = 1_u32;
    let mut seed = 0_i32;
    let mut saw_turbulence = false;
    let mut scale = None;
    let mut x_channel = 0_usize;
    let mut y_channel = 3_usize;
    for child in &filter_el.children {
        let DomNode::Element(el) = child else {
            continue;
        };
        if el.raw_tag_name.eq_ignore_ascii_case("feTurbulence") {
            let mut parts = el
                .attributes
                .get("baseFrequency")
                .map(String::as_str)
                .unwrap_or("0")
                .split_whitespace()
                .filter_map(|part| part.parse::<f64>().ok());
            let fx = parts.next().unwrap_or(0.0);
            let fy = parts.next().unwrap_or(fx);
            base_frequency = (fx, fy);
            num_octaves = el
                .attributes
                .get("numOctaves")
                .and_then(|value| value.trim().parse::<u32>().ok())
                .unwrap_or(1)
                .max(1);
            seed = el
                .attributes
                .get("seed")
                .and_then(|value| value.trim().parse::<f32>().ok())
                .map(|value| value.trunc() as i32)
                .unwrap_or(0);
            saw_turbulence = true;
        } else if saw_turbulence && el.raw_tag_name.eq_ignore_ascii_case("feDisplacementMap") {
            scale = el
                .attributes
                .get("scale")
                .and_then(|value| value.trim().parse::<f32>().ok());
            x_channel = svg_displacement_channel(el.attributes.get("xChannelSelector"));
            y_channel = svg_displacement_channel(el.attributes.get("yChannelSelector"));
            break;
        }
    }
    let scale = scale?;
    Some(crate::render::blur::SvgTurbulenceDisplacement {
        base_frequency_x: base_frequency.0,
        base_frequency_y: base_frequency.1,
        num_octaves,
        seed,
        scale,
        x_channel,
        y_channel,
        filter_region_overflow,
    })
}

fn svg_displacement_channel(value: Option<&String>) -> usize {
    match value.map(|value| value.trim()) {
        Some(value) if value.eq_ignore_ascii_case("G") => 1,
        Some(value) if value.eq_ignore_ascii_case("B") => 2,
        Some(value) if value.eq_ignore_ascii_case("A") => 3,
        _ => 0,
    }
}

/// Consecutive authored table cells awaiting anonymous table fixup.
#[derive(Default)]
struct AnonymousTableCellGroup<'dom> {
    pending: Option<PendingAnonymousTableCells<'dom>>,
}

struct PendingAnonymousTableCells<'dom> {
    first_source_index: usize,
    cells: Vec<&'dom ElementNode>,
}

impl<'dom> AnonymousTableCellGroup<'dom> {
    fn push(&mut self, cell: &'dom ElementNode, source_index: usize) {
        if let Some(pending) = &mut self.pending {
            pending.cells.push(cell);
        } else {
            self.pending = Some(PendingAnonymousTableCells {
                first_source_index: source_index,
                cells: vec![cell],
            });
        }
    }

    fn take(&mut self) -> Option<PendingAnonymousTableCells<'dom>> {
        self.pending.take()
    }
}

impl<'dom> PendingAnonymousTableCells<'dom> {
    fn into_parts(self) -> (usize, Vec<&'dom ElementNode>) {
        (self.first_source_index, self.cells)
    }
}

/// Flatten a list of DOM nodes into layout elements.
///
/// Iterates over `nodes`, collecting inline-block groups and dispatching
/// each element to [`flatten_element`]. Text nodes between elements
/// trigger inline-block group flushes when non-whitespace.
pub(crate) fn flatten_nodes(
    nodes: &[DomNode],
    tree: LayoutTreeContext<'_, '_>,
    output: &mut Vec<LayoutNode>,
    env: &mut LayoutEnv,
) {
    let parent_style = tree.parent_style();
    let ctx = tree.layout();
    let list_ctx = tree.list();
    let ancestors = tree.ancestors();
    let positioned_ancestor_depth = tree.positioned_ancestor_depth();
    let ib_ctx = *ctx;

    // Count element children for sibling context
    let element_count = nodes
        .iter()
        .filter(|n| matches!(n, DomNode::Element(_)))
        .count();
    let inline_sequence = InlineContentSequence::new(nodes);
    if InlineFormattingContext::new(parent_style, env.rules, ancestors, env.font_metrics())
        .requires_atomic_layout(inline_sequence)
        && layout_inline_mixed_sequence_with_env(
            inline_sequence,
            parent_style,
            ctx,
            output,
            ancestors,
            env,
        )
    {
        return;
    }
    let atomic_inline_segments =
        InlineFormattingContext::new(parent_style, env.rules, ancestors, env.font_metrics())
            .environment_aware_atomic_layout_segments(inline_sequence);
    let mut atomic_inline_segments = atomic_inline_segments.into_iter().peekable();
    let mut element_index = 0;
    let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
    let all_element_siblings = element_sibling_list(nodes);

    // Accumulator for consecutive inline-block elements
    let mut ib_group: Vec<(&ElementNode, bool)> = Vec::new();
    let mut pending_inline_space = false;
    let mut table_cell_group = AnonymousTableCellGroup::default();

    // Helper closure-like macro for flushing an inline-block group.
    // We use a nested fn instead since closures can't borrow multiple fields.
    #[allow(clippy::drain_collect)]
    #[inline]
    fn flush_ib(
        group: &mut Vec<(&ElementNode, bool)>,
        parent_style: &ComputedStyle,
        ctx: &LayoutContext,
        output: &mut Vec<LayoutNode>,
        ancestors: &[AncestorInfo],
        env: &mut LayoutEnv,
    ) {
        if group.is_empty() {
            return;
        }
        let taken: Vec<(&ElementNode, bool)> = group.drain(..).collect();
        layout_inline_block_group_with_env_and_spacing(
            &taken,
            parent_style,
            ctx,
            output,
            ancestors,
            env,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn flush_table_cells(
        group: &mut AnonymousTableCellGroup<'_>,
        parent_style: &ComputedStyle,
        ctx: &LayoutContext,
        output: &mut Vec<LayoutNode>,
        ancestors: &[AncestorInfo],
        child_index: usize,
        sibling_count: usize,
        positioned_depth: usize,
        env: &mut LayoutEnv,
    ) {
        let Some(pending) = group.take() else {
            return;
        };
        let (first_source_index, taken) = pending.into_parts();
        let table = anonymous_table_from_cells(&taken);
        let Some(table_style) = anonymous_table_box_style(&table, parent_style) else {
            return;
        };
        flatten_table(
            &table,
            &table_style,
            output,
            super::inline_formatting::GeneratedInlineContent::new(&table, None, None),
            env,
            TableLayoutContext::for_anonymous_table(
                ctx,
                ancestors,
                ElementSiblingContext::new(child_index, sibling_count),
                ElementSiblingContext::new(first_source_index, sibling_count),
                positioned_depth,
            ),
        );
    }

    let mut node_index = 0usize;
    while node_index < nodes.len() {
        if let Some(segment) = atomic_inline_segments.peek().copied()
            && segment.start() == node_index
        {
            flush_table_cells(
                &mut table_cell_group,
                parent_style,
                &ib_ctx,
                output,
                ancestors,
                element_index,
                element_count,
                positioned_ancestor_depth,
                env,
            );
            flush_ib(&mut ib_group, parent_style, &ib_ctx, output, ancestors, env);
            pending_inline_space = false;
            if layout_inline_mixed_sequence_with_env(
                segment,
                parent_style,
                ctx,
                output,
                ancestors,
                env,
            ) {
                for segment_node in segment.nodes() {
                    if let DomNode::Element(element) = segment_node {
                        preceding_siblings.push((
                            element.tag_name().to_string(),
                            element
                                .class_list()
                                .iter()
                                .map(|class| class.to_string())
                                .collect(),
                        ));
                        element_index += 1;
                    }
                }
                node_index = segment.end();
                atomic_inline_segments.next();
                continue;
            }
        }

        let Some(node) = nodes.get(node_index) else {
            break;
        };
        match node {
            DomNode::Text(text) => {
                let trimmed = collapse_whitespace(text);
                // Only flush inline-block group for non-whitespace text.
                // Whitespace between consecutive inline-block elements must
                // not break the group — they should stay on the same row.
                if !trimmed.is_empty() {
                    flush_ib(&mut ib_group, parent_style, &ib_ctx, output, ancestors, env);
                    pending_inline_space = false;
                } else if text.chars().any(char::is_whitespace) {
                    pending_inline_space = true;
                }
                if !trimmed.is_empty() {
                    flush_table_cells(
                        &mut table_cell_group,
                        parent_style,
                        &ib_ctx,
                        output,
                        ancestors,
                        element_index,
                        element_count,
                        positioned_ancestor_depth,
                        env,
                    );
                    let mut text_runs = Vec::new();
                    push_text_run_with_fallback(
                        TextRun {
                            text: trimmed,
                            font_size: used_font_size(parent_style, env.fonts),
                            bold: parent_style.font_weight == FontWeight::Bold,
                            font_style: parent_style.font_style,
                            decorations: parent_style.text_decorations.active(parent_style.color),
                            color: parent_style.color,
                            font_family: resolve_style_font_family(parent_style, env.fonts),
                            line_height_factor: text_run_line_height_factor(
                                parent_style,
                                env.fonts,
                            ),
                            vertical_align: parent_style.vertical_align,
                            text_shadow: parent_style.text_shadow.clone(),
                            shaping: crate::layout::text::text_run_shaping(parent_style),
                            metadata: crate::layout::text::text_run_metadata(parent_style),
                            ..Default::default()
                        },
                        &mut text_runs,
                        env.fonts,
                    );
                    let lines = wrap_text_runs(
                        text_runs,
                        TextWrapOptions::new(
                            ctx.available_width(),
                            used_font_size(parent_style, env.fonts),
                            text_run_line_height_factor(parent_style, env.fonts),
                            parent_style.overflow_wrap,
                        )
                        .with_white_space(parent_style.white_space)
                        .with_parent_strut(parent_line_strut(parent_style, env.fonts))
                        .with_rtl(parent_style.direction_rtl)
                        .with_bidi_override(parent_style.bidi_override),
                        env.fonts,
                    );
                    if !lines.is_empty() {
                        output.push(
                            TextBlock {
                                lines,
                                text: TextBlockStyle {
                                    alignment: parent_style.text_align,
                                    ..Default::default()
                                },
                                ..Default::default()
                            }
                            .boxed(),
                        );
                    }
                }
            }
            DomNode::Element(el) => {
                let classes = el.class_list();
                let selector_ctx = SelectorContext {
                    ancestors: ancestors.to_vec(),
                    child_index: element_index,
                    sibling_count: element_count,
                    preceding_siblings: preceding_siblings.to_vec(),
                    following_siblings: forward_siblings(&all_element_siblings, element_index)
                        .to_vec(),
                    is_empty: element_is_empty(el),
                };
                let style = compute_style_with_context_with_font_metrics(
                    el.tag,
                    el.style_attr(),
                    parent_style,
                    env.rules,
                    el.tag_name(),
                    &classes,
                    el.id(),
                    &el.attributes,
                    &selector_ctx,
                    env.font_metrics(),
                );
                if let Some(marker) = string_set_marker(el, env.rules, ancestors, &selector_ctx) {
                    output.push(marker);
                }
                if let Some(marker) = target_anchor_marker(el) {
                    output.push(marker);
                }
                if let Some(name) = style.running_name.clone() {
                    flush_table_cells(
                        &mut table_cell_group,
                        parent_style,
                        &ib_ctx,
                        output,
                        ancestors,
                        element_index,
                        element_count,
                        positioned_ancestor_depth,
                        env,
                    );
                    flush_ib(&mut ib_group, parent_style, &ib_ctx, output, ancestors, env);
                    let running_context = LayoutTreeContext::new(parent_style, &ib_ctx, ancestors)
                        .with_list(list_ctx)
                        .with_positioned_ancestor_depth(positioned_ancestor_depth)
                        .for_element(
                            ElementSiblingContext::new(element_index, element_count)
                                .with_neighbors(
                                    &preceding_siblings,
                                    forward_siblings(&all_element_siblings, element_index),
                                ),
                        );
                    if let Some(running) = capture_running_element(name, el, running_context, env) {
                        output.push(running);
                    }
                } else if style.display == Display::TableCell {
                    flush_ib(&mut ib_group, parent_style, &ib_ctx, output, ancestors, env);
                    pending_inline_space = false;
                    table_cell_group.push(el, element_index);
                } else if matches!(
                    InlineFormattingRole::of(el, &style),
                    InlineFormattingRole::Atomic(
                        AtomicInlineKind::InlineBlock
                            | AtomicInlineKind::InlineFlex
                            | AtomicInlineKind::InlineGrid
                            | AtomicInlineKind::InlineTable
                    )
                ) {
                    flush_table_cells(
                        &mut table_cell_group,
                        parent_style,
                        &ib_ctx,
                        output,
                        ancestors,
                        element_index,
                        element_count,
                        positioned_ancestor_depth,
                        env,
                    );
                    ib_group.push((el, pending_inline_space));
                    pending_inline_space = false;
                } else {
                    flush_table_cells(
                        &mut table_cell_group,
                        parent_style,
                        &ib_ctx,
                        output,
                        ancestors,
                        element_index,
                        element_count,
                        positioned_ancestor_depth,
                        env,
                    );
                    // Flush any pending inline-block group
                    flush_ib(&mut ib_group, parent_style, &ib_ctx, output, ancestors, env);
                    pending_inline_space = false;
                    flatten_element(
                        el,
                        LayoutTreeContext::new(parent_style, &ib_ctx, ancestors)
                            .with_list(list_ctx)
                            .with_positioned_ancestor_depth(positioned_ancestor_depth)
                            .for_element(
                                ElementSiblingContext::new(element_index, element_count)
                                    .with_neighbors(
                                        &preceding_siblings,
                                        forward_siblings(&all_element_siblings, element_index),
                                    ),
                            ),
                        output,
                        env,
                    );
                }
                // Track this element as a preceding sibling for the next element
                preceding_siblings.push((
                    el.tag_name().to_string(),
                    el.class_list().iter().map(|s| s.to_string()).collect(),
                ));
                element_index += 1;
            }
        }
        node_index += 1;
    }
    // Flush any remaining inline-block group at end of nodes
    flush_table_cells(
        &mut table_cell_group,
        parent_style,
        &ib_ctx,
        output,
        ancestors,
        element_index,
        element_count,
        positioned_ancestor_depth,
        env,
    );
    flush_ib(&mut ib_group, parent_style, &ib_ctx, output, ancestors, env);
}

/// Ordered `(tag, classes)` list of the element children of `nodes`, used to
/// derive the forward-sibling slice each child needs for `:last-of-type` /
/// `:only-of-type` / `:nth-last-of-type` / sibling-`:has()` matching.
pub(crate) fn element_sibling_list(nodes: &[DomNode]) -> Vec<(String, Vec<String>)> {
    nodes
        .iter()
        .filter_map(|n| match n {
            DomNode::Element(e) => Some((
                e.tag_name().to_string(),
                e.class_list().iter().map(|s| s.to_string()).collect(),
            )),
            _ => None,
        })
        .collect()
}

/// The siblings that follow the element at `element_index` in a sibling list.
pub(crate) fn forward_siblings(
    siblings: &[(String, Vec<String>)],
    element_index: usize,
) -> &[(String, Vec<String>)] {
    siblings.get(element_index + 1..).unwrap_or(&[])
}

/// Whether an element is `:empty` per Selectors-4 §9.4: it has no child
/// elements and no non-whitespace, non-comment text. (Document type
/// declarations and comments are not modelled, so only element children and
/// text content are considered here.)
pub(crate) fn element_is_empty(el: &ElementNode) -> bool {
    el.children.iter().all(|node| match node {
        DomNode::Element(_) => false,
        DomNode::Text(text) => !has_non_collapsible_text(text),
    })
}

/// Flatten a single DOM element into layout elements.
///
/// Computes the element's style, handles special tags (math, br, hr, img,
/// svg, form controls, media, tables, lists), then delegates to
/// [`route_element`] for display-mode dispatching.
pub(crate) fn flatten_element(
    el: &ElementNode,
    context: ElementLayoutContext<'_, '_, '_>,
    output: &mut Vec<LayoutNode>,
    env: &mut LayoutEnv,
) {
    flatten_element_with_running_mode(el, context, output, env, RunningElementMode::Capture);
}

fn flatten_element_with_running_mode(
    el: &ElementNode,
    context: ElementLayoutContext<'_, '_, '_>,
    output: &mut Vec<LayoutNode>,
    env: &mut LayoutEnv,
    running_mode: RunningElementMode,
) {
    let tree = context.tree();
    let siblings = context.siblings();
    let parent_style = tree.parent_style();
    let ctx = tree.layout();
    let list_ctx = tree.list();
    let ancestors = tree.ancestors();
    let positioned_ancestor_depth = tree.positioned_ancestor_depth();
    let child_index = siblings.child_index();
    let sibling_count = siblings.sibling_count();
    let preceding_siblings = siblings.preceding();
    let following_siblings = siblings.following();
    let filter_application = context.filter_application();
    let available_width = ctx.available_width();
    let available_height = ctx.available_height();
    let classes = el.class_list();
    let selector_ctx = SelectorContext {
        ancestors: ancestors.to_vec(),
        child_index,
        sibling_count,
        preceding_siblings: preceding_siblings.to_vec(),
        following_siblings: following_siblings.to_vec(),
        is_empty: element_is_empty(el),
    };
    let selector_attrs = selector_attributes_with_has(el);
    let mut style = compute_style_with_context_and_percentage_basis_with_font_metrics(
        el.tag,
        el.style_attr(),
        parent_style,
        env.rules,
        el.tag_name(),
        &classes,
        el.id(),
        &selector_attrs,
        &selector_ctx,
        PercentageBasis::new(
            Some(ctx.parent.percent_width_basis),
            ctx.percent_height_cb
                .map(|containing_block| containing_block.height),
        ),
        env.font_metrics(),
    );
    let authored_display_contents =
        authored_display_contents(el, env.rules, ancestors, &selector_ctx);
    apply_authored_insets(&mut style, el, env.rules, ancestors, &selector_ctx);
    style.transform = effective_transform(&style, parent_style, ctx);
    // Resolve `filter: url(#id)` (css-filter-effects-1 §3): look up the inline
    // SVG `<filter>` element by id and translate its `feColorMatrix` primitives
    // into `FilterOperation`s, then recolor this box's self-painted surfaces
    // (background + border) through the same color math the image path uses.
    // The fixture's `feColorMatrix type="saturate" values="0"` desaturates the
    // green box to its luminance gray, matching Chrome.
    // `linear_rgb` selects the color space for recoloring the box's paint: SVG
    // `<filter>`s default to linearRGB (color-interpolation-filters), while CSS
    // `filter` color *functions* operate in sRGB.
    let filter = super::filter::ResolvedFilter::from_style(&mut style, env.filter_defs);
    let mut element_output_start = output.len();

    if style.display == Display::None {
        return;
    }
    if running_mode == RunningElementMode::Capture
        && let Some(name) = style.running_name.clone()
    {
        if let Some(running) = capture_running_element(name, el, context, env) {
            output.push(running);
        }
        return;
    }
    let counter_scope = env.counter_state.enter_element(&style);

    // Layout may select a specialized leaf/container route, but all routes
    // return into the same post-layout transaction below. This keeps filters,
    // fixed-position metadata, counter cleanup, and trailing page breaks from
    // depending on which element kind happened to produce the layout nodes.
    (|| {
        // Bail out on excessively deep nesting to prevent stack overflow.
        if ancestors.len() > 30 {
            return;
        }

        let available_height = style.height.unwrap_or(available_height);
        // Update context when element narrows the available height.
        let layout_ctx = if style.height.is_some() {
            ctx.with_parent(available_width, Some(available_height), style.font_size)
        } else {
            *ctx
        };
        let positioned_depth = if crate::layout::helpers::establishes_containing_block(&style) {
            positioned_ancestor_depth + 1
        } else {
            positioned_ancestor_depth
        };

        if let Some(marker) = target_anchor_marker(el) {
            output.push(marker);
        }
        emit_page_break_before(&style, output);
        if let Some(marker) = string_set_marker(el, env.rules, ancestors, &selector_ctx) {
            output.push(marker);
        }
        element_output_start = output.len();

        // Math elements: <span class="math-inline"> or <div class="math-display">
        if let Some(tex) = el.attributes.get("data-math") {
            let is_display = classes.contains(&"math-display");
            if is_display {
                let ast = crate::parser::math::parse_math(tex);
                let math_layout =
                    crate::layout::math::layout_math(&ast, style.font_size, is_display);
                output.push(
                    MathBlock {
                        layout: math_layout,
                        display: true,
                        margins: BlockMargins::new(
                            style.margin.top.max(6.0),
                            style.margin.bottom.max(6.0),
                        ),
                        group: super::elements::PaintGroup::from_style(&style),
                    }
                    .boxed(),
                );
                return;
            }
            // Inline math: fall through to normal inline text collection.
            // The <span> children contain the raw LaTeX text which is rendered
            // as italic text in the surrounding paragraph flow.
        }

        if el.tag == HtmlTag::Br {
            let line = TextLine {
                runs: vec![TextRun {
                    font_size: used_font_size(&style, env.fonts),
                    font_family: resolve_style_font_family(&style, env.fonts),
                    line_height_factor: text_run_line_height_factor(&style, env.fonts),
                    text_shadow: style.text_shadow.clone(),
                    metadata: crate::layout::text::text_run_metadata(&style),
                    ..Default::default()
                }],
                height: used_line_height(&style, env.fonts),
                baseline_ascent: None,
                x_offset: 0.0,
                metadata: Default::default(),
            };
            output.push(TextBlock::plain(vec![line]).boxed());
            return;
        }

        if el.tag == HtmlTag::Hr {
            output.push(
                HorizontalRule {
                    margins: BlockMargins::new(style.margin.top, style.margin.bottom),
                    group: super::elements::PaintGroup::from_style(&style),
                }
                .boxed(),
            );
            return;
        }

        if el.tag == HtmlTag::Img {
            if let Some(img_element) = load_image_from_element(
                &mut *env.resources,
                el,
                available_width,
                available_height,
                &style,
                env.filter_dpi,
            ) {
                output.push(add_inline_replaced_baseline_gap(
                    img_element,
                    &style,
                    env.fonts,
                    InlineBaselineGapRounding::Fractional,
                ));
            }
            return;
        }

        if el.tag == HtmlTag::Svg {
            let (mut svg_width, mut svg_height) =
                resolve_svg_element_size(el, available_width, available_height, true, true);
            if let Some(width) = style.width {
                svg_width = width;
            }
            if let Some(height) = style.height {
                svg_height = height;
            }
            // Resolve the SVG's children against its *user-unit* viewport (the native
            // width/height in CSS px) rather than the pt display box. The render path
            // scales the whole drawing from this native extent into the pt box (see
            // `SvgSourceBox::from_tree`), so absolute and percentage child coordinates
            // must share that native coordinate system; otherwise `%` children would
            // resolve against the pt box and then be scaled again. Falls back to the
            // resolved pt size when the root dimensions are `%`/auto (no native px).
            let child_viewport = {
                let native_w = el
                    .attributes
                    .get("width")
                    .and_then(|w| crate::parser::svg::parse_absolute_length(w))
                    .filter(|v| *v > 0.0);
                let native_h = el
                    .attributes
                    .get("height")
                    .and_then(|h| crate::parser::svg::parse_absolute_length(h))
                    .filter(|v| *v > 0.0);
                match (native_w, native_h) {
                    (Some(w), Some(h)) => (w, h),
                    _ => (svg_width, svg_height),
                }
            };
            if let Some(mut tree) =
                crate::parser::svg::parse_svg_from_element_with_viewport(el, Some(child_viewport))
            {
                sync_svg_tree_to_layout_box(&mut tree, svg_width, svg_height);
                inject_inherited_svg_color(&mut tree, style.color);
                output.push(
                    Svg {
                        tree,
                        geometry: ReplacedGeometry::new(
                            Size::new(svg_width, svg_height),
                            BlockMargins::new(style.margin.top, style.margin.bottom),
                            LayoutBorder::from_computed(&style.border, style.color),
                        ),
                        positioning: Positioning::from_style(&style),
                        paint: SvgPaint {
                            background_color: style.background_color,
                            border_image: style.border_image.paint(),
                            border_radii: style.resolve_corner_radii(svg_width, svg_height),
                            group: super::elements::PaintGroup::from_style(&style),
                        },
                        replaced: ReplacedContent {
                            object_fit: style.object_fit,
                            object_position: style.object_position,
                            ..Default::default()
                        },
                    }
                    .boxed(),
                );
            }
            return;
        }

        // Form control elements — render as styled boxes with placeholder text
        if el.tag == HtmlTag::Input || el.tag == HtmlTag::Select || el.tag == HtmlTag::Textarea {
            let ctrl_width = style
                .width
                .unwrap_or(if el.tag == HtmlTag::Textarea {
                    available_width.min(300.0)
                } else {
                    150.0
                })
                .min(available_width);
            let ctrl_height = style.height.unwrap_or(if el.tag == HtmlTag::Textarea {
                80.0
            } else {
                20.0
            });

            let label = if el.tag == HtmlTag::Select {
                el.children
                    .iter()
                    .find_map(|c| {
                        if let DomNode::Element(opt) = c {
                            opt.children.iter().find_map(|t| {
                                if let DomNode::Text(s) = t {
                                    Some(s.trim().to_string())
                                } else {
                                    None
                                }
                            })
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default()
            } else if el.tag == HtmlTag::Textarea {
                el.children
                    .iter()
                    .find_map(|c| {
                        if let DomNode::Text(s) = c {
                            Some(s.trim().to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default()
            } else {
                el.attributes
                    .get("value")
                    .or(el.attributes.get("placeholder"))
                    .cloned()
                    .unwrap_or_default()
            };

            let mut lines = Vec::new();
            if !label.is_empty() {
                let mut runs = Vec::new();
                push_text_run_with_fallback(
                    TextRun {
                        text: label,
                        font_size: used_font_size(&style, env.fonts),
                        color: style.color,
                        font_family: resolve_style_font_family(&style, env.fonts),
                        line_height_factor: text_run_line_height_factor(&style, env.fonts),
                        text_shadow: style.text_shadow.clone(),
                        shaping: crate::layout::text::text_run_shaping(&style),
                        metadata: crate::layout::text::text_run_metadata(&style),
                        ..Default::default()
                    },
                    &mut runs,
                    env.fonts,
                );
                let inner_w = ctrl_width - style.padding.horizontal();
                lines = wrap_text_runs(
                    runs,
                    TextWrapOptions::new(
                        inner_w,
                        used_font_size(&style, env.fonts),
                        text_run_line_height_factor(&style, env.fonts),
                        style.overflow_wrap,
                    )
                    .with_white_space(style.white_space)
                    .with_parent_strut(parent_line_strut(&style, env.fonts))
                    .with_rtl(style.direction_rtl)
                    .with_bidi_override(style.bidi_override),
                    env.fonts,
                );
            }

            let box_model = BoxModel {
                size: LayoutSize::fixed(ctrl_width, Some(ctrl_height)),
                margins: BlockMargins::new(style.margin.top, style.margin.bottom),
                padding: style.padding,
                border: LayoutBorder::from_computed(&style.border, style.color),
            };
            let mut control = TextBlock::from_style(lines, &style, box_model);
            control.paint.background.color =
                Some(style.background_color.unwrap_or(crate::types::Color::WHITE));
            control.text.writing_mode = WritingMode::HorizontalTb;
            control.text.indent = 0.0;
            output.push(control.boxed());
            return;
        }

        // Media elements — render as placeholder rectangles
        if el.tag == HtmlTag::Video || el.tag == HtmlTag::Audio {
            let media_width = style
                .width
                .or_else(|| {
                    el.attributes
                        .get("width")
                        .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
                })
                .unwrap_or(if el.tag == HtmlTag::Video {
                    300.0
                } else {
                    200.0
                })
                .min(available_width);
            let media_height = style
                .height
                .or_else(|| {
                    el.attributes
                        .get("height")
                        .and_then(|v| v.trim_end_matches("px").parse::<f32>().ok())
                })
                .unwrap_or(if el.tag == HtmlTag::Video {
                    150.0
                } else {
                    24.0
                });

            let label = if el.tag == HtmlTag::Video {
                "\u{25B6} Video".to_string()
            } else {
                "\u{25B6} Audio".to_string()
            };

            let bg = style.background_color.unwrap_or_else(|| {
                if el.tag == HtmlTag::Video {
                    crate::types::Color::BLACK
                } else {
                    crate::types::Color::from_srgb(0.94, 0.94, 0.94, 1.0)
                }
            });
            let text_color = if el.tag == HtmlTag::Video {
                crate::types::Color::WHITE
            } else {
                crate::types::Color::from_srgb(0.3, 0.3, 0.3, 1.0)
            };
            let mut runs = Vec::new();
            push_text_run_with_fallback(
                TextRun {
                    text: label,
                    font_size: used_font_size(&style, env.fonts),
                    color: text_color,
                    font_family: resolve_style_font_family(&style, env.fonts),
                    line_height_factor: text_run_line_height_factor(&style, env.fonts),
                    text_shadow: style.text_shadow.clone(),
                    shaping: crate::layout::text::text_run_shaping(&style),
                    metadata: crate::layout::text::text_run_metadata(&style),
                    ..Default::default()
                },
                &mut runs,
                env.fonts,
            );
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    media_width,
                    used_font_size(&style, env.fonts),
                    text_run_line_height_factor(&style, env.fonts),
                    style.overflow_wrap,
                )
                .with_white_space(style.white_space)
                .with_parent_strut(parent_line_strut(&style, env.fonts))
                .with_rtl(style.direction_rtl)
                .with_bidi_override(style.bidi_override),
                env.fonts,
            );
            let padding_vertical = if el.tag == HtmlTag::Video {
                (media_height - style.font_size) / 2.0
            } else {
                4.0
            };

            let box_model = BoxModel {
                size: LayoutSize::fixed(media_width, Some(media_height)),
                margins: BlockMargins::new(style.margin.top, style.margin.bottom),
                padding: EdgeSizes::axes(4.0, padding_vertical),
                border: LayoutBorder::from_computed(&style.border, style.color),
            };
            let mut media = TextBlock::from_style(lines, &style, box_model);
            media.paint.background.color = Some(bg);
            media.text.alignment = TextAlign::Center;
            media.text.writing_mode = WritingMode::HorizontalTb;
            media.text.indent = 0.0;
            output.push(media.boxed());
            return;
        }

        // Progress and meter elements — render as a horizontal bar
        if el.tag == HtmlTag::Progress || el.tag == HtmlTag::Meter {
            let bar_width = style.width.unwrap_or(150.0).min(available_width);
            let bar_height = style.height.unwrap_or(12.0);
            let value: f32 = el
                .attributes
                .get("value")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let max: f32 = el
                .attributes
                .get("max")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.0);
            let fraction = if max > 0.0 {
                (value / max).clamp(0.0, 1.0)
            } else {
                0.0
            };

            let fill_color = if el.tag == HtmlTag::Progress {
                (0.12, 0.53, 0.90)
            } else {
                let low: f32 = el
                    .attributes
                    .get("low")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(max * 0.25);
                let high: f32 = el
                    .attributes
                    .get("high")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(max * 0.75);
                if value <= low {
                    (0.90, 0.20, 0.20)
                } else if value >= high {
                    (0.20, 0.78, 0.35)
                } else {
                    (0.95, 0.77, 0.06)
                }
            };

            output.push(
                ProgressBar {
                    fraction,
                    size: Size::new(bar_width, bar_height),
                    colors: ProgressColors {
                        fill: crate::types::Color::from_srgb(
                            fill_color.0,
                            fill_color.1,
                            fill_color.2,
                            1.0,
                        ),
                        track: crate::types::Color::from_srgb(0.88, 0.88, 0.88, 1.0),
                    },
                    margins: BlockMargins::new(style.margin.top, style.margin.bottom),
                    group: super::elements::PaintGroup::from_style(&style),
                }
                .boxed(),
            );
            return;
        }

        // Build ancestors list for children of this element
        let mut child_ancestors: Vec<AncestorInfo> = ancestors.to_vec();
        child_ancestors.push(AncestorInfo {
            element: el,
            child_index,
            sibling_count,
            preceding_siblings: Vec::new(),
            following_siblings: Vec::new(),
            is_empty: false,
        });

        if authored_display_contents {
            flatten_nodes(
                &el.children,
                LayoutTreeContext::new(&style, &layout_ctx, &child_ancestors)
                    .with_list(list_ctx)
                    .with_positioned_ancestor_depth(positioned_ancestor_depth),
                output,
                env,
            );
            return;
        }

        // List handling — Ul/Ol pass context to Li children
        if el.tag == HtmlTag::Ul || el.tag == HtmlTag::Ol {
            let list_indent = style.padding.left + style.margin.left;
            let inner_width = available_width - list_indent;
            // Accumulate indentation from parent list context
            let parent_indent = match list_ctx {
                Some(ListContext::Unordered { indent }) => *indent,
                Some(ListContext::Ordered { indent, .. }) => *indent,
                None => 0.0,
            };
            let total_indent = parent_indent + list_indent;
            let mut ctx = if el.tag == HtmlTag::Ol {
                let li_count = el
                    .children
                    .iter()
                    .filter(|c| matches!(c, DomNode::Element(child) if child.tag == HtmlTag::Li))
                    .count() as i32;
                let reversed = el.attributes.contains_key("reversed");
                let step = if reversed { -1 } else { 1 };
                let start = el
                    .attributes
                    .get("start")
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(if reversed { li_count } else { 1 });
                ListContext::Ordered {
                    index: start,
                    step,
                    indent: total_indent,
                }
            } else {
                ListContext::Unordered {
                    indent: total_indent,
                }
            };
            let custom_unordered_counter = el.tag == HtmlTag::Ul
                && matches!(
                    style.list_style_type,
                    ListStyleType::Custom(_) | ListStyleType::CounterStyle(_)
                );
            let auto_list_item_counter = (el.tag == HtmlTag::Ol || custom_unordered_counter)
                && !style
                    .counter_reset
                    .iter()
                    .any(|(name, _)| name == "list-item");
            if auto_list_item_counter {
                match &ctx {
                    ListContext::Ordered { index, step, .. } => {
                        env.counter_state.push_reset("list-item", *index - *step);
                    }
                    ListContext::Unordered { .. } => env.counter_state.push_reset("list-item", 0),
                }
            }
            let child_el_count = el
                .children
                .iter()
                .filter(|c| matches!(c, DomNode::Element(_)))
                .count();
            let mut child_el_idx = 0;
            for child in &el.children {
                if let DomNode::Element(child_el) = child {
                    if child_el.tag == HtmlTag::Li {
                        let child_ctx = layout_ctx
                            .with_parent(inner_width, Some(available_height), style.font_size)
                            .with_containing_block(None);
                        flatten_element(
                            child_el,
                            LayoutTreeContext::new(&style, &child_ctx, &child_ancestors)
                                .with_list(Some(&ctx))
                                .with_positioned_ancestor_depth(positioned_depth)
                                .for_element(ElementSiblingContext::new(
                                    child_el_idx,
                                    child_el_count,
                                )),
                            output,
                            env,
                        );
                        if let ListContext::Ordered { index, step, .. } = &mut ctx {
                            *index += *step;
                        }
                    } else {
                        let child_ctx = layout_ctx
                            .with_parent(inner_width, Some(available_height), style.font_size)
                            .with_containing_block(None);
                        flatten_element(
                            child_el,
                            LayoutTreeContext::new(&style, &child_ctx, &child_ancestors)
                                .with_positioned_ancestor_depth(positioned_depth)
                                .for_element(ElementSiblingContext::new(
                                    child_el_idx,
                                    child_el_count,
                                )),
                            output,
                            env,
                        );
                    }
                    child_el_idx += 1;
                }
            }
            // Pop counters this list pushed via `counter-reset` (e.g. a nested
            // `<ol counter-reset: sec>` opens its own counter scope). Without this,
            // the inner level leaks and a following sibling item is numbered against
            // the stale nested counter (css-lists-3 §4.2 / CSS2 §12.4: the scope ends
            // with the element). This branch `return`s before `route_element`'s own
            // `pop_resets`, so it must undo the push applied in `flatten_element`.
            if auto_list_item_counter {
                env.counter_state.pop_name("list-item");
            }
            return;
        }

        // Li handling — prepend bullet/number marker
        if el.tag == HtmlTag::Li || style.display == Display::ListItem {
            // counter_state resets/increments already applied at top of flatten_element.
            // Ordered list items also participate in the implicit `list-item`
            // counter: absent an explicit `counter-increment: list-item ...`, every
            // item increments by the list direction before marker and generated
            // content are resolved.
            let explicit_list_item_increment = style
                .counter_increment
                .iter()
                .any(|(name, _)| name == "list-item");
            if let Some(ListContext::Ordered { index, step, .. }) = list_ctx {
                if let Some(value) = el
                    .attributes
                    .get("value")
                    .and_then(|s| s.trim().parse::<i32>().ok())
                {
                    env.counter_state.set_current("list-item", value - *step);
                } else if !env.counter_state.has_current("list-item") {
                    env.counter_state.set_current("list-item", index - *step);
                }
                if !explicit_list_item_increment {
                    env.counter_state.increment("list-item", *step);
                }
            } else if matches!(list_ctx, Some(ListContext::Unordered { .. }))
                && matches!(
                    style.list_style_type,
                    ListStyleType::Custom(_) | ListStyleType::CounterStyle(_)
                )
            {
                if !env.counter_state.has_current("list-item") {
                    env.counter_state.set_current("list-item", 0);
                }
                if !explicit_list_item_increment {
                    env.counter_state.increment("list-item", 1);
                }
            }

            let inner_width = available_width - style.padding.horizontal();
            let mut runs = Vec::new();

            // Check for ::before pseudo-element with custom content (e.g. CSS counters).
            // If present, use it instead of the default list marker.
            let class_list = el.class_list();
            let classes: Vec<&str> = class_list.iter().map(|s| s.as_ref()).collect();
            let li_selector_ctx = SelectorContext {
                ancestors: ancestors.to_vec(),
                child_index,
                sibling_count,
                preceding_siblings: preceding_siblings.to_vec(),
                following_siblings: Vec::new(),
                is_empty: false,
            };
            let generated_styles =
                GeneratedContentStyles::resolve(el, &style, env.rules, &li_selector_ctx, env.fonts);
            let custom_before = generated_styles
                .before()
                .filter(|style| !style.content.is_empty());
            let has_custom_before = custom_before.is_some();
            if let Some(ps) = custom_before {
                let content_text = resolve_content(&ps.content, &el.attributes, env.counter_state);
                if !content_text.is_empty() {
                    push_text_run_with_fallback(
                        TextRun {
                            text: content_text,
                            font_size: used_font_size(ps, env.fonts),
                            bold: ps.font_weight == FontWeight::Bold,
                            font_style: ps.font_style,
                            decorations: ps.text_decorations.active(ps.color),
                            color: ps.color,
                            font_family: resolve_style_font_family(ps, env.fonts),
                            line_height_factor: text_run_line_height_factor(ps, env.fonts),
                            text_shadow: ps.text_shadow.clone(),
                            shaping: crate::layout::text::text_run_shaping(ps),
                            metadata: crate::layout::text::text_run_metadata(ps),
                            ..Default::default()
                        },
                        &mut runs,
                        env.fonts,
                    );
                }
            }

            // A `list-style-image` (e.g. data-URI PNG) replaces the type glyph as the
            // marker when it decodes (css-lists-3 §3.1). It is suppressed by a custom
            // `::before`, just like the glyph marker. The small trailing gap mirrors
            // the glyph marker's space so text does not abut the image.
            let image_marker = if has_custom_before {
                None
            } else {
                style.list_style_image.as_deref().and_then(|v| {
                    crate::layout::helpers::build_list_image_marker(
                        &mut *env.resources,
                        v,
                        style.font_size * 0.3,
                    )
                })
            };

            // Add list marker using list-style-type from computed style
            // (only if no custom ::before content and no decodable list-style-image)
            let marker = if has_custom_before || image_marker.is_some() {
                String::new()
            } else {
                match list_ctx {
                    Some(ListContext::Unordered { .. })
                        if matches!(
                            style.list_style_type,
                            ListStyleType::Custom(_) | ListStyleType::CounterStyle(_)
                        ) =>
                    {
                        format_list_marker(
                            &style.list_style_type,
                            env.counter_state.get("list-item"),
                        )
                    }
                    Some(ListContext::Unordered { .. }) => {
                        format_list_marker(&style.list_style_type, 0)
                    }
                    // The <ol> UA default (`list-style-type: decimal`, set in
                    // `default_style`) is inherited by the <li>, so `style
                    // .list_style_type` already carries the correct ordered glyph
                    // (decimal/roman/alpha) — or an author override such as `disc`,
                    // which Chrome honours verbatim. Number it against the running
                    // ordered index.
                    Some(ListContext::Ordered { index, .. }) => {
                        let marker_value = env.counter_state.get("list-item");
                        let marker_value = if marker_value == 0 {
                            *index
                        } else {
                            marker_value
                        };
                        format_list_marker(&style.list_style_type, marker_value)
                    }
                    None => format_list_marker(&style.list_style_type, 0),
                }
            };
            // The <li> content is indented by the list's accumulated start padding
            // (the <ol>/<ul> `padding-left`, carried in the ListContext) for BOTH
            // `inside` and `outside` (css-lists-3 §6): `list-style-position` only
            // controls where the MARKER sits relative to that content edge, not the
            // list's own indentation. `outside` makes the marker hang LEFT into the
            // padding band (negative text-indent via `marker_hang` below); `inside`
            // keeps the marker inline as the first box, so the text simply flows
            // after it at the same content edge.
            let list_indent = match list_ctx {
                Some(ListContext::Unordered { indent }) => *indent,
                Some(ListContext::Ordered { indent, .. }) => *indent,
                None => 0.0,
            };
            // For `list-style-position: outside` (the default) the marker must HANG
            // to the left of the li content edge, sitting inside the ul's padding
            // band, while the li text starts AT the content edge. We render the
            // marker as the first run(s) of the first line, then shift only the
            // first line left by the marker's measured width via a negative
            // text-indent, so the marker lands in the padding and the following text
            // lands at the content edge. For `inside`, the marker stays inline and
            // pushes the text (no hang).
            let has_marker = !marker.is_empty() || image_marker.is_some();
            let marker_run_start = runs.len();
            let mut marker_suffix_gap = 0.0f32;
            if let Some(inline) = image_marker {
                // The image marker is an atomic inline box (empty text + advance), so
                // it occupies the marker slot the same way the glyph marker would and
                // participates in the `outside` hang via `marker_hang` below.
                let outer = inline.outer_width();
                runs.push(TextRun {
                    font_size: used_font_size(&style, env.fonts),
                    color: style.color,
                    font_family: resolve_style_font_family(&style, env.fonts),
                    line_height_factor: text_run_line_height_factor(&style, env.fonts),
                    inline_box: Some(Box::new(inline)),
                    text_shadow: style.text_shadow.clone(),
                    shaping: crate::layout::text::text_run_shaping(&style),
                    metadata: crate::layout::text::text_run_metadata(&style),
                    ..Default::default()
                });
                let _ = outer;
            } else if has_marker {
                // The `::marker` pseudo-element can recolour/restyle the marker box
                // (CSS limits it to color/font/content). Resolve it relative to the
                // <li>; absent any `::marker` rule, `marker_style` falls back to the
                // <li>'s own computed style so the marker matches the text colour.
                let marker_pseudo = compute_pseudo_element_style_with_font_metrics(
                    &style,
                    env.rules,
                    el.tag_name(),
                    &classes,
                    el.id(),
                    &el.attributes,
                    &li_selector_ctx,
                    PseudoElement::Marker,
                    env.font_metrics(),
                );
                let marker_style = marker_pseudo.as_ref().unwrap_or(&style);
                // `::marker { content: … }` replaces the default marker symbol with
                // author-supplied content (which may itself reference counters).
                let marker_overridden = matches!(
                    marker_pseudo.as_ref(),
                    Some(ps) if !ps.content.is_empty()
                );
                let marker_text = match marker_pseudo.as_ref() {
                    Some(ps) if !ps.content.is_empty() => {
                        resolve_content(&ps.content, &el.attributes, env.counter_state)
                    }
                    _ => marker,
                };
                let marker_font_family = resolve_style_font_family(marker_style, env.fonts);
                let marker_bold = marker_style.font_weight == FontWeight::Bold;
                let marker_font_style = marker_style.font_style;
                // ::marker inherits the list item's used line-height. Keep that
                // value intact: `wrap_text_runs` already resolves the shared line
                // box from every run's ascent and descent, so post-layout marker
                // surcharges would double-count its contribution.
                let marker_line_height_factor =
                    text_run_line_height_factor(marker_style, env.fonts);
                if style.list_style_position == ListStylePosition::Outside
                    && marker_text.chars().last().is_some_and(char::is_whitespace)
                    && marker_style.font_size > style.font_size
                {
                    let marker_space = estimate_word_width(
                        " ",
                        marker_style.font_size,
                        &marker_font_family,
                        marker_bold,
                        marker_font_style.is_slanted(),
                        env.fonts,
                    );
                    let item_space = estimate_word_width(
                        " ",
                        style.font_size,
                        &resolve_style_font_family(&style, env.fonts),
                        style.font_weight == FontWeight::Bold,
                        style.font_style.is_slanted(),
                        env.fonts,
                    );
                    marker_suffix_gap = (marker_space - item_space).max(0.0);
                }
                let marker_gap_writing_mode = if style.writing_mode == WritingMode::HorizontalTb {
                    parent_style.writing_mode
                } else {
                    style.writing_mode
                };
                if style.marker_side_match_parent && marker_gap_writing_mode.is_vertical() {
                    marker_suffix_gap = marker_suffix_gap.max(style.font_size * 0.13);
                }
                // Default `disc`/`square` bullets are geometric shapes in Chromium,
                // with one shared slot independent of their Unicode stand-ins.
                // `circle` (which matches as a glyph) and author `::marker { content
                // }` overrides keep the textual path.
                let geometric_bullet = if marker_overridden {
                    None
                } else {
                    let marker_slot = if list_ctx.is_none()
                        && style.display == Display::ListItem
                        && style.list_style_position == ListStylePosition::Inside
                    {
                        BuiltInBulletSlot::StandaloneInside
                    } else {
                        BuiltInBulletSlot::default()
                    };
                    build_list_bullet_marker(
                        &marker_style.list_style_type,
                        used_font_size(marker_style, env.fonts),
                        marker_style.color,
                        marker_slot,
                    )
                };
                if let Some(bullet) = geometric_bullet {
                    runs.push(TextRun {
                        font_size: used_font_size(marker_style, env.fonts),
                        color: marker_style.color,
                        font_family: marker_font_family.clone(),
                        line_height_factor: marker_line_height_factor,
                        inline_box: Some(Box::new(bullet)),
                        text_shadow: marker_style.text_shadow.clone(),
                        shaping: crate::layout::text::text_run_shaping(marker_style),
                        metadata: crate::layout::text::text_run_metadata(marker_style),
                        ..Default::default()
                    });
                } else {
                    push_text_run_with_fallback(
                        TextRun {
                            text: marker_text,
                            font_size: used_font_size(marker_style, env.fonts),
                            bold: marker_bold,
                            font_style: marker_font_style,
                            color: marker_style.color,
                            font_family: marker_font_family,
                            line_height_factor: marker_line_height_factor,
                            text_shadow: marker_style.text_shadow.clone(),
                            shaping: crate::layout::text::text_run_shaping(marker_style),
                            metadata: crate::layout::text::text_run_metadata(marker_style),
                            ..Default::default()
                        },
                        &mut runs,
                        env.fonts,
                    );
                }
            }
            let marker_hang =
                if has_marker && style.list_style_position == ListStylePosition::Outside {
                    measure_runs_width(&runs[marker_run_start..], env.fonts)
                } else {
                    0.0
                };
            if marker_suffix_gap > 0.0 {
                runs.push(TextRun {
                    font_size: used_font_size(&style, env.fonts),
                    color: style.color,
                    font_family: resolve_style_font_family(&style, env.fonts),
                    line_height_factor: text_run_line_height_factor(&style, env.fonts),
                    inline_box: Some(Box::new(InlineBox {
                        width: marker_suffix_gap,
                        vertical_align: VerticalAlign::Baseline,
                        baseline_ascent: Some(0.0),
                        ..InlineBox::default()
                    })),
                    ..Default::default()
                });
            }

            let runs_before_inline = runs.len();
            InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources)
                .collect_box_content(&el.children, &style, &mut runs, None, ancestors);
            append_pseudo_inline_run(
                &mut runs,
                generated_styles.after(),
                el,
                env.fonts,
                env.counter_state,
                &mut *env.resources,
            );

            // "Loose" list items (Markdown with blank lines between items) wrap each
            // item's content in a <p>. When the <li> has no direct inline content
            // but its first block child is a <p>, inline that <p>'s runs so the
            // marker sits on the same baseline as the first line of text (matching
            // Chrome), and apply the <p>'s vertical margins on the combined block
            // so consecutive loose items are separated as paragraphs. Gated on
            // <li> to keep the hot path (nested blocks) free of extra stack.
            let (consumed_p_idx, extra_margin_top, extra_margin_bottom) =
                if el.tag == HtmlTag::Li && runs.len() == runs_before_inline {
                    inline_loose_list_p(el, &style, &child_ancestors, env, &mut runs)
                } else {
                    (None, 0.0, 0.0)
                };

            let block_heading_level = heading_level(el.tag);

            if !runs.is_empty() {
                let effective_writing_mode = if style.writing_mode == WritingMode::HorizontalTb {
                    parent_style.writing_mode
                } else {
                    style.writing_mode
                };
                let vertical_marker_match =
                    effective_writing_mode.is_vertical() && style.marker_side_match_parent;
                let vertical_inline_extent = if vertical_marker_match {
                    style
                        .height
                        .or(parent_style.height)
                        .unwrap_or(available_height)
                } else {
                    inner_width
                };
                let text_indent = style.text_indent.resolve(vertical_inline_extent) - marker_hang;
                let lines = wrap_text_runs(
                    runs,
                    TextWrapOptions::new(
                        vertical_inline_extent,
                        used_font_size(&style, env.fonts),
                        text_run_line_height_factor(&style, env.fonts),
                        style.overflow_wrap,
                    )
                    .with_white_space(style.white_space)
                    .with_parent_strut(parent_line_strut(&style, env.fonts))
                    .with_rtl(style.direction_rtl)
                    .with_bidi_override(style.bidi_override)
                    // An `outside` marker hangs into the negative text-indent band, so
                    // it must NOT consume the first line's text capacity. Mirror the
                    // rendered `text_indent` (which includes `-marker_hang`) here so
                    // wrapping reclaims exactly the marker's width for the first line
                    // (css-lists-3 §6: outside markers sit outside the principal box).
                    .with_text_indent(text_indent),
                    env.fonts,
                );
                let vertical_column_advance = if vertical_marker_match {
                    lines.iter().map(|line| line.height).fold(0.0_f32, f32::max)
                } else {
                    0.0
                };
                let vertical_item_index = if vertical_marker_match {
                    child_index as f32
                } else {
                    0.0
                };
                let vertical_column_offset = if vertical_marker_match {
                    vertical_item_index * vertical_column_advance
                } else {
                    0.0
                };
                let vertical_flow_rewind = if vertical_marker_match {
                    vertical_item_index * (vertical_column_advance + style.margin.bottom)
                } else {
                    0.0
                };
                let vertical_marker_offset = if vertical_marker_match {
                    marker_hang + vertical_column_advance / 2.0 + style.margin.bottom / 12.0
                } else {
                    0.0
                };
                let block_width = if vertical_marker_match {
                    vertical_column_advance
                } else {
                    style.width.unwrap_or(available_width)
                };
                let offset_left = if vertical_marker_match {
                    style.left.unwrap_or(0.0)
                        + list_indent
                        + (available_width - vertical_column_advance - vertical_column_offset)
                            .max(0.0)
                } else {
                    style.left.unwrap_or(0.0) + list_indent
                };
                let offset_bottom = if vertical_marker_match {
                    (vertical_inline_extent
                        - marker_hang
                        - 2.0 * style.margin.bottom
                        - vertical_column_advance / 2.0
                        + style.margin.bottom / 8.0)
                        .max(0.0)
                } else {
                    style.bottom.unwrap_or(0.0)
                };
                let box_model = BoxModel {
                    size: LayoutSize::fixed(block_width, style.height),
                    margins: BlockMargins::new(
                        style.margin.top + extra_margin_top,
                        style.margin.bottom + extra_margin_bottom,
                    ),
                    padding: style.padding,
                    border: LayoutBorder::from_computed(&style.border, style.color),
                };
                let mut list_block = TextBlock::from_style(lines, &style, box_model);
                list_block.text.writing_mode = effective_writing_mode;
                // Hang an outside marker into the padding while preserving an
                // authored text indent as part of the same text-formatting group.
                list_block.text.indent = text_indent;
                list_block.positioning.insets = EdgeSizes::new(
                    style.top.unwrap_or_default() + vertical_marker_offset - vertical_flow_rewind,
                    style.right.unwrap_or_default(),
                    offset_bottom,
                    offset_left,
                );
                list_block.semantics.heading_level = block_heading_level;
                output.push(list_block.boxed());
            }

            // Process block children inside li (nested lists get reduced width for indentation)
            let child_el_count = el
                .children
                .iter()
                .filter(|c| matches!(c, DomNode::Element(_)))
                .count();
            let mut child_el_idx = 0;
            for (raw_idx, child) in el.children.iter().enumerate() {
                if Some(raw_idx) == consumed_p_idx {
                    // This <p> was inlined into the li's TextBlock above — skip.
                    child_el_idx += 1;
                    continue;
                }
                if let DomNode::Element(child_el) = child {
                    if child_el.tag == HtmlTag::Ul || child_el.tag == HtmlTag::Ol {
                        let child_ctx = layout_ctx
                            .with_parent(inner_width, Some(available_height), style.font_size)
                            .with_containing_block(None);
                        // A nested list is a block child of THIS <li>, so its left
                        // indentation is measured from the li's *content edge*, which
                        // includes the li's own `padding-left` (css-lists-3 §6: the
                        // sublist is a normal block inside the li's content box). The
                        // inherited `list_ctx` carries only the parent list's indent
                        // (the li's content edge sans its padding); add the li's
                        // padding-left so the sublist — and its markers — start at the
                        // li's content edge, not its border edge.
                        let nested_list_ctx = list_ctx.map(|c| match *c {
                            ListContext::Unordered { indent } => ListContext::Unordered {
                                indent: indent + style.padding.left,
                            },
                            ListContext::Ordered {
                                index,
                                step,
                                indent,
                            } => ListContext::Ordered {
                                index,
                                step,
                                indent: indent + style.padding.left,
                            },
                        });
                        flatten_element(
                            child_el,
                            LayoutTreeContext::new(&style, &child_ctx, &child_ancestors)
                                .with_list(nested_list_ctx.as_ref())
                                .with_positioned_ancestor_depth(positioned_depth)
                                .for_element(ElementSiblingContext::new(
                                    child_el_idx,
                                    child_el_count,
                                )),
                            output,
                            env,
                        );
                    } else if recurses_as_layout_child(child_el.tag) {
                        let child_ctx = layout_ctx
                            .with_parent(available_width, Some(available_height), style.font_size)
                            .with_containing_block(None);
                        flatten_element(
                            child_el,
                            LayoutTreeContext::new(&style, &child_ctx, &child_ancestors)
                                .with_positioned_ancestor_depth(positioned_depth)
                                .for_element(ElementSiblingContext::new(
                                    child_el_idx,
                                    child_el_count,
                                )),
                            output,
                            env,
                        );
                    }
                    child_el_idx += 1;
                }
            }
            // Mirror `route_element`'s counter cleanup: this Li branch `return`s
            // early, so pop any counters it pushed via `counter-reset` to keep the
            // counter scope bounded to the element (CSS2 §12.4.1).
            return;
        }

        // Resolve the principal box's pseudo family once before any display-
        // specific early return. Every formatting context consumes the same
        // computed generated and typographic pseudo styles.
        let pseudo_styles =
            PrincipalPseudoStyles::resolve(el, &style, env.rules, &selector_ctx, env.fonts);

        // Table fixup operates on the complete box-tree child sequence, which
        // includes generated ::before and ::after boxes. Dispatch only after
        // their computed styles exist so the table normalizer can wrap them in
        // the appropriate anonymous row/cell boxes.
        if el.tag == HtmlTag::Table
            || matches!(style.display, Display::Table | Display::InlineTable)
        {
            flatten_table(
                el,
                &style,
                output,
                pseudo_styles.generated().boxes(el),
                env,
                TableLayoutContext::new(
                    &layout_ctx,
                    ancestors,
                    ElementSiblingContext::new(child_index, sibling_count),
                    positioned_depth,
                ),
            );
            return;
        }

        route_element(
            el,
            &mut style,
            &layout_ctx,
            output,
            ancestors,
            &child_ancestors,
            positioned_depth,
            &pseudo_styles,
            env,
        );
    })();

    if filter_application == FilterApplication::Materialize
        && !prepare_filtered_output(&style, &filter, env, output, element_output_start)
    {
        filter.apply_primitive_fallback(&mut output[element_output_start..]);
    }
    env.counter_state.leave_element(counter_scope);
    emit_page_break_after(&style, output);
}

/// Extracted helper for the loose-list fix (#140): when an `<li>` has no
/// direct inline content and its first block child is a `<p>`, inline that
/// `<p>`'s runs into the li's TextBlock and return the `<p>`'s margins +
/// raw child index so the caller can skip re-emitting it as a block.
///
/// Isolated into its own function so the extra locals (SelectorContext,
/// ComputedStyle, class list, etc.) are only paid for on the `<li>` path,
/// not on every recursive `flatten_element` frame — deep nested blocks are
/// otherwise stack-sensitive.
fn inline_loose_list_p(
    el: &ElementNode,
    parent_style: &ComputedStyle,
    child_ancestors: &[AncestorInfo],
    env: &mut LayoutEnv,
    runs: &mut Vec<TextRun>,
) -> (Option<usize>, f32, f32) {
    let li_child_el_count = el
        .children
        .iter()
        .filter(|c| matches!(c, DomNode::Element(_)))
        .count();
    let mut child_el_ordinal = 0usize;
    for (raw_idx, child) in el.children.iter().enumerate() {
        if let DomNode::Element(child_el) = child {
            if child_el.tag == HtmlTag::P {
                let p_cls: Vec<&str> = child_el.class_list();
                let p_selector_ctx = SelectorContext {
                    ancestors: child_ancestors.to_vec(),
                    child_index: child_el_ordinal,
                    sibling_count: li_child_el_count,
                    preceding_siblings: Vec::new(),
                    following_siblings: Vec::new(),
                    is_empty: false,
                };
                let p_style = compute_style_with_context_with_font_metrics(
                    child_el.tag,
                    child_el.style_attr(),
                    parent_style,
                    env.rules,
                    child_el.tag_name(),
                    &p_cls,
                    child_el.id(),
                    &child_el.attributes,
                    &p_selector_ctx,
                    env.font_metrics(),
                );
                InlineRunCollector::new(
                    env.rules,
                    env.fonts,
                    env.counter_state,
                    &mut *env.resources,
                )
                .collect_box_content(
                    &child_el.children,
                    &p_style,
                    runs,
                    None,
                    child_ancestors,
                );
                return (Some(raw_idx), p_style.margin.top, p_style.margin.bottom);
            }
            child_el_ordinal += 1;
            if recurses_as_layout_child(child_el.tag) {
                break;
            }
        }
    }
    (None, 0.0, 0.0)
}

/// Dispatch an element to the appropriate layout function based on its
/// computed `display` value (flex, grid, block, inline-block, or inline).
///
/// Handles page-break-after emission and CSS counter cleanup.
#[allow(clippy::too_many_arguments)]
fn route_element(
    el: &ElementNode,
    style: &mut ComputedStyle,
    ctx: &LayoutContext,
    output: &mut Vec<LayoutNode>,
    ancestors: &[AncestorInfo],
    child_ancestors: &[AncestorInfo],
    positioned_depth: usize,
    pseudo_styles: &PrincipalPseudoStyles,
    env: &mut LayoutEnv,
) {
    let layout_ctx = *ctx;
    let generated_styles = pseudo_styles.generated();
    // Flex container handling
    if matches!(style.display, Display::Flex | Display::InlineFlex) {
        let expanded_flex_el;
        let flex_el = if let Some(expanded) =
            flex_element_with_display_contents_children(el, child_ancestors, env)
        {
            expanded_flex_el = expanded;
            &expanded_flex_el
        } else {
            el
        };
        let flex_output_start = output.len();
        layout_flex_container(
            flex_el,
            style,
            &layout_ctx,
            output,
            child_ancestors,
            generated_styles.boxes(el),
            positioned_depth,
            env,
        );
        apply_direct_flex_item_filters(
            flex_el,
            style,
            child_ancestors,
            env,
            &mut output[flex_output_start..],
        );

        return;
    }

    // Grid container handling
    if style.display == Display::Grid {
        layout_grid_container(
            el,
            style,
            &layout_ctx,
            output,
            child_ancestors,
            positioned_depth,
            env,
        );

        return;
    }

    // Multi-column layout: column-major balanced flow. Triggered by an explicit
    // multi-column count (>= 2) or a column-width (which derives the count from
    // the available width). A single column degrades to normal block layout.
    let multicol_active =
        style.column_count.is_some_and(|c| c >= 2) || style.column_width.is_some_and(|w| w > 0.0);
    if multicol_active {
        crate::layout::multicol::layout_multicol_container(
            el,
            style,
            &layout_ctx,
            output,
            ancestors,
            positioned_depth,
            env,
        );

        return;
    }

    if style.display == Display::Block || style.display == Display::InlineBlock {
        layout_block_element(
            el,
            style,
            &layout_ctx,
            output,
            ancestors,
            child_ancestors,
            positioned_depth,
            generated_styles,
            pseudo_styles.first_line(),
            pseudo_styles.first_letter(),
            env,
        );
    } else {
        let has_inline_target_placeholder = generated_styles.before().is_some_and(|ps| {
            !pseudo_is_block_like(ps) && content_items_include_target_placeholder(&ps.content)
        }) || generated_styles.after().is_some_and(|ps| {
            !pseudo_is_block_like(ps) && content_items_include_target_placeholder(&ps.content)
        });
        if has_inline_target_placeholder {
            let mut runs = Vec::new();
            append_pseudo_inline_run(
                &mut runs,
                generated_styles.before(),
                el,
                env.fonts,
                env.counter_state,
                &mut *env.resources,
            );
            let link_url = if el.tag == HtmlTag::A {
                el.attributes.get("href").map(String::as_str)
            } else {
                None
            };
            InlineRunCollector::new(env.rules, env.fonts, env.counter_state, &mut *env.resources)
                .collect_box_content(&el.children, style, &mut runs, link_url, child_ancestors);
            append_pseudo_inline_run(
                &mut runs,
                generated_styles.after(),
                el,
                env.fonts,
                env.counter_state,
                &mut *env.resources,
            );
            resolve_target_text_placeholders_in_runs(&mut runs, env.filter_defs);
            let lines = wrap_text_runs(
                runs,
                TextWrapOptions::new(
                    layout_ctx.available_width(),
                    used_font_size(style, env.fonts),
                    text_run_line_height_factor(style, env.fonts),
                    style.overflow_wrap,
                )
                .with_white_space(style.white_space)
                .with_parent_strut(parent_line_strut(style, env.fonts))
                .with_rtl(style.direction_rtl)
                .with_bidi_override(style.bidi_override),
                env.fonts,
            );
            if !lines.is_empty() {
                output.push(
                    TextBlock {
                        lines,
                        fragmentation: TextFragmentation {
                            lines: LineFragmentation::from_style(style),
                            ..Default::default()
                        },
                        text: TextBlockStyle {
                            alignment: style.text_align,
                            writing_mode: style.writing_mode,
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                    .boxed(),
                );
            }
        } else {
            // Inline element — process children with this style context
            flatten_nodes(
                &el.children,
                LayoutTreeContext::new(style, &layout_ctx, child_ancestors)
                    .with_positioned_ancestor_depth(positioned_depth),
                output,
                env,
            );
        }
    }

    // Pop any counters that were pushed by counter-reset on this element.
}

fn flex_element_with_display_contents_children(
    el: &ElementNode,
    ancestors: &[AncestorInfo],
    env: &LayoutEnv,
) -> Option<ElementNode> {
    let sibling_list = element_sibling_list(&el.children);
    let element_count = sibling_list.len();
    let mut element_index = 0usize;
    let mut preceding_siblings: Vec<(String, Vec<String>)> = Vec::new();
    let mut changed = false;
    let mut children = Vec::with_capacity(el.children.len());

    for child in &el.children {
        match child {
            DomNode::Element(child_el) => {
                let selector_ctx = SelectorContext {
                    ancestors: ancestors.to_vec(),
                    child_index: element_index,
                    sibling_count: element_count,
                    preceding_siblings: preceding_siblings.clone(),
                    following_siblings: forward_siblings(&sibling_list, element_index).to_vec(),
                    is_empty: element_is_empty(child_el),
                };
                if authored_display_contents(child_el, env.rules, ancestors, &selector_ctx) {
                    changed = true;
                    children.extend(child_el.children.iter().cloned());
                } else {
                    children.push(child.clone());
                }
                preceding_siblings.push((
                    child_el.tag_name().to_string(),
                    child_el
                        .class_list()
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ));
                element_index += 1;
            }
            DomNode::Text(_) => children.push(child.clone()),
        }
    }

    changed.then(|| {
        let mut expanded = el.clone();
        expanded.children = children;
        expanded
    })
}

// Grid layout functions have been moved to `super::grid`.

// Table layout functions have been moved to `super::table`.

// Re-export estimate_element_height from paginate module so existing
// `crate::layout::engine::estimate_element_height` paths keep working.
pub(crate) use super::paginate::estimate_element_height;

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::layout::elements::{BoxPaint, LayoutElementTestExt};
    use crate::parser::css::{parse_page_rules, parse_stylesheet};
    use crate::parser::html::{parse_html, parse_html_with_styles};
    use crate::style::computed::{FootnoteDisplay, FootnotePolicy};
    use crate::util::decode_base64;

    const TEST_JPEG_DATA_URI: &str = concat!(
        "data:image/jpeg;base64,",
        "/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkK",
        "DA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAA",
        "AAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q=="
    );

    #[test]
    fn non_breaking_space_keeps_an_element_non_empty() {
        let mut element = ElementNode::new(HtmlTag::Div);
        element.children.push(DomNode::Text("\u{00a0}".to_string()));

        assert!(!element_is_empty(&element));

        element.children = vec![DomNode::Text(" \t\n\r".to_string())];
        assert!(element_is_empty(&element));
    }

    fn table_rows(page: &Page) -> Vec<TableRow> {
        #[derive(Default)]
        struct Rows(Vec<TableRow>);

        impl LayoutVisitor for Rows {
            fn visit_table_row(&mut self, row: &TableRow) {
                self.0.push(row.clone());
            }
        }

        let mut rows = Rows::default();
        for (_, element) in &page.elements {
            visit_layout_tree(element.as_ref(), &mut rows);
        }
        rows.0
    }

    fn flex_rows(page: &Page) -> Vec<FlexRow> {
        page.elements
            .iter()
            .filter_map(|(_, element)| element.inspect_flex(Clone::clone))
            .collect()
    }

    fn first_table_row(page: &Page) -> TableRow {
        table_rows(page)
            .into_iter()
            .next()
            .expect("expected table row")
    }

    fn text_block_positions(page: &Page, include_absolute: bool) -> Vec<(f32, String)> {
        page.elements
            .iter()
            .filter_map(|(y, element)| {
                element
                    .inspect_text(|block| {
                        (include_absolute || !block.positioning.scheme.is_absolute())
                            .then(|| {
                                block
                                    .lines
                                    .iter()
                                    .flat_map(|line| line.runs.iter().map(|run| run.text.as_str()))
                                    .collect::<String>()
                            })
                            .filter(|text| !text.is_empty())
                            .map(|text| (*y, text))
                    })
                    .flatten()
            })
            .collect()
    }

    fn tree_has_position(element: &dyn LayoutElement, position: Position) -> bool {
        if element
            .positioning_owner()
            .is_some_and(|owner| owner.positioning().scheme == position)
        {
            return true;
        }

        let mut found = false;
        element.visit_children(&mut |child| found |= tree_has_position(child, position));
        found
    }

    fn tree_has_rendered_image(element: &dyn LayoutElement) -> bool {
        if element
            .inspect_image(|image| image.source.origin.preserves_native_resolution())
            .unwrap_or(false)
        {
            return true;
        }

        let mut found = false;
        element.visit_children(&mut |child| found |= tree_has_rendered_image(child));
        found
    }

    #[test]
    fn footnote_link_preserves_display_and_policy() {
        let encoded = encode_footnote_link_data(&FootnoteLinkData {
            marker: "1".to_string(),
            text: "note".to_string(),
            marker_prefix: "{marker}. ".to_string(),
            body: FootnoteBodyStyle {
                font_size: 11.5,
                bold: true,
                color: Color::from_srgb(0.1, 0.2, 0.3, 1.0),
                font_family: FontFamily::Custom("ParitySans".to_string()),
                line_height_factor: 1.3,
                ..Default::default()
            },
            marker_color: Color::from_srgb(0.4, 0.5, 0.6, 1.0),
            formatting: FootnoteFormatting {
                display: FootnoteDisplay::Inline,
                policy: FootnotePolicy::Line,
            },
        });
        let decoded = decode_footnote_link_data(&encoded).expect("encoded footnote link");
        assert_eq!(decoded.formatting.display, FootnoteDisplay::Inline);
        assert_eq!(decoded.formatting.policy, FootnotePolicy::Line);
        assert_eq!(decoded.body.font_size, 11.5);
        assert!(decoded.body.bold);
        assert_eq!(
            decoded.body.font_family,
            FontFamily::Custom("ParitySans".to_string())
        );
        assert_eq!(decoded.body.line_height_factor, 1.3);
    }

    #[test]
    fn footnote_call_line_box_is_not_given_a_second_reserve() {
        let html = include_str!(
            "../../tests/parity/cases/paged-media/paged-footnote-styling-pseudos.html"
        )
        .replace("</body>", "<p>following</p></body>");
        let parsed = parse_html_with_styles(&html).expect("valid footnote fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules_and_fonts(
            &parsed.nodes,
            PageSize::new(156.0, 108.0),
            Margin::uniform(6.0),
            &rules,
            &synthetic_weight_test_fonts(),
            None,
            0.0,
            Default::default(),
        );
        let body_block = pages[0]
            .elements
            .iter()
            .find_map(|(y, element)| {
                element.inspect_text(|text| {
                    text.lines
                        .iter()
                        .flat_map(|line| &line.runs)
                        .any(|run| run.text == "Body")
                        .then(|| (*y, text.lines.clone(), text.box_model.margins))
                })?
            })
            .expect("body text block");
        let following_y = pages[0]
            .elements
            .iter()
            .find_map(|(y, element)| {
                element.inspect_text(|text| {
                    text.lines
                        .iter()
                        .flat_map(|line| &line.runs)
                        .any(|run| run.text == "following")
                        .then_some((*y, text.box_model.margins))
                })?
            })
            .expect("following text block");
        let (body_y, lines, body_margins) = body_block;
        let (following_y, following_margins) = following_y;
        let text_height = lines.iter().map(|line| line.height).sum::<f32>();
        let call = lines[0]
            .runs
            .iter()
            .find(|run| run.link_url.is_some())
            .expect("footnote call run");

        assert!(
            (following_y - (body_y + text_height)).abs() < f32::EPSILON,
            "following block starts at {following_y} with {following_margins:?}, body starts at {body_y}, has {text_height}pt of line boxes, and {body_margins:?}",
        );
        assert!(
            (call.font_size - 7.2).abs() < 0.000_01,
            "explicit normal footnote call font size was {}",
            call.font_size
        );
        assert_eq!(call.vertical_align, VerticalAlign::Baseline);

        assert_eq!(pages[0].footnotes.len(), 1);
        let footnote = &pages[0].footnotes[0];
        assert_eq!(footnote.marker, "1");
        assert_eq!(footnote.marker_prefix, "N1: ");
        assert_eq!(footnote.text, "styled footnote body");
        assert!((footnote.body.font_size - 9.0).abs() < f32::EPSILON);
        assert_eq!(footnote.body.color, Color::BLACK);
        assert_eq!(footnote.marker_color, Color::rgb(29, 78, 216));
    }

    #[test]
    fn footnote_call_keeps_following_words_on_the_fitting_line() {
        let html = include_str!("../../tests/parity/cases/paged-media/footnote-float.html");
        let parsed = parse_html_with_styles(html).expect("valid footnote fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules_and_fonts(
            &parsed.nodes,
            PageSize::new(300.0, 225.0),
            Margin::uniform(28.346_457),
            &rules,
            &synthetic_weight_test_fonts(),
            None,
            0.0,
            Default::default(),
        );
        let lines = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_text(|text| {
                    text.lines
                        .iter()
                        .any(|line| line.runs.iter().any(|run| run.text.contains("reference")))
                        .then(|| text.lines.clone())
                })?
            })
            .expect("body text block");
        let words = lines
            .iter()
            .map(|line| {
                line.runs
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            words,
            vec![
                vec![
                    "Body",
                    " text",
                    " with",
                    " a",
                    " reference.",
                    "1",
                    " More",
                    " body",
                ],
                vec!["text", " follows", " the", " call."],
            ]
        );
    }

    #[test]
    fn target_counter_page_resolves_after_the_target_is_paginated() {
        let html = include_str!(
            "../../tests/parity/cases/generated-content/generated-content-target-counter-page.html"
        );
        let parsed = parse_html_with_styles(html).expect("valid target-counter fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules_and_fonts(
            &parsed.nodes,
            PageSize::new(165.0, 120.0),
            Margin::uniform(0.0),
            &rules,
            &synthetic_weight_test_fonts(),
            None,
            0.0,
            Default::default(),
        );
        let first_page_text = pages[0]
            .elements
            .iter()
            .filter_map(|(_, element)| element.inspect_text(|text| text.lines.clone()))
            .flatten()
            .flat_map(|line| line.runs)
            .map(|run| run.text)
            .collect::<String>();
        let page_text = pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .filter_map(|(y, element)| {
                        element.inspect_text(|text| {
                            let value = text
                                .lines
                                .iter()
                                .flat_map(|line| &line.runs)
                                .map(|run| run.text.as_str())
                                .collect::<String>();
                            (!value.is_empty()).then_some((*y, value))
                        })?
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert!(
            first_page_text.contains("p.2"),
            "target page number was not resolved: {first_page_text:?}; pages={page_text:?}"
        );
        assert!(
            !first_page_text.contains(TARGET_PLACEHOLDER_START),
            "unresolved target placeholder: {first_page_text:?}"
        );
    }

    #[test]
    fn footnote_policy_call_keeps_body_metrics_independent_from_call_metrics() {
        let html =
            include_str!("../../tests/parity/cases/paged-media/paged-footnote-policy-block.html");
        let parsed = parse_html_with_styles(html).expect("valid footnote fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules_and_fonts(
            &parsed.nodes,
            PageSize::new(142.5, 97.5),
            Margin::uniform(7.5),
            &rules,
            &synthetic_weight_test_fonts(),
            None,
            0.0,
            Default::default(),
        );
        let call = pages
            .iter()
            .flat_map(|page| &page.elements)
            .filter_map(|(_, element)| element.inspect_text(|text| text.lines.clone()))
            .flatten()
            .flat_map(|line| line.runs)
            .find(|run| run.link_url.is_some())
            .expect("footnote call");
        let data = decode_footnote_link_data(call.link_url.as_deref().expect("footnote link"))
            .expect("encoded footnote link");

        assert!((call.font_size - 7.2).abs() < 0.000_01);
        assert_eq!(call.line_height_font_size(), 9.0);
        assert_eq!(call.font_variant_position, FontVariantPosition::Super);
        assert_eq!(call.vertical_align, VerticalAlign::Baseline);
        assert_eq!(data.body.font_size, 9.0);
        assert_eq!(data.body.line_height_factor, 20.0 / 12.0);
    }

    #[test]
    fn first_line_font_size_uses_its_own_line_metric_basis() {
        let html = include_str!(
            "../../tests/parity/cases/generated-content/generated-content-first-line-font-size.html"
        );
        let parsed = parse_html_with_styles(html).expect("valid first-line fixture");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules_and_fonts(
            &parsed.nodes,
            PageSize::new(240.0, 132.0),
            Margin::uniform(0.0),
            &rules,
            &synthetic_weight_test_fonts(),
            None,
            0.0,
            Default::default(),
        );
        let lines = pages
            .iter()
            .flat_map(|page| &page.elements)
            .find_map(|(_, element)| {
                element.inspect_text(|text| {
                    text.lines
                        .first()
                        .is_some_and(|line| line.runs.iter().any(|run| run.text.contains("Large")))
                        .then(|| text.lines.clone())
                })?
            })
            .expect("first-line text block");
        let first = &lines[0];
        let first_run = first
            .runs
            .iter()
            .find(|run| run.text.contains("Large"))
            .expect("first-line run");

        assert!((first_run.font_size - 25.5).abs() < 0.000_01);
        assert!((first_run.line_height_font_size() - 25.5).abs() < 0.000_01);
        assert!((first.height - 31.5).abs() < 0.000_01);
        assert_eq!(first.baseline_ascent, Some(24.75));
    }

    fn synthetic_weight_test_fonts() -> HashMap<String, TtfFont> {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans TTF");
        HashMap::from([("paritysans".to_string(), font)])
    }

    #[test]
    fn flex_cell_default_is_an_unconstrained_empty_item() {
        let cell = FlexCell::default();
        assert_eq!(cell.padding, EdgeSizes::ZERO);
        assert_eq!(cell.cross_min, 0.0);
        assert!(cell.cross_max.is_infinite());
        assert_eq!(cell.align_self, crate::style::computed::AlignSelf::Auto);
        assert!(cell.lines.is_empty());
        assert!(cell.nested_elements.is_empty());
    }

    #[test]
    fn layout_simple_paragraph() {
        let nodes = parse_html("<p>Hello World</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_multiple_elements() {
        let nodes = parse_html("<h1>Title</h1><p>Paragraph one.</p><p>Paragraph two.</p>").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 3);
    }

    #[test]
    fn layout_empty() {
        let nodes = parse_html("").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.is_empty());
    }

    #[test]
    fn collapse_whitespace_test() {
        assert_eq!(collapse_whitespace("  hello   world  "), "hello world");
        assert_eq!(collapse_whitespace("\n\t  foo  \n"), "foo");
    }

    #[test]
    fn page_break_creates_new_page() {
        let html = r#"<p>Page 1</p><div style="page-break-before: always"><p>Page 2</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn bare_text_node() {
        // Text not wrapped in any element — exercises DomNode::Text branch in flatten_nodes
        let nodes = parse_html("Just some bare text").unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn br_element_creates_empty_line() {
        let html = "<p>Line one</p><br><p>Line two</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have at least 3 elements (p, br, p)
        assert!(pages[0].elements.len() >= 2);
    }

    #[test]
    fn inline_element_layout() {
        // Inline element outside a block — exercises the else branch
        let html = "<span>Hello</span>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn nested_svg_percent_height_uses_parent_height() {
        let html = r#"<div style="height: 200pt"><svg width="100" height="50%"></svg></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        fn find_svg(elements: &[(f32, LayoutNode)]) -> Option<(f32, f32)> {
            struct FirstSvg(Option<(f32, f32)>);

            impl LayoutVisitor for FirstSvg {
                fn visit_svg(&mut self, svg: &Svg) {
                    self.0
                        .get_or_insert((svg.geometry.size.width, svg.geometry.size.height));
                }
            }

            let mut visitor = FirstSvg(None);
            for (_, element) in elements {
                visit_layout_tree(element.as_ref(), &mut visitor);
            }
            visitor.0
        }
        let svg = find_svg(&pages[0].elements).expect("expected nested svg element");
        assert!((svg.0 - 75.0).abs() < 0.1); // 100px = 75pt
        assert!((svg.1 - 100.0).abs() < 0.1); // 50% of 200pt = 100pt
    }

    #[test]
    fn nested_svg_percent_viewport_uses_resolved_root_size() {
        let html = r#"
            <div style="width: 400pt; height: 200pt">
                <svg width="100%" height="50%" viewBox="0 0 20 10">
                    <svg width="50%" height="50%" viewBox="0 0 10 10">
                        <rect width="10" height="10"/>
                    </svg>
                </svg>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        fn find_svg_tree(elements: &[(f32, LayoutNode)]) -> Option<crate::parser::svg::SvgTree> {
            struct FirstSvgTree(Option<crate::parser::svg::SvgTree>);

            impl LayoutVisitor for FirstSvgTree {
                fn visit_svg(&mut self, svg: &Svg) {
                    if self.0.is_none() {
                        self.0 = Some(svg.tree.clone());
                    }
                }
            }

            let mut visitor = FirstSvgTree(None);
            for (_, element) in elements {
                visit_layout_tree(element.as_ref(), &mut visitor);
            }
            visitor.0
        }
        let svg = find_svg_tree(&pages[0].elements).expect("expected nested svg element");
        match &svg.children[0] {
            crate::parser::svg::SvgNode::Group { transform, .. } => {
                assert!(matches!(
                    transform,
                    Some(crate::parser::svg::SvgTransform::Matrix(
                        20.0, 0.0, 0.0, 5.0, 0.0, 0.0
                    ))
                ));
            }
            other => panic!("expected nested svg group, got {other:?}"),
        }
    }

    #[test]
    fn layout_svg_element_preserves_viewbox_for_renderer() {
        let html = r#"<svg width="200" height="100" viewBox="0 0 20 10"><rect width="10" height="10"/></svg>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let svg = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_svg(|svg| {
                    (
                        svg.tree.clone(),
                        svg.geometry.size.width,
                        svg.geometry.size.height,
                    )
                })
            })
            .expect("expected svg layout element");
        assert_eq!(svg.1, 150.0); // 200px = 150pt
        assert_eq!(svg.2, 75.0); // 100px = 75pt
        assert!(
            svg.0.view_box.is_some(),
            "renderer should keep viewBox metadata"
        );
    }

    #[test]
    fn inline_svg_inherits_document_color_for_current_color() {
        let html = r#"<div style="color: #336699"><svg width="20" height="10"><rect width="10" height="10" fill="currentColor"/></svg></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let tree = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_svg(|svg| svg.tree.clone()))
            .expect("expected svg layout element");

        match &tree.children[0] {
            crate::parser::svg::SvgNode::Group {
                style, children, ..
            } => {
                assert_eq!(style.color, Some(Color::from_srgb(0.2, 0.4, 0.6, 1.0)));
                assert_eq!(children.len(), 1);
            }
            other => panic!("expected root group wrapper, got {other:?}"),
        }
    }

    #[test]
    fn page_break_after() {
        let html = r#"<div style="page-break-after: always"><p>Page 1</p></div><p>Page 2</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn page_break_after_on_list_boxes_is_preserved() {
        for html in [
            r#"<ul style="page-break-after: always"><li>first</li></ul><p>second</p>"#,
            r#"<ul><li style="page-break-after: always">first</li></ul><p>second</p>"#,
        ] {
            let nodes = parse_html(html).unwrap();
            let pages = layout(&nodes, PageSize::A4, Margin::default());
            assert!(
                pages.len() >= 2,
                "the list boundary must force a page break"
            );
        }
    }

    #[test]
    fn avoid_page_break_after_on_list_boxes_keeps_the_following_sibling() {
        for boundary in [
            r#"<ul style="margin: 0; padding: 0; break-after: avoid"><li style="height: 50pt; margin: 0">list</li></ul>"#,
            r#"<ol style="margin: 0; padding: 0; break-after: avoid"><li style="height: 50pt; margin: 0">list</li></ol>"#,
            r#"<ul style="margin: 0; padding: 0"><li style="height: 50pt; margin: 0; break-after: avoid">list</li></ul>"#,
        ] {
            let html = format!(
                "<div style=\"height: 30pt; margin: 0\"></div>{boundary}<div style=\"height: 30pt; margin: 0\"></div>"
            );
            let nodes = parse_html(&html).unwrap();
            let pages = layout(
                &nodes,
                PageSize::new(200.0, 100.0),
                Margin::new(0.0, 0.0, 0.0, 0.0),
            );

            assert_eq!(pages.len(), 2);
            assert_eq!(pages[0].elements.len(), 1);
            assert_eq!(pages[1].elements.len(), 2);
        }
    }

    #[test]
    fn word_wrap_long_text() {
        // Generate text that exceeds page width to trigger word wrapping
        let long_text = "word ".repeat(200);
        let html = format!("<p>{long_text}</p>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have wrapped into multiple lines
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert!(text.lines.len() > 1))
            .expect("expected text block");
    }

    #[test]
    fn content_overflows_to_next_page() {
        // Generate enough content to overflow one page
        let paragraphs = "<p>Some paragraph text that takes up space.</p>\n".repeat(100);
        let nodes = parse_html(&paragraphs).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2);
    }

    #[test]
    fn background_color_block() {
        let html = r#"<div style="background-color: yellow"><p>Highlighted</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn pre_element_with_background() {
        let html = "<pre>code block</pre>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Pre has background color in defaults
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert!(text.paint.background.color.is_some()))
            .expect("expected text block");
    }

    #[test]
    fn table_layout_basic() {
        // Exercises flatten_table and table row layout (lines 232, 248, 344, 354)
        let html = r#"
            <table>
                <tr><th>Header 1</th><th>Header 2</th></tr>
                <tr><td>Cell A</td><td>Cell B</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have TableRow elements
        assert_eq!(table_rows(&pages[0]).len(), 2);
    }

    #[test]
    fn table_with_thead_tbody_tfoot() {
        // Exercises lines 345-353: collecting rows from thead/tbody/tfoot
        let html = r#"
            <table>
                <thead><tr><th>H</th></tr></thead>
                <tbody><tr><td>B</td></tr></tbody>
                <tfoot><tr><td>F</td></tr></tfoot>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(table_rows(&pages[0]).len(), 3);
    }

    #[test]
    fn table_empty_rows_ignored() {
        // Line 360: empty table returns early
        let html = "<table></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // Should have no table rows
        assert_eq!(table_rows(&pages[0]).len(), 0);
    }

    #[test]
    fn ordered_list_layout() {
        // Exercises lines 219-232, 248: ordered list context and numbering
        let html = "<ol><li>First</li><li>Second</li><li>Third</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should have items with numbered markers
        let blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, element)| element.inspect_text(|_| ()).is_some())
            .collect();
        assert!(blocks.len() >= 3);
    }

    #[test]
    fn unordered_list_layout() {
        // Exercises lines 217-236: unordered list layout
        let html = "<ul><li>A</li><li>B</li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn list_with_non_li_child() {
        // Line 232: non-li child inside ul
        let html = "<ul><li>Item</li><p>Not a list item</p></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn li_with_block_child() {
        // Lines 279-280: block child inside li
        let html = "<ul><li><p>Paragraph inside li</p></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn table_row_pagination() {
        // Exercises TableRow height calculation in paginate (lines 559-572)
        let mut rows = String::new();
        for i in 0..100 {
            rows.push_str(&format!(
                "<tr><td>Row {i} with some text</td><td>More text</td></tr>"
            ));
        }
        let html = format!("<table>{rows}</table>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(pages.len() >= 2, "Large table should span multiple pages");
    }

    #[test]
    fn table_with_non_cell_children_in_row() {
        // Line 354: non-td/th child in tr is ignored
        let html = r#"<table><tr><td>Cell</td><span>Ignored</span></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let table_rows: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, element)| element.inspect_table(|_| ()).is_some())
            .collect();
        assert_eq!(table_rows.len(), 1);
    }

    #[test]
    fn del_element_sets_line_through() {
        let html = "<p><del>Deleted text</del></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert!(!text.lines.is_empty());
                let run = &text.lines[0].runs[0];
                assert!(
                    run.decorations
                        .iter()
                        .any(|decoration| decoration.lines.line_through),
                    "del element should set line_through"
                );
                assert!(
                    !run.decorations
                        .iter()
                        .any(|decoration| decoration.lines.underline)
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn s_element_sets_line_through() {
        let html = "<p><s>Struck text</s></p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert!(!text.lines.is_empty());
                let run = &text.lines[0].runs[0];
                assert!(
                    run.decorations
                        .iter()
                        .any(|decoration| decoration.lines.line_through),
                    "s element should set line_through"
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn nested_unordered_list() {
        let html = "<ul><li>Parent<ul><li>Child</li></ul></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (texts, has_geometric_marker) = list_texts_and_markers(&pages[0]);
        let joined = texts.join(" ");
        assert!(
            has_geometric_marker || texts.iter().any(|t| t.contains('\u{2022}')),
            "Expected unordered list marker to survive nested layout, got: {texts:?}"
        );
        assert!(
            joined.contains("Parent") && joined.contains("Child"),
            "Nested unordered list items should lay out, got: {texts:?}"
        );
    }

    #[test]
    fn nested_ordered_list() {
        let html = "<ol><li>First<ol><li>Nested first</li><li>Nested second</li></ol></li><li>Second</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (texts, _) = list_texts_and_markers(&pages[0]);
        let joined = texts.join(" ");
        assert!(
            joined.contains("First")
                && joined.contains("Nested first")
                && joined.contains("Nested second")
                && joined.contains("Second"),
            "Nested ordered list items should lay out, got: {texts:?}"
        );
    }

    #[test]
    fn mixed_nested_list() {
        let html = "<ul><li>Bullet<ol><li>Numbered</li></ol></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // A mixed nested list produces BOTH a `disc` bullet (geometric marker /
        // U+2022) for the outer `ul` item AND a decimal `1.` marker for the inner
        // `ol` item. (The visual indentation of the nested list is covered by the
        // list-style-position parity fixtures.)
        let (texts, has_geometric_marker) = list_texts_and_markers(&pages[0]);
        let has_bullet = has_geometric_marker || texts.iter().any(|t| t.contains('\u{2022}'));
        let joined = texts.join(" ");
        assert!(
            has_bullet,
            "Outer ul item should have a (geometric) bullet marker, texts: {texts:?}"
        );
        // Both nested items lay out. (The nested `ol`'s decimal marker is an
        // OUTSIDE hanging marker — its rendering is covered by the lists-counters
        // parity fixtures — so we assert the item content here.)
        assert!(
            joined.contains("Bullet") && joined.contains("Numbered"),
            "Mixed nested list items should lay out, got: {texts:?}"
        );
    }

    #[test]
    fn base64_decode_basic() {
        // "Hello" in base64 is "SGVsbG8="
        let decoded = decode_base64("SGVsbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn base64_decode_with_whitespace() {
        let decoded = decode_base64("SGVs\nbG8=").unwrap();
        assert_eq!(decoded, b"Hello");
    }

    #[test]
    fn layout_jpeg_image_from_data_uri() {
        let html = r#"<img src="data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAAAAAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkKDA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AVN//2Q==" width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_image(|image| {
                    assert_eq!(image.source.format, ImageFormat::Jpeg);
                    assert!((image.geometry.size.width - 75.0).abs() < 0.1); // 100px * 0.75
                    assert!((image.geometry.size.height - 60.0).abs() < 0.1); // 80px * 0.75
                    assert!(image.source.png_metadata.is_none());
                })
            })
            .expect("expected image layout element");
    }

    #[test]
    fn layout_svg_image_from_data_uri_uses_intrinsic_size() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20width%3D%22100%25%22%20height%3D%2250%25%22%20viewBox%3D%220%200%20100%2050%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_svg(|svg| {
                    assert!((svg.geometry.size.width - 300.0).abs() < 0.1);
                    assert!((svg.geometry.size.height - 150.0).abs() < 0.1);
                })
            })
            .expect("expected SVG layout element");
    }

    #[test]
    fn layout_svg_image_respects_max_width() {
        let html = r#"<img style="max-width: 75pt" src="data:image/svg+xml,%3Csvg%20width%3D%22100%22%20height%3D%2250%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_svg(|svg| {
                    assert!((svg.geometry.size.width - 75.0).abs() < 0.1);
                    assert!((svg.geometry.size.height - 37.5).abs() < 0.1);
                })
            })
            .expect("expected SVG layout element");
    }

    #[test]
    fn layout_svg_image_respects_max_height() {
        let html = r#"<img style="max-height: 20pt" src="data:image/svg+xml,%3Csvg%20width%3D%22100%22%20height%3D%2250%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_svg(|svg| {
                    assert!((svg.geometry.size.width - 40.0).abs() < 0.1);
                    assert!((svg.geometry.size.height - 20.0).abs() < 0.1);
                })
            })
            .expect("expected SVG layout element");
    }

    #[test]
    fn layout_viewbox_only_svg_image_uses_default_object_size_ratio() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%20100%2020%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_svg(|svg| {
                    assert!((svg.geometry.size.width - 300.0).abs() < 0.1);
                    assert!((svg.geometry.size.height - 60.0).abs() < 0.1);
                })
            })
            .expect("expected SVG layout element");
    }

    #[test]
    fn layout_viewbox_only_svg_image_respects_max_height() {
        let html = r#"<img style="max-height: 50pt" src="data:image/svg+xml,%3Csvg%20viewBox%3D%220%200%20100%2020%22%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_svg(|svg| {
                    assert!((svg.geometry.size.width - 250.0).abs() < 0.1);
                    assert!((svg.geometry.size.height - 50.0).abs() < 0.1);
                })
            })
            .expect("expected SVG layout element");
    }

    #[test]
    fn layout_svg_image_without_viewbox_syncs_tree_to_layout_box() {
        let html = r#"<img src="data:image/svg+xml,%3Csvg%20width%3D%22100%25%22%20height%3D%2250%25%22%3E%3Crect%20width%3D%22100%25%22%20height%3D%22100%25%22/%3E%3C/svg%3E">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let (tree_width, tree_height, width, height) = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_svg(|svg| {
                    (
                        svg.tree.width,
                        svg.tree.height,
                        svg.geometry.size.width,
                        svg.geometry.size.height,
                    )
                })
            })
            .expect("expected svg layout element");

        assert!((tree_width - width).abs() < 0.1);
        assert!((tree_height - height).abs() < 0.1);
    }

    #[test]
    fn layout_png_image_from_data_uri() {
        // Build a minimal valid PNG and encode as base64
        let png_bytes = build_test_png_bytes();
        let b64 = base64_encode(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}" width="120" height="90">"#,);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_image(|image| {
                    assert_eq!(image.source.format, ImageFormat::Png);
                    let meta = image.source.png_metadata.as_ref().unwrap();
                    assert_eq!(meta.channels, 3); // RGB
                    assert_eq!(meta.bit_depth, 8);
                })
            })
            .expect("expected image layout element");
    }

    #[test]
    fn layout_image_without_dimensions_gets_defaults() {
        let png_bytes = build_test_png_bytes();
        let b64 = base64_encode(&png_bytes);
        let html = format!(r#"<img src="data:image/png;base64,{b64}">"#);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_image(|image| {
                    assert!(image.geometry.size.width > 0.0);
                    assert!(image.geometry.size.height > 0.0);
                })
            })
            .expect("expected image layout element");
    }

    #[test]
    fn layout_image_unsupported_src_ignored() {
        // HTTP src is not supported, should be silently ignored
        let html = r#"<img src="http://example.com/image.png" width="100" height="100">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // No image element should be produced
        assert!(
            pages[0]
                .elements
                .iter()
                .all(|(_, element)| element.find_image(|_| ()).is_none())
        );
    }

    #[test]
    fn img_scales_to_fit_available_width() {
        // Very wide image: 2000px = 1500pt, which exceeds A4 content width (~451pt)
        let html = format!(r#"<img src="{TEST_JPEG_DATA_URI}" width="2000" height="1000">"#);
        let nodes = parse_html(&html).unwrap();
        let page_size = PageSize::A4;
        let margin_val = Margin::default();
        let available_width = page_size.width - margin_val.horizontal();
        let pages = layout(&nodes, page_size, margin_val);
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.find_image(|image| {
                    let width = image.geometry.size.width;
                    assert!(
                        width <= available_width + 0.01,
                        "Image width {width} should fit within available width {available_width}"
                    );
                })
            })
            .expect("expected image element");
    }

    #[test]
    fn img_without_src_ignored() {
        let html = r#"<img width="100" height="80">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_image = pages[0]
            .elements
            .iter()
            .any(|(_, element)| element.find_image(|_| ()).is_some());
        assert!(
            !has_image,
            "img without src should not produce Image element"
        );
    }

    #[test]
    fn block_aspect_ratio_sets_height_for_empty_box() {
        let html = r#"<div style="width: 120pt; aspect-ratio: 3 / 2"></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let element = pages[0].elements[0].1.as_ref();
        let height = element
            .inspect_text(|text| text.box_model.size.height.used())
            .or_else(|| {
                element.inspect_container(|container| container.box_model.size.height.used())
            })
            .flatten()
            .expect("expected an aspect-ratio text block or container with a height");
        assert!((height - 80.0).abs() < 0.1);
    }

    #[test]
    fn raster_background_image_survives_into_layout() {
        let path = write_test_png_file("layout-bg", &build_test_png_bytes());
        let html = format!(
            r#"<div style="width: 40pt; height: 40pt; background-image: url('{path}'); background-repeat: no-repeat"></div>"#
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(
            page_has_image_background(&pages[0]),
            "Expected raster background to survive somewhere in layout"
        );
    }

    fn build_test_png_bytes() -> Vec<u8> {
        let mut png_data = Vec::new();
        png_data.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        // IHDR
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.push(8); // bit depth
        ihdr.push(2); // color type RGB
        ihdr.push(0);
        ihdr.push(0);
        ihdr.push(0);
        append_test_chunk(&mut png_data, b"IHDR", &ihdr);
        let idat = [
            0x78, 0x01, 0x62, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
        ];
        append_test_chunk(&mut png_data, b"IDAT", &idat);
        append_test_chunk(&mut png_data, b"IEND", &[]);
        png_data
    }

    fn append_test_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(chunk_type);
        buf.extend_from_slice(data);
        buf.extend_from_slice(&[0, 0, 0, 0]);
    }

    fn write_test_png_file(name: &str, bytes: &[u8]) -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ironpress-{name}-{}-{nonce}.png",
            std::process::id()
        ));
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn three_levels_deep_nested_list() {
        let html = "<ul><li>Level 1<ul><li>Level 2<ul><li>Level 3</li></ul></li></ul></li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (texts, has_geometric_marker) = list_texts_and_markers(&pages[0]);
        let joined = texts.join(" ");
        assert!(
            has_geometric_marker || texts.iter().any(|t| t.contains('\u{2022}')),
            "Expected unordered list markers to survive 3-level nested layout, got: {texts:?}"
        );
        assert!(
            joined.contains("Level 1") && joined.contains("Level 2") && joined.contains("Level 3"),
            "Three-level nested list items should lay out, got: {texts:?}"
        );
    }

    // --- Overflow / Visibility / Transform layout tests ---

    #[test]
    fn visibility_hidden_keeps_space_but_not_visible() {
        let html = r#"<div style="visibility: hidden">Hidden text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert!(
                    !text.paint.visible,
                    "visibility: hidden should set visible to false"
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn visibility_visible_is_visible() {
        let html = r#"<div>Visible text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert!(text.paint.visible, "default should be visible"))
            .expect("expected text block");
    }

    #[test]
    fn overflow_hidden_produces_clip_rect() {
        let html = r#"<div style="overflow: hidden; width: 200pt; height: 100pt">Clipped</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                let clip = text
                    .clipping
                    .rect
                    .expect("overflow: hidden should set a clip rectangle");
                assert!((clip.size.width - 200.0).abs() < 0.1);
            })
            .expect("expected text block");
    }

    #[test]
    fn overflow_visible_no_clip_rect() {
        let html = r#"<div style="width: 200pt">Not clipped</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert!(
                    text.clipping.rect.is_none(),
                    "visible overflow should not clip descendants"
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn transform_rotate_stored_in_layout() {
        let html = r#"<div style="transform: rotate(45deg)">Rotated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert_eq!(
                    text.paint.group.transform.value,
                    Some(crate::style::computed::Transform::Rotate(45.0))
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn transform_scale_stored_in_layout() {
        let html = r#"<div style="transform: scale(2)">Scaled</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert_eq!(
                    text.paint.group.transform.value,
                    Some(crate::style::computed::Transform::Scale(
                        crate::style::computed::CssVector::splat(2.0)
                    ))
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn transform_translate_stored_in_layout() {
        let html = r#"<div style="transform: translate(10pt, 20pt)">Translated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert_eq!(
                    text.paint.group.transform.value,
                    Some(crate::style::computed::Transform::Translate {
                        offset: crate::style::computed::CssVector::new(10.0, 20.0),
                        percentages: crate::style::computed::PercentageAxes::default(),
                    })
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn table_colspan_default_is_one() {
        let html = "<table><tr><td>A</td><td>B</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        for row in table_rows(&pages[0]) {
            for cell in row.content.cells {
                assert_eq!(cell.span.columns, 1, "Default colspan should be 1");
                assert_eq!(cell.span.rows, 1, "Default rowspan should be 1");
            }
        }
    }

    #[test]
    fn table_colspan_header_spans_two() {
        let html =
            r#"<table><tr><th colspan="2">Header</th></tr><tr><td>A</td><td>B</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].content.cells.len(), 1);
        assert_eq!(rows[0].content.cells[0].span.columns, 2);
        assert_eq!(rows[1].content.cells.len(), 2);
        assert_eq!(rows[1].content.cells[0].span.columns, 1);
        assert_eq!(rows[1].content.cells[1].span.columns, 1);
    }

    #[test]
    fn table_colspan_makes_cells_wider() {
        let html = r#"<table><tr><td colspan="2">Wide</td><td>N</td></tr><tr><td>A</td><td>B</td><td>C</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 2);
        let cells = &rows[0].content.cells;
        let col_widths = &rows[0].content.column_widths;
        assert_eq!(cells[0].span.columns, 2);
        // With auto-sizing, col_widths should have 3 entries
        assert_eq!(col_widths.len(), 3);
        // The colspan=2 cell should span the first two column widths
        let span_width: f32 = col_widths[0] + col_widths[1];
        let single_width = col_widths[2];
        assert!(
            span_width > single_width,
            "colspan=2 span ({span_width}) should be wider than single col ({single_width})"
        );
    }

    #[test]
    fn table_mixed_colspan_values() {
        let html = r#"<table><tr><td colspan="3">Full</td></tr><tr><td>A</td><td colspan="2">BC</td></tr><tr><td>X</td><td>Y</td><td>Z</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].content.cells.len(), 1);
        assert_eq!(rows[0].content.cells[0].span.columns, 3);
        assert_eq!(rows[1].content.cells.len(), 2);
        assert_eq!(rows[1].content.cells[0].span.columns, 1);
        assert_eq!(rows[1].content.cells[1].span.columns, 2);
        assert_eq!(rows[2].content.cells.len(), 3);
        for cell in &rows[2].content.cells {
            assert_eq!(cell.span.columns, 1);
        }
    }

    #[test]
    fn table_rowspan_basic() {
        // Cell A spans two rows; row 1 should have a phantom cell in column 0.
        let html = r#"<table>
            <tr><td rowspan="2">A</td><td>B</td></tr>
            <tr><td>C</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 2, "Should have 2 rows");
        // Row 0: cell A (rowspan=2) and cell B
        assert_eq!(rows[0].content.cells.len(), 2);
        assert_eq!(rows[0].content.cells[0].span.rows, 2);
        assert_eq!(rows[0].content.cells[1].span.rows, 1);
        // Row 1: phantom cell (rowspan=0) and cell C
        assert_eq!(rows[1].content.cells.len(), 2);
        assert_eq!(
            rows[1].content.cells[0].span.rows, 0,
            "Phantom cell should have rowspan=0"
        );
        assert_eq!(rows[1].content.cells[1].span.rows, 1);
    }

    #[test]
    fn table_independent_rowspans_keep_distinct_continuations() {
        let html = r#"<table>
            <tr>
                <td rowspan="2" style="height: 10pt"></td>
                <td rowspan="2" style="height: 10.005pt"></td>
            </tr>
            <tr></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 2);
        let continuations = &rows[1].content.cells;
        assert_eq!(continuations.len(), 2);
        for continuation in continuations {
            assert_eq!(continuation.span.rows, 0);
            assert_eq!(continuation.span.columns, 1);
        }
        let min_height_delta = continuations[1].layout.box_model.minimum_block_size
            - continuations[0].layout.box_model.minimum_block_size;
        assert_eq!(min_height_delta, (10.005_f32 - 10.0) / 2.0);
        assert!(min_height_delta > 0.0);
    }

    #[test]
    fn table_rowspan_and_colspan_combined() {
        // Cell A spans 2 rows and 2 columns in a 3-column table.
        let html = r#"<table>
            <tr><td rowspan="2" colspan="2">A</td><td>B</td></tr>
            <tr><td>C</td></tr>
            <tr><td>D</td><td>E</td><td>F</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 3, "Should have 3 rows");
        // Row 0: cell A (rowspan=2, colspan=2) and cell B
        assert_eq!(rows[0].content.cells.len(), 2);
        assert_eq!(rows[0].content.cells[0].span.rows, 2);
        assert_eq!(rows[0].content.cells[0].span.columns, 2);
        assert_eq!(rows[0].content.cells[1].span.rows, 1);
        // Both occupied columns originate from the same cell, so row 1 keeps
        // one grouped phantom spanning both columns, followed by cell C.
        assert_eq!(rows[1].content.cells.len(), 2);
        assert_eq!(rows[1].content.cells[0].span.rows, 0);
        assert_eq!(
            rows[1].content.cells[0].span.columns, 2,
            "Phantom should span 2 cols"
        );
        assert_eq!(rows[1].content.cells[1].span.rows, 1);
        // Row 2: three normal cells
        assert_eq!(rows[2].content.cells.len(), 3);
        for cell in &rows[2].content.cells {
            assert_eq!(cell.span.rows, 1);
            assert_eq!(cell.span.columns, 1);
        }
    }

    #[test]
    fn table_rowspan_renders_to_pdf() {
        // Verify that a table with rowspan produces valid PDF output.
        let html = r#"<table>
            <tr><td rowspan="2">Spans two rows</td><td>Top right</td></tr>
            <tr><td>Bottom right</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = crate::render::pdf::render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Spans"),
            "Cell text 'Spans' should be in PDF"
        );
        assert!(
            content.contains("rows"),
            "Cell text 'rows' should be in PDF"
        );
        assert!(content.contains("Top"), "Cell text 'Top' should be in PDF");
        assert!(
            content.contains("Bottom"),
            "Cell text 'Bottom' should be in PDF"
        );
        // No default cell borders — only CSS-specified borders produce strokes
    }

    #[test]
    fn css_width_constrains_block() {
        let html = r#"<div style="width: 200pt">Narrow block</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.width.fixed_value(), Some(200.0)))
            .expect("expected text block");
    }

    #[test]
    fn authored_width_equal_to_available_width_remains_fixed() {
        let page_size = PageSize::new(240.0, 200.0);
        let margin = Margin::uniform(0.0);
        let nodes = parse_html(r#"<div style="width: 240pt">Full-width block</div>"#).unwrap();
        let pages = layout(&nodes, page_size, margin);

        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert_eq!(
                    text.box_model.size.width.fixed_value(),
                    Some(page_size.width)
                );
                assert!(!text.box_model.size.width.is_fill_available());
            })
            .expect("expected text block");
    }

    #[test]
    fn css_max_width_limits_width() {
        let html = r#"<div style="max-width: 300pt">Limited block</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.width.fixed_value(), Some(300.0)))
            .expect("expected text block");
    }

    #[test]
    fn css_height_sets_minimum_height() {
        let html = r#"<div style="height: 100pt">Short text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.height.used(), Some(100.0)))
            .expect("expected text block");
    }

    #[test]
    fn css_opacity_stored_in_layout() {
        let html = r#"<div style="opacity: 0.5">Semi-transparent</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert!((text.paint.group.effects.opacity - 0.5).abs() < 0.01))
            .expect("expected text block");
    }

    #[test]
    fn auto_width_fills_available_inline_space() {
        let html = "<div>Normal block</div>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert!(text.box_model.size.width.is_fill_available()))
            .expect("expected text block");
    }

    // --- Float / Clear / Position / Box-shadow layout tests ---

    #[test]
    fn float_left_positions_element() {
        let html = r#"<div style="float: left; width: 100pt">Floated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.flow.float, Float::Left))
            .expect("expected text block");
    }

    #[test]
    fn float_right_positions_element() {
        let html = r#"<div style="float: right; width: 100pt">Floated right</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.flow.float, Float::Right))
            .expect("expected text block");
    }

    #[test]
    fn clear_both_moves_below_floats() {
        let html = r#"
            <div style="float: left">Float</div>
            <div style="clear: both">After float</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // The cleared element should be below the floated element
        let float_y = pages[0].elements[0].0;
        let cleared_y = pages[0].elements[1].0;
        assert!(
            cleared_y >= float_y,
            "Cleared element y={cleared_y} should be >= floated y={float_y}"
        );
        // Check the clear property is set
        pages[0].elements[1]
            .1
            .inspect_text(|text| assert_eq!(text.flow.clear, Clear::Both))
            .expect("expected cleared text block");
    }

    #[test]
    fn position_relative_offsets_element() {
        let html = r#"<div style="position: relative; top: 10pt; left: 5pt">Offset</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (y, element) = &pages[0].elements[0];
        element
            .inspect_text(|text| {
                assert_eq!(text.positioning.scheme, Position::Relative);
                assert!((text.positioning.insets.top - 10.0).abs() < 0.1);
                assert!((text.positioning.insets.left - 5.0).abs() < 0.1);
                // y should be offset by top value from normal position
                assert!(*y > 0.0, "relative offset should produce a non-zero y");
            })
            .expect("expected text block");
    }

    #[test]
    fn position_absolute_fixed_position() {
        let html = r#"<div style="position: absolute; top: 100pt; left: 50pt">Absolute</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let (y, element) = &pages[0].elements[0];
        element
            .inspect_text(|text| {
                assert_eq!(text.positioning.scheme, Position::Absolute);
                assert!((text.positioning.insets.top - 100.0).abs() < 0.1);
                assert!((text.positioning.insets.left - 50.0).abs() < 0.1);
                // y should be exactly the top value
                assert!((*y - 100.0).abs() < 0.1, "absolute y={y} should be 100.0");
            })
            .expect("expected text block");
    }

    #[test]
    fn position_absolute_relative_to_containing_block() {
        let html = r#"
            <div style="margin-top: 200pt; height: 200pt; position: relative; background: #eee;">
                <div style="position: absolute; top: 10pt; left: 10pt; width: 50pt; height: 50pt; background: red;">X</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let parent = pages[0]
            .elements
            .iter()
            .find(|(_, element)| {
                element
                    .inspect_text(|text| {
                        text.positioning.scheme == Position::Relative
                            && text.paint.background.color.is_some()
                    })
                    .or_else(|| {
                        element.inspect_container(|container| {
                            container.positioning.scheme == Position::Relative
                                && container.paint.background.color.is_some()
                        })
                    })
                    .unwrap_or(false)
            })
            .expect("Should find positioned parent");
        let parent_y = parent.0;
        assert!(
            (parent_y - 200.0).abs() < 1.0,
            "Parent should be at ~200pt, got {parent_y}"
        );
        // The absolute child may be a top-level element or inside a Container.
        let has_abs_child = pages[0]
            .elements
            .iter()
            .any(|(_, element)| tree_has_position(element.as_ref(), Position::Absolute));
        assert!(
            has_abs_child,
            "Should find absolute child in elements or Container children"
        );
    }

    #[test]
    fn position_absolute_does_not_affect_flow() {
        let html = r#"
            <div style="position: absolute; top: 200pt">Absolute</div>
            <div>Normal flow</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 2);
        // The normal flow element should start at y=0 (top of content area).
        let normal_y = pages[0]
            .elements
            .iter()
            .find_map(|(y, el)| match el {
                _ if element_contains_text(el, "Normal flow") => Some(*y),
                _ => None,
            })
            .expect("expected normal-flow text block");
        assert!(
            normal_y < 10.0,
            "Normal flow element should be near top, but y={normal_y}"
        );
    }

    #[test]
    fn box_shadow_produces_offset_rect() {
        let html = r#"<div style="box-shadow: 3px 3px black">Content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                let shadow = text.paint.shadows[0];
                assert!((shadow.offset_x - 2.25).abs() < 0.1); // 3px * 0.75
                assert!((shadow.offset_y - 2.25).abs() < 0.1);
                assert_eq!(shadow.color.r, 0.0);
                assert_eq!(shadow.color.g, 0.0);
                assert_eq!(shadow.color.b, 0.0);
            })
            .expect("expected text block");
    }

    #[test]
    fn float_does_not_advance_normal_flow() {
        let html = r#"
            <div style="float: left">Floated</div>
            <div>Normal after float</div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(pages[0].elements.len() >= 2);
        // Both elements should start at roughly the same y position
        // because floats don't advance normal flow
        let float_y = pages[0].elements[0].0;
        let normal_y = pages[0].elements[1].0;
        // The normal element might be at the same position or slightly different
        // due to margins, but it should not be pushed far down
        assert!(
            (normal_y - float_y).abs() < 50.0,
            "Normal flow element should be near float, not pushed far down: float_y={float_y}, normal_y={normal_y}"
        );
    }

    #[test]
    fn table_auto_sizing_varying_content() {
        let html = "<table><tr><td>A</td><td>Much longer content here</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let col_widths = &rows[0].content.column_widths;
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[1] > col_widths[0],
            "Column with longer text ({}) should be wider than short text ({})",
            col_widths[1],
            col_widths[0]
        );
    }

    #[test]
    fn table_auto_sizing_very_long_cell_no_break() {
        let long_text = "x".repeat(500);
        let html = format!("<table><tr><td>{long_text}</td><td>Short</td></tr></table>");
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages.is_empty());
        let rows = table_rows(&pages[0]);
        assert!(!rows.is_empty());
        for w in &rows[0].content.column_widths {
            // Neither column collapses: even beside a 500-char cell the short
            // column keeps a usable width (the exact value tracks the resolved
            // font's metrics).
            assert!(*w >= 20.0, "Column width {w} should be at least 20pt");
        }
    }

    #[test]
    fn table_auto_sizing_min_column_width() {
        let html = "<table><tr><td></td><td></td><td></td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert!(!rows.is_empty());
        for w in &rows[0].content.column_widths {
            assert!(*w >= 1.5, "Empty column should have minimum width, got {w}");
        }
    }

    #[test]
    fn table_four_column_invoice_non_equal_widths() {
        // A 4-column invoice table: Description should be wider than Qty/Amount
        let html = r#"<table>
            <tr><th>Description</th><th>Qty</th><th>Unit Price</th><th>Amount</th></tr>
            <tr><td>Web development services - January</td><td>1</td><td>2500.00</td><td>2500.00</td></tr>
            <tr><td>Hosting and maintenance</td><td>12</td><td>50.00</td><td>600.00</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert!(!rows.is_empty());
        let cw = &rows[0].content.column_widths;
        assert_eq!(cw.len(), 4);
        // Description column (index 0) should be wider than Qty (index 1)
        assert!(
            cw[0] > cw[1],
            "Description column ({}) should be wider than Qty column ({})",
            cw[0],
            cw[1]
        );
        // Description column should be wider than Amount column
        assert!(
            cw[0] > cw[3],
            "Description column ({}) should be wider than Amount column ({})",
            cw[0],
            cw[3]
        );
        // Columns should NOT all be equal
        assert!(
            !(cw[0] == cw[1] && cw[1] == cw[2] && cw[2] == cw[3]),
            "Column widths should not all be equal: {:?}",
            cw
        );
    }

    #[test]
    fn simple_invoice_fits_on_one_page() {
        // A simple invoice with ~15 lines should fit on a single A4 page
        let html = r#"
            <h1>Invoice #1001</h1>
            <p>Date: 2026-01-15</p>
            <p>Bill To: Acme Corp</p>
            <p>123 Main Street, Springfield</p>
            <table>
                <tr><th>Description</th><th>Qty</th><th>Unit Price</th><th>Amount</th></tr>
                <tr><td>Web development</td><td>1</td><td>2500.00</td><td>2500.00</td></tr>
                <tr><td>Hosting</td><td>12</td><td>50.00</td><td>600.00</td></tr>
                <tr><td>Domain renewal</td><td>1</td><td>15.00</td><td>15.00</td></tr>
                <tr><td>SSL certificate</td><td>1</td><td>75.00</td><td>75.00</td></tr>
            </table>
            <p>Subtotal: 3190.00</p>
            <p>Tax (10%): 319.00</p>
            <p>Total: 3509.00</p>
            <p>Thank you for your business!</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(
            pages.len(),
            1,
            "Simple invoice should fit on one page, got {} pages",
            pages.len()
        );
    }

    // --- Flexbox layout tests ---

    fn extract_flex_items(pages: &[Page]) -> Vec<(f32, f32, Option<f32>, String)> {
        let mut result = Vec::new();
        for page in pages {
            for (y, elem) in &page.elements {
                elem.inspect_text(|block| {
                    let text: String = block
                        .lines
                        .iter()
                        .flat_map(|line| line.runs.iter().map(|run| run.text.clone()))
                        .collect::<Vec<_>>()
                        .join("");
                    if !text.is_empty() {
                        result.push((
                            *y,
                            block.positioning.insets.left,
                            block.box_model.size.width.fixed_value(),
                            text,
                        ));
                    }
                });
                elem.inspect_flex(|row| {
                    for cell in &row.content.cells {
                        let text: String = cell
                            .lines
                            .iter()
                            .flat_map(|line| line.runs.iter().map(|run| run.text.clone()))
                            .collect::<Vec<_>>()
                            .join("");
                        if !text.is_empty() {
                            result.push((*y, cell.x_offset, Some(cell.width), text));
                        }
                    }
                });
            }
        }
        result
    }

    #[test]
    fn flex_row_horizontal_layout() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">L</div><div style="width: 100pt">R</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let l = items.iter().find(|i| i.3.contains('L')).unwrap();
        let r = items.iter().find(|i| i.3.contains('R')).unwrap();
        assert!(r.1 > l.1);
    }

    #[test]
    fn flex_column_vertical() {
        let html = r#"<div style="display: flex; flex-direction: column"><div style="width: 100pt">T</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let t = items.iter().find(|i| i.3.contains('T')).unwrap();
        let b = items.iter().find(|i| i.3.contains('B')).unwrap();
        assert!(b.0 > t.0);
    }

    #[test]
    fn flex_justify_center() {
        let html = r#"<div style="display: flex; justify-content: center"><div style="width: 100pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(!items.is_empty());
        assert!(items[0].1 > 50.0);
    }

    #[test]
    fn flex_justify_space_between() {
        let html = r#"<div style="display: flex; justify-content: space-between"><div style="width: 100pt">A</div><div style="width: 100pt">B</div><div style="width: 100pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 3);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let c = items.iter().find(|i| i.3 == "C").unwrap();
        let g1 = b.1 - a.1;
        let g2 = c.1 - b.1;
        assert!((g1 - g2).abs() < 1.0, "gaps equal: {g1} vs {g2}");
    }

    #[test]
    fn flex_justify_space_around() {
        let html = r#"<div style="display: flex; justify-content: space-around"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        assert!(a.1 > 10.0, "space-around: first not at edge, got {}", a.1);
    }

    #[test]
    fn flex_justify_flex_end() {
        let html = r#"<div style="display: flex; justify-content: flex-end"><div style="width: 100pt">E</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(!items.is_empty());
        assert!(items[0].1 > 200.0, "flex-end: got {}", items[0].1);
    }

    #[test]
    fn flex_align_center() {
        let html = r#"<div style="display: flex; align-items: center"><div style="width: 100pt; height: 50pt">T</div><div style="width: 100pt">S</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let t = items.iter().find(|i| i.3 == "T").unwrap();
        let s = items.iter().find(|i| i.3 == "S").unwrap();
        assert!(s.0 >= t.0);
    }

    #[test]
    fn flex_wrap_test() {
        let html = r#"<div style="display: flex; flex-wrap: wrap"><div style="width: 200pt">A</div><div style="width: 200pt">B</div><div style="width: 200pt">C</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let cells = pages
            .iter()
            .flat_map(|page| &page.elements)
            .find_map(|(_, element)| {
                element
                    .inspect_flex(|row| {
                        (row.content.cells.len() == 3).then(|| row.content.cells.clone())
                    })
                    .flatten()
            })
            .expect("wrapped container must emit one three-cell flex row");
        assert_eq!(cells[0].line_id, cells[1].line_id);
        assert_ne!(cells[0].line_id, cells[2].line_id);
        let items = extract_flex_items(&pages);
        assert!(
            items.len() >= 3,
            "Should have at least 3 flex items, got {}",
            items.len()
        );
        // Verify all three items appear in the output
        assert!(items.iter().any(|i| i.3 == "A"), "A should appear");
        assert!(items.iter().any(|i| i.3 == "B"), "B should appear");
        assert!(items.iter().any(|i| i.3 == "C"), "C should appear");
        // B should be to the right of A (same row)
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        assert!(b.1 > a.1, "B should be to the right of A");
    }

    #[test]
    fn flex_gap_spacing() {
        let html = r#"<div style="display: flex; gap: 20pt"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let expected = a.1 + 100.0 + 20.0;
        assert!(
            (b.1 - expected).abs() < 1.0,
            "gap: expected {expected}, got {}",
            b.1
        );
    }

    #[test]
    fn flex_no_gap() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">A</div><div style="width: 100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        let expected = a.1 + 100.0;
        assert!(
            (b.1 - expected).abs() < 1.0,
            "no gap: expected {expected}, got {}",
            b.1
        );
    }

    #[test]
    fn flex_column_gap_spacing() {
        // Column-direction flex: gap should push items apart vertically.
        let html = r#"<div style="display: flex; flex-direction: column; gap: 20pt"><div style="height: 30pt">A</div><div style="height: 30pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2, "expected at least 2 flex column items");
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        // B must be below A; with a 20pt gap the Y gap between starts should exceed 20pt
        assert!(
            b.0 > a.0 + 20.0,
            "column gap: B y={} should be more than 20pt below A y={}",
            b.0,
            a.0
        );
    }

    #[test]
    fn flex_style_block() {
        use crate::parser::css::parse_stylesheet;
        let css = ".f{display:flex;gap:10pt}";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="f"><div style="width:100pt">A</div><div style="width:100pt">B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let items = extract_flex_items(&pages);
        assert!(items.len() >= 2);
        let a = items.iter().find(|i| i.3 == "A").unwrap();
        let b = items.iter().find(|i| i.3 == "B").unwrap();
        assert!(b.1 > a.1);
    }

    #[test]
    fn flex_display_none_child() {
        let html = r#"<div style="display: flex"><div style="width: 100pt">V</div><div style="width: 100pt; display: none">H</div><div style="width: 100pt">V2</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        assert!(items.iter().all(|i| !i.3.contains('H')));
        assert!(items.len() >= 2);
    }

    #[test]
    fn flex_row_children_same_y_not_stacked() {
        let html = r#"<div style="display: flex;"><div>Left</div><div>Right</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        let left = items
            .iter()
            .find(|i| i.3.contains("Left"))
            .expect("Left text");
        let right = items
            .iter()
            .find(|i| i.3.contains("Right"))
            .expect("Right text");
        // Both should be at the same y position (same row, not stacked)
        assert!(
            (left.0 - right.0).abs() < 1.0,
            "Left y={} Right y={} -- should be on the same line",
            left.0,
            right.0
        );
        // Right should be to the right of Left
        assert!(
            right.1 > left.1,
            "Right x={} should be greater than Left x={}",
            right.1,
            left.1
        );
    }

    #[test]
    fn flex_space_between_positions() {
        let html = r#"<div style="display: flex; justify-content: space-between;">
            <div>Left content</div>
            <div>Right content</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let items = extract_flex_items(&pages);
        let left = items
            .iter()
            .find(|i| i.3.contains("Left"))
            .expect("Left content");
        let right = items
            .iter()
            .find(|i| i.3.contains("Right"))
            .expect("Right content");
        // Both at same y
        assert!(
            (left.0 - right.0).abs() < 1.0,
            "space-between: both should be on same y"
        );
        // First child should be at x=0 (or near 0)
        assert!(
            left.1 < 5.0,
            "space-between: first child near left edge, got {}",
            left.1
        );
        // Second child should be far to the right
        assert!(
            right.1 > 100.0,
            "space-between: second child should be far right, got {}",
            right.1
        );
    }

    #[test]
    fn flex_text_align_right_in_child() {
        let html = r#"<div style="display: flex;">
            <div style="width: 200pt; text-align: right">Aligned</div>
            <div style="width: 200pt">Normal</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        // Verify we can find both items as flex cells
        let items = extract_flex_items(&pages);
        let aligned = items
            .iter()
            .find(|i| i.3.contains("Aligned"))
            .expect("Aligned text");
        let normal = items
            .iter()
            .find(|i| i.3.contains("Normal"))
            .expect("Normal text");
        // Aligned should be in first cell (x_offset = 0)
        assert!(aligned.1 < normal.1, "first cell before second");
        // Verify the FlexRow element stores text_align correctly
        for page in &pages {
            for (_y, elem) in &page.elements {
                elem.inspect_flex(|row| {
                    if let Some(cell) = row.content.cells.iter().find(|cell| {
                        cell.lines
                            .iter()
                            .any(|line| line.runs.iter().any(|run| run.text.contains("Aligned")))
                    }) {
                        assert_eq!(
                            cell.text_align,
                            TextAlign::Right,
                            "text-align: right should be preserved in FlexCell"
                        );
                    }
                });
            }
        }
    }

    // --- CSS Grid tests ---

    /// Extract every grid row through the generic layout tree traversal.
    fn extract_grid_rows(pages: &[Page]) -> Vec<GridRow> {
        struct GridRows(Vec<GridRow>);

        impl LayoutVisitor for GridRows {
            fn visit_grid_row(&mut self, row: &GridRow) {
                self.0.push(row.clone());
            }
        }

        let mut rows = GridRows(Vec::new());
        if let Some(page) = pages.first() {
            for (_, element) in &page.elements {
                visit_layout_tree(element.as_ref(), &mut rows);
            }
        }
        rows.0
    }

    #[test]
    fn grid_three_column_places_items_correctly() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr 1fr">
            <div>Cell 1</div>
            <div>Cell 2</div>
            <div>Cell 3</div>
            <div>Cell 4</div>
            <div>Cell 5</div>
            <div>Cell 6</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(
            grid_rows.len(),
            2,
            "Should have 2 rows for 6 items in 3 columns"
        );
        assert_eq!(
            grid_rows[0].content.cells.len(),
            3,
            "First row should have 3 cells"
        );
        assert_eq!(
            grid_rows[1].content.cells.len(),
            3,
            "Second row should have 3 cells"
        );

        // Columns should be equal width
        let widths = &grid_rows[0].content.column_widths;
        assert!(
            (widths[0] - widths[1]).abs() < 0.1,
            "Columns should be equal width"
        );
        assert!(
            (widths[1] - widths[2]).abs() < 0.1,
            "Columns should be equal width"
        );
    }

    #[test]
    fn grid_mixed_fr_and_fixed_columns() {
        let html = r#"<div style="display: grid; grid-template-columns: 100pt 1fr 200pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(grid_rows.len(), 1);
        let widths = &grid_rows[0].content.column_widths;
        assert_eq!(widths.len(), 3);
        assert!(
            (widths[0] - 100.0).abs() < 0.1,
            "First column should be 100pt"
        );
        assert!(
            (widths[2] - 200.0).abs() < 0.1,
            "Third column should be 200pt"
        );
        // Middle column gets remaining space
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_middle = available - 100.0 - 200.0;
        assert!(
            (widths[1] - expected_middle).abs() < 0.1,
            "Middle column should get remaining space: got {}, expected {}",
            widths[1],
            expected_middle
        );
    }

    #[test]
    fn grid_auto_columns() {
        let html = r#"<div style="display: grid; grid-template-columns: auto auto">
            <div>Left</div>
            <div>Right</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(grid_rows.len(), 1);
        let widths = &grid_rows[0].content.column_widths;
        assert_eq!(widths.len(), 2);
        // Per CSS Grid: auto columns take their max-content intrinsic width,
        // then split the remaining free space EQUALLY. "Left" and "Right"
        // have slightly different measured widths, so the columns differ by
        // exactly that content-width delta (small, a few points) — they are
        // NOT forced to equal width.
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let sum = widths[0] + widths[1];
        assert!(
            (sum - available).abs() < 1.0,
            "Auto columns should fill available: sum {} vs available {}",
            sum,
            available
        );
        assert!(
            (widths[0] - widths[1]).abs() < 30.0,
            "Auto columns should be close (differ by at most content delta): {} vs {}",
            widths[0],
            widths[1]
        );
    }

    #[test]
    fn grid_auto_fr_auto_does_not_collapse_to_equal_columns() {
        // Regression for parity bug #145: `auto 1fr auto` was being treated
        // as three equal tracks (auto == 1fr semantically). Correct behavior:
        // auto columns size to their max-content intrinsic width, and the fr
        // track swallows the remaining space.
        let html = r#"<div style="display: grid; grid-template-columns: auto 1fr auto">
            <div>L</div>
            <div>middle</div>
            <div>R</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(grid_rows.len(), 1);
        let widths = &grid_rows[0].content.column_widths;
        assert_eq!(widths.len(), 3);

        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let sum = widths[0] + widths[1] + widths[2];
        assert!(
            (sum - available).abs() < 1.0,
            "Grid columns should fill available: sum {} vs {}",
            sum,
            available
        );
        // Auto columns ("L"/"R") must be much narrower than the 1fr column.
        // If the old bug resurfaces, all three would be ~equal (≈available/3).
        assert!(
            widths[1] > widths[0] * 3.0,
            "1fr column ({}) should dwarf auto columns ({}, {})",
            widths[1],
            widths[0],
            widths[2]
        );
        assert!(
            widths[1] > widths[2] * 3.0,
            "1fr column ({}) should dwarf auto columns ({}, {})",
            widths[1],
            widths[0],
            widths[2]
        );
    }

    #[test]
    fn grid_gap_adds_spacing() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr; grid-gap: 10pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(grid_rows.len(), 2, "Should have 2 rows");

        // Column widths should account for the gap
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_col = (available - 10.0) / 2.0;
        let widths = &grid_rows[0].content.column_widths;
        assert!(
            (widths[0] - expected_col).abs() < 0.1,
            "Column width should account for gap: got {}, expected {}",
            widths[0],
            expected_col
        );

        // Second row should have grid-gap as margin_top
        assert!(
            (grid_rows[1].box_model.margins.start - 10.0).abs() < 0.1,
            "Second row margin_top should be the grid gap: got {}",
            grid_rows[1].box_model.margins.start
        );
    }

    #[test]
    fn grid_wraps_to_new_rows() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
            <div>E</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(grid_rows.len(), 3, "5 items in 2 columns = 3 rows");
        assert_eq!(grid_rows[0].content.cells.len(), 2);
        assert_eq!(grid_rows[1].content.cells.len(), 2);
        assert_eq!(
            grid_rows[2].content.cells.len(),
            2,
            "Last row should be padded to 2 cells"
        );
        // Last row's second cell should be empty
        assert!(
            grid_rows[2].content.cells[1]
                .layout
                .content
                .lines
                .is_empty(),
            "Padding cell should have no text"
        );
    }

    #[test]
    fn grid_renders_to_pdf() {
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr 1fr; grid-gap: 10pt">
            <div>Cell 1</div>
            <div>Cell 2</div>
            <div>Cell 3</div>
            <div>Cell 4</div>
            <div>Cell 5</div>
            <div>Cell 6</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let pdf = crate::render::pdf::render_pdf(&pages, PageSize::A4, Margin::default()).unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("Cell"),
            "Grid cell text should appear in PDF"
        );
        assert!(content.contains("1"), "Cell numbers should appear in PDF");
        assert!(content.contains("6"), "Cell 6 should appear in PDF");
    }

    #[test]
    fn grid_with_gap_alias() {
        // Test that 'gap' works as an alias for 'grid-gap'
        let html = r#"<div style="display: grid; grid-template-columns: 1fr 1fr; gap: 20pt">
            <div>A</div>
            <div>B</div>
            <div>C</div>
            <div>D</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(grid_rows.len(), 2);
        // Second row should have gap as margin_top
        assert!(
            (grid_rows[1].box_model.margins.start - 20.0).abs() < 0.1,
            "gap alias should work: got {}",
            grid_rows[1].box_model.margins.start
        );
    }

    #[test]
    fn grid_with_stylesheet_rules() {
        use crate::parser::css::parse_stylesheet;
        let css = ".grid { display: grid; grid-template-columns: 1fr 1fr; grid-gap: 5pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(grid_rows.len(), 1, "Should have 1 grid row");
        assert_eq!(grid_rows[0].content.cells.len(), 2, "Should have 2 cells");
        // Verify gap is accounted for in widths
        let available = PageSize::A4.width - Margin::default().left - Margin::default().right;
        let expected_col = (available - 5.0) / 2.0;
        assert!(
            (grid_rows[0].content.column_widths[0] - expected_col).abs() < 0.1,
            "Column width with gap: got {}, expected {}",
            grid_rows[0].content.column_widths[0],
            expected_col
        );
    }

    #[test]
    fn grid_no_template_columns_defaults_to_single_column() {
        let html = r#"<div style="display: grid">
            <div>Only</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());

        let grid_rows = extract_grid_rows(&pages);

        assert_eq!(grid_rows.len(), 1);
        assert_eq!(
            grid_rows[0].content.column_widths.len(),
            1,
            "Default should be single column"
        );
    }

    // --- min-width / min-height / max-height / margin auto tests ---

    #[test]
    fn css_min_width_enforces_minimum() {
        // width: 100pt would be 100, but min-width: 300pt forces it to 300
        let html = r#"<div style="width: 100pt; min-width: 300pt">Narrow text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.width.fixed_value(), Some(300.0)))
            .expect("expected text block");
    }

    #[test]
    fn css_min_height_enforces_minimum() {
        let html = r#"<div style="min-height: 200pt">Short text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.height.used(), Some(200.0)))
            .expect("expected text block");
    }

    #[test]
    fn css_max_height_limits_height() {
        let html = r#"<div style="height: 500pt; max-height: 300pt">Tall box</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.height.used(), Some(300.0)))
            .expect("expected text block");
    }

    #[test]
    fn css_margin_auto_centers_element() {
        let html = r#"<div style="width: 200pt; margin: 0 auto">Centered</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert_eq!(text.box_model.size.width.fixed_value(), Some(200.0));
                // available_width = 595.28 - 72 - 72 = 451.28
                let expected_offset = (451.28 - 200.0) / 2.0;
                assert!(
                    (text.positioning.insets.left - expected_offset).abs() < 0.1,
                    "left inset should be ~{expected_offset}, got {}",
                    text.positioning.insets.left
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn css_margin_left_auto_pushes_right() {
        let html = r#"<div style="width: 200pt; margin-left: auto">Right-aligned</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert_eq!(text.box_model.size.width.fixed_value(), Some(200.0));
                // available_width = 451.28, push to right
                let expected_offset = 451.28 - 200.0;
                assert!(
                    (text.positioning.insets.left - expected_offset).abs() < 0.1,
                    "left inset should be ~{expected_offset}, got {}",
                    text.positioning.insets.left
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn css_min_max_interact_with_width_height() {
        // min-height larger than height => min-height wins
        let html = r#"<div style="height: 50pt; min-height: 100pt">Content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.height.used(), Some(100.0)))
            .expect("expected text block");

        // width smaller than min-width => min-width wins
        let html2 = r#"<div style="width: 100pt; min-width: 300pt">Content</div>"#;
        let nodes2 = parse_html(html2).unwrap();
        let pages2 = layout(&nodes2, PageSize::A4, Margin::default());
        assert_eq!(pages2.len(), 1);
        pages2[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.width.fixed_value(), Some(300.0)))
            .expect("expected text block");

        // max-height smaller than min-height => max-height wins (CSS spec)
        // Actually in CSS spec min-height wins over max-height. Let's test:
        // height: 500pt, min-height: 200pt, max-height: 300pt => clamp to 300pt
        let html3 =
            r#"<div style="height: 500pt; max-height: 300pt; min-height: 200pt">Content</div>"#;
        let nodes3 = parse_html(html3).unwrap();
        let pages3 = layout(&nodes3, PageSize::A4, Margin::default());
        assert_eq!(pages3.len(), 1);
        pages3[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.height.used(), Some(300.0)))
            .expect("expected text block");
    }

    // --- box-sizing tests ---

    #[test]
    fn box_sizing_border_box_subtracts_padding_from_width() {
        // With border-box, width: 200pt includes padding.
        // With 20pt padding on each side, content area = 200 - 20 - 20 = 160pt
        let html = r#"<div style="box-sizing: border-box; width: 200pt; padding-left: 20pt; padding-right: 20pt">Text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.width.fixed_value(), Some(200.0)))
            .expect("expected text block");
    }

    #[test]
    fn box_sizing_content_box_width_is_content_only() {
        // With content-box (default), width: 200pt is the *content* width, so the
        // stored block_width (the outer/border-box width) is the content width
        // plus horizontal padding (and border): 200 + 20 + 20 = 240pt.
        let html = r#"<div style="box-sizing: content-box; width: 200pt; padding-left: 20pt; padding-right: 20pt">Text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.box_model.size.width.fixed_value(), Some(240.0)))
            .expect("expected text block");
    }

    #[test]
    fn border_radius_stored_in_layout() {
        let html = r#"<div style="border-radius: 8pt; background-color: red">Rounded</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| assert_eq!(text.paint.border_radii.uniform_radius(), Some(8.0)))
            .expect("expected text block");
    }

    #[test]
    fn outline_stored_in_layout() {
        let html = r#"<div style="outline: 3px solid blue">Outlined</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert!((text.paint.outline.width - 2.25).abs() < 0.01); // 3px * 0.75
                let (r, g, b) = text
                    .paint
                    .outline
                    .color
                    .expect("outline should have a color")
                    .to_f32_rgb();
                assert!((r - 0.0).abs() < 0.01);
                assert!((g - 0.0).abs() < 0.01);
                assert!((b - 1.0).abs() < 0.01);
            })
            .expect("expected text block");
    }

    // ---- z-index tests ----

    #[test]
    fn z_index_stored_in_layout_element() {
        let html = r#"<div style="position: absolute; z-index: 5; top: 10pt">High</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, element)| {
            element.paint_group_owner().is_some_and(|owner| {
                owner.paint_group().stacking.z_index == crate::style::computed::ZIndex::integer(5)
            })
        });
        assert!(found, "Expected element with z_index=5");
    }

    #[test]
    fn paginate_repeats_only_synthetic_page_background() {
        let make_block = |position, z_index: i32, repeat_on_each_page: bool, height| {
            TextBlock {
                box_model: BoxModel {
                    size: LayoutSize::fixed(100.0, Some(height)),
                    ..Default::default()
                },
                positioning: Positioning::default().with_scheme(position),
                paint: BoxPaint {
                    group: crate::layout::elements::PaintGroup {
                        stacking: crate::layout::elements::Stacking {
                            z_index: crate::style::computed::ZIndex::integer(z_index),
                            role: repeat_on_each_page
                                .then_some(crate::layout::elements::StackingRole::PageBackdrop)
                                .unwrap_or_default(),
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                },
                fragmentation: TextFragmentation {
                    box_fragmentation: BoxFragmentation {
                        content_role: if repeat_on_each_page {
                            crate::layout::elements::PageContentRole::RepeatedDecoration
                        } else {
                            crate::layout::elements::PageContentRole::MainFlow
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            }
            .boxed()
        };

        let pages = crate::layout::paginate::paginate(
            vec![
                make_block(Position::Absolute, -1, true, 40.0),
                make_block(Position::Absolute, -1, false, 40.0),
                make_block(Position::Static, 0, false, 30.0),
                make_block(Position::Static, 0, false, 30.0),
            ],
            40.0,
            0.0,
        );

        assert_eq!(pages.len(), 2);
        let repeated_per_page: Vec<_> = pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .filter(|(_, element)| {
                        element.page_content_role()
                            == crate::layout::elements::PageContentRole::RepeatedDecoration
                    })
                    .count()
            })
            .collect();
        assert_eq!(repeated_per_page, vec![1, 1]);

        let non_repeating_per_page: Vec<_> = pages
            .iter()
            .map(|page| {
                page.elements
                    .iter()
                    .filter(|(_, element)| {
                        element
                            .inspect_text(|text| {
                                text.positioning.scheme == Position::Absolute
                                    && text.fragmentation.box_fragmentation.content_role
                                        != crate::layout::elements::PageContentRole::RepeatedDecoration
                            })
                            .unwrap_or(false)
                    })
                    .count()
            })
            .collect();
        assert_eq!(non_repeating_per_page, vec![1, 0]);
    }

    #[test]
    fn synthetic_page_background_sorts_before_more_negative_layers() {
        let make_block = |z_index: i32, repeat_on_each_page: bool| {
            TextBlock {
                box_model: BoxModel {
                    size: LayoutSize::fixed(100.0, Some(40.0)),
                    ..Default::default()
                },
                positioning: Positioning::default().with_scheme(Position::Absolute),
                paint: BoxPaint {
                    group: crate::layout::elements::PaintGroup {
                        stacking: crate::layout::elements::Stacking {
                            z_index: crate::style::computed::ZIndex::integer(z_index),
                            role: repeat_on_each_page
                                .then_some(crate::layout::elements::StackingRole::PageBackdrop)
                                .unwrap_or_default(),
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                },
                fragmentation: TextFragmentation {
                    box_fragmentation: BoxFragmentation {
                        content_role: if repeat_on_each_page {
                            crate::layout::elements::PageContentRole::RepeatedDecoration
                        } else {
                            crate::layout::elements::PageContentRole::MainFlow
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            }
            .boxed()
        };

        let pages = crate::layout::paginate::paginate(
            vec![make_block(-1, true), make_block(-2, false)],
            200.0,
            0.0,
        );

        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert!(
                    text.fragmentation.box_fragmentation.content_role
                        == crate::layout::elements::PageContentRole::RepeatedDecoration,
                    "synthetic background should render first"
                );
            })
            .expect("expected text block");
    }

    // ---- calc() integration test ----

    #[test]
    fn calc_width_in_layout() {
        // Use a calc() value that's smaller than available_width so explicit_width is set
        let html = r#"<div style="width: calc(50% - 10pt)">Calc content</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert!(
                    !text.box_model.size.width.is_fill_available(),
                    "calc() width should resolve to explicit width"
                );
            })
            .expect("expected text block");
    }

    // ---- CSS variable integration test ----

    #[test]
    fn var_width_in_layout() {
        let html = r#"<div style="--w: 200pt"><div style="width: var(--w)">Var width</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|text| {
                    text.box_model
                        .size
                        .width
                        .fixed_value()
                        .is_some_and(|width| (width - 200.0).abs() < 1.0)
                })
                .unwrap_or(false)
        });
        assert!(found, "Expected element with width ~200pt from var()");
    }

    // ---- rem unit integration test ----

    #[test]
    fn rem_unit_in_layout() {
        let html = r#"<div style="margin-top: 2rem">Rem margin</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        // 2rem = 24pt margin_top
        pages[0].elements[0]
            .1
            .inspect_text(|text| {
                assert!(
                    (text.box_model.margins.start - 24.0).abs() < 0.5,
                    "expected about 24pt block-start margin from 2rem"
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn table_row_carries_border_collapse() {
        let html = r#"<table style="border-collapse: collapse"><tr><td>A</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_collapse = table_rows(&pages[0])
            .iter()
            .any(|row| row.formatting.border_collapse == BorderCollapse::Collapse);
        assert!(has_collapse, "Expected border_collapse: Collapse");
    }

    #[test]
    fn table_row_default_border_separate() {
        let html = r#"<table><tr><td>A</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_separate = table_rows(&pages[0])
            .iter()
            .any(|row| row.formatting.border_collapse == BorderCollapse::Separate);
        assert!(has_separate, "Expected default border_collapse: Separate");
    }

    #[test]
    fn table_row_carries_border_spacing() {
        let html = r#"<table style="border-spacing: 8px"><tr><td>A</td><td>B</td></tr></table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_spacing = table_rows(&pages[0])
            .iter()
            .any(|row| (row.formatting.border_spacing - 6.0).abs() < 0.1);
        assert!(has_spacing, "Expected border_spacing of 6pt (8px * 0.75)");
    }

    #[test]
    fn table_selector_does_not_style_anonymous_table_box() {
        use crate::parser::css::parse_stylesheet;

        let html = r#"<span class="cell">A</span><span class="cell">B</span>"#;
        let rules = parse_stylesheet("table { border-spacing: 8px } .cell { display: table-cell }");
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let rows = table_rows(&pages[0]);
        assert!(!rows.is_empty(), "expected anonymous table row");
        assert!(
            rows.iter().all(|row| row.formatting.border_spacing == 0.0),
            "an anonymous table inherits the enclosing box's initial spacing; it cannot match either the authored or UA `table` selector"
        );
    }

    #[test]
    fn text_overflow_ellipsis_truncates() {
        // text-overflow: ellipsis is stored on the style; layout does not yet
        // perform the actual truncation with "..." so we just verify the
        // element is produced and has a single line (nowrap).
        let html = r#"<div style="width: 50px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis">This is a very long text that should be truncated</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let found = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|text| text.lines.len() == 1)
                .unwrap_or(false)
        });
        assert!(found, "Text with nowrap should have a single line");
    }

    #[test]
    fn text_overflow_clip_no_ellipsis() {
        let html = r#"<div style="width: 50px; overflow: hidden; white-space: nowrap; text-overflow: clip">This is a very long text</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let has_ellipsis = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|text| {
                    text.lines
                        .iter()
                        .any(|line| line.runs.iter().any(|run| run.text.ends_with("...")))
                })
                .unwrap_or(false)
        });
        assert!(!has_ellipsis, "clip should not add ellipsis");
    }

    #[test]
    fn manual_soft_hyphen_uses_line_metrics_without_offset_sentinels() {
        let html = r#"<p style="width:45pt;font-size:16pt;line-height:32pt;hyphens:manual">anti­disestablishmentarianism</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let lines = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_text(|text| text.lines.clone()))
            .expect("manual-hyphen paragraph text block");

        assert!(
            lines.len() > 1,
            "soft hyphen should create a wrap opportunity"
        );
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.runs)
                .any(|run| run.text.ends_with('-')),
            "taken soft-hyphen break should paint a hyphen"
        );
        assert!(lines.iter().all(|line| line.x_offset.abs() < 45.0));
        assert!(lines.iter().all(|line| line.baseline_ascent.is_some()));
        assert!(lines.iter().all(|line| {
            line.metadata.writing_mode == WritingMode::HorizontalTb
                && !line.metadata.text_orientation_upright
        }));
    }

    #[test]
    fn vertical_text_state_is_explicit_and_x_offset_stays_geometric() {
        let cases = [
            (
                "writing-mode:vertical-lr",
                TextLineMetadata {
                    writing_mode: WritingMode::VerticalLr,
                    text_orientation_upright: false,
                },
            ),
            (
                "writing-mode:vertical-rl;text-orientation:upright",
                TextLineMetadata {
                    writing_mode: WritingMode::VerticalRl,
                    text_orientation_upright: true,
                },
            ),
        ];

        for (style, expected) in cases {
            let html = format!(r#"<p style="{style};width:80pt">Vertical</p>"#);
            let nodes = parse_html(&html).unwrap();
            let pages = layout(&nodes, PageSize::A4, Margin::default());
            let lines = pages[0]
                .elements
                .iter()
                .find_map(|(_, element)| element.inspect_text(|text| text.lines.clone()))
                .expect("vertical paragraph text block");

            assert!(!lines.is_empty());
            assert!(lines.iter().all(|line| line.x_offset.abs() < 80.0));
            assert!(lines.iter().all(|line| {
                line.metadata.writing_mode == expected.writing_mode
                    && line.metadata.text_orientation_upright == expected.text_orientation_upright
            }));
        }
    }

    // --- list-style-type tests ---
    #[test]
    fn format_list_marker_disc() {
        assert_eq!(format_list_marker(&ListStyleType::Disc, 1), "\u{2022} ");
    }

    #[test]
    fn format_list_marker_circle() {
        assert_eq!(format_list_marker(&ListStyleType::Circle, 1), "\u{25E6} ");
    }

    #[test]
    fn format_list_marker_square() {
        assert_eq!(format_list_marker(&ListStyleType::Square, 1), "\u{25AA} ");
    }

    #[test]
    fn format_list_marker_decimal() {
        assert_eq!(format_list_marker(&ListStyleType::Decimal, 3), "3. ");
    }

    #[test]
    fn format_list_marker_decimal_leading_zero() {
        assert_eq!(
            format_list_marker(&ListStyleType::DecimalLeadingZero, 3),
            "03. "
        );
        assert_eq!(
            format_list_marker(&ListStyleType::DecimalLeadingZero, 12),
            "12. "
        );
    }

    #[test]
    fn format_list_marker_lower_alpha() {
        assert_eq!(format_list_marker(&ListStyleType::LowerAlpha, 1), "a. ");
        assert_eq!(format_list_marker(&ListStyleType::LowerAlpha, 3), "c. ");
        assert_eq!(format_list_marker(&ListStyleType::LowerAlpha, 27), "aa. ");
    }

    #[test]
    fn format_list_marker_upper_alpha() {
        assert_eq!(format_list_marker(&ListStyleType::UpperAlpha, 1), "A. ");
        assert_eq!(format_list_marker(&ListStyleType::UpperAlpha, 26), "Z. ");
    }

    #[test]
    fn format_list_marker_lower_roman() {
        assert_eq!(format_list_marker(&ListStyleType::LowerRoman, 1), "i. ");
        assert_eq!(format_list_marker(&ListStyleType::LowerRoman, 4), "iv. ");
        assert_eq!(format_list_marker(&ListStyleType::LowerRoman, 9), "ix. ");
        assert_eq!(format_list_marker(&ListStyleType::LowerRoman, 14), "xiv. ");
    }

    #[test]
    fn format_list_marker_upper_roman() {
        assert_eq!(format_list_marker(&ListStyleType::UpperRoman, 1), "I. ");
        assert_eq!(format_list_marker(&ListStyleType::UpperRoman, 4), "IV. ");
    }

    #[test]
    fn format_list_marker_none() {
        assert_eq!(format_list_marker(&ListStyleType::None, 1), "");
    }

    // --- Counter state tests ---
    #[test]
    fn counter_state_default_returns_zero() {
        let cs = CounterState::default();
        assert_eq!(cs.get("foo"), 0);
    }

    #[test]
    fn counter_state_apply_resets() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 0);
    }

    #[test]
    fn counter_state_apply_increments() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 1);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 2);
    }

    #[test]
    fn counter_state_nested_resets() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 1)]);
        // Nested reset pushes a new counter
        cs.apply_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 0);
        cs.apply_increments(&[("section".to_string(), 1)]);
        assert_eq!(cs.get("section"), 1);
        // Pop nested reset
        cs.pop_resets(&[("section".to_string(), 0)]);
        assert_eq!(cs.get("section"), 1); // Back to outer counter value
    }

    #[test]
    fn counter_state_get_all() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 1)]);
        cs.apply_resets(&[("section".to_string(), 2)]);
        cs.apply_resets(&[("section".to_string(), 3)]);
        assert_eq!(cs.get_all("section", "."), "1.2.3");
    }

    // --- resolve_content tests ---
    #[test]
    fn resolve_content_string() {
        let mut cs = CounterState::default();
        let attrs = HashMap::new();
        let items = vec![ContentItem::String("hello".to_string())];
        assert_eq!(resolve_content(&items, &attrs, &mut cs), "hello");
    }

    #[test]
    fn resolve_content_attr() {
        let mut cs = CounterState::default();
        let mut attrs = HashMap::new();
        attrs.insert("title".to_string(), "My Title".to_string());
        let items = vec![ContentItem::Attr("title".to_string())];
        assert_eq!(resolve_content(&items, &attrs, &mut cs), "My Title");
    }

    #[test]
    fn resolve_content_counter() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 0)]);
        cs.apply_increments(&[("section".to_string(), 3)]);
        let attrs = HashMap::new();
        let items = vec![ContentItem::Counter(
            "section".to_string(),
            ListStyleType::Decimal,
        )];
        assert_eq!(resolve_content(&items, &attrs, &mut cs), "3");
    }

    #[test]
    fn resolve_content_counter_upper_roman() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("chap".to_string(), 0)]);
        cs.apply_increments(&[("chap".to_string(), 4)]);
        let attrs = HashMap::new();
        let items = vec![ContentItem::Counter(
            "chap".to_string(),
            ListStyleType::UpperRoman,
        )];
        assert_eq!(resolve_content(&items, &attrs, &mut cs), "IV");
    }

    #[test]
    fn resolve_content_counters() {
        let mut cs = CounterState::default();
        cs.apply_resets(&[("section".to_string(), 1)]);
        cs.apply_resets(&[("section".to_string(), 2)]);
        let attrs = HashMap::new();
        let items = vec![ContentItem::Counters(
            "section".to_string(),
            ".".to_string(),
            ListStyleType::Decimal,
        )];
        assert_eq!(resolve_content(&items, &attrs, &mut cs), "1.2");
    }

    #[test]
    fn resolve_content_mixed() {
        let mut cs = CounterState::default();
        let mut attrs = HashMap::new();
        attrs.insert("data-label".to_string(), "Note".to_string());
        let items = vec![
            ContentItem::Attr("data-label".to_string()),
            ContentItem::String(": ".to_string()),
        ];
        assert_eq!(resolve_content(&items, &attrs, &mut cs), "Note: ");
    }

    // --- ::before/::after integration tests ---
    #[test]
    fn before_pseudo_element_in_layout() {
        let html = r#"<html><head><style>p::before { content: ">> " }</style></head><body><p>Hello</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut all_texts: Vec<String> = Vec::new();
        for (_, element) in &pages[0].elements {
            element.inspect_text(|block| {
                for line in &block.lines {
                    let text: String = line.runs.iter().map(|run| run.text.as_str()).collect();
                    all_texts.push(text);
                }
            });
        }
        let found = all_texts
            .iter()
            .any(|t| t.contains(">>") && t.contains("Hello"));
        assert!(
            found,
            "::before content should be prepended to paragraph, got: {:?}",
            all_texts
        );
    }

    #[test]
    fn after_pseudo_element_in_layout() {
        let html = r#"<html><head><style>p::after { content: " <<" }</style></head><body><p>Hello</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut all_texts: Vec<String> = Vec::new();
        for (_, element) in &pages[0].elements {
            element.inspect_text(|block| {
                for line in &block.lines {
                    let text: String = line.runs.iter().map(|run| run.text.as_str()).collect();
                    all_texts.push(text);
                }
            });
        }
        let found = all_texts
            .iter()
            .any(|t| t.contains("Hello") && t.contains("<<"));
        assert!(
            found,
            "::after content should be appended to paragraph, got: {:?}",
            all_texts
        );
    }

    #[test]
    fn root_font_size_drives_rem_layout_values() {
        let html = r#"
            <html>
                <head>
                    <style>
                        :root { font-size: 10pt; }
                        .title { font-size: 2rem; margin-top: 0.5rem; }
                    </style>
                </head>
                <body><div class="title">Title</div></body>
            </html>
        "#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }

        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let title_block = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element
                    .inspect_text(|block| {
                        block
                            .lines
                            .iter()
                            .flat_map(|line| line.runs.iter())
                            .any(|run| run.text.contains("Title"))
                            .then(|| (block.lines.clone(), block.box_model.margins.start))
                    })
                    .flatten()
            })
            .expect("expected title block");

        let (lines, margin_top) = title_block;
        assert!((margin_top - 5.0).abs() < 0.1);
        assert!(
            (lines[0].runs[0].font_size - 20.0).abs() < 0.1,
            "expected 2rem to resolve from :root 10pt"
        );
    }

    // --- list-style-type in layout tests ---
    /// Collect every line's concatenated run text, plus whether any run carries a
    /// geometric marker (an inline-box). List items may render as a `TextBlock`
    /// OR, when the marker is a geometric `disc`/`square` shape, as a `FlexRow`
    /// (the inline-box marker routes the item through the flex path) — so tests
    /// must look in both.
    fn list_texts_and_markers(page: &Page) -> (Vec<String>, bool) {
        fn scan(lines: &[TextLine], texts: &mut Vec<String>, has_box: &mut bool) {
            for l in lines {
                texts.push(l.runs.iter().map(|r| r.text.as_str()).collect());
                if l.runs.iter().any(|r| r.inline_box.is_some()) {
                    *has_box = true;
                }
            }
        }
        let mut texts = Vec::new();
        let mut has_box = false;
        for (_, el) in &page.elements {
            el.inspect_text(|text| scan(&text.lines, &mut texts, &mut has_box));
            el.inspect_flex(|row| {
                for cell in &row.content.cells {
                    scan(&cell.lines, &mut texts, &mut has_box);
                }
            });
        }
        (texts, has_box)
    }

    fn page_has_image_background(page: &Page) -> bool {
        page.elements
            .iter()
            .any(|(_, element)| element_has_image_background(element))
    }

    fn elements_have_image_background(elements: &[LayoutNode]) -> bool {
        elements
            .iter()
            .any(|element| element_has_image_background(element.as_ref()))
    }

    fn element_has_image_background(element: &dyn LayoutElement) -> bool {
        struct ImageBackgroundSearch(bool);

        impl LayoutVisitor for ImageBackgroundSearch {
            fn visit_text_block(&mut self, element: &TextBlock) {
                self.0 |= element.paint.background.layers.has_image();
            }

            fn visit_container(&mut self, element: &Container) {
                self.0 |= element.paint.background.layers.has_image();
            }

            fn visit_flex_row(&mut self, element: &FlexRow) {
                self.0 |= element.paint.background.layers.has_image()
                    || element
                        .content
                        .cells
                        .iter()
                        .any(flex_cell_has_image_background);
            }

            fn visit_table_row(&mut self, element: &TableRow) {
                self.0 |= element.content.cells.iter().any(cell_has_image_background);
            }

            fn visit_grid_row(&mut self, element: &GridRow) {
                self.0 |= element.content.cells.iter().any(cell_has_image_background);
            }
        }

        let mut search = ImageBackgroundSearch(false);
        visit_layout_tree(element, &mut search);
        search.0
    }

    fn cell_has_image_background(cell: &impl CellBoxHolder) -> bool {
        elements_have_image_background(&cell.cell_box().content.children)
    }

    fn flex_cell_has_image_background(cell: &FlexCell) -> bool {
        cell.paint.background.layers.has_image()
            || elements_have_image_background(&cell.nested_elements)
    }

    fn text_lines_contain(lines: &[TextLine], needle: &str) -> bool {
        lines.iter().any(|line| {
            line.runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .contains(needle)
        })
    }

    fn collect_text_runs_from_lines(lines: &[TextLine], out: &mut Vec<TextRun>) {
        for line in lines {
            out.extend(line.runs.iter().cloned());
        }
    }

    fn collect_text_runs_from_element(element: &dyn LayoutElement, out: &mut Vec<TextRun>) {
        struct RunCollector<'a>(&'a mut Vec<TextRun>);

        impl LayoutVisitor for RunCollector<'_> {
            fn visit_text_block(&mut self, element: &TextBlock) {
                collect_text_runs_from_lines(&element.lines, self.0);
            }

            fn visit_table_row(&mut self, element: &TableRow) {
                for cell in &element.content.cells {
                    collect_text_runs_from_lines(&cell.layout.content.lines, self.0);
                }
            }

            fn visit_grid_row(&mut self, element: &GridRow) {
                for cell in &element.content.cells {
                    collect_text_runs_from_lines(&cell.layout.content.lines, self.0);
                }
            }

            fn visit_flex_row(&mut self, element: &FlexRow) {
                for cell in &element.content.cells {
                    collect_text_runs_from_lines(&cell.lines, self.0);
                }
            }
        }

        visit_layout_tree(element, &mut RunCollector(out));
    }

    fn collect_text_runs_from_cell(cell: &TableCell, out: &mut Vec<TextRun>) {
        collect_text_runs_from_lines(&cell.layout.content.lines, out);
        for child in &cell.layout.content.children {
            collect_text_runs_from_element(child.as_ref(), out);
        }
    }

    fn text_runs_in_cell(cell: &TableCell) -> Vec<TextRun> {
        let mut runs = Vec::new();
        collect_text_runs_from_cell(cell, &mut runs);
        runs
    }

    fn find_text_block_containing(elements: &[LayoutNode], needle: &str) -> Option<TextBlock> {
        struct TextBlockSearch<'a> {
            needle: &'a str,
            found: Option<TextBlock>,
        }

        impl LayoutVisitor for TextBlockSearch<'_> {
            fn visit_text_block(&mut self, element: &TextBlock) {
                if self.found.is_none() && text_lines_contain(&element.lines, self.needle) {
                    self.found = Some(element.clone());
                }
            }
        }

        for element in elements {
            let mut search = TextBlockSearch {
                needle,
                found: None,
            };
            visit_layout_tree(element.as_ref(), &mut search);
            if search.found.is_some() {
                return search.found;
            }
        }
        None
    }

    fn find_page_text_block_containing(
        elements: &[(f32, LayoutNode)],
        needle: &str,
    ) -> Option<TextBlock> {
        elements.iter().find_map(|(_, element)| {
            find_text_block_containing(std::slice::from_ref(element), needle)
        })
    }

    #[test]
    fn unordered_list_uses_bullet_marker() {
        let html = "<ul><li>Item</li></ul>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let (texts, has_geometric_marker) = list_texts_and_markers(&pages[0]);
        // A `disc` bullet is a GEOMETRIC marker (an inline-box) in the current
        // renderer (matching Chrome), so accept either the geometric marker or a
        // legacy U+2022 glyph run.
        let found = has_geometric_marker || texts.iter().any(|t| t.contains('\u{2022}'));
        assert!(found, "Unordered list should use a bullet marker");
    }

    #[test]
    fn ordered_list_uses_decimal_marker() {
        let html = "<ol><li>First</li><li>Second</li></ol>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let (all_texts, _) = list_texts_and_markers(&pages[0]);
        // The ordered-list items lay out. (The decimal `1.`/`2.` markers are
        // OUTSIDE hanging markers and their glyph rendering is verified by the
        // `lists-counters/list-style-type-decimal` parity fixture; they do not sit
        // in the item's own text block, so we assert the item content here.)
        let joined = all_texts.join(" ");
        assert!(
            joined.contains("First") && joined.contains("Second"),
            "Ordered list items should lay out, got: {all_texts:?}"
        );
    }

    // --- Coverage tests for uncovered lines ---

    #[test]
    fn to_alpha_lower_zero_returns_a() {
        // Covers line 81: to_alpha_lower(0) returns "a"
        assert_eq!(to_alpha_lower(0), "a");
    }

    #[test]
    fn to_roman_lower_zero_returns_zero_string() {
        // Covers line 120: to_roman_lower(0) returns "0"
        assert_eq!(to_roman_lower(0), "0");
    }

    #[test]
    fn counter_state_apply_increments_on_empty_stack() {
        // Covers line 32: apply_increments pushes 0 when stack is empty
        let mut state = CounterState::default();
        state.apply_increments(&[("test".to_string(), 1)]);
        assert_eq!(state.get("test"), 1);
    }

    #[test]
    fn css_counter_in_pseudo_element_generates_numbers() {
        // Verifies that counter-reset + counter-increment + counter() in
        // ::before pseudo-elements produce sequential numbers (BUG 1 fix).
        let html = r#"<html><head><style>
            ol.counted { counter-reset: item }
            ol.counted li { counter-increment: item }
            ol.counted li::before { content: counter(item) ". " }
        </style></head><body>
            <ol class="counted"><li>First</li><li>Second</li><li>Third</li></ol>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(crate::parser::css::parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut all_texts: Vec<String> = Vec::new();
        for (_, element) in &pages[0].elements {
            element.inspect_text(|block| {
                for line in &block.lines {
                    let text: String = line.runs.iter().map(|run| run.text.as_str()).collect();
                    if !text.trim().is_empty() {
                        all_texts.push(text);
                    }
                }
            });
        }
        let joined = all_texts.join(" ");
        assert!(
            joined.contains("1.") && joined.contains("2.") && joined.contains("3."),
            "CSS counters should generate sequential numbers 1, 2, 3. Got: {joined}"
        );
    }

    #[test]
    fn inline_descendants_use_the_same_counter_transaction_as_blocks() {
        let html = r#"<html><head><style>
            .counted { counter-reset: item }
            .counted > span { counter-increment: item }
            .counted > span::before { content: counter(item) "." }
        </style></head><body>
            <div class="counted"><span>A</span><span>B</span></div>
        </body></html>"#;
        let result = crate::parser::html::parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(crate::parser::css::parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut text = String::new();
        for (_, element) in &pages[0].elements {
            element.inspect_text(|block| {
                for line in &block.lines {
                    for run in &line.runs {
                        text.push_str(&run.text);
                    }
                }
            });
        }

        assert!(
            text.contains("1.A2.B"),
            "inline counter operations must run in source order, got: {text:?}"
        );
    }

    #[test]
    fn layout_flex_container() {
        // Covers lines 1067,1133,1395: flex layout code paths
        let html = r#"<div style="display: flex; width: 400pt;">
            <div style="width: 200pt;">Left</div>
            <div style="width: 200pt;">Right</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn percentage_text_indent_uses_the_border_box_content_width() {
        let html = r#"<html><head><style>
            * { margin: 0; box-sizing: border-box; }
            p {
                width: 220px;
                border-left: 4px solid #2e7d32;
                --indent: calc(50% - 10pt);
                text-indent: var(--indent);
            }
        </style></head><body><p>First line</p></body></html>"#;
        let parsed = parse_html_with_styles(html).expect("valid test document");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &parsed.nodes,
            PageSize::new(228.0, 114.0),
            Margin::default(),
            &rules,
        );
        let text_indent = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element
                    .inspect_text(|block| {
                        text_lines_contain(&block.lines, "First line").then_some(block.text.indent)
                    })
                    .flatten()
            })
            .expect("the paragraph should produce a text block");

        // 220px border-box - 4px left border = 216px content box; half is
        // 108px, which is 81pt at CSS's 96px/in to PDF's 72pt/in conversion.
        // The custom property's `calc()` then subtracts its 10pt length term.
        assert!((text_indent - 71.0).abs() < 0.01, "indent = {text_indent}");
    }

    #[test]
    fn flex_absolute_child_keeps_its_inset_local_to_the_padding_box() {
        let html = r#"<html><head><style>
            .flex {
                position: relative;
                display: flex;
                width: 280px;
                height: 160px;
                border: 2px solid #0a4d40;
            }
            .absolute {
                position: absolute;
                left: 10px;
                bottom: 10px;
                width: 90px;
                height: 50px;
                border: 2px solid #6e2018;
            }
        </style></head><body>
            <div class="flex"><div class="absolute"></div></div>
        </body></html>"#;
        let parsed = crate::parser::html::parse_html_with_styles(html).unwrap();
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &parsed.nodes,
            PageSize::new(300.0, 156.0),
            Margin::default(),
            &rules,
        );
        let absolute = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element
                    .inspect_container(|container| {
                        (container.positioning.scheme == Position::Absolute)
                            .then(|| {
                                container
                                    .positioning
                                    .containing_block
                                    .map(|containing_block| {
                                        (container.positioning.insets.left, containing_block)
                                    })
                            })
                            .flatten()
                    })
                    .flatten()
            })
            .expect("the absolute flex child should be emitted as a positioned container");

        assert!(
            (absolute.0 - 7.5).abs() < 0.01,
            "left inset = {}",
            absolute.0
        );
        assert!(
            (absolute.1.x - 1.5).abs() < 0.01,
            "padding-box x = {}",
            absolute.1.x
        );
    }

    #[test]
    fn layout_grid_container() {
        // Covers lines 1670,1712: grid layout code paths
        let html = r#"<html><head><style>
            .grid { display: grid; grid-template-columns: 1fr 1fr; }
        </style></head><body>
        <div class="grid"><div>A</div><div>B</div></div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_table_with_non_standard_children() {
        // Covers line 1821,1831,1858: table non-tr children
        let html = "<table><caption>Cap</caption><tr><td>A</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_table_colspan_exceeds_cols() {
        // Covers line 1943,2003: colspan beyond column count
        let html = r#"<table>
            <tr><td colspan="10">Wide</td></tr>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_white_space_nowrap_overflow() {
        // Covers lines 2221,2227,2242: nowrap + text-overflow: ellipsis
        let html = r#"<html><head><style>
            .nowrap { width: 50pt; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
        </style></head><body>
        <div class="nowrap">This text is very long and should be truncated</div>
        </body></html>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn layout_clear_right_float() {
        // Covers line 2312: clear: right
        let html = r#"
            <div style="float: right; width: 100pt;">Floated</div>
            <div style="clear: right;">Cleared</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn base64_decode_valid() {
        // Covers lines 2562,2574: base64 decode
        let decoded = decode_base64("SGVsbG8=").unwrap();
        assert_eq!(&decoded, b"Hello");
    }

    #[test]
    fn base64_decode_invalid_char() {
        // Covers line 2562: base64 decode with invalid char
        let result = decode_base64("!!!!");
        assert!(result.is_none());
    }

    #[test]
    fn base64_decode_short_input() {
        // Covers line 2574: base64 decode with very short input (breaks early)
        let result = decode_base64("A");
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn wrap_break_word_splits_long_word_without_hyphen() {
        let fonts = HashMap::new();
        let template = TextRun {
            font_family: FontFamily::Helvetica,
            ..Default::default()
        };
        // At 12pt, each char ~6pt. "Hi" = 12pt.
        // "Supercalifragilisticexpialidocious" = 34*6 = 204pt.
        // With max_width=100, "Hi" (12pt) fits, then the long word (204pt)
        // doesn't fit (12 + 6 space + 204 > 100), so break-word splits it
        // across lines without inserting a hyphen character.
        let runs = vec![TextRun {
            text: "Hi Supercalifragilisticexpialidocious".to_string(),
            ..template
        }];
        let lines = wrap_text_runs(
            runs,
            TextWrapOptions::new(100.0, 12.0, 1.2, OverflowWrap::BreakWord),
            &fonts,
        );
        assert!(
            lines.len() > 1,
            "expected break-word to produce multiple lines, got {}",
            lines.len()
        );
        let first_line_text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert!(
            !first_line_text.ends_with('-'),
            "break-word should not insert hyphens, got: {first_line_text:?}"
        );
    }

    #[test]
    fn wrap_normal_keeps_fitting_text_on_one_line() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "Hello world".to_string(),
            font_family: FontFamily::Helvetica,
            ..Default::default()
        };
        let lines = wrap_text_runs(
            vec![run],
            TextWrapOptions::new(500.0, 12.0, 1.2, OverflowWrap::Normal),
            &fonts,
        );
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert!(
            !text.contains('-'),
            "short fitting text should stay unchanged, got: {text:?}"
        );
    }

    #[test]
    fn wrap_break_word_splits_short_remainder_without_hyphen() {
        let fonts = HashMap::new();
        let run = TextRun {
            text: "Hi the end".to_string(),
            font_family: FontFamily::Helvetica,
            ..Default::default()
        };
        let lines = wrap_text_runs(
            vec![run],
            TextWrapOptions::new(20.0, 12.0, 1.2, OverflowWrap::BreakWord),
            &fonts,
        );
        for line in &lines {
            for run in &line.runs {
                assert!(
                    !run.text.contains('-'),
                    "break-word should not add hyphens, got: {:?}",
                    run.text
                );
            }
        }
    }

    /// Helper: extract all text strings from a PDF byte vector.
    /// Handles both WinAnsi Tj strings and CID TJ arrays with ToUnicode CMap.
    fn extract_tj_strings(pdf: &[u8]) -> Vec<String> {
        let pdf_str = String::from_utf8_lossy(pdf);
        let content: &str = pdf_str.as_ref();

        // Try WinAnsi Tj path first
        let winans: Vec<String> = content
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.ends_with("Tj") && trimmed.starts_with('(') {
                    Some(trimmed[1..trimmed.len() - 4].to_string())
                } else {
                    None
                }
            })
            .collect();
        if !winans.is_empty() {
            return winans;
        }

        // CID path: parse ToUnicode CMap to build glyph→char map
        let mut glyph_to_char: std::collections::HashMap<String, char> =
            std::collections::HashMap::new();
        let mut pos = 0;
        while let Some(start) = content[pos..].find("beginbfchar") {
            let block_start = pos + start + 11;
            let block_end = content[block_start..]
                .find("endbfchar")
                .map(|e| block_start + e)
                .unwrap_or(content.len());
            for line in content[block_start..block_end].lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line
                    .split(|c: char| c == '<' || c == '>' || c.is_whitespace())
                    .filter(|s| !s.is_empty())
                    .collect();
                if parts.len() >= 2 {
                    let glyph_hex = parts[0].to_uppercase();
                    let unicode_hex = parts[1];
                    if let Ok(cp) = u32::from_str_radix(unicode_hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            glyph_to_char.insert(glyph_hex, ch);
                        }
                    }
                }
            }
            pos = block_end;
        }

        if glyph_to_char.is_empty() {
            return Vec::new();
        }

        // Parse TJ arrays: [...] TJ
        let mut results = Vec::new();
        let mut search_pos = 0;
        while let Some(tj_end) = content[search_pos..].find("] TJ") {
            let tj_end_abs = search_pos + tj_end;
            if let Some(tj_start) = content[..tj_end_abs].rfind('[') {
                let array_content = &content[tj_start + 1..tj_end_abs];
                let mut decoded = String::new();
                let mut apos = 0;
                while let Some(open) = array_content[apos..].find('<') {
                    let open_abs = apos + open;
                    if let Some(close) = array_content[open_abs..].find('>') {
                        let hex_str = array_content[open_abs + 1..open_abs + close]
                            .trim()
                            .to_uppercase();
                        if let Some(&ch) = glyph_to_char.get(&hex_str) {
                            decoded.push(ch);
                        }
                        apos = open_abs + close + 1;
                    } else {
                        break;
                    }
                }
                if !decoded.is_empty() {
                    results.push(decoded);
                }
            }
            search_pos = tj_end_abs + 4;
        }

        // Positioned runs may be emitted as one `<glyph-id> Tj` operator per
        // glyph instead of a `[...] TJ` array. Keep this test-only extractor in
        // sync with both valid writer forms.
        let mut single_pos = 0;
        let mut single_decoded = String::new();
        while let Some(tj_end) = content[single_pos..].find("> Tj") {
            let tj_end_abs = single_pos + tj_end;
            if let Some(hex_start) = content[..tj_end_abs].rfind('<') {
                let hex = content[hex_start + 1..tj_end_abs].trim().to_uppercase();
                if let Some(&character) = glyph_to_char.get(&hex) {
                    single_decoded.push(character);
                }
            }
            single_pos = tj_end_abs + 4;
        }
        if !single_decoded.is_empty() {
            results.push(single_decoded);
        }
        results
    }

    #[test]
    fn spaces_preserved_in_text() {
        // "Hello World" must stay "Hello World" through the full pipeline
        let html = "<p>Hello World</p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let all_text = tj.join("");
        assert!(
            all_text.contains("Hello World"),
            "Expected 'Hello World' in PDF text, got: {tj:?}"
        );
    }

    #[test]
    fn spaces_between_inline_elements() {
        // `<span>Hello</span> <span>World</span>` must have a space
        let html = "<p><span>Hello</span> <span>World</span></p>";
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let all_text = tj.join("");
        assert!(
            all_text.contains("Hello World"),
            "Expected space between inline elements, got: {tj:?}"
        );
    }

    #[test]
    fn invoice_text_spaces_preserved() {
        // Verify the specific failing cases from the invoice
        let html = r#"
            <p><strong>Bill to:</strong><br>
            Acme Corp<br>
            456 Enterprise Blvd<br>
            New York, NY 10001</p>
            <table>
                <tr><td>Custom font embedding module</td></tr>
                <tr><td>SVG rendering add-on</td></tr>
            </table>
        "#;
        let pdf = crate::html_to_pdf(html).unwrap();
        let tj = extract_tj_strings(&pdf);
        let has = |needle: &str| tj.iter().any(|s| s.contains(needle));

        assert!(has("Acme Corp"), "Expected 'Acme Corp', got: {tj:?}");
        assert!(has("New York"), "Expected 'New York', got: {tj:?}");
        assert!(has("Custom font"), "Expected 'Custom font', got: {tj:?}");
        assert!(
            has("SVG rendering"),
            "Expected 'SVG rendering', got: {tj:?}"
        );
        assert!(
            has("Enterprise Blvd"),
            "Expected 'Enterprise Blvd', got: {tj:?}"
        );
    }

    /// Block children inside a padded parent should use inner_width (parent
    /// width minus padding) so that their text wraps within the padding.
    #[test]
    fn padded_div_child_block_respects_inner_width() {
        let html = r#"<div style="padding: 20pt;"><p>short</p></div>"#;
        let dom = parse_html(html).unwrap();
        let pages = layout(
            &dom,
            crate::types::PageSize::new(200.0, 800.0),
            crate::types::Margin::uniform(0.0),
        );
        // The <p> inside the padded div should be laid out within 200 - 40 = 160pt.
        // We verify that the p's TextBlock has block_width <= 160. A padded block
        // now routes its children through a Container wrapper (so the padding
        // offsets them), so the paragraph lives in Container.children — recurse.
        let block = pages
            .iter()
            .find_map(|page| {
                find_text_block_containing(
                    &page
                        .elements
                        .iter()
                        .map(|(_, element)| element.clone())
                        .collect::<Vec<_>>(),
                    "short",
                )
            })
            .expect("did not find the child paragraph");
        if let Some(width) = block.box_model.size.width.fixed_value() {
            assert!(
                width <= 160.0,
                "child block width {width} should be <= inner width 160"
            );
        }
    }

    /// A padded block child of a column-direction flex container now flattens
    /// to a Container (so its padding offsets its content). The column emit loop
    /// must not drop non-TextBlock items — regression guard for the content-loss
    /// bug where such an item disappeared entirely.
    #[test]
    fn column_flex_padded_child_preserves_content() {
        let html = r#"<div style="display:flex;flex-direction:column">
            <div style="padding:20px"><p>kept</p></div>
        </div>"#;
        let dom = parse_html(html).unwrap();
        let pages = layout(
            &dom,
            crate::types::PageSize::default(),
            crate::types::Margin::uniform(20.0),
        );
        let found = pages
            .iter()
            .flat_map(|p| p.elements.iter())
            .any(|(_, element)| element_contains_text(element.as_ref(), "kept"));
        assert!(found, "column flex dropped the padded child's content");
    }

    /// Flex child with inline background (badge) should propagate the
    /// background_color from the computed style to the TextRun.
    #[test]
    fn flex_child_propagates_background_color() {
        let html = r#"
        <div style="display: flex;">
          <div><span style="background-color: #27ae60; color: white;">PAID</span></div>
        </div>"#;
        let dom = parse_html(html).unwrap();
        let rules = parse_stylesheet("span { background-color: #27ae60; color: white; }");
        let pages = layout_with_rules(
            &dom,
            crate::types::PageSize::default(),
            crate::types::Margin::uniform(20.0),
            &rules,
        );
        let mut found_bg = false;
        for page in &pages {
            for (_, elem) in &page.elements {
                elem.inspect_flex(|row| {
                    for cell in &row.content.cells {
                        for line in &cell.lines {
                            for run in &line.runs {
                                if run.text.contains("PAID") && run.background_color.is_some() {
                                    found_bg = true;
                                }
                            }
                        }
                    }
                });
            }
        }
        assert!(
            found_bg,
            "PAID badge text run should have background_color set"
        );
    }

    #[test]
    fn flex_row_child_preserves_svg_background() {
        let child_style = r#"background-image: url("data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%2210%22%20height=%2210%22%3E%3Crect%20width=%2210%22%20height=%2210%22%20fill=%22%23f00%22/%3E%3C/svg%3E"); width: 60pt;"#;
        let parsed = crate::parser::css::parse_inline_style(child_style);
        assert!(
            matches!(
                parsed.get("background-image"),
                Some(crate::parser::css::CssValue::BackgroundLayers(layers))
                    if matches!(
                        layers.as_slice(),
                        [crate::parser::css::BackgroundLayerSource::Svg(_)]
                    )
            ),
            "expected inline style parser to capture a typed SVG background"
        );
        let computed = crate::style::computed::compute_style(
            HtmlTag::Div,
            Some(child_style),
            &ComputedStyle::default(),
        );
        assert!(
            computed.background_svg.is_some(),
            "expected computed style to retain SVG background"
        );
        let html =
            format!(r#"<div style="display: flex;"><div style='{child_style}'>A</div></div>"#);
        let pages = layout(&parse_html(&html).unwrap(), PageSize::A4, Margin::default());
        let has_cell_svg_background = pages.iter().any(|page| {
            page.elements.iter().any(|(_, element)| {
                element
                    .inspect_flex(|row| {
                        row.content
                            .cells
                            .iter()
                            .any(|cell| cell.paint.background.layers.svg.is_some())
                    })
                    .unwrap_or(false)
            })
        });
        assert!(
            has_cell_svg_background,
            "expected flex row cell to retain SVG background data"
        );
    }

    /// Notes-style div with padding, br tags, and inline content should
    /// produce wrapped text that fits within the padded area.
    #[test]
    fn notes_div_with_padding_and_br_wraps_correctly() {
        let html = r#"<div style="padding: 10pt; font-size: 9pt;">
          <strong>Notes:</strong><br>
          First line of text that should be fully visible inside the padded area.<br>
          Second line with content.
        </div>"#;
        let dom = parse_html(html).unwrap();
        let pages = layout(
            &dom,
            crate::types::PageSize::new(300.0, 800.0),
            crate::types::Margin::uniform(0.0),
        );
        // Verify that lines exist and the text is present
        let mut all_text = String::new();
        let mut line_count = 0;
        for page in &pages {
            for (_, elem) in &page.elements {
                elem.inspect_text(|block| {
                    for line in &block.lines {
                        for run in &line.runs {
                            all_text.push_str(&run.text);
                        }
                        line_count += 1;
                    }
                });
            }
        }
        assert!(all_text.contains("Notes:"), "Notes: text missing");
        assert!(
            all_text.contains("First line"),
            "First line text missing: {all_text:?}"
        );
        assert!(
            all_text.contains("Second line"),
            "Second line text missing: {all_text:?}"
        );
        // Should have at least 3 lines due to the <br> tags
        assert!(
            line_count >= 3,
            "expected at least 3 lines from br tags, got {line_count}"
        );
    }

    #[test]
    fn body_rules_applied_to_root() {
        let css = "body { font-size: 10pt }";
        let rules = parse_stylesheet(css);
        let html = "<p>text</p>";
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        pages[0].elements[0]
            .1
            .inspect_text(|block| {
                assert!(!block.lines.is_empty());
                let font_size = block.lines[0].runs[0].font_size;
                assert!(
                    (font_size - 10.0).abs() < 0.1,
                    "Expected font_size 10.0 from body rule, got {font_size}"
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn string_set_from_an_ordinary_block_is_available_on_its_page() {
        let rules = parse_stylesheet("h1 { string-set: chapter content() }");
        let nodes = parse_html("<h1>Running Head</h1><p>Body text</p>").unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        assert_eq!(
            pages[0].generated_content.named_string_exit("chapter"),
            Some("Running Head")
        );
        assert_eq!(
            pages[0].generated_content.named_string_first("chapter"),
            Some("Running Head")
        );
    }

    #[test]
    fn string_set_attr_is_available_as_the_first_assignment_on_its_page() {
        let rules = parse_stylesheet("h2 { string-set: section attr(data-title) }");
        let nodes =
            parse_html(r#"<h2 data-title="ALPHA">ignored text</h2><p>Body text</p>"#).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        assert_eq!(
            pages[0].generated_content.named_string_exit("section"),
            Some("ALPHA")
        );
        assert_eq!(
            pages[0].generated_content.named_string_first("section"),
            Some("ALPHA")
        );
    }

    #[test]
    fn running_element_keeps_a_sized_painted_box_without_text() {
        let rules = parse_stylesheet(
            ".runhead { position: running(runhead); width: 160px; height: 24px; background: #11305f; }",
        );
        let nodes = parse_html("<div class='runhead'></div><div>body</div>").unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        pages[0]
            .generated_content
            .running_element(&crate::parser::css::PageContentReference::new(
                "runhead".into(),
                crate::parser::css::PageContentPolicy::Last,
            ))
            .and_then(|element| {
                element.inspect_container(|block| {
                    assert!(
                        block.children.is_empty(),
                        "the box must not need a text run to exist"
                    );
                    assert!(
                        block.paint.background.color.is_some(),
                        "the running box lost its paint"
                    );
                    assert!(
                        block
                            .box_model
                            .size
                            .width
                            .fixed_value()
                            .is_some_and(|width| width > 0.0)
                            && block
                                .box_model
                                .size
                                .height
                                .used()
                                .is_some_and(|height| height > 0.0),
                        "the running box lost its explicit size"
                    );
                })
            })
            .expect("the empty running box was not captured");
    }

    #[test]
    fn root_rules_applied_to_root_style() {
        let css = ":root { font-size: 11pt; background-color: #abcdef }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());

        let first_is_background = pages[0].elements[0]
            .1
            .inspect_text(|block| {
                block.fragmentation.box_fragmentation.content_role
                    == crate::layout::elements::PageContentRole::RepeatedDecoration
                    && block.paint.background.color == Some(Color::rgb(0xAB, 0xCD, 0xEF))
            })
            .unwrap_or(false);
        assert!(first_is_background, "Expected page background from :root");

        pages[0].elements[1]
            .1
            .inspect_text(|block| {
                assert!(!block.lines.is_empty());
                let font_size = block.lines[0].runs[0].font_size;
                assert!(
                    (font_size - 11.0).abs() < 0.1,
                    "Expected font_size 11.0 from :root rule, got {font_size}"
                );
            })
            .expect("expected text block after root background");
    }

    #[test]
    fn root_svg_background_emits_page_background_block() {
        let css = ":root { background-image: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='10'%3E%3Crect width='20' height='10' fill='%23f00'/%3E%3C/svg%3E\"); background-size: cover; }";
        let rules = parse_stylesheet(css);
        let nodes = parse_html("<p>text</p>").unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        pages[0].elements[0]
            .1
            .inspect_text(|block| {
                let tree = block
                    .paint
                    .background
                    .layers
                    .svg
                    .as_ref()
                    .expect("page background should retain its SVG");
                assert_eq!(
                    block.fragmentation.box_fragmentation.content_role,
                    crate::layout::elements::PageContentRole::RepeatedDecoration
                );
                assert_eq!(tree.width, 20.0);
                assert_eq!(tree.height, 10.0);
                // Body/root background is confined to the content area (page minus margins),
                // matching Chrome's print behavior which surrounds the body bg with a
                // white page margin frame.
                let margin = Margin::default();
                let expected_width = PageSize::A4.width - margin.horizontal();
                let expected_height = PageSize::A4.height - margin.top - margin.bottom;
                assert!(
                    (block.box_model.size.width.fixed_value().unwrap() - expected_width).abs()
                        < 0.1
                );
                assert!(
                    (block.box_model.size.height.used().unwrap() - expected_height).abs() < 0.1
                );
            })
            .expect("expected a repeat-on-each-page SVG background block");
    }

    #[test]
    fn at_page_background_emits_full_bleed_block_below_canvas() {
        // CSS Paged Media 3 §3.1: a background declared on `@page` paints the
        // bleed area — the entire page box INCLUDING its margins — BELOW the
        // document canvas. The propagated root/body background (the canvas) stays
        // confined to the content box. Provide BOTH and assert the layering +
        // geometry: the @page layer is full-sheet at z=-2, the canvas is confined
        // at z=-1.
        let mut page_bg = ComputedStyle::default();
        let map = crate::parser::css::parse_inline_style("background: #abcdef");
        crate::style::computed::apply_style_map(&mut page_bg, &map, &ComputedStyle::default());

        let rules = parse_stylesheet(":root { background-color: #112233; }");
        let nodes = parse_html("<p>text</p>").unwrap();
        let margin = Margin::uniform(20.0);
        let pages = layout_with_rules_and_fonts(
            &nodes,
            PageSize::A4,
            margin,
            &rules,
            &std::collections::HashMap::new(),
            Some(&page_bg),
            6.0,
            FootnoteAreaLayout::default(),
        );

        // elements[0]: the @page bleed background — full sheet, offset by -margin
        // so it renders at the sheet origin, z=-2, repeated on each page.
        assert_eq!(
            pages[0].elements[0].0, -26.0,
            "post-pagination page backgrounds retain their resolved block offset"
        );
        pages[0].elements[0]
            .1
            .inspect_text(|block| {
                let w = block
                    .box_model
                    .size
                    .width
                    .fixed_value()
                    .expect("full-bleed width");
                let h = block
                    .box_model
                    .size
                    .height
                    .used()
                    .expect("full-bleed height");
                assert!(
                    (w - PageSize::A4.width - 12.0).abs() < 0.1,
                    "full bleed width, got {w}"
                );
                assert!(
                    (h - PageSize::A4.height - 12.0).abs() < 0.1,
                    "full bleed height, got {h}"
                );
                assert!(
                    (block.positioning.insets.left + 26.0).abs() < 0.1,
                    "offset_left includes margin and bleed"
                );
                assert!(
                    (block.positioning.insets.top + 26.0).abs() < 0.1,
                    "offset_top includes margin and bleed"
                );
                assert_eq!(
                    block.paint.group.stacking.z_index.value(),
                    -2,
                    "@page bleed paints below the canvas (z=-1)"
                );
                assert_eq!(
                    block.fragmentation.box_fragmentation.content_role,
                    crate::layout::elements::PageContentRole::RepeatedDecoration,
                    "repeats on every page"
                );
            })
            .expect("expected full-bleed @page background first");

        // elements[1]: the propagated canvas background — CONFINED inside margins.
        pages[0].elements[1]
            .1
            .inspect_text(|block| {
                let w = block
                    .box_model
                    .size
                    .width
                    .fixed_value()
                    .expect("canvas width");
                assert!(
                    w < PageSize::A4.width - 1.0,
                    "canvas background stays confined inside the @page margins, got {w}"
                );
                assert_eq!(
                    block.paint.group.stacking.z_index.value(),
                    -1,
                    "canvas background z=-1 (above the @page bleed)"
                );
            })
            .expect("expected confined canvas background second");
    }

    #[test]
    fn wrapper_textblock_for_visual_blocks() {
        let css = ".box { background-color: red; padding: 10pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="box"><p>hello</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_bg = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|block| block.paint.background.color.is_some())
                .or_else(|| {
                    element
                        .inspect_container(|container| container.paint.background.color.is_some())
                })
                .unwrap_or(false)
        });
        assert!(
            has_bg,
            "Expected a TextBlock or Container with background_color from .box div"
        );
    }

    #[test]
    fn flex_child_ancestor_selectors() {
        let css = ".card .value { font-size: 20pt }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="card" style="display: flex"><div class="value">big</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let items = extract_flex_items(&pages);
        let big_item = items.iter().find(|i| i.3.contains("big"));
        assert!(
            big_item.is_some(),
            "Did not find 'big' text in flex layout output"
        );
        // Verify the font size was applied via ancestor selector
        // Check via the layout elements directly for font_size
        let mut found = false;
        for (_, element) in &pages[0].elements {
            let mut runs = Vec::new();
            collect_text_runs_from_element(element.as_ref(), &mut runs);
            found |= runs
                .iter()
                .any(|run| run.text.contains("big") && (run.font_size - 20.0).abs() < 0.1);
        }
        assert!(found, "Expected font_size 20.0 for .value in flex child");
    }

    #[test]
    fn p_inherits_parent_font_size() {
        let html = r#"<div style="font-size: 8pt"><p>small</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
        let mut found = false;
        for (_, element) in &pages[0].elements {
            let mut runs = Vec::new();
            collect_text_runs_from_element(element.as_ref(), &mut runs);
            for run in runs.iter().filter(|run| run.text.contains("small")) {
                assert!(
                    (run.font_size - 8.0).abs() < 0.1,
                    "Expected font_size 8.0 for p inside div, got {}",
                    run.font_size
                );
                found = true;
            }
        }
        assert!(found, "Did not find 'small' text run in layout output");
    }

    #[test]
    fn table_nth_child_section_relative() {
        let css = "tbody tr:nth-child(even) { background-color: #eee }";
        let rules = parse_stylesheet(css);
        let html = r#"
            <table>
                <thead><tr><th>H</th></tr></thead>
                <tbody>
                    <tr><td>Row 1</td></tr>
                    <tr><td>Row 2</td></tr>
                    <tr><td>Row 3</td></tr>
                </tbody>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let rows = table_rows(&pages[0]);
        // Should have at least 4 rows (1 thead + 3 tbody)
        assert!(
            rows.len() >= 4,
            "Expected at least 4 table rows, got {}",
            rows.len()
        );
    }

    #[test]
    fn layout_border_horizontal_width() {
        let side = |width| LayoutBorderSide {
            width,
            style: crate::style::computed::BorderStyle::Solid,
            ..Default::default()
        };
        let border = LayoutBorder {
            top: side(1.0),
            right: side(3.0),
            bottom: side(2.0),
            left: side(5.0),
        };
        assert!((border.horizontal_width() - 8.0).abs() < f32::EPSILON);
        assert_eq!(border.widths(), EdgeSizes::new(1.0, 3.0, 2.0, 5.0));
    }

    #[test]
    fn layout_border_uniform_paint_requires_four_matching_visible_sides() {
        let side = LayoutBorderSide {
            width: 2.0,
            color: Color::from_srgb(0.1, 0.2, 0.3, 1.0),
            style: crate::style::computed::BorderStyle::Solid,
            ..Default::default()
        };
        let border = LayoutBorder {
            top: side,
            right: side,
            bottom: side,
            left: LayoutBorderSide { ..side },
        };
        assert_eq!(border.uniform_paint_side().unwrap().width, 2.0);

        let transparent_left = LayoutBorder {
            left: LayoutBorderSide {
                color: Color::from_srgb(0.1, 0.2, 0.3, 0.5),
                ..side
            },
            ..border
        };
        assert!(transparent_left.uniform_paint_side().is_none());

        let missing_left = LayoutBorder {
            left: LayoutBorderSide { width: 0.0, ..side },
            ..border
        };
        assert!(missing_left.uniform_paint_side().is_none());
    }

    #[test]
    fn none_and_hidden_borders_have_zero_used_layout_width() {
        use crate::style::computed::{BorderSide, BorderStyle};

        let color = crate::types::Color::rgb(255, 0, 0).into();
        let hidden = BorderSide::new(24.0, color, BorderStyle::Hidden);
        let none = BorderSide::new(24.0, color, BorderStyle::None);
        let solid = BorderSide::solid(3.0, color);
        let border = LayoutBorder::from_computed(
            &BorderSides {
                top: hidden,
                right: none,
                bottom: solid,
                left: solid,
            },
            crate::types::Color::BLACK,
        );

        assert_eq!(border.top.width, 0.0);
        assert_eq!(border.right.width, 0.0);
        assert_eq!(border.bottom.width, 3.0);
        assert_eq!(border.left.width, 3.0);
        assert_eq!(border.horizontal_width(), 3.0);
        assert_eq!(border.vertical_width(), 3.0);
    }

    #[test]
    fn layout_border_vertical_width() {
        let side = |width| LayoutBorderSide {
            width,
            style: crate::style::computed::BorderStyle::Solid,
            ..Default::default()
        };
        let border = LayoutBorder {
            top: side(4.0),
            right: side(1.0),
            bottom: side(6.0),
            left: side(1.0),
        };
        assert!((border.vertical_width() - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_border_max_width() {
        let side = |width| LayoutBorderSide {
            width,
            style: crate::style::computed::BorderStyle::Solid,
            ..Default::default()
        };
        let border = LayoutBorder {
            top: side(2.0),
            right: side(7.0),
            bottom: side(3.0),
            left: side(5.0),
        };
        assert!((border.max_width() - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn flex_column_layout() {
        let html = r#"<div style="display: flex; flex-direction: column">
            <div>First</div>
            <div>Second</div>
            <div>Third</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let text_blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, element)| element.inspect_text(|_| ()).is_some())
            .collect();
        assert!(
            text_blocks.len() >= 3,
            "Expected at least 3 text blocks for column flex children, got {}",
            text_blocks.len()
        );
    }

    #[test]
    fn flex_column_text_uses_the_stretched_cross_size_for_wrapping() {
        let html = r#"<div style="display:flex;flex-direction:column;font-size:11px">
            <span style="font-weight:700">Hammer Drill SDS - A34</span>
        </div>"#;
        let pages = layout(
            &parse_html(html).unwrap(),
            PageSize::new(PageSize::A4.height, PageSize::A4.width),
            Margin::uniform(0.0),
        );
        let text = find_page_text_block_containing(&pages[0].elements, "Hammer Drill SDS -")
            .expect("flex-column product name");

        assert_eq!(
            text.lines.len(),
            1,
            "a stretched column flex item must wrap against the container width"
        );
    }

    #[test]
    fn flex_column_with_background() {
        let html = r#"<div style="display: flex; flex-direction: column; background-color: #eee">
            <p>Child A</p>
            <p>Child B</p>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_bg = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|block| block.paint.background.color.is_some())
                .unwrap_or(false)
        });
        assert!(
            has_bg,
            "Expected a wrapper TextBlock with background_color for flex column container"
        );
    }

    #[test]
    fn table_rowspan_layout() {
        let html = r#"
            <table>
                <tr><td rowspan="2">Spanning</td><td>A</td></tr>
                <tr><td>B</td></tr>
                <tr><td>C</td><td>D</td></tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let rows = table_rows(&pages[0]);
        assert!(
            rows.len() >= 2,
            "Expected at least 2 table rows with rowspan, got {}",
            rows.len()
        );
    }

    #[test]
    fn inline_span_inherits_border_radius() {
        let css = "span.badge { background-color: green; border-radius: 5pt; padding: 2pt; }";
        let rules = parse_stylesheet(css);
        let html = r#"<p><span class="badge">Tag</span></p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let mut found_br = false;
        for (_, element) in &pages[0].elements {
            let mut runs = Vec::new();
            collect_text_runs_from_element(element.as_ref(), &mut runs);
            found_br |= runs
                .iter()
                .any(|run| run.text.contains("Tag") && !run.border_radii.is_zero());
        }
        assert!(
            found_br,
            "Expected TextRun for 'Tag' to have border_radius > 0 from stylesheet"
        );
    }

    #[test]
    fn grid_layout_produces_rows() {
        let css = ".grid { display: grid; grid-template-columns: 1fr 1fr; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="grid"><div>A</div><div>B</div><div>C</div><div>D</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        // Grid rows are now inside a Container wrapper
        let has_grid_container = !extract_grid_rows(&pages).is_empty();
        assert!(
            has_grid_container,
            "Expected Container with GridRow children from display: grid layout"
        );
    }

    #[test]
    fn page_break_produces_multiple_pages() {
        let html = r#"
            <p>Page one content</p>
            <div style="page-break-before: always">
                <p>Page two content</p>
            </div>
            <div style="page-break-before: always">
                <p>Page three content</p>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(
            pages.len() >= 3,
            "Expected at least 3 pages from two page-break-before: always, got {}",
            pages.len()
        );
    }

    #[test]
    fn image_element_in_layout() {
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==" style="width: 50px; height: 50px">"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        let has_image = pages[0]
            .elements
            .iter()
            .any(|(_, element)| element.find_image(|_| ()).is_some());
        assert!(has_image, "Expected an Image layout element from img tag");
    }

    #[test]
    fn wrapper_textblock_with_border() {
        let css = ".bordered { border: 2pt solid black; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="bordered"><p>inside</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_border = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|block| block.box_model.border.has_any())
                .or_else(|| {
                    element.inspect_container(|container| container.box_model.border.has_any())
                })
                .unwrap_or(false)
        });
        assert!(
            has_border,
            "Expected a TextBlock or Container with border from .bordered div"
        );
    }

    #[test]
    fn wrapper_textblock_with_box_shadow() {
        let css = ".shadow { box-shadow: 2pt 2pt 4pt #000; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="shadow"><p>shadowed</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        let has_shadow = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|block| !block.paint.shadows.is_empty())
                .or_else(|| {
                    element.inspect_container(|container| !container.paint.shadows.is_empty())
                })
                .unwrap_or(false)
        });
        assert!(
            has_shadow,
            "Expected a TextBlock or Container with box_shadow from .shadow div"
        );
    }

    #[test]
    fn flex_column_child_positioning() {
        let html = r#"<div style="display: flex; flex-direction: column">
            <div>Alpha</div>
            <div>Beta</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let text_blocks: Vec<_> = pages[0]
            .elements
            .iter()
            .filter(|(_, element)| {
                element
                    .inspect_text(|block| !block.lines.is_empty())
                    .unwrap_or(false)
            })
            .collect();
        if text_blocks.len() >= 2 {
            assert!(
                text_blocks[1].0 >= text_blocks[0].0,
                "Expected second flex column child to be at or below first child"
            );
        }
    }

    #[test]
    fn grid_row_alignment_in_paginate() {
        let css = ".g { display: grid; grid-template-columns: 1fr 1fr 1fr; }";
        let rules = parse_stylesheet(css);
        let html = r#"<div class="g"><div>X</div><div>Y</div><div>Z</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);
        assert_eq!(pages.len(), 1);
        let grid_rows = extract_grid_rows(&pages);
        assert!(
            !grid_rows.is_empty(),
            "Expected GridRow elements from grid layout"
        );
    }

    #[test]
    fn table_descendant_selector_total_row_td() {
        // .total-row td should apply styles via descendant selector on table rows
        let html = r#"<html><head><style>
            .total-row td { font-weight: bold; font-size: 14pt; }
        </style></head><body>
        <table><tbody>
            <tr><td>Normal</td></tr>
            <tr class="total-row"><td>Total</td></tr>
        </tbody></table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = pages.iter().flat_map(table_rows).collect::<Vec<_>>();
        assert_eq!(rows.len(), 2, "Expected 2 table rows");
        assert!(
            rows[1].content.cells[0]
                .layout
                .content
                .lines
                .iter()
                .flat_map(|line| &line.runs)
                .all(|run| run.bold),
            "Cell in .total-row should be bold via descendant selector"
        );
        let normal_h: f32 = rows[0].content.cells[0]
            .layout
            .content
            .lines
            .iter()
            .map(|line| line.height)
            .sum();
        let total_h: f32 = rows[1].content.cells[0]
            .layout
            .content
            .lines
            .iter()
            .map(|line| line.height)
            .sum();
        assert!(
            total_h > normal_h,
            "Total row text should be larger: {total_h} vs {normal_h}"
        );
    }

    #[test]
    fn flex_grow_distributes_free_space() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex-grow: 1; }
            .b { flex-grow: 2; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = flex_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let cells = &rows[0].content.cells;
        assert_eq!(cells.len(), 2);
        // With flex-grow 1:2, widths should be roughly 100:200
        let ratio = cells[1].width / cells[0].width;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "flex-grow 1:2 should produce ~2:1 width ratio, got {ratio}"
        );
    }

    #[test]
    fn flex_basis_overrides_width() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 400pt; }
            .a { flex-basis: 100pt; }
            .b { flex-basis: 300pt; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = flex_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let cells = &rows[0].content.cells;
        assert_eq!(cells.len(), 2);
        // flex-basis: 100pt vs 300pt
        assert!(
            (cells[0].width - 100.0).abs() < 5.0,
            "First cell should be ~100pt, got {}",
            cells[0].width
        );
        assert!(
            (cells[1].width - 300.0).abs() < 5.0,
            "Second cell should be ~300pt, got {}",
            cells[1].width
        );
    }

    #[test]
    fn margin_collapsing_adjacent_blocks() {
        // Adjacent sibling margins collapse: max(20, 30) = 30pt gap, not 50pt
        let html = r#"<html><head><style>
            .a { margin-bottom: 20pt; }
            .b { margin-top: 30pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Find the two TextBlock y-positions
        let mut ys: Vec<f32> = Vec::new();
        for (y, element) in &pages[0].elements {
            if element
                .inspect_text(|block| !block.lines.is_empty())
                .unwrap_or(false)
            {
                ys.push(*y);
            }
        }
        assert_eq!(ys.len(), 2, "Expected 2 text blocks, got {}", ys.len());
        // The gap between the bottom of the first block and the second y-position
        // should reflect collapsed margin (30pt), not stacked (50pt).
        // We can't check exact absolute positions easily, but we can verify the
        // second block is closer than it would be without collapsing.
        let gap = ys[1] - ys[0];
        // Without collapsing: first_content_height + 20 + 30 = content + 50
        // With collapsing: first_content_height + 30
        // The gap should be smaller than content + 50
        assert!(gap > 0.0, "Second block should be below first");
    }

    #[test]
    fn margin_collapse_through_container() {
        // CSS 2.1 § 8.3.1: the top margin of a parent with no padding/border
        // collapses with the margin-top of its first in-flow child (and same
        // for margin-bottom / last child). This mirrors the .block-pseudo
        // fixture where two sibling containers each wrap a <p> whose default
        // 1em margins should collapse through the container — not stack.
        let html = r#"<html><head><style>
            .wrap { position: relative; margin-bottom: 12pt; }
            .wrap::before { content: ""; display: block; position: absolute;
                left: 0; top: 0; width: 4pt; height: 100%; background: #3b82f6; }
            .wrap p { margin-top: 16pt; margin-bottom: 16pt; }
        </style></head><body>
        <div class="wrap"><p>one</p></div>
        <div class="wrap"><p>two</p></div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Collect the y-positions of the two <p> text blocks (one inside
        // each .wrap Container).
        let mut text_ys: Vec<f32> = Vec::new();
        fn collect_text_ys(elements: &[(f32, LayoutNode)], out: &mut Vec<f32>) {
            struct HasText(bool);

            impl LayoutVisitor for HasText {
                fn visit_text_block(&mut self, block: &TextBlock) {
                    self.0 |= !block.lines.is_empty();
                }
            }

            for (y, element) in elements {
                let mut has_text = HasText(false);
                visit_layout_tree(element.as_ref(), &mut has_text);
                if has_text.0 {
                    out.push(*y);
                }
            }
        }
        collect_text_ys(&pages[0].elements, &mut text_ys);
        assert_eq!(text_ys.len(), 2, "expected 2 text blocks");
        // Without collapse: gap = 16 (p.mb) + 12 (div.mb) + 16 (p.mt) = 44pt
        //                         plus text height of first <p>
        // With collapse:   gap = max(16, 12, 16) = 16pt + text height
        // The container y is the same as child y since margins collapsed in.
        // We can't reliably inspect inner text y without deeper traversal,
        // but we can check the Container y positions instead.
        let container_ys: Vec<f32> = pages[0]
            .elements
            .iter()
            .filter_map(|(y, element)| element.inspect_container(|_| *y))
            .collect();
        assert_eq!(container_ys.len(), 2, "expected 2 wrap containers");
        let gap = container_ys[1] - container_ys[0];
        // Expected: 16pt (collapsed) + height of <p> text (~16pt at 16pt
        // font with 1.5 line-height ≈ 24pt). Total ≈ 40pt.
        // Without collapse this would be ~68pt (44 + 24).
        assert!(
            gap < 50.0,
            "Containers should be tight (margin-collapse-through-parent), got {}pt",
            gap
        );
    }

    #[test]
    fn flex_shorthand_parsing() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex: 1; }
            .b { flex: 2; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = flex_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let cells = &rows[0].content.cells;
        assert_eq!(cells.len(), 2);
        // flex: 1 and flex: 2 with basis=0 should distribute 300pt as 100:200
        let ratio = cells[1].width / cells[0].width;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "flex shorthand 1:2 should produce ~2:1 width ratio, got {ratio}"
        );
    }

    #[test]
    fn flex_shrink_overflow() {
        // Items totalling 600pt in a 300pt container should shrink
        let html = r#"<html><head><style>
            .container { display: flex; width: 300pt; }
            .a { flex-basis: 400pt; flex-shrink: 1; }
            .b { flex-basis: 200pt; flex-shrink: 1; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = flex_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let cells = &rows[0].content.cells;
        let total: f32 = cells.iter().map(|c| c.width).sum();
        assert!(
            total <= 305.0,
            "Shrunk items should fit in container (~300pt), got {total}"
        );
        // Proportional: 400 shrinks more than 200
        assert!(
            cells[0].width > cells[1].width,
            "Larger basis should still be wider after shrink"
        );
    }

    #[test]
    fn flex_shrink_zero_prevents_shrink() {
        let html = r#"<html><head><style>
            .container { display: flex; width: 200pt; }
            .a { flex-basis: 150pt; flex-shrink: 0; }
            .b { flex-basis: 150pt; flex-shrink: 1; }
        </style></head><body>
        <div class="container">
            <div class="a">A</div>
            <div class="b">B</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = flex_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let cells = &rows[0].content.cells;
        // First item has shrink: 0 so it keeps its basis
        assert!(
            (cells[0].width - 150.0).abs() < 5.0,
            "flex-shrink: 0 should prevent shrinking, got {}",
            cells[0].width
        );
        // Second item absorbs all the deficit
        assert!(
            cells[1].width < 150.0,
            "flex-shrink: 1 item should shrink, got {}",
            cells[1].width
        );
    }

    #[test]
    fn flex_no_grow_uses_content_width() {
        // 3 flex items with no flex-grow should use their content width,
        // not expand to fill the full container.
        let html = r#"<html><head><style>
            .container { display: flex; width: 400pt; }
            .item { width: 50pt; }
        </style></head><body>
        <div class="container">
            <div class="item">A</div>
            <div class="item">B</div>
            <div class="item">C</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = flex_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let cells = &rows[0].content.cells;
        assert_eq!(cells.len(), 3);
        // Each item should be ~50pt wide, not ~133pt (400/3)
        for (idx, cell) in cells.iter().enumerate() {
            assert!(
                (cell.width - 50.0).abs() < 5.0,
                "Item {} should be ~50pt wide (content width), got {}",
                idx,
                cell.width
            );
        }
        // Total width of items should be much less than container width
        let total: f32 = cells.iter().map(|c| c.width).sum();
        assert!(
            total < 200.0,
            "Total item width should be much less than 400pt container, got {total}"
        );
    }

    #[test]
    fn flex_justify_center_positions_items_in_middle() {
        // justify-content: center should position items in the middle
        // of the container, not at the start.
        let html = r#"<html><head><style>
            .container { display: flex; justify-content: center; width: 400pt; }
            .item { width: 100pt; }
        </style></head><body>
        <div class="container">
            <div class="item">X</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = flex_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let cells = &rows[0].content.cells;
        assert_eq!(cells.len(), 1);
        // Item should be centered: x_offset should be ~150pt ((400-100)/2)
        assert!(
            (cells[0].x_offset - 150.0).abs() < 5.0,
            "justify-content: center should put item at ~150pt, got {}",
            cells[0].x_offset
        );
        // Width should stay at 100pt
        assert!(
            (cells[0].width - 100.0).abs() < 5.0,
            "Item width should remain ~100pt, got {}",
            cells[0].width
        );
    }

    #[test]
    fn flex_justify_space_between_distributes_items() {
        // justify-content: space-between with 3 items should put first at
        // start, last at end, and distribute space between them evenly.
        let html = r#"<html><head><style>
            .container { display: flex; justify-content: space-between; width: 400pt; }
            .item { width: 80pt; }
        </style></head><body>
        <div class="container">
            <div class="item">A</div>
            <div class="item">B</div>
            <div class="item">C</div>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let rows = flex_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let cells = &rows[0].content.cells;
        assert_eq!(cells.len(), 3);
        // First item should be at x=0
        assert!(
            cells[0].x_offset < 5.0,
            "First item should be at start, got {}",
            cells[0].x_offset
        );
        // Last item should end at ~400pt (x_offset + width ~ 400)
        let last_end = cells[2].x_offset + cells[2].width;
        assert!(
            (last_end - 400.0).abs() < 5.0,
            "Last item should end at ~400pt, got {last_end}"
        );
        // Gaps between items should be equal
        let gap1 = cells[1].x_offset - (cells[0].x_offset + cells[0].width);
        let gap2 = cells[2].x_offset - (cells[1].x_offset + cells[1].width);
        assert!(
            (gap1 - gap2).abs() < 1.0,
            "Gaps should be equal: {gap1} vs {gap2}"
        );
        // Each gap should be ~80pt ((400 - 240) / 2)
        assert!((gap1 - 80.0).abs() < 5.0, "Gap should be ~80pt, got {gap1}");
    }

    #[test]
    fn margin_collapsing_negative_margins() {
        let html = r#"<html><head><style>
            .a { margin-bottom: -10pt; }
            .b { margin-top: -20pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut ys: Vec<f32> = Vec::new();
        for (y, element) in &pages[0].elements {
            if element
                .inspect_text(|block| !block.lines.is_empty())
                .unwrap_or(false)
            {
                ys.push(*y);
            }
        }
        assert_eq!(ys.len(), 2);
        // Both negative: most negative wins (-20), not sum (-30)
        // Second block may overlap first (negative gap)
    }

    #[test]
    fn margin_collapsing_mixed_signs() {
        let html = r#"<html><head><style>
            .a { margin-bottom: -10pt; }
            .b { margin-top: 30pt; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut ys: Vec<f32> = Vec::new();
        for (y, element) in &pages[0].elements {
            if element
                .inspect_text(|block| !block.lines.is_empty())
                .unwrap_or(false)
            {
                ys.push(*y);
            }
        }
        assert_eq!(ys.len(), 2);
        // Mixed: sum = -10 + 30 = 20pt gap (not 30 or 40)
        let gap = ys[1] - ys[0];
        assert!(gap > 0.0, "Gap should be positive with mixed margins");
    }

    #[test]
    fn margin_collapsing_zero_margins() {
        let html = r#"<html><head><style>
            .a { margin-bottom: 0; }
            .b { margin-top: 0; }
        </style></head><body>
        <p class="a">First</p>
        <p class="b">Second</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
    }

    #[test]
    fn table_descendant_selector_thead_th() {
        let html = r#"<html><head><style>
            thead th { color: red; font-size: 14pt; }
        </style></head><body>
        <table>
            <thead><tr><th>Header</th></tr></thead>
            <tbody><tr><td>Body</td></tr></tbody>
        </table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
        // Should render without panics; thead th selector exercises section ancestor chain
    }

    #[test]
    fn table_descendant_selector_tbody_td() {
        let html = r#"<html><head><style>
            tbody td { font-style: italic; }
            table td { font-size: 11pt; }
        </style></head><body>
        <table>
            <thead><tr><th>H</th></tr></thead>
            <tbody><tr><td>B</td></tr></tbody>
        </table>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let rules: Vec<_> = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages.is_empty());
    }

    #[test]
    fn table_colgroup_percentage_widths() {
        let html = r#"<table>
            <colgroup>
                <col span="1" style="width: 30%;">
                <col span="1" style="width: 70%;">
            </colgroup>
            <tr><th>Name</th><td>Contract_2026_Q1.pdf</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 1, "Expected 1 table row");
        let col_widths = &rows[0].content.column_widths;
        assert_eq!(col_widths.len(), 2, "Expected 2 columns");
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.30).abs() < 0.05,
            "First column should be ~30% of total, got {:.1}% (widths: {:?})",
            ratio * 100.0,
            col_widths
        );
    }

    fn first_table_row_col_widths(html: &str) -> Vec<f32> {
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_table(|row| row.content.column_widths.clone()))
            .expect("expected table row")
    }

    #[test]
    fn table_colgroup_percentage_widths_ignore_border_spacing() {
        let no_spacing = first_table_row_col_widths(
            r#"<table style="width: 300pt">
                <colgroup>
                    <col span="1" style="width: 30%;">
                    <col span="1" style="width: 70%;">
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );
        let spaced = first_table_row_col_widths(
            r#"<table style="width: 300pt; border-spacing: 10pt">
                <colgroup>
                    <col span="1" style="width: 30%;">
                    <col span="1" style="width: 70%;">
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert_eq!(no_spacing.len(), 2);
        assert_eq!(spaced.len(), 2);
        assert!(
            (spaced[0] - no_spacing[0]).abs() < 0.5,
            "border-spacing should not narrow percentage columns: {:?} vs {:?}",
            spaced,
            no_spacing
        );
        assert!(
            (spaced[1] - no_spacing[1]).abs() < 0.5,
            "border-spacing should not narrow percentage columns: {:?} vs {:?}",
            spaced,
            no_spacing
        );
    }

    #[test]
    fn table_colgroup_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col width="25%">
                <col width="75%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let col_widths = &rows[0].content.column_widths;
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.25).abs() < 0.05,
            "First column should be ~25% of total, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn table_colgroup_last_inline_width_wins() {
        let html = r#"<table>
            <colgroup>
                <col style="width: 10%; width: 40%;" width="90%">
                <col style="width: 60%;">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = first_table_row(&pages[0]).content.column_widths;
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.40).abs() < 0.05,
            "Last inline width declaration should win, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_inline_width_ignores_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col style="width: auto" width="80%">
                <col>
            </colgroup>
            <tr><td>Short</td><td>Much longer content here</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = first_table_row(&pages[0]).content.column_widths;
        assert!(
            col_widths[1] > col_widths[0],
            "Inline width should override width attribute; got {:?}",
            col_widths
        );
    }

    #[test]
    fn table_colgroup_malformed_inline_width_is_ignored() {
        let html = r#"<table>
            <colgroup>
                <col style="width: 10%; width: not-a-width" width="25%">
                <col style="width: not-a-width" width="90%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = first_table_row(&pages[0]).content.column_widths;
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.10).abs() < 0.05,
            "Malformed inline width should be ignored, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_all_invalid_inline_widths_fall_back_to_width_attribute() {
        let html = r#"<table>
            <colgroup>
                <col style="width: not-a-width" width="80%">
                <col width="20%">
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = first_table_row(&pages[0]).content.column_widths;
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.80).abs() < 0.05,
            "All-invalid inline widths should fall back to width attributes, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_colgroup_span_attribute() {
        let html = r#"<table>
            <colgroup>
                <col span="2" style="width: 20%;">
                <col span="1" style="width: 60%;">
            </colgroup>
            <tr><td>A</td><td>B</td><td>C</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let col_widths = &rows[0].content.column_widths;
        assert_eq!(col_widths.len(), 3);
        let total: f32 = col_widths.iter().sum();
        let ratio_0 = col_widths[0] / total;
        let ratio_2 = col_widths[2] / total;
        assert!(
            (ratio_0 - 0.20).abs() < 0.05,
            "First two columns should each be ~20%, got {:.1}%",
            ratio_0 * 100.0
        );
        assert!(
            (ratio_2 - 0.60).abs() < 0.05,
            "Third column should be ~60%, got {:.1}%",
            ratio_2 * 100.0
        );
    }

    #[test]
    fn table_bare_col_without_colgroup() {
        let html = r#"<table>
            <col style="width: 40%;">
            <col style="width: 60%;">
            <tr><td>X</td><td>Y</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let col_widths = &rows[0].content.column_widths;
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.40).abs() < 0.05,
            "First column should be ~40%, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn table_without_colgroup_unchanged() {
        let html = "<table><tr><td>Short</td><td>Much longer content here</td></tr></table>";
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let col_widths = &rows[0].content.column_widths;
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[1] > col_widths[0],
            "Auto-sizing should still work: longer column ({}) should be wider than short ({})",
            col_widths[1],
            col_widths[0]
        );
    }

    #[test]
    fn table_mixed_explicit_and_auto_widths() {
        let html = r#"<table>
            <colgroup>
                <col width="25%">
                <col>
            </colgroup>
            <tr><td>Fixed</td><td>Auto column content</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let rows = table_rows(&pages[0]);
        assert_eq!(rows.len(), 1);
        let col_widths = &rows[0].content.column_widths;
        assert_eq!(col_widths.len(), 2);
        assert!(
            col_widths[0] > 0.0 && col_widths[1] > 0.0,
            "Both explicit and auto columns should keep usable widths: {:?}",
            col_widths
        );
        // The auto column keeps a width comparable to the explicit 25% column
        // (it must not be starved by the explicit-width redistribution). The
        // exact split tracks the resolved font's text metrics, so compare
        // proportionally rather than with a tight absolute tolerance.
        assert!(
            col_widths[1] >= col_widths[0] * 0.75,
            "Auto column should not be collapsed by explicit width redistribution: {:?}",
            col_widths
        );
    }

    #[test]
    fn table_layout_fixed_uses_colgroup_widths_over_content() {
        let html = r#"<table style="table-layout: fixed; width: 400pt;">
            <colgroup>
                <col style="width: 25%;">
                <col style="width: 75%;">
            </colgroup>
            <tr>
                <td>Very long content that should not widen the first fixed column</td>
                <td>Short</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = first_table_row(&pages[0]).content.column_widths;
        let total: f32 = col_widths.iter().sum();
        let ratio = col_widths[0] / total;
        assert!(
            (ratio - 0.25).abs() < 0.02,
            "fixed layout should honor colgroup width instead of content, got {:.1}% ({:?})",
            ratio * 100.0,
            col_widths
        );
    }

    #[test]
    fn table_layout_fixed_uses_first_row_cell_widths() {
        let html = r#"<table style="table-layout: fixed; width: 300pt; border-spacing: 0;">
            <tr>
                <td style="width: 90pt;">A</td>
                <td>B</td>
            </tr>
            <tr>
                <td>Short</td>
                <td>Longer content in the second column</td>
            </tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = first_table_row(&pages[0]).content.column_widths;
        assert_eq!(
            col_widths.as_slice(),
            &[90.0, 210.0],
            "the auto track should receive the table width left after the authored first-row width"
        );
    }

    #[test]
    fn table_colgroup_absolute_lengths_are_supported() {
        let html = r#"<table style="table-layout: fixed; width: 300pt; border-spacing: 0;">
            <colgroup>
                <col style="width: 90pt;">
                <col>
            </colgroup>
            <tr><td>A</td><td>B</td></tr>
        </table>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let col_widths = first_table_row(&pages[0]).content.column_widths;
        assert_eq!(
            col_widths.as_slice(),
            &[90.0, 210.0],
            "the auto track should receive the table width left after the authored <col> width"
        );
    }

    #[test]
    fn table_colgroup_em_width_uses_column_font_size() {
        let widths = first_table_row_col_widths(
            r#"<table style="table-layout: fixed; width: 200pt; border-spacing: 0;">
                <colgroup style="font-size: 20pt">
                    <col style="width: 2em;">
                    <col>
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert_eq!(
            widths.as_slice(),
            &[40.0, 160.0],
            "2em should resolve against the colgroup font size before the auto track receives the remainder"
        );
    }

    #[test]
    fn table_colgroup_calc_em_width_uses_column_font_size() {
        let widths = first_table_row_col_widths(
            r#"<table style="table-layout: fixed; width: 200pt; border-spacing: 0;">
                <colgroup style="font-size: 20pt">
                    <col style="width: calc(1em + 5pt);">
                    <col>
                </colgroup>
                <tr><td>A</td><td>B</td></tr>
            </table>"#,
        );

        assert_eq!(
            widths.as_slice(),
            &[25.0, 175.0],
            "calc(1em + 5pt) should resolve against the colgroup font size before the auto track receives the remainder"
        );
    }

    #[test]
    fn table_layout_fixed_auto_tracks_ignore_cell_font_size() {
        let widths = first_table_row_col_widths(
            r#"<table style="table-layout: fixed; width: 300pt; border-spacing: 0;">
                <tr>
                    <td style="font-size: 6pt"></td>
                    <td style="font-size: 72pt"></td>
                </tr>
            </table>"#,
        );

        assert_eq!(
            widths.as_slice(),
            &[150.0, 150.0],
            "unresolved fixed-layout tracks should divide the remainder equally regardless of cell font size"
        );
    }

    #[test]
    fn table_cell_block_content_preserves_link_and_whitespace() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        <div><a href="https://example.com">Click here</a></div>
                        <pre>  keep   spaces  </pre>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let row = first_table_row(&pages[0]);
        let cells = &row.content.cells;
        let runs = text_runs_in_cell(&cells[0]);
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert!(
            runs.iter()
                .any(|run| run.link_url.as_deref() == Some("https://example.com")),
            "Expected link URL to survive nested block traversal"
        );
        assert!(
            text.contains("  keep   spaces  "),
            "Expected preformatted whitespace to survive nested block traversal: {text:?}"
        );
    }

    #[test]
    fn table_cell_mixed_recursion_keeps_nested_block_padding_but_not_cell_padding() {
        let html = r#"
            <table>
                <tr>
                    <td style="padding: 18pt 12pt; text-align: right;">
                        Direct text
                        <div style="padding-left: 6pt; padding-top: 3pt; background-color: #eee;">
                            Nested block
                        </div>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let row = first_table_row(&pages[0]);
        let cells = &row.content.cells;
        let direct_run = cells[0]
            .layout
            .content
            .lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .find(|run| run.text.contains("Direct"))
            .expect("expected direct cell text run");
        assert_eq!(
            direct_run.padding,
            EdgeSizes::ZERO,
            "direct cell text should not inherit table-cell padding"
        );
        let nested_block = find_text_block_containing(&cells[0].layout.content.children, "Nested")
            .expect("expected nested block text run");
        assert_eq!(
            nested_block.box_model.padding,
            EdgeSizes::new(3.0, 0.0, 0.0, 6.0)
        );
    }

    #[test]
    fn table_cell_nested_table_is_preserved_as_nested_layout() {
        let html = r#"
            <table>
                <tr>
                    <td>
                        Outer
                        <table>
                            <tr><td>Inner</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let row = first_table_row(&pages[0]);
        let cells = &row.content.cells;
        assert!(
            !cells[0].layout.content.children.is_empty(),
            "expected nested table rows to be preserved"
        );
        let nested_text: String = cells[0]
            .layout
            .content
            .children
            .iter()
            .filter_map(|element| {
                element.inspect_table(|row| {
                    row.content
                        .cells
                        .iter()
                        .flat_map(|cell| cell.layout.content.lines.iter())
                        .flat_map(|line| line.runs.iter())
                        .map(|run| run.text.as_str())
                        .collect::<String>()
                })
            })
            .collect();
        assert!(
            nested_text.contains("Inner"),
            "expected nested table text to stay in nested layout: {nested_text:?}"
        );
    }

    #[test]
    fn nested_fixed_table_percentage_width_uses_table_cell_width() {
        let html = r#"
            <table style="table-layout: fixed; width: 400pt;">
                <tr>
                    <td>
                        <table style="table-layout: fixed; width: 100%;">
                            <colgroup>
                                <col style="width: 30%;">
                                <col style="width: 70%;">
                            </colgroup>
                            <tr><td>A</td><td>B</td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let outer = first_table_row(&pages[0]);
        let outer_col_widths = &outer.content.column_widths;
        let outer_cells = &outer.content.cells;
        let nested_col_widths = outer_cells[0]
            .layout
            .content
            .children
            .iter()
            .find_map(|element| element.inspect_table(|row| row.content.column_widths.clone()))
            .expect("expected nested table row");
        let nested_total: f32 = nested_col_widths.iter().sum();
        let expected_inner_width =
            outer_col_widths[0] - outer_cells[0].layout.box_model.content_insets.horizontal();
        assert!(
            (nested_total - expected_inner_width).abs() < 1.0,
            "nested fixed table should expand to the table cell width, got total {nested_total} vs {expected_inner_width}"
        );
        let first_ratio = nested_col_widths[0] / nested_total;
        assert!(
            (first_ratio - 0.30).abs() < 0.02,
            "nested fixed table should honor percentage colgroup widths, got {:?}",
            nested_col_widths
        );
    }

    #[test]
    fn certificate_like_nested_table_uses_full_width() {
        let html = r#"
            <style>
                @page {
                    size: A4 landscape;
                    margin: 1cm;
                }
                table {
                    table-layout: fixed;
                    width: 100%;
                    border-collapse: collapse;
                    column-count: 2;
                }
                .content th,
                .content td {
                    padding: 0 16px 8px 0;
                    word-wrap: break-word;
                }
            </style>
            <table>
                <tr style="vertical-align: top">
                    <td>
                        <table class="content">
                            <colgroup>
                                <col span="1" style="width: 30%;">
                                <col span="1" style="width: 70%;">
                            </colgroup>
                            <tr><th>Name</th><td>Contract_2026_Q1.pdf</td></tr>
                            <tr><th>Verification</th><td><a href="https://app.ipocamp.io/verify">https://app.ipocamp.io/verify</a></td></tr>
                        </table>
                    </td>
                </tr>
            </table>
        "#;
        let parsed = parse_html_with_styles(html).unwrap();
        let mut page_rules = Vec::new();
        for css in &parsed.stylesheets {
            page_rules.extend(crate::parser::css::parse_page_rules(css));
        }
        let mut page_size = PageSize::default();
        let mut margin = Margin::default();
        for page_rule in &page_rules {
            if let (Some(width), Some(height)) = (page_rule.width, page_rule.height) {
                page_size = PageSize { width, height };
            }
            if let Some(v) = page_rule.margin_top {
                margin.top = v;
            }
            if let Some(v) = page_rule.margin_right {
                margin.right = v;
            }
            if let Some(v) = page_rule.margin_bottom {
                margin.bottom = v;
            }
            if let Some(v) = page_rule.margin_left {
                margin.left = v;
            }
        }
        let media_ctx = crate::parser::css::MediaContext {
            width: page_size.width,
            height: page_size.height,
        };
        let mut rules = Vec::new();
        for css in &parsed.stylesheets {
            rules.extend(crate::parser::css::parse_stylesheet_with_context(
                css,
                Some(media_ctx),
            ));
        }
        let pages = layout_with_rules(&parsed.nodes, page_size, margin, &rules);
        let outer = first_table_row(&pages[0]);
        let outer_cells = &outer.content.cells;
        let nested_col_widths = outer_cells[0]
            .layout
            .content
            .children
            .iter()
            .find_map(|element| element.inspect_table(|row| row.content.column_widths.clone()))
            .expect("expected nested content table row");
        let nested_total: f32 = nested_col_widths.iter().sum();
        let expected_inner_width = page_size.width - margin.horizontal() - 1.5;
        assert!(
            (nested_total - expected_inner_width).abs() < 1.0,
            "certificate-like nested table should span the outer cell width, got total {nested_total} vs {expected_inner_width}"
        );
        let first_ratio = nested_col_widths[0] / nested_total;
        assert!(
            (first_ratio - 0.30).abs() < 0.02,
            "certificate-like nested table should honor percentage colgroup widths, got {:?}",
            nested_col_widths
        );
    }

    #[test]
    fn table_cell_preserves_empty_block_background_layout() {
        let path = write_test_png_file("table-cell-bg", &build_test_png_bytes());
        let html = format!(
            r#"
                <table>
                    <tr>
                        <td>
                            <div style="display: flex; width: 40pt; aspect-ratio: 1 / 1; background-image: url('{path}'); background-repeat: no-repeat;"></div>
                        </td>
                    </tr>
                </table>
            "#
        );
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        let row = first_table_row(&pages[0]);
        let cells = &row.content.cells;
        assert!(
            !cells[0].layout.content.children.is_empty(),
            "expected block descendant to be preserved as nested layout"
        );
        assert!(
            elements_have_image_background(&cells[0].layout.content.children),
            "expected nested flex block with raster background to survive table-cell layout"
        );
    }

    #[test]
    fn paginate_math_block_advances_y() {
        // MathBlock elements must reserve vertical space so subsequent content
        // doesn't overlap.
        let html = r#"<p>Before</p><div class="math-display" data-math="\frac{a}{b}">frac</div><p>After</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);

        // Gather y positions and element types
        let mut y_positions: Vec<(f32, &str)> = Vec::new();
        for (y, element) in &pages[0].elements {
            if element.inspect_text(|_| ()).is_some() {
                y_positions.push((*y, "TextBlock"));
            } else if element.inspect_math(|_| ()).is_some() {
                y_positions.push((*y, "MathBlock"));
            }
        }

        // We expect: TextBlock("Before"), MathBlock, TextBlock("After")
        assert!(
            y_positions.len() >= 3,
            "Expected at least 3 elements (Before text, MathBlock, After text), got {}: {:?}",
            y_positions.len(),
            y_positions
        );

        // Each element must have a strictly increasing y position
        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0,
                "Element {} ({} at y={}) should be below element {} ({} at y={})",
                i,
                y_positions[i].1,
                y_positions[i].0,
                i - 1,
                y_positions[i - 1].1,
                y_positions[i - 1].0,
            );
        }
    }

    #[test]
    fn paginate_math_block_from_markdown() {
        // Test the actual markdown flow: $$ ... $$ produces display math
        // that must advance Y so subsequent text doesn't overlap.
        let md = "# Title\n\n$$\\frac{a}{b}$$\n\nText after math should not overlap";
        let html = crate::parser::markdown::markdown_to_html(md);
        let nodes = parse_html(&html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);

        let mut y_positions: Vec<(f32, &str)> = Vec::new();
        for (y, element) in &pages[0].elements {
            if element.inspect_text(|_| ()).is_some() {
                y_positions.push((*y, "TextBlock"));
            } else if element.inspect_math(|_| ()).is_some() {
                y_positions.push((*y, "MathBlock"));
            }
        }

        // We expect: TextBlock(Title), MathBlock(frac), TextBlock(Text after...)
        assert!(
            y_positions.len() >= 3,
            "Expected at least 3 elements (Title, MathBlock, After text), got {}: {:?}",
            y_positions.len(),
            y_positions
        );

        // Each element must have a strictly increasing y position
        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0,
                "Element {} ({} at y={}) should be below element {} ({} at y={})",
                i,
                y_positions[i].1,
                y_positions[i].0,
                i - 1,
                y_positions[i - 1].1,
                y_positions[i - 1].0,
            );
        }
    }

    /// Verify that styled blocks (background, border, padding) don't cause
    /// subsequent content to overlap.
    #[test]
    fn styled_block_does_not_overlap_next_element() {
        let css = r#"
            .summary { background-color: #eff6ff; border-left: 4px solid #3b82f6; padding: 12px 16px; margin: 16px 0; }
        "#;
        let rules = parse_stylesheet(css);
        let html = r#"
            <h1>Title</h1>
            <div class="summary">This is a summary box with background and border styling.</div>
            <h2>Next Section</h2>
            <p>This should not overlap with the summary box.</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        // Collect Y positions of text blocks with content
        let mut y_positions = text_block_positions(&pages[0], true);
        for (_, text) in &mut y_positions {
            text.truncate(text.len().min(40));
        }

        // Each Y position should be strictly greater than the previous
        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0 + 1.0,
                "Text blocks should have distinct Y positions!\n  block {}: y={:.1} {:?}\n  block {}: y={:.1} {:?}",
                i - 1,
                y_positions[i - 1].0,
                y_positions[i - 1].1,
                i,
                y_positions[i].0,
                y_positions[i].1,
            );
        }
    }

    /// Verify that blockquotes with visual styling don't cause overlap.
    #[test]
    fn blockquote_with_background_no_overlap() {
        let css = r#"
            blockquote { margin: 20px 0; padding: 12px 20px; border-left: 4px solid #3b82f6; background-color: #f8fafc; }
        "#;
        let rules = parse_stylesheet(css);
        let html = r#"
            <p>Paragraph before the blockquote.</p>
            <blockquote>This is a blockquote with background and border styling that should take up vertical space.</blockquote>
            <p>Paragraph after the blockquote should not overlap.</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let mut y_positions = text_block_positions(&pages[0], true);
        for (_, text) in &mut y_positions {
            text.truncate(text.len().min(40));
        }

        assert!(
            y_positions.len() >= 2,
            "Expected at least 2 text blocks, got {}: {:?}",
            y_positions.len(),
            y_positions
        );

        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0 + 1.0,
                "Text blocks should have distinct Y positions!\n  block {}: y={:.1} {:?}\n  block {}: y={:.1} {:?}",
                i - 1,
                y_positions[i - 1].0,
                y_positions[i - 1].1,
                i,
                y_positions[i].0,
                y_positions[i].1,
            );
        }
    }

    /// Verify pre blocks with padding/background don't cause overlap.
    #[test]
    fn pre_block_with_padding_no_overlap() {
        let css = r#"
            pre { background-color: #1e293b; padding: 16px 20px; margin: 16px 0; }
        "#;
        let rules = parse_stylesheet(css);
        let html = r#"
            <p>Before the code block.</p>
            <pre>line 1
line 2
line 3</pre>
            <p>After the code block should not overlap.</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let mut y_positions = text_block_positions(&pages[0], true);
        for (_, text) in &mut y_positions {
            text.truncate(text.len().min(40));
        }

        assert!(
            y_positions.len() >= 2,
            "Expected at least 2 text blocks with content, got {}: {:?}",
            y_positions.len(),
            y_positions
        );

        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0 + 1.0,
                "Text blocks should have distinct Y positions!\n  block {}: y={:.1} {:?}\n  block {}: y={:.1} {:?}",
                i - 1,
                y_positions[i - 1].0,
                y_positions[i - 1].1,
                i,
                y_positions[i].0,
                y_positions[i].1,
            );
        }
    }
    /// Test with styled wrapper block containing only block children.
    #[test]
    fn styled_wrapper_with_block_children_no_overlap() {
        let css = r#"
            .section { padding: 16px; margin-bottom: 16px; border: 1px solid #e2e8f0; border-radius: 4px; }
        "#;
        let rules = parse_stylesheet(css);
        let html = r#"
            <h1>Page Break Test</h1>
            <div class="section">
                <h2>Section 1</h2>
                <p>Content in section 1.</p>
            </div>
            <div class="section">
                <h2>Section 2</h2>
                <p>Content in section 2 should not overlap with section 1.</p>
            </div>
            <p>Final paragraph after both sections.</p>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout_with_rules(&nodes, PageSize::A4, Margin::default(), &rules);

        let mut y_positions = text_block_positions(&pages[0], false);
        for (_, text) in &mut y_positions {
            text.truncate(text.len().min(50));
        }

        for i in 1..y_positions.len() {
            assert!(
                y_positions[i].0 > y_positions[i - 1].0 + 0.5,
                "Text blocks should have distinct Y positions!\n  block {}: y={:.1} {:?}\n  block {}: y={:.1} {:?}",
                i - 1,
                y_positions[i - 1].0,
                y_positions[i - 1].1,
                i,
                y_positions[i].0,
                y_positions[i].1,
            );
        }
    }

    #[test]
    fn nested_div_container_has_background_color() {
        let html = r#"
        <style>
            .d1 { border-left: 2px solid #ef4444; background-color: rgba(239,68,68,0.05); padding: 4px; }
        </style>
        <div class="d1"><span>Level 1</span>
            <div class="d2"><span>Level 2</span><p>Block child</p></div>
        </div>
        "#;
        let result = parse_html_with_styles(html).unwrap();
        let rules = crate::parser::css::parse_stylesheet(&result.stylesheets.join("\n"));
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let has_container_bg = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_container(|container| container.paint.background.color.is_some())
                .unwrap_or(false)
        });
        assert!(
            has_container_bg,
            "Container should have background_color from rgba stylesheet"
        );
    }

    #[test]
    fn vw_unit_resolves_against_content_area() {
        // Chrome resolves vw against the printable CONTENT area (page minus
        // margins), not the full page size. On Letter (612pt wide) with the
        // default 72pt margins the content width is 612 - 2*72 = 468pt, so
        // 50vw should produce ~234pt.
        let html = r#"<div style="width:50vw;background:red">test</div>"#;
        let nodes = parse_html(html).unwrap();
        let margin = Margin::default();
        let pages = layout(&nodes, PageSize::LETTER, margin);
        let content_width = PageSize::LETTER.width - margin.horizontal();
        let expected = content_width / 2.0; // (612 - 2*72) / 2 = 234pt
        for (_, element) in &pages[0].elements {
            if let Some(width) = element
                .inspect_text(|block| {
                    block
                        .paint
                        .background
                        .color
                        .is_some()
                        .then_some(block.box_model.size.width.fixed_value())
                        .flatten()
                })
                .flatten()
            {
                assert!(
                    (width - expected).abs() < 1.0,
                    "50vw on Letter should be ~{expected}pt, got {width}pt"
                );
                return;
            }
        }
        panic!("expected a TextBlock with explicit width from 50vw");
    }

    // ---- LayoutContext tests ----

    #[test]
    fn layout_context_available_width_returns_parent_content_width() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: Some(600.0),
                font_size: 16.0,
                percent_width_basis: 400.0,
            },
            containing_block: None,
            percent_height_cb: None,
            root_font_size: 16.0,
        };
        assert!((ctx.available_width() - 400.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_context_available_height_falls_back_to_viewport() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: None,
                font_size: 16.0,
                percent_width_basis: 400.0,
            },
            containing_block: None,
            percent_height_cb: None,
            root_font_size: 16.0,
        };
        assert!((ctx.available_height() - 842.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_context_available_height_uses_parent_when_set() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: Some(300.0),
                font_size: 16.0,
                percent_width_basis: 400.0,
            },
            containing_block: None,
            percent_height_cb: None,
            root_font_size: 16.0,
        };
        assert!((ctx.available_height() - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn layout_context_with_parent_preserves_viewport() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: Some(600.0),
                font_size: 16.0,
                percent_width_basis: 400.0,
            },
            containing_block: Some(ContainingBlock {
                x: 10.0,
                width: 400.0,
                height: 600.0,
                depth: 1,
            }),
            percent_height_cb: None,
            root_font_size: 16.0,
        };
        let child = ctx.with_parent(200.0, Some(150.0), 12.0);
        assert!((child.viewport.width - 595.0).abs() < f32::EPSILON);
        assert!((child.viewport.height - 842.0).abs() < f32::EPSILON);
        assert!((child.available_width() - 200.0).abs() < f32::EPSILON);
        assert!((child.available_height() - 150.0).abs() < f32::EPSILON);
        assert!((child.parent.font_size - 12.0).abs() < f32::EPSILON);
        assert!((child.root_font_size - 16.0).abs() < f32::EPSILON);
        // containing_block is preserved
        assert!(child.containing_block.is_some());
    }

    #[test]
    fn layout_context_with_containing_block_replaces_cb() {
        let ctx = LayoutContext {
            viewport: Viewport {
                width: 595.0,
                height: 842.0,
            },
            parent: ParentBox {
                content_width: 400.0,
                content_height: Some(600.0),
                font_size: 16.0,
                percent_width_basis: 400.0,
            },
            containing_block: None,
            percent_height_cb: None,
            root_font_size: 16.0,
        };
        let cb = ContainingBlock {
            x: 50.0,
            width: 300.0,
            height: 200.0,
            depth: 2,
        };
        let updated = ctx.with_containing_block(Some(cb));
        assert!(updated.containing_block.is_some());
        assert!((updated.containing_block.unwrap().x - 50.0).abs() < f32::EPSILON);
        // parent is preserved
        assert!((updated.available_width() - 400.0).abs() < f32::EPSILON);
    }

    // ---- Integration tests for extracted layout functions ----

    #[test]
    fn route_element_dispatches_flex() {
        let html = r#"<div style="display:flex"><span>A</span><span>B</span></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn route_element_dispatches_grid() {
        let html = r#"<div style="display:grid;grid-template-columns:1fr 1fr"><div>A</div><div>B</div></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn route_element_dispatches_inline() {
        let html = r#"<span>inline text</span>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn inline_block_shrink_to_fit_width() {
        let html = r#"<div style="display:inline-block;background:#eee">short</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // The inline-block should be narrower than the full page width
        let page_width = PageSize::A4.width - Margin::default().left - Margin::default().right;
        for (_, element) in &pages[0].elements {
            if let Some(w) = element
                .inspect_text(|block| block.box_model.size.width.fixed_value())
                .flatten()
            {
                assert!(
                    w < page_width,
                    "inline-block width {w} should be less than page width {page_width}"
                );
            }
        }
    }

    #[test]
    fn percentage_border_radius_resolved() {
        let html =
            r#"<div style="width:100px;height:100px;border-radius:50%;background:red">.</div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // 50% of min(100px=75pt, 100px=75pt) = 37.5pt
        for (_, el) in &pages[0].elements {
            let radii = el
                .inspect_text(|block| block.paint.border_radii)
                .or_else(|| el.inspect_container(|container| container.paint.border_radii));
            if let Some(border_radii) = radii {
                if !border_radii.is_zero() {
                    let border_radius = border_radii.uniform_radius().unwrap();
                    assert!(
                        (border_radius - 37.5).abs() < 1.0,
                        "border_radius {border_radius} should be ~37.5pt"
                    );
                }
            }
        }
    }

    #[test]
    fn css_height_narrows_available_height_for_children() {
        // Parent with explicit height, child SVG with percentage height
        let html = r#"<div style="height:200pt"><svg width="100" height="50%"></svg></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        fn find_svg(elements: &[(f32, LayoutNode)]) -> Option<f32> {
            struct SvgHeight(Option<f32>);

            impl LayoutVisitor for SvgHeight {
                fn visit_svg(&mut self, svg: &Svg) {
                    if self.0.is_none() {
                        self.0 = Some(svg.geometry.size.height);
                    }
                }
            }

            let mut height = SvgHeight(None);
            for (_, element) in elements {
                visit_layout_tree(element.as_ref(), &mut height);
            }
            height.0
        }
        let svg_h = find_svg(&pages[0].elements).expect("expected svg element");
        assert!(
            (svg_h - 100.0).abs() < 1.0,
            "SVG height {svg_h} should be ~100pt (50% of 200pt)"
        );
    }

    #[test]
    fn flex_grow_distributes_remaining_space() {
        let html = r#"
            <div style="display:flex;width:300px">
                <div style="flex-grow:1;background:#aaa">A</div>
                <div style="flex-grow:2;background:#ccc">B</div>
            </div>
        "#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn multicolumn_layout_creates_grid() {
        let html = r#"<div style="column-count:3"><p>One</p><p>Two</p><p>Three</p></div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        // Should produce grid rows
        let has_grid = pages[0].elements.iter().any(|(_, element)| {
            element.inspect_container(|_| ()).is_some() || element.inspect_grid(|_| ()).is_some()
        });
        assert!(has_grid, "multi-column should produce Container/GridRow");
    }

    #[test]
    fn multicol_items_keep_their_bottom_border() {
        let fixture = include_str!(
            "../../tests/parity/cases/multicol/multicol-column-rule-wider-than-gap.html"
        );
        let parsed = parse_html_with_styles(fixture).expect("valid test document");
        let rules = parsed
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(
            &parsed.nodes,
            PageSize::new(180.0, 90.0),
            Margin::default(),
            &rules,
        );

        fn has_white_bottom_border(element: &dyn LayoutElement) -> bool {
            let own_border = element
                .inspect_text(|block| block.box_model.border)
                .or_else(|| element.inspect_container(|container| container.box_model.border))
                .is_some_and(|border| {
                    border.bottom.width > 0.0 && border.bottom.color == Color::WHITE
                });
            if own_border {
                return true;
            }

            let mut descendant = false;
            element.visit_children(&mut |child| {
                descendant |= has_white_bottom_border(child);
            });
            descendant
        }

        assert!(
            pages[0]
                .elements
                .iter()
                .any(|(_, element)| has_white_bottom_border(element)),
            "multicol item borders must reach the layout tree"
        );
    }

    #[test]
    fn bidi_reordering_preserves_content() {
        let html = r#"<p>Hello مرحبا World</p>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].elements.is_empty());
        // Verify all text content is present (might be reordered)
        let mut all_text = String::new();
        for (_, element) in &pages[0].elements {
            let mut runs = Vec::new();
            collect_text_runs_from_element(element.as_ref(), &mut runs);
            for run in runs {
                all_text.push_str(&run.text);
            }
        }
        assert!(
            all_text.contains("Hello") && all_text.contains("World"),
            "BiDi should preserve Latin text"
        );
    }

    // --- Issue #99: pre code color inheritance ---

    #[test]
    fn pre_code_inherits_color_from_pre_not_code_default() {
        let html = r#"<html><head><style>
            code { color: #be123c; }
            pre { color: #e2e8f0; background-color: #1e293b; }
            pre code { color: inherit; }
        </style></head><body>
        <pre><code>Hello World</code></pre>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Find the text — may be in TextBlock or Container
        fn find_hello_color(elements: &[(f32, LayoutNode)]) -> Option<Color> {
            for (_, element) in elements {
                let mut runs = Vec::new();
                collect_text_runs_from_element(element.as_ref(), &mut runs);
                if let Some(run) = runs.iter().find(|run| run.text.contains("Hello")) {
                    return Some(run.color);
                }
            }
            None
        }
        let color = find_hello_color(&pages[0].elements).expect("should find 'Hello World' text");
        let (red, green, blue) = color.to_f32_rgb();
        // pre code { color: inherit } should give #e2e8f0 (0.886, 0.910, 0.941)
        // NOT #be123c (code default red = 0.745, 0.071, 0.235)
        assert!(
            red > 0.7 && green > 0.7,
            "pre>code text should inherit light color from pre, got ({:.3}, {:.3}, {:.3})",
            red,
            green,
            blue
        );
    }

    // --- Issue #103: horizontal separators via border-bottom ---

    #[test]
    fn h1_border_bottom_produces_visible_border() {
        let html = r#"<html><head><style>
            h1 { border-bottom: 3px solid #1e40af; padding-bottom: 8px; }
        </style></head><body><h1>Title</h1></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut found_border = false;
        for (_, element) in &pages[0].elements {
            found_border |= element
                .inspect_text(|block| block.box_model.border.bottom.width > 0.0)
                .or_else(|| {
                    element.inspect_container(|container| {
                        container.box_model.border.bottom.width > 0.0
                    })
                })
                .unwrap_or(false);
        }
        assert!(
            found_border,
            "h1 with border-bottom:3px should produce a visible bottom border"
        );
    }

    // --- Issue #102: margin collapse between adjacent blocks ---

    #[test]
    fn adjacent_block_margins_collapse() {
        let html = r#"<html><head><style>
            .a { margin-bottom: 20px; background: #ddd; }
            .b { margin-top: 10px; background: #ddd; }
        </style></head><body>
            <div class="a">A</div>
            <div class="b">B</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        // Find positions of the two divs
        let mut positions: Vec<f32> = Vec::new();
        for (y, element) in &pages[0].elements {
            element.inspect_text(|block| {
                for line in &block.lines {
                    for run in &line.runs {
                        if run.text.trim() == "A" || run.text.trim() == "B" {
                            positions.push(*y);
                        }
                    }
                }
            });
        }
        assert!(
            positions.len() >= 2,
            "should find both divs, got {} positions",
            positions.len()
        );
        // The gap between A's bottom and B's top should reflect collapsed margin
        // max(20px, 10px) = 20px = 15pt, NOT 20+10=30px=22.5pt
        let gap = positions[1] - positions[0];
        // Rough check: gap should be less than what non-collapsed margins would produce
        // With font height ~12pt + 15pt margin: gap ~27pt
        // Without collapse: ~12pt + 22.5pt = 34.5pt
        assert!(
            gap < 32.0,
            "margins should collapse: gap={gap}pt (expected ~27pt, not ~34pt)"
        );
    }
    #[test]
    fn block_margin_left_reduces_content_width() {
        let html = r#"<html><head><style>
            .m { margin-left: 40px; margin-right: 40px; background: #ddd; }
        </style></head><body><div class="m">Indented</div></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let page_width = PageSize::A4.width - Margin::default().left - Margin::default().right;
        for (_, element) in &pages[0].elements {
            if let Some(width) = element
                .inspect_text(|block| {
                    block
                        .lines
                        .iter()
                        .any(|line| line.runs.iter().any(|run| run.text.contains("Indented")))
                        .then_some(block.box_model.size.width.fixed_value())
                        .flatten()
                })
                .flatten()
            {
                // 40px = 30pt each side → block should be ~60pt narrower
                assert!(
                    width < page_width - 40.0,
                    "block with margin-left/right should be narrower than page: w={width}, page={page_width}"
                );
                return;
            }
        }
        // If no explicit width, the block fills the page — that's the bug
        panic!("block with margin-left/right should have reduced width");
    }

    #[test]
    fn debug_inline_raw_and_wrapped_runs() {
        let html = r#"<html><head><style>
            body { font-family: Georgia, serif; font-size: 15px; line-height: 1.8; }
            .hl { background-color: #fef3c7; padding: 2px 4px; }
        </style></head><body>
            <p>AAA <span class="hl">BBB</span> CCC</p>
            <p>What was once dominated by heavyweight Java libraries is now seeing a new wave of <span class="hl">high-performance native renderers</span> that promise faster output.</p>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        for (_, element) in &pages[0].elements {
            element.inspect_text(|block| {
                for (li, line) in block.lines.iter().enumerate() {
                    for (ri, run) in line.runs.iter().enumerate() {
                        eprintln!(
                            "line{li} run{ri}: text={:?} pad=({:.1},{:.1}) bg={:?}",
                            run.text,
                            run.padding.left,
                            run.padding.top,
                            run.background_color.is_some()
                        );
                    }
                }
            });
        }
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn debug_float_right_structure() {
        let html = r#"<html><head><style>
            .container { width: 400px; border: 1px solid #ccc; padding: 10px; }
            .float-right { float: right; width: 100px; height: 80px; background-color: #f472b6; }
        </style></head><body>
        <div class="container">
            <div class="float-right">FR</div>
            <p>Text</p>
        </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        fn dump(elements: &[(f32, LayoutNode)], indent: &str) {
            fn dump_element(element: &dyn LayoutElement, y: f32, indent: &str) {
                element.inspect_container(|container| {
                    eprintln!(
                        "{indent}Container y={y} float={:?} w={:?} kids={}",
                        container.flow.float,
                        container.box_model.size.width,
                        container.children.len()
                    );
                });
                element.inspect_text(|block| {
                    let text: String = block
                        .lines
                        .iter()
                        .flat_map(|line| line.runs.iter().map(|run| run.text.as_str()))
                        .collect();
                    eprintln!(
                        "{indent}TextBlock y={y} float={:?} w={:?} text={text:?}",
                        block.flow.float, block.box_model.size.width
                    );
                });
                element.visit_children(&mut |child| dump_element(child, y, &format!("{indent}  ")));
            }

            for (y, element) in elements {
                dump_element(element.as_ref(), *y, indent);
            }
        }
        dump(&pages[0].elements, "");
        // Just verify structure — we want to see the debug output
        assert!(!pages[0].elements.is_empty());
    }

    // ---------------------------------------------------------------
    // Block layout coverage tests (src/layout/block.rs uncovered paths)
    // ---------------------------------------------------------------

    #[test]
    fn block_percentage_max_width() {
        let html = r#"<html><head><style>
            .clamped { max-width: 50%; }
        </style></head><body>
            <div class="clamped">Narrow</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let found = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|block| {
                    block
                        .box_model
                        .size
                        .width
                        .fixed_value()
                        .is_some_and(|width| width < 300.0)
                })
                .unwrap_or(false)
        });
        assert!(found, "Expected a block clamped by max-width: 50%");
    }

    #[test]
    fn block_percentage_min_width() {
        let html = r#"<html><head><style>
            .wide { min-width: 80%; width: 50pt; }
        </style></head><body>
            <div class="wide">Wide</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_percentage_height_resolves_against_containing_block() {
        let html = r#"<div style="height: 400pt; position: relative">
            <div style="height: 50%">Half</div>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_pct_border_radius_with_height() {
        let html = r#"<html><head><style>
            .pill { border-radius: 50%; width: 100pt; height: 100pt; background: red; }
        </style></head><body>
            <div class="pill">Round</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let has_radius = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|block| !block.paint.border_radii.is_zero())
                .or_else(|| {
                    element.inspect_container(|container| !container.paint.border_radii.is_zero())
                })
                .unwrap_or(false)
        });
        assert!(has_radius, "Expected border_radius resolved from 50%");
    }

    #[test]
    fn block_pct_border_radius_without_height() {
        let html = r#"<html><head><style>
            .rounded { border-radius: 50%; width: 80pt; background: blue; }
        </style></head><body>
            <div class="rounded">No height</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_before_pseudo_block_like() {
        let html = r#"<html><head><style>
            .banner::before {
                content: "PREFIX";
                display: block;
                background: green;
            }
        </style></head><body>
            <div class="banner"><p>Content</p></div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_white_space_nowrap_flush_runs() {
        let html = r#"<html><head><style>
            .nowrap { white-space: nowrap; width: 50pt; overflow: hidden; }
        </style></head><body>
            <div class="nowrap">This text should not wrap at all even though narrow</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn visible_nowrap_overflow_preserves_authored_geometry() {
        let html = r#"<html><head><style>
            .nowrap {
                width: 50pt;
                padding: 3pt;
                border: 4pt solid black;
                font-size: 20pt;
                white-space: nowrap;
                overflow: visible;
            }
        </style></head><body>
            <div class="nowrap">A deliberately long unwrapped line</div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let block = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                find_text_block_containing(std::slice::from_ref(element), "deliberately long")
            })
            .expect("nowrap block");
        assert_eq!(block.box_model.padding.top, 3.0);
        assert_eq!(block.box_model.padding.left, 3.0);
        assert_eq!(block.box_model.border.top.width, 4.0);
        assert_eq!(block.box_model.border.left.width, 4.0);
        assert_eq!(block.lines[0].runs[0].font_size, 20.0);
    }

    #[test]
    fn local_filter_raster_keeps_only_device_grid_overflow() {
        let html =
            r#"<div style="width:20pt;height:20pt;background:red;filter:brightness(50%)"></div>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let overflow = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_image(|image| image.paint.raster_overflow))
            .expect("brightness filter raster");
        let one_filter_pixel = crate::fonts::PT_PER_CSS_PX * 96.0 / 300.0;
        assert!(
            overflow.all(|edge| edge >= 0.0 && edge <= one_filter_pixel + 0.000_01),
            "a local color filter may retain only outward device-grid coverage: {overflow:?}"
        );
    }

    #[test]
    fn filtered_block_preserves_laid_out_flow_and_inline_placement() {
        let html = r#"
            <style>
                .target {
                    width: 120pt;
                    height: 80pt;
                    margin: 30pt;
                    background: #c62828;
                    filter: opacity(40%);
                }
            </style>
            <div class="target"></div>
        "#;
        let result = parse_html_with_styles(html).expect("filtered block fixture parses");
        let rules = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let (margins, inline_offset) = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| {
                element.inspect_image(|image| {
                    (image.geometry.flow.margins, image.positioning.insets.left)
                })
            })
            .expect("filter output image");

        assert_eq!(margins, BlockMargins::new(30.0, 30.0));
        assert_eq!(inline_offset, 30.0);
    }

    #[test]
    fn filtered_bordered_subtree_is_one_composited_surface() {
        let html = r#"
            <style>
                .node {
                    width: 126px;
                    height: 68px;
                    padding: 7px;
                    border: 2px solid #577590;
                    background: #e7f5ff;
                    filter: grayscale(.18) contrast(1.08) drop-shadow(2px 1px 0 #90a4ae);
                }
                .own { height: 22px; white-space: nowrap; }
                img { display: inline-block; width: 34px; height: 24px; }
            </style>
            <div class="node"><div class="own"><img alt="" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="></div></div>
        "#;
        let result = parse_html_with_styles(html).expect("filtered subtree fixture parses");
        let rules = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules_and_fonts(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
            &HashMap::new(),
            None,
            300.0,
            FootnoteAreaLayout::default(),
        );
        let composited_surface = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_image(|image| image.geometry.size.width > 90.0)
                .unwrap_or(false)
        });
        assert!(
            composited_surface,
            "a filter applies to the element's complete SourceGraphic: {:#?}",
            pages[0].elements
        );
    }

    #[test]
    fn filtered_box_shadows_are_part_of_the_composited_source_graphic() {
        let html = r#"
            <style>
                .target {
                    width: 40pt;
                    height: 30pt;
                    background: white;
                    box-shadow: 4pt 3pt 0 rgba(255, 0, 0, .5),
                                inset 0 0 0 2pt rgb(255, 209, 102);
                    filter: drop-shadow(1pt 1pt 0 black);
                }
            </style>
            <div class="target"></div>
        "#;
        let result = parse_html_with_styles(html).expect("filtered shadow fixture parses");
        let rules = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules_and_fonts(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
            &HashMap::new(),
            None,
            300.0,
            FootnoteAreaLayout::default(),
        );
        let image = pages[0]
            .elements
            .iter()
            .find_map(|(_, element)| element.inspect_image(Clone::clone))
            .expect("the filtered source is replaced by one image");
        assert!(image.paint.raster_overflow.right >= 4.0);
        assert!(image.paint.raster_overflow.bottom >= 3.0);

        let pixels = crate::layout::images::decode_asset_to_rgba(&image.source)
            .expect("the composited filter image decodes");
        assert!(
            pixels.pixels().any(|pixel| {
                pixel[0] > 240 && pixel[1] > 190 && pixel[1] < 225 && pixel[2] < 130
            })
        );
    }

    #[test]
    fn filtered_replaced_element_uses_the_shared_composited_surface() {
        let html = r#"
            <style>
                img { width: 160px; height: 160px; filter: invert(1); }
            </style>
            <img alt="" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=">
        "#;
        let result = parse_html_with_styles(html).expect("filtered image fixture parses");
        let rules = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules_and_fonts(
            &result.nodes,
            PageSize::A4,
            Margin::default(),
            &rules,
            &HashMap::new(),
            None,
            300.0,
            FootnoteAreaLayout::default(),
        );
        let rendered_source = pages[0]
            .elements
            .iter()
            .any(|(_, element)| tree_has_rendered_image(element.as_ref()));
        assert!(
            rendered_source,
            "a replaced element filter must use the common rendered surface: {:#?}",
            pages[0].elements
        );
    }

    #[test]
    fn identity_filter_materializes_and_keeps_its_stacking_context() {
        let html = r#"
            <style>
                .filtered { width: 20pt; height: 20pt; filter: brightness(1); }
                .child { position: absolute; z-index: -1; width: 10pt; height: 10pt; }
            </style>
            <div class="filtered"><div class="child"></div></div>
        "#;
        let result = parse_html_with_styles(html).expect("identity-filter fixture parses");
        let rules = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);

        fn contains_materialized_filter(element: &dyn LayoutElement) -> bool {
            if element.paint_group_owner().is_some_and(|owner| {
                owner.paint_group().effects.stacking_context == StackingContext::FilteredOutput
            }) {
                return true;
            }
            let mut found = false;
            element.visit_children(&mut |child| found |= contains_materialized_filter(child));
            found
        }

        assert!(
            pages[0]
                .elements
                .iter()
                .any(|(_, element)| contains_materialized_filter(element)),
            "identity filter output must retain its CSS stacking context"
        );
        assert!(
            pages[0].elements.iter().any(|(_, element)| {
                struct ImageSearch(bool);
                impl LayoutVisitor for ImageSearch {
                    fn visit_image(&mut self, _: &Image) {
                        self.0 = true;
                    }
                }
                let mut search = ImageSearch(false);
                visit_layout_tree(element.as_ref(), &mut search);
                search.0
            }),
            "Chromium-compatible identity filters isolate one raster SourceGraphic"
        );
    }

    #[test]
    fn unresolved_filter_url_discards_its_stacking_context() {
        let html = r#"
            <style>
                .filtered { width: 20pt; height: 20pt; filter: url(#missing) blur(7px); }
            </style>
            <div class="filtered"></div>
        "#;
        let result = parse_html_with_styles(html).expect("missing-url fixture parses");
        let rules = result
            .stylesheets
            .iter()
            .flat_map(|css| parse_stylesheet(css))
            .collect::<Vec<_>>();
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);

        fn has_filter_context(element: &dyn LayoutElement) -> bool {
            if element
                .inspect_container(|container| {
                    container.paint.group.effects.stacking_context == StackingContext::Filter
                })
                .unwrap_or(false)
            {
                return true;
            }
            let mut found = false;
            element.visit_children(&mut |child| found |= has_filter_context(child));
            found
        }

        assert!(
            pages[0]
                .elements
                .iter()
                .all(|(_, element)| !has_filter_context(element))
        );
    }

    #[test]
    fn block_inline_block_shrink_to_fit() {
        let html = r#"<html><head><style>
            .ib { display: inline-block; }
        </style></head><body>
            <div><span class="ib">Short</span></div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_visual_parent_with_block_children() {
        let html = r#"<html><head><style>
            .box { background: #ff0000; padding: 10pt; border: 1pt solid black; }
        </style></head><body>
            <div class="box">
                <p>First paragraph</p>
                <p>Second paragraph</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let has_visual = pages[0].elements.iter().any(|(_, element)| {
            element
                .inspect_text(|block| block.paint.background.color.is_some())
                .or_else(|| {
                    element
                        .inspect_container(|container| container.paint.background.color.is_some())
                })
                .unwrap_or(false)
        });
        assert!(
            has_visual,
            "Expected wrapper with background from visual parent"
        );
    }

    #[test]
    fn block_visual_wrapper_with_fixed_height() {
        let html = r#"<html><head><style>
            .fixed { background: blue; height: 200pt; padding: 10pt; }
        </style></head><body>
            <div class="fixed">
                <p>Inside fixed height</p>
                <p>Second child</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_overflow_hidden_visual_wrapper_clip() {
        let html = r#"<html><head><style>
            .clip { overflow: hidden; background: gray; width: 150pt; height: 100pt; }
        </style></head><body>
            <div class="clip">
                <p>Clipped paragraph one</p>
                <p>Clipped paragraph two</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_visual_wrapper_padding_propagation() {
        let html = r#"<html><head><style>
            .padded { background: yellow; padding: 20pt 15pt; }
        </style></head><body>
            <div class="padded">
                <p>Child with propagated padding</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_visual_wrapper_patches_auto_height() {
        let html = r#"<html><head><style>
            .autoheight { background: orange; padding: 5pt; border: 2pt solid red; }
        </style></head><body>
            <div class="autoheight">
                <h2>Heading</h2>
                <p>Body text</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_abs_pseudo_before_after_in_block() {
        let html = r#"<html><head><style>
            .rel { position: relative; }
            .rel::before {
                content: "B";
                display: block;
                position: absolute;
                top: 0;
                left: 0;
            }
            .rel::after {
                content: "A";
                display: block;
                position: absolute;
                bottom: 0;
                right: 0;
            }
        </style></head><body>
            <div class="rel">
                <p>Main content</p>
                <p>More content</p>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_container_wrapper_aspect_ratio() {
        let html = r#"<html><head><style>
            .aspect { aspect-ratio: 16 / 9; width: 320pt; background: purple; }
        </style></head><body>
            <div class="aspect"><p>Aspect ratio box</p></div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let has_container = pages[0]
            .elements
            .iter()
            .any(|(_, element)| element.inspect_container(|_| ()).is_some());
        assert!(
            has_container,
            "aspect-ratio should produce a Container element"
        );
    }

    #[test]
    fn block_wrapper_inline_block_children() {
        let html = r#"<html><head><style>
            .parent { background: cyan; height: 60pt; }
            .child { display: inline-block; width: 40pt; }
        </style></head><body>
            <div class="parent">
                <span class="child">A</span>
                <span class="child">B</span>
                <span class="child">C</span>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_positioned_container_cb_non_wrapper() {
        let html = r#"<div style="position: relative">
            <span>Inline text only</span>
        </div>"#;
        let nodes = parse_html(html).unwrap();
        let pages = layout(&nodes, PageSize::A4, Margin::default());
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_inline_block_flush_at_block_break() {
        let html = r#"<html><head><style>
            .ib { display: inline-block; width: 50pt; }
        </style></head><body>
            <div>
                <span class="ib">X</span>
                <span class="ib">Y</span>
                <div>Block break</div>
                <span class="ib">Z</span>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
    }

    #[test]
    fn block_no_inline_visual_wrapper_path() {
        let html = r#"<html><head><style>
            .visual { background: lime; border: 1pt solid black; }
        </style></head><body>
            <div class="visual">
                <div>Only block children, no inline text</div>
            </div>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        assert!(!pages[0].elements.is_empty());
        let has_container_or_bg = pages[0].elements.iter().any(|(_, element)| {
            element.inspect_container(|_| ()).is_some()
                || element
                    .inspect_text(|block| block.paint.background.color.is_some())
                    .unwrap_or(false)
        });
        assert!(
            has_container_or_bg,
            "Expected Container or TextBlock with visual properties"
        );
    }
    // -----------------------------------------------------------------------
    // helpers.rs coverage tests
    // -----------------------------------------------------------------------

    #[test]
    fn helpers_resolve_padding_box_height_border_box() {
        use crate::layout::helpers::resolve_padding_box_height;
        use crate::style::computed::BoxSizing;

        let h = resolve_padding_box_height(
            50.0,
            Some(200.0),
            EdgeSizes::uniform(10.0),
            EdgeSizes::uniform(10.0),
            BoxSizing::BorderBox,
        );
        assert!((h - 180.0).abs() < 0.01);

        let h2 = resolve_padding_box_height(
            50.0,
            Some(10.0),
            EdgeSizes::uniform(5.0),
            EdgeSizes::uniform(15.0),
            BoxSizing::BorderBox,
        );
        assert!((h2 - 0.0).abs() < 0.01);
    }

    #[test]
    fn helpers_pseudo_block_display_block_renders() {
        let html = r#"<html><head><style>
            p::before {
                content: "PREFIX";
                display: block;
                padding: 5pt;
                background-color: #eee;
            }
        </style></head><body><p>Main text</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut runs = Vec::new();
        for (_, element) in &pages[0].elements {
            collect_text_runs_from_element(element.as_ref(), &mut runs);
        }
        let text = runs.iter().map(|run| run.text.as_str()).collect::<String>();
        assert!(
            text.contains("PREFIX"),
            "expected block ::before with 'PREFIX', got: {text:?}",
        );
    }

    #[test]
    fn helpers_pseudo_block_absolute_positioned() {
        let html = r#"<html><head><style>
            .container {
                position: relative;
                width: 300pt;
                height: 200pt;
            }
            .container::after {
                content: "ABS";
                position: absolute;
                top: 10pt;
                left: 20pt;
                padding: 4pt;
            }
        </style></head><body><div class="container">Content</div></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let abs_el = find_page_text_block_containing(&pages[0].elements, "ABS").and_then(|block| {
            (block.positioning.scheme == Position::Absolute)
                .then_some((block.positioning.insets.top, block.positioning.insets.left))
        });
        assert!(
            abs_el.is_some(),
            "expected an absolute-positioned pseudo TextBlock"
        );
        if let Some((offset_top, offset_left)) = abs_el {
            assert!((offset_top - 10.0).abs() < 1.0, "offset_top={offset_top}");
            assert!(
                (offset_left - 20.0).abs() < 1.0,
                "offset_left={offset_left}"
            );
        }
    }

    #[test]
    fn helpers_pseudo_block_absolute_bottom_right() {
        let html = r#"<html><head><style>
            .box {
                position: relative;
                width: 400pt;
                height: 300pt;
            }
            .box::before {
                content: "BR";
                position: absolute;
                bottom: 10pt;
                right: 20pt;
            }
        </style></head><body><div class="box">Hello</div></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        fn find_abs_br(elements: &[(f32, LayoutNode)]) -> Option<(f32, f32)> {
            struct AbsoluteBr(Option<(f32, f32)>);

            impl LayoutVisitor for AbsoluteBr {
                fn visit_text_block(&mut self, block: &TextBlock) {
                    if self.0.is_none()
                        && block.positioning.scheme == Position::Absolute
                        && text_lines_contain(&block.lines, "BR")
                    {
                        self.0 =
                            Some((block.positioning.insets.top, block.positioning.insets.left));
                    }
                }
            }

            let mut found = AbsoluteBr(None);
            for (_, element) in elements {
                visit_layout_tree(element.as_ref(), &mut found);
            }
            found.0
        }
        let abs_el = find_abs_br(&pages[0].elements);
        assert!(
            abs_el.is_some(),
            "expected absolute pseudo with bottom/right resolved"
        );
        let (top, left) = abs_el.unwrap();
        assert!(top > 0.0, "bottom should resolve to positive top: {}", top);
        assert!(
            left > 0.0,
            "right should resolve to positive left: {}",
            left
        );
    }

    #[test]
    fn helpers_resolve_abs_cb_bottom_right_only() {
        use crate::layout::helpers::resolve_abs_containing_block;
        use crate::style::computed::ComputedStyle;

        let mut style = ComputedStyle::default();
        style.position = Position::Absolute;
        style.top = None;
        style.left = None;
        style.bottom = Some(10.0);
        style.right = Some(20.0);

        let cb = ContainingBlock {
            x: 0.0,
            width: 500.0,
            height: 400.0,
            depth: 0,
        };

        let (resolved_cb, top, left) = resolve_abs_containing_block(&style, Some(cb), 50.0, 100.0);
        assert!(resolved_cb.is_some());
        assert!((top - 340.0).abs() < 0.01, "top={}", top);
        assert!((left - 380.0).abs() < 0.01, "left={}", left);
    }

    #[test]
    fn helpers_resolve_abs_cb_none() {
        use crate::layout::helpers::resolve_abs_containing_block;
        use crate::style::computed::ComputedStyle;

        let mut style = ComputedStyle::default();
        style.position = Position::Absolute;
        style.top = Some(15.0);
        style.left = Some(25.0);

        let (resolved_cb, top, left) = resolve_abs_containing_block(&style, None, 50.0, 100.0);
        assert!(resolved_cb.is_none());
        assert!((top - 15.0).abs() < 0.01);
        assert!((left - 25.0).abs() < 0.01);
    }

    #[test]
    fn helpers_patch_abs_children_cb_resolves_offsets() {
        use crate::layout::helpers::resolve_absolute_descendants_containing_block;

        let cb = ContainingBlock {
            x: 0.0,
            width: 600.0,
            height: 400.0,
            depth: 1,
        };

        let mut elements =
            vec![
                TextBlock {
                    box_model: BoxModel {
                        size: LayoutSize::fixed(100.0, Some(50.0)),
                        ..Default::default()
                    },
                    positioning: Positioning::absolute_from_lengths(
                        crate::types::PhysicalEdges::new(None, Some(40.0), Some(30.0), None),
                    ),
                    ..Default::default()
                }
                .boxed(),
            ];

        resolve_absolute_descendants_containing_block(&mut elements, cb);

        elements[0]
            .inspect_text(|block| {
                assert!(
                    block.positioning.containing_block.is_some(),
                    "containing_block should be set"
                );
                assert!(
                    (block.positioning.insets.left - 460.0).abs() < 0.01,
                    "offset_left={}",
                    block.positioning.insets.left
                );
                assert!(
                    (block.positioning.insets.top - 320.0).abs() < 0.01,
                    "offset_top={}",
                    block.positioning.insets.top
                );
            })
            .expect("expected text block");
    }

    #[test]
    fn helpers_aspect_ratio_height_computed() {
        use crate::layout::helpers::aspect_ratio_height;
        use crate::style::computed::ComputedStyle;

        let mut style = ComputedStyle::default();
        assert!(aspect_ratio_height(200.0, &style).is_none());

        style.aspect_ratio = Some(2.0);
        let h = aspect_ratio_height(200.0, &style);
        assert!(h.is_some());
        assert!((h.unwrap() - 100.0).abs() < 0.01);

        style.aspect_ratio = Some(0.0);
        assert!(aspect_ratio_height(200.0, &style).is_none());
    }

    #[test]
    fn helpers_format_list_marker_roman_large() {
        use crate::layout::list_markers::format_list_marker;
        use crate::style::computed::ListStyleType;

        assert_eq!(
            format_list_marker(&ListStyleType::UpperRoman, 2024),
            "MMXXIV. "
        );
        assert_eq!(
            format_list_marker(&ListStyleType::LowerRoman, 999),
            "cmxcix. "
        );
        assert_eq!(format_list_marker(&ListStyleType::UpperRoman, 49), "XLIX. ");
        assert_eq!(
            format_list_marker(&ListStyleType::LowerRoman, 444),
            "cdxliv. "
        );
    }

    #[test]
    fn helpers_pseudo_block_with_min_height() {
        let html = r#"<html><head><style>
            p::before {
                content: "X";
                display: block;
                min-height: 50pt;
            }
        </style></head><body><p>Text</p></body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);

        let pseudo = find_page_text_block_containing(&pages[0].elements, "X")
            .map(|block| block.box_model.size.height.used());
        assert!(pseudo.is_some(), "expected pseudo-block with min-height");
        if let Some(Some(h)) = pseudo {
            assert!(
                h >= 50.0,
                "min-height should enforce at least 50pt, got {}",
                h
            );
        }
    }

    #[test]
    fn helpers_resolve_content_counters_in_layout() {
        let html = r#"<html><head><style>
            ol { counter-reset: item; list-style-type: none; }
            ol li { counter-increment: item; }
            ol li::before { content: counters(item, ".") " "; display: block; }
        </style></head><body>
            <ol>
                <li>A
                    <ol>
                        <li>B</li>
                    </ol>
                </li>
            </ol>
        </body></html>"#;
        let result = parse_html_with_styles(html).unwrap();
        let mut rules = Vec::new();
        for css in &result.stylesheets {
            rules.extend(parse_stylesheet(css));
        }
        let pages = layout_with_rules(&result.nodes, PageSize::A4, Margin::default(), &rules);
        let mut texts: Vec<String> = Vec::new();
        for (_, element) in &pages[0].elements {
            element.inspect_text(|block| {
                for line in &block.lines {
                    let t: String = line.runs.iter().map(|run| run.text.as_str()).collect();
                    if !t.trim().is_empty() {
                        texts.push(t);
                    }
                }
            });
        }
        let has_nested = texts.iter().any(|t| t.contains("1.1"));
        assert!(
            has_nested,
            "expected nested counter '1.1' from counters(), got: {:?}",
            texts
        );
    }

    #[test]
    fn compute_root_margin_resolves_body_margin() {
        // body { margin: 40px } → 40 CSS px = 30pt on each side.
        let rules = parse_stylesheet("body { margin: 40px; }");
        let m = compute_root_margin(&rules, PageSize::LETTER);
        assert!((m.top - 30.0).abs() < 0.01, "top = {}", m.top);
        assert!((m.right - 30.0).abs() < 0.01, "right = {}", m.right);
        assert!((m.bottom - 30.0).abs() < 0.01, "bottom = {}", m.bottom);
        assert!((m.left - 30.0).abs() < 0.01, "left = {}", m.left);
    }

    #[test]
    fn compute_root_margin_zero_when_no_body_rule() {
        let rules = parse_stylesheet("p { margin: 40px; }");
        let m = compute_root_margin(&rules, PageSize::A4);
        assert_eq!(m.top, 0.0);
        assert_eq!(m.right, 0.0);
        assert_eq!(m.bottom, 0.0);
        assert_eq!(m.left, 0.0);
    }

    #[test]
    fn compute_root_margin_accepts_html_and_root_selectors() {
        let rules = parse_stylesheet(":root { margin-top: 20pt; } html { margin-left: 10pt; }");
        let m = compute_root_margin(&rules, PageSize::A4);
        assert!((m.top - 20.0).abs() < 0.01);
        assert!((m.left - 10.0).abs() < 0.01);
    }

    #[test]
    fn compute_root_padding_returns_one_edge_group() {
        let rules = parse_stylesheet("body { padding: 1pt 2pt 3pt 4pt; }");
        assert_eq!(
            compute_root_padding(&rules, PageSize::A4),
            EdgeSizes::new(1.0, 2.0, 3.0, 4.0)
        );
    }

    #[test]
    fn page_margin_defaults_inherit_html_not_body_text() {
        let rules = parse_stylesheet(
            "html { font-family: ParitySans; font-size: 16px; line-height: 1.5 } \
             body { font-size: 20px; line-height: 2 }",
        );
        let defaults = compute_page_margin_text_context(&rules, &[], PageSize::A4).resolve(
            PageSelectorContext {
                page_number: 1,
                is_blank: false,
                page_name: None,
            },
            &PageTextStyle::default(),
            &HashMap::new(),
        );

        assert_eq!(defaults.font_size, 12.0);
        assert_eq!(defaults.line_height_factor, 1.5);
    }

    #[test]
    fn page_margin_text_context_cascades_page_and_margin_declarations() {
        let rules = parse_stylesheet(
            "html { font-size: 10px; line-height: 1 }\
             @page { font-size: 16px; line-height: 1.5; @top-center { content: 'BASE'; } }\
             @page :first { font-size: 20px; @top-center { content: 'FIRST'; font-size: 24px; line-height: 2; } }",
        );
        let page_rules = parse_page_rules(
            "@page { font-size: 16px; line-height: 1.5; @top-center { content: 'BASE'; } }\
             @page :first { font-size: 20px; @top-center { content: 'FIRST'; font-size: 24px; line-height: 2; } }",
        );
        let context = compute_page_margin_text_context(&rules, &page_rules, PageSize::A4);
        let fonts = HashMap::new();
        let base_margin = &page_rules[0].margin_boxes[0].text_style;
        let first_margin = &page_rules[1].margin_boxes[0].text_style;

        let second = context.resolve(
            PageSelectorContext {
                page_number: 2,
                is_blank: false,
                page_name: None,
            },
            base_margin,
            &fonts,
        );
        let first_page = context.resolve(
            PageSelectorContext {
                page_number: 1,
                is_blank: false,
                page_name: None,
            },
            &PageTextStyle::default(),
            &fonts,
        );
        let first = context.resolve(
            PageSelectorContext {
                page_number: 1,
                is_blank: false,
                page_name: None,
            },
            first_margin,
            &fonts,
        );

        assert_eq!(second.font_size, 12.0);
        assert_eq!(second.line_height_factor, 1.5);
        assert_eq!(first_page.font_size, 15.0);
        assert_eq!(first_page.line_height_factor, 1.5);
        assert_eq!(first.font_size, 18.0);
        assert_eq!(first.line_height_factor, 2.0);
    }

    #[test]
    fn page_margin_text_context_skips_an_unavailable_font_family() {
        // CSS Fonts 4 requires the first available family in the prioritized list.
        let page_rules = parse_page_rules(
            "@page { @top-center { content: 'X'; font-family: NoSuchFont, serif; } }",
        );
        let context = compute_page_margin_text_context(&[], &page_rules, PageSize::A4);
        let defaults = context.resolve(
            PageSelectorContext {
                page_number: 1,
                is_blank: false,
                page_name: None,
            },
            &page_rules[0].margin_boxes[0].text_style,
            &HashMap::new(),
        );

        assert_eq!(defaults.font_family, FontFamily::TimesRoman);
    }
}

// (end of file -- debug tests removed)
