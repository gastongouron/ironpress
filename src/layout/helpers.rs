use crate::layout::elements::LayoutNode;
use crate::layout::flow_metrics::BlockMargins;
use crate::parser::css::{AncestorInfo, CssRule, CssValue, PseudoElement, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::{GlyphSideBearings, TtfFont};
use crate::style::computed::{
    BackgroundClip, BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize,
    BoxSizing, ComputedStyle, ConicGradient, ContentItem, Display, FontFamily, FontWeight,
    IntrinsicWidthKeyword, LEADER_PLACEHOLDER_END, LEADER_PLACEHOLDER_START, LinearGradient,
    RadialGradient, VerticalAlign, Visibility, compute_style_with_context,
};
use crate::types::EdgeSizes;
use std::collections::HashMap;

pub(crate) fn selector_attributes_with_has(el: &ElementNode) -> HashMap<String, String> {
    let mut attrs = el.attributes.clone();
    let child_classes = el
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(child_el) => Some(child_el.class_list().join(" ")),
            DomNode::Text(_) => None,
        })
        .filter(|classes| !classes.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !child_classes.is_empty() {
        attrs.insert("__ironpress_has_child_classes".to_string(), child_classes);
    }

    let mut descendant = Vec::new();
    collect_descendant_classes(el, &mut descendant);
    if !descendant.is_empty() {
        attrs.insert(
            "__ironpress_has_descendant_classes".to_string(),
            descendant.join(" "),
        );
    }
    attrs
}

fn collect_descendant_classes(el: &ElementNode, out: &mut Vec<String>) {
    for child in &el.children {
        if let DomNode::Element(child_el) = child {
            out.extend(child_el.class_list().into_iter().map(str::to_string));
            collect_descendant_classes(child_el, out);
        }
    }
}

pub(crate) fn authored_keyword_property(
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
    property: &str,
) -> Option<String> {
    match authored_property_value(el, rules, ancestors, selector_ctx, property)? {
        CssValue::Keyword(value) => Some(value.to_ascii_lowercase()),
        _ => None,
    }
}

pub(crate) fn authored_pseudo_keyword_property(
    el: &ElementNode,
    rules: &[CssRule],
    selector_ctx: &SelectorContext,
    pseudo: PseudoElement,
    property: &str,
) -> Option<String> {
    let mut winner: Option<(bool, u8, u32, usize, CssValue)> = None;
    let classes = el.class_list();
    let class_refs: Vec<&str> = classes.iter().map(|s| s.as_ref()).collect();
    let selector_attrs = selector_attributes_with_has(el);

    for (source_idx, rule) in rules.iter().enumerate() {
        if rule.pseudo_element != Some(pseudo) {
            continue;
        }
        if !crate::parser::css::selector_matches_with_context(
            &rule.selector,
            el.tag_name(),
            &class_refs,
            el.id(),
            &selector_attrs,
            selector_ctx,
        ) {
            continue;
        }
        let Some(value) = rule.declarations.get(property) else {
            continue;
        };
        let candidate = (
            rule.declarations.is_important(property),
            0,
            crate::parser::css::specificity(&rule.selector),
            source_idx,
            value.clone(),
        );
        if winner
            .as_ref()
            .is_none_or(|current| candidate_key(&candidate) >= candidate_key(current))
        {
            winner = Some(candidate);
        }
    }

    match winner.map(|(_, _, _, _, value)| value)? {
        CssValue::Keyword(value) => Some(value.to_ascii_lowercase()),
        _ => None,
    }
}

pub(crate) fn authored_property_value(
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
    property: &str,
) -> Option<CssValue> {
    let mut winner: Option<(bool, u8, u32, usize, CssValue)> = None;
    let classes = el.class_list();
    let class_refs: Vec<&str> = classes.iter().map(|s| s.as_ref()).collect();
    let selector_attrs = selector_attributes_with_has(el);

    for (source_idx, rule) in rules.iter().enumerate() {
        if rule.pseudo_element.is_some() {
            continue;
        }
        if !crate::parser::css::selector_matches_with_context(
            &rule.selector,
            el.tag_name(),
            &class_refs,
            el.id(),
            &selector_attrs,
            selector_ctx,
        ) {
            continue;
        }
        let Some(value) = rule.declarations.get(property) else {
            continue;
        };
        let candidate = (
            rule.declarations.is_important(property),
            0,
            crate::parser::css::specificity(&rule.selector),
            source_idx,
            value.clone(),
        );
        if winner
            .as_ref()
            .is_none_or(|current| candidate_key(&candidate) >= candidate_key(current))
        {
            winner = Some(candidate);
        }
    }

    if let Some(style_attr) = el.style_attr() {
        let inline = crate::parser::css::parse_inline_style(style_attr);
        if let Some(value) = inline.get(property) {
            let candidate = (
                inline.is_important(property),
                1,
                u32::MAX,
                usize::MAX,
                value.clone(),
            );
            if winner
                .as_ref()
                .is_none_or(|current| candidate_key(&candidate) >= candidate_key(current))
            {
                winner = Some(candidate);
            }
        }
    }

    let _ = ancestors;
    winner.map(|(_, _, _, _, value)| value)
}

fn candidate_key<T>(candidate: &(bool, u8, u32, usize, T)) -> (bool, u8, u32, usize) {
    (candidate.0, candidate.1, candidate.2, candidate.3)
}

pub(crate) fn selector_context_from_ancestors<'a>(
    ancestors: &[AncestorInfo<'a>],
    el: &'a ElementNode,
) -> SelectorContext<'a> {
    if let Some(current) = ancestors
        .last()
        .filter(|info| std::ptr::eq(info.element, el))
    {
        SelectorContext {
            ancestors: ancestors[..ancestors.len().saturating_sub(1)].to_vec(),
            child_index: current.child_index,
            sibling_count: current.sibling_count,
            preceding_siblings: current.preceding_siblings.clone(),
            following_siblings: current.following_siblings.clone(),
            is_empty: current.is_empty,
        }
    } else {
        SelectorContext {
            ancestors: ancestors.to_vec(),
            ..SelectorContext::default()
        }
    }
}

pub(crate) fn authored_intrinsic_width_keyword(
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    property: &str,
) -> Option<IntrinsicWidthKeyword> {
    let selector_ctx = selector_context_from_ancestors(ancestors, el);
    match authored_keyword_property(el, rules, ancestors, &selector_ctx, property)?.as_str() {
        "min-content" => Some(IntrinsicWidthKeyword::MinContent),
        "max-content" => Some(IntrinsicWidthKeyword::MaxContent),
        "fit-content" => Some(IntrinsicWidthKeyword::FitContent),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutOverflowKeyword {
    Visible,
    Clip,
    Hidden,
    Scroll,
    Auto,
}

impl LayoutOverflowKeyword {
    pub(crate) fn clips(self) -> bool {
        !matches!(self, Self::Visible)
    }

    pub(crate) fn establishes_bfc(self) -> bool {
        matches!(self, Self::Hidden | Self::Scroll | Self::Auto)
    }

    fn is_scrollable(self) -> bool {
        matches!(self, Self::Hidden | Self::Scroll | Self::Auto)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LayoutOverflowAxes {
    pub(crate) x: LayoutOverflowKeyword,
    pub(crate) y: LayoutOverflowKeyword,
}

impl LayoutOverflowAxes {
    pub(crate) fn clips_any(self) -> bool {
        self.x.clips() || self.y.clips()
    }

    pub(crate) fn establishes_bfc(self) -> bool {
        self.x.establishes_bfc() || self.y.establishes_bfc()
    }
}

fn parse_layout_overflow_keyword(value: &str) -> LayoutOverflowKeyword {
    match value.trim().to_ascii_lowercase().as_str() {
        "clip" => LayoutOverflowKeyword::Clip,
        "hidden" => LayoutOverflowKeyword::Hidden,
        "scroll" => LayoutOverflowKeyword::Scroll,
        "auto" => LayoutOverflowKeyword::Auto,
        _ => LayoutOverflowKeyword::Visible,
    }
}

fn coerce_layout_overflow_axis(
    this: LayoutOverflowKeyword,
    other: LayoutOverflowKeyword,
) -> LayoutOverflowKeyword {
    match this {
        LayoutOverflowKeyword::Visible if other.is_scrollable() => LayoutOverflowKeyword::Auto,
        LayoutOverflowKeyword::Clip if other.is_scrollable() => LayoutOverflowKeyword::Hidden,
        value => value,
    }
}

pub(crate) fn authored_overflow_axes(
    el: &ElementNode,
    style: &ComputedStyle,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
) -> LayoutOverflowAxes {
    let from_computed = |value| match value {
        crate::style::computed::Overflow::Visible => LayoutOverflowKeyword::Visible,
        crate::style::computed::Overflow::Hidden => LayoutOverflowKeyword::Hidden,
        crate::style::computed::Overflow::Scroll => LayoutOverflowKeyword::Scroll,
        crate::style::computed::Overflow::Auto => LayoutOverflowKeyword::Auto,
    };
    let mut x = from_computed(style.overflow_x);
    let mut y = from_computed(style.overflow_y);
    let mut saw_authored = false;

    if let Some(value) = authored_keyword_property(el, rules, ancestors, selector_ctx, "overflow") {
        let mut parts = value.split_whitespace();
        if let Some(first) = parts.next() {
            x = parse_layout_overflow_keyword(first);
            y = parts.next().map(parse_layout_overflow_keyword).unwrap_or(x);
            saw_authored = true;
        }
    }
    if let Some(value) = authored_keyword_property(el, rules, ancestors, selector_ctx, "overflow-x")
    {
        x = parse_layout_overflow_keyword(&value);
        saw_authored = true;
    }
    if let Some(value) = authored_keyword_property(el, rules, ancestors, selector_ctx, "overflow-y")
    {
        y = parse_layout_overflow_keyword(&value);
        saw_authored = true;
    }
    // Horizontal writing-mode is the only mode currently laid out by the block
    // engine, so logical inline/block overflow map to x/y here.
    if let Some(value) =
        authored_keyword_property(el, rules, ancestors, selector_ctx, "overflow-inline")
    {
        x = parse_layout_overflow_keyword(&value);
        saw_authored = true;
    }
    if let Some(value) =
        authored_keyword_property(el, rules, ancestors, selector_ctx, "overflow-block")
    {
        y = parse_layout_overflow_keyword(&value);
        saw_authored = true;
    }

    if saw_authored {
        LayoutOverflowAxes {
            x: coerce_layout_overflow_axis(x, y),
            y: coerce_layout_overflow_axis(y, x),
        }
    } else {
        LayoutOverflowAxes { x, y }
    }
}

pub(crate) fn authored_line_clamp(
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
) -> Option<usize> {
    for property in ["line-clamp", "-webkit-line-clamp"] {
        let Some(value) = authored_property_value(el, rules, ancestors, selector_ctx, property)
        else {
            continue;
        };
        let n = match value {
            CssValue::Number(n) => n,
            CssValue::Keyword(raw) => raw
                .split_whitespace()
                .find_map(|part| part.parse::<f32>().ok())?,
            _ => continue,
        };
        if n.is_finite() && n >= 1.0 {
            return Some(n.floor() as usize);
        }
    }
    None
}

pub(crate) fn authored_overflow_clip_margin(
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
) -> f32 {
    match authored_property_value(el, rules, ancestors, selector_ctx, "overflow-clip-margin") {
        Some(CssValue::Length(v)) if v.is_finite() => v.max(0.0),
        Some(CssValue::Number(v)) if v.is_finite() => v.max(0.0),
        _ => 0.0,
    }
}

pub(crate) fn authored_scrollbar_gutter(
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
) -> (f32, f32) {
    let Some(value) =
        authored_keyword_property(el, rules, ancestors, selector_ctx, "scrollbar-gutter")
    else {
        return (0.0, 0.0);
    };
    if !value.split_whitespace().any(|part| part == "stable") {
        return (0.0, 0.0);
    }
    let gutter = 15.0 * 0.75;
    if value.split_whitespace().any(|part| part == "both-edges") {
        (gutter, gutter)
    } else {
        (0.0, gutter)
    }
}

pub(crate) fn authored_display_contents(
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
) -> bool {
    authored_keyword_property(el, rules, ancestors, selector_ctx, "display")
        .is_some_and(|value| value == "contents")
}

pub(crate) fn apply_authored_insets(
    style: &mut ComputedStyle,
    el: &ElementNode,
    rules: &[CssRule],
    ancestors: &[AncestorInfo],
    selector_ctx: &SelectorContext,
) {
    if let Some(value) = authored_property_value(el, rules, ancestors, selector_ctx, "inset") {
        let values = match value {
            CssValue::Keyword(raw) => raw
                .split_whitespace()
                .filter_map(parse_inset_component)
                .collect::<Vec<_>>(),
            CssValue::Length(_) | CssValue::Percentage(_) => vec![value],
            _ => Vec::new(),
        };
        if !values.is_empty() {
            let top = values[0].clone();
            let right = values.get(1).cloned().unwrap_or_else(|| top.clone());
            let bottom = values.get(2).cloned().unwrap_or_else(|| top.clone());
            let left = values.get(3).cloned().unwrap_or_else(|| right.clone());
            set_inset_side(style, "top", &top);
            set_inset_side(style, "right", &right);
            set_inset_side(style, "bottom", &bottom);
            set_inset_side(style, "left", &left);
        }
    }

    for (property, side) in [
        ("top", "top"),
        ("right", "right"),
        ("bottom", "bottom"),
        ("left", "left"),
        ("inset-block-start", "top"),
        ("inset-inline-end", "right"),
        ("inset-block-end", "bottom"),
        ("inset-inline-start", "left"),
    ] {
        if let Some(value) = authored_property_value(el, rules, ancestors, selector_ctx, property) {
            set_inset_side(style, side, &value);
        }
    }
}

fn parse_inset_component(raw: &str) -> Option<CssValue> {
    crate::parser::css::parse_length(raw)
}

fn set_inset_side(style: &mut ComputedStyle, side: &str, value: &CssValue) {
    match (side, value) {
        ("top", CssValue::Length(v)) => {
            style.top = Some(*v);
            style.percentage_insets.top = None;
        }
        ("right", CssValue::Length(v)) => {
            style.right = Some(*v);
            style.percentage_insets.right = None;
        }
        ("bottom", CssValue::Length(v)) => {
            style.bottom = Some(*v);
            style.percentage_insets.bottom = None;
        }
        ("left", CssValue::Length(v)) => {
            style.left = Some(*v);
            style.percentage_insets.left = None;
        }
        ("top", CssValue::Percentage(v)) => {
            style.top = None;
            style.percentage_insets.top = Some(*v);
        }
        ("right", CssValue::Percentage(v)) => {
            style.right = None;
            style.percentage_insets.right = Some(*v);
        }
        ("bottom", CssValue::Percentage(v)) => {
            style.bottom = None;
            style.percentage_insets.bottom = Some(*v);
        }
        ("left", CssValue::Percentage(v)) => {
            style.left = None;
            style.percentage_insets.left = Some(*v);
        }
        _ => {}
    }
}

use super::context::ContainingBlock;
use super::elements::{
    BackgroundPaint, BlockFlow, BlockSize, BoxModel, BoxPaint, InlineSize, IntoLayoutNode,
    LayoutSize, OutlinePaint, Positioning, SizeConstraints, TextBlock, TextBlockStyle,
    TextFragmentation, TextSemantics,
};
use super::engine::{
    CounterState, InlineBox, InlineBoxPaint, LayoutBorder, LayoutElement, TextLine, TextRun,
};
use super::images::build_raster_background_tree;
use super::list_markers::{build_list_bullet_marker, format_counter_value, format_list_marker};
use super::text::{
    TextWrapOptions, collapse_whitespace, estimate_word_width, parent_line_strut,
    push_text_run_with_fallback, resolve_style_font_family, text_run_line_height_factor,
    used_font_size, wrap_text_runs,
};

// ---------------------------------------------------------------------------
// Group 4 — Box sizing
// ---------------------------------------------------------------------------

pub(crate) fn resolve_padding_box_height(
    content_height: f32,
    specified_height: Option<f32>,
    padding: EdgeSizes,
    border: EdgeSizes,
    box_sizing: BoxSizing,
) -> f32 {
    let content_based_height = content_height + padding.vertical();
    match specified_height {
        Some(height) => {
            // When height is explicitly set, use it (don't expand to fit content).
            // This is essential for overflow: hidden to clip correctly.
            match box_sizing {
                BoxSizing::BorderBox => (height - border.vertical()).max(0.0),
                BoxSizing::ContentBox => height + padding.vertical(),
            }
        }
        None => content_based_height,
    }
}

/// Resolve a box's *content-box* height from its specified (effective) height.
///
/// Per CSS 2.1 § 10.5, an in-flow child's percentage height resolves against
/// the containing block's **content** height (the box's `height` minus its own
/// padding and border for `box-sizing: border-box`; equal to `height` for
/// `content-box`). This differs from `resolve_padding_box_height`, which
/// returns the padding box (the containing block used for *absolute*
/// descendants). Returns 0 when the result would be negative.
pub(crate) fn resolve_content_box_height(
    specified_height: f32,
    padding: EdgeSizes,
    border: EdgeSizes,
    box_sizing: BoxSizing,
) -> f32 {
    let padding_box = match box_sizing {
        BoxSizing::BorderBox => specified_height - border.vertical(),
        BoxSizing::ContentBox => specified_height + padding.vertical(),
    };
    (padding_box - padding.vertical()).max(0.0)
}

/// Strip the first child's top margin and the last child's bottom margin when
/// they would otherwise collapse through the parent (no top/bottom padding or
/// border). Returns the adjusted children-area height.
///
/// This mirrors CSS margin-collapsing and is used to compute the containing
/// block height for absolute-positioned descendants (e.g. ::before/::after
/// bars): their `height: 100%` should match the parent's content-box
/// excluding collapsed outer margins, not the padded wrapper height.
pub(crate) fn collapse_outer_child_margins(
    children: &[LayoutNode],
    children_height: f32,
    padding: EdgeSizes,
    border: EdgeSizes,
) -> f32 {
    let strip_top = padding.top == 0.0 && border.top == 0.0;
    let strip_bottom = padding.bottom == 0.0 && border.bottom == 0.0;
    let first_mt = if strip_top {
        children
            .first()
            .map_or(0.0, |child| outer_margin_top(child.as_ref()))
    } else {
        0.0
    };
    let last_mb = if strip_bottom {
        children
            .last()
            .map_or(0.0, |child| outer_margin_bottom(child.as_ref()))
    } else {
        0.0
    };
    (children_height - first_mt - last_mb).max(0.0)
}

pub(crate) fn outer_margin_top(element: &dyn LayoutElement) -> f32 {
    element
        .block_flow_participant()
        .filter(|flow| flow.collapses_outer_margins())
        .map_or(0.0, |flow| flow.margins().start)
}

pub(crate) fn outer_margin_bottom(element: &dyn LayoutElement) -> f32 {
    element
        .block_flow_participant()
        .filter(|flow| flow.collapses_outer_margins())
        .map_or(0.0, |flow| flow.margins().end)
}

/// True for flow-participating block children. Absolute/fixed/float elements
/// don't participate in margin collapsing.
fn is_in_flow_block(element: &LayoutNode) -> bool {
    element
        .block_flow_participant()
        .is_some_and(|flow| flow.is_in_flow_block())
}

/// Return the index of the first/last in-flow child that participates in
/// margin collapsing. Skips absolute/fixed/float children.
pub(crate) fn first_in_flow_idx(children: &[LayoutNode]) -> Option<usize> {
    children.iter().position(is_in_flow_block)
}

pub(crate) fn last_in_flow_idx(children: &[LayoutNode]) -> Option<usize> {
    children.iter().rposition(is_in_flow_block)
}

/// Take the element's margin-top (and clear it), skipping elements that
/// don't participate in margin collapsing.
pub(crate) fn take_margin_top(element: &mut dyn LayoutElement) -> f32 {
    element
        .block_flow_participant_mut()
        .filter(|flow| flow.collapses_outer_margins())
        .map_or(0.0, |flow| std::mem::take(&mut flow.margins_mut().start))
}

pub(crate) fn take_margin_bottom(element: &mut dyn LayoutElement) -> f32 {
    element
        .block_flow_participant_mut()
        .filter(|flow| flow.collapses_outer_margins())
        .map_or(0.0, |flow| std::mem::take(&mut flow.margins_mut().end))
}

/// Collapse the first in-flow child's margin-top into `container_margin_top`,
/// and the last in-flow child's margin-bottom into `container_margin_bottom`,
/// whenever there is no top/bottom padding or border to block the collapse.
///
/// This mirrors CSS 2.1 § 8.3.1: the top margin of a block box collapses with
/// the margin of its first in-flow child if the box has no border/padding/line
/// boxes above it, and symmetrically for the bottom margin.
///
/// The child's margin is zeroed so that flow layout (pagination and
/// `render_container_children`) doesn't double-count it.
///
/// `suppress_top` / `suppress_bottom` block the respective collapse-through
/// independently of padding/border. Per CSS 2.1 § 8.3.1, collapsing of a box's
/// margin with its first/last child is suppressed when the box establishes a new
/// block formatting context (e.g. `overflow != visible`), is a flex/grid item or
/// container, or floats/absolutely-positions. The *bottom* collapse-through is
/// additionally suppressed when the box has a definite (non-`auto`) height — the
/// child's bottom margin is then contained inside that height.
#[allow(clippy::too_many_arguments)]
pub(crate) fn collapse_margins_through_parent(
    children: &mut [LayoutNode],
    container_margin_top: &mut f32,
    container_margin_bottom: &mut f32,
    padding: EdgeSizes,
    border: EdgeSizes,
    suppress_top: bool,
    suppress_bottom: bool,
) {
    if !suppress_top
        && padding.top == 0.0
        && border.top == 0.0
        && let Some(i) = first_in_flow_idx(children)
    {
        let child_mt = take_margin_top(children[i].as_mut());
        *container_margin_top = collapse_margin_pair(*container_margin_top, child_mt);
    }
    if !suppress_bottom
        && padding.bottom == 0.0
        && border.bottom == 0.0
        && let Some(i) = last_in_flow_idx(children)
    {
        let child_mb = take_margin_bottom(children[i].as_mut());
        *container_margin_bottom = collapse_margin_pair(*container_margin_bottom, child_mb);
    }
}

/// Collapse two adjoining margins (CSS 2.1 § 8.3.1): both non-negative → the
/// larger; both negative → the more negative; mixed signs → the sum (max
/// positive plus min negative). Used both for parent/child collapse-through and
/// as the canonical rule shared with the paginate sibling-collapse path.
pub(crate) fn collapse_margin_pair(a: f32, b: f32) -> f32 {
    if a >= 0.0 && b >= 0.0 {
        a.max(b)
    } else if a < 0.0 && b < 0.0 {
        a.min(b)
    } else {
        a + b
    }
}

/// True when `style` establishes a new block formatting context that suppresses
/// margin collapsing between the box and its in-flow children (CSS 2.1 § 9.4.1 +
/// § 8.3.1). Covers `overflow != visible`, floats, and absolute positioning;
/// flex/grid containers route through their own layout and never reach the block
/// margin-collapse path, so they need no entry here.
#[allow(dead_code)]
pub(crate) fn establishes_bfc(style: &ComputedStyle) -> bool {
    style.overflow.clips()
        || style.float != crate::style::computed::Float::None
        || style.position.is_absolute()
}

pub(crate) fn establishes_bfc_with_overflow(
    style: &ComputedStyle,
    overflow_axes: LayoutOverflowAxes,
) -> bool {
    overflow_axes.establishes_bfc()
        || style.float != crate::style::computed::Float::None
        || style.position.is_absolute()
}

// ---------------------------------------------------------------------------
// Group 6 — Element classification
// ---------------------------------------------------------------------------

pub(crate) fn recurses_as_layout_child(tag: HtmlTag) -> bool {
    tag.is_block() || tag == HtmlTag::Svg || tag == HtmlTag::Img
}

pub(crate) fn collects_as_inline_text(tag: HtmlTag) -> bool {
    // `<svg>` and `<img>` are replaced elements: they produce their own layout
    // element (vector / raster) rather than contributing inline text runs.
    tag != HtmlTag::Svg && tag != HtmlTag::Img && tag.is_inline()
}

// ---------------------------------------------------------------------------
// css-sizing-3 § 5.1 — intrinsic widths (min-content / max-content / fit-content)
// ---------------------------------------------------------------------------

/// css-sizing-3 § 5.1 intrinsic width result for one box: its *content-box*
/// min-content and max-content widths (padding/border excluded).
#[derive(Clone, Copy, Debug)]
struct IntrinsicWidths {
    /// Narrowest the content can be without inner overflow.
    min_content: f32,
    /// Widest the content wants to be with no line wrapping.
    max_content: f32,
}

/// Horizontal padding+border of a box, used to convert between its content-box
/// and border-box (outer) widths.
fn box_horizontal_extra(style: &ComputedStyle) -> f32 {
    style.padding.horizontal() + style.border.horizontal_width()
}

/// Compute the *outer* (margin-box) min/max-content contribution of a single
/// child box, per css-sizing-3 § 5.1: the box's own content min/max-content plus
/// its padding, border and horizontal margins.
///
/// `style` is the child's already-computed style and `el` its DOM subtree.
/// `parent_style` is needed to cascade grandchildren during recursion.
fn outer_intrinsic_widths(
    el: &ElementNode,
    style: &ComputedStyle,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
) -> IntrinsicWidths {
    let inner = content_intrinsic_widths(el, style, rules, fonts);
    // A definite length width fixes both intrinsic sizes to the content width
    // (border-box widths are converted back to content width). Percentage and
    // `auto`/keyword widths fall through to the content-based size.
    let inner = if let Some(w) = style.width {
        let content_w = if style.box_sizing == BoxSizing::BorderBox {
            (w - box_horizontal_extra(style)).max(0.0)
        } else {
            w
        };
        IntrinsicWidths {
            min_content: content_w,
            max_content: content_w,
        }
    } else {
        inner
    };
    // Min-/max-width clamps apply to the *content* width here (both box-sizing
    // modes track content width at this point). Min wins over max.
    let mut min_c = inner.min_content;
    let mut max_c = inner.max_content;
    if let Some(mw) = style.max_width {
        let cap = if style.box_sizing == BoxSizing::BorderBox {
            (mw - box_horizontal_extra(style)).max(0.0)
        } else {
            mw
        };
        min_c = min_c.min(cap);
        max_c = max_c.min(cap);
    }
    if let Some(mw) = style.min_width {
        let floor = if style.box_sizing == BoxSizing::BorderBox {
            (mw - box_horizontal_extra(style)).max(0.0)
        } else {
            mw
        };
        min_c = min_c.max(floor);
        max_c = max_c.max(floor);
    }
    // Convert the content widths to outer (margin-box) widths.
    let outer_extra = box_horizontal_extra(style) + style.margin.horizontal();
    IntrinsicWidths {
        min_content: min_c + outer_extra,
        max_content: max_c + outer_extra,
    }
}

/// Compute the *content-box* min/max-content widths of an element from its
/// children (css-sizing-3 § 5.1). Block children stack vertically, so the
/// element's content width is the max of its children's outer contributions;
/// inline content (text / inline boxes) contributes its run width (max-content =
/// unwrapped width, min-content = widest single word).
fn content_intrinsic_widths(
    el: &ElementNode,
    style: &ComputedStyle,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
) -> IntrinsicWidths {
    let mut block_min = 0.0f32;
    let mut block_max = 0.0f32;
    // Inline run accumulated across consecutive inline children/text nodes. Min
    // content is the widest single word; max content is the running line width.
    let mut inline_min = 0.0f32;
    let mut inline_line = 0.0f32;

    let font_family = resolve_style_font_family(style, fonts);
    let bold = style.font_weight == FontWeight::Bold;
    let italic = style.font_style.is_slanted();

    let flush_inline =
        |block_min: &mut f32, block_max: &mut f32, inline_min: &mut f32, inline_line: &mut f32| {
            *block_min = block_min.max(*inline_min);
            *block_max = block_max.max(*inline_line);
            *inline_min = 0.0;
            *inline_line = 0.0;
        };

    for child in &el.children {
        match child {
            DomNode::Text(text) => {
                accumulate_text_intrinsic(
                    text,
                    style.font_size,
                    &font_family,
                    bold,
                    italic,
                    fonts,
                    &mut inline_min,
                    &mut inline_line,
                );
            }
            DomNode::Element(child_el) => {
                let cls = child_el.class_list();
                let cls_refs: Vec<&str> = cls.iter().map(|s| s.as_ref()).collect();
                let child_style = compute_style_with_context(
                    child_el.tag,
                    child_el.style_attr(),
                    style,
                    rules,
                    child_el.tag_name(),
                    &cls_refs,
                    child_el.id(),
                    &child_el.attributes,
                    &SelectorContext::default(),
                );
                let is_block = child_style.display == Display::Block
                    || child_style.display == Display::Flex
                    || child_style.display == Display::Grid
                    || recurses_as_layout_child(child_el.tag);
                if is_block && !collects_as_inline_text(child_el.tag) {
                    flush_inline(
                        &mut block_min,
                        &mut block_max,
                        &mut inline_min,
                        &mut inline_line,
                    );
                    let widths = outer_intrinsic_widths(child_el, &child_style, rules, fonts);
                    block_min = block_min.max(widths.min_content);
                    block_max = block_max.max(widths.max_content);
                } else {
                    // Inline element: its text contributes to the current line.
                    // Recurse into inline children for nested inline text.
                    accumulate_inline_element(
                        child_el,
                        &child_style,
                        rules,
                        fonts,
                        &mut inline_min,
                        &mut inline_line,
                    );
                }
            }
        }
    }
    flush_inline(
        &mut block_min,
        &mut block_max,
        &mut inline_min,
        &mut inline_line,
    );

    IntrinsicWidths {
        min_content: block_min,
        max_content: block_max,
    }
}

/// Accumulate a text node's intrinsic-width contribution to the current inline
/// line. `inline_line` grows by the unwrapped run width (max-content); the
/// widest single word feeds `inline_min` (min-content).
#[allow(clippy::too_many_arguments)]
fn accumulate_text_intrinsic(
    text: &str,
    font_size: f32,
    font_family: &crate::style::computed::FontFamily,
    bold: bool,
    italic: bool,
    fonts: &HashMap<String, TtfFont>,
    inline_min: &mut f32,
    inline_line: &mut f32,
) {
    let collapsed = collapse_whitespace(text);
    if collapsed.is_empty() {
        return;
    }
    // Max-content: the whole run on one line (with its collapsed spaces).
    let run_w = estimate_word_width(&collapsed, font_size, font_family, bold, italic, fonts);
    *inline_line += run_w;
    // Min-content: the widest single word (no soft-wrap opportunity inside it).
    for word in collapsed.split(' ') {
        if word.is_empty() {
            continue;
        }
        let w = estimate_word_width(word, font_size, font_family, bold, italic, fonts);
        *inline_min = inline_min.max(w);
    }
}

/// Accumulate an inline element's text (recursively) into the current line.
fn accumulate_inline_element(
    el: &ElementNode,
    style: &ComputedStyle,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
    inline_min: &mut f32,
    inline_line: &mut f32,
) {
    let font_family = resolve_style_font_family(style, fonts);
    let bold = style.font_weight == FontWeight::Bold;
    let italic = style.font_style.is_slanted();
    for child in &el.children {
        match child {
            DomNode::Text(text) => {
                accumulate_text_intrinsic(
                    text,
                    style.font_size,
                    &font_family,
                    bold,
                    italic,
                    fonts,
                    inline_min,
                    inline_line,
                );
            }
            DomNode::Element(child_el) if collects_as_inline_text(child_el.tag) => {
                let cls = child_el.class_list();
                let cls_refs: Vec<&str> = cls.iter().map(|s| s.as_ref()).collect();
                let child_style = compute_style_with_context(
                    child_el.tag,
                    child_el.style_attr(),
                    style,
                    rules,
                    child_el.tag_name(),
                    &cls_refs,
                    child_el.id(),
                    &child_el.attributes,
                    &SelectorContext::default(),
                );
                accumulate_inline_element(
                    child_el,
                    &child_style,
                    rules,
                    fonts,
                    inline_min,
                    inline_line,
                );
            }
            // A nested block / replaced element inside inline content is rare in
            // print fixtures; ignore it for intrinsic measurement.
            DomNode::Element(_) => {}
        }
    }
}

/// css-sizing-3 § 5.1: resolve a block box's border-box width for an intrinsic
/// `width` keyword. `available_width` is the stretch-fit basis (the containing
/// block's content width). Returns the resolved *border-box* width.
pub(crate) fn resolve_intrinsic_keyword_width(
    el: &ElementNode,
    style: &ComputedStyle,
    keyword: IntrinsicWidthKeyword,
    available_width: f32,
    rules: &[CssRule],
    fonts: &HashMap<String, TtfFont>,
) -> f32 {
    let inner = content_intrinsic_widths(el, style, rules, fonts);
    let extra = box_horizontal_extra(style);
    // Border-box widths for each keyword (content width + padding + border).
    let min_content = inner.min_content + extra;
    let max_content = inner.max_content + extra;
    let resolved = match keyword {
        IntrinsicWidthKeyword::MinContent => min_content,
        IntrinsicWidthKeyword::MaxContent => max_content,
        IntrinsicWidthKeyword::FitContent => {
            // fit-content = min(max-content, max(min-content, stretch-fit)).
            // The stretch-fit term is the available (border-box) width less the
            // box's horizontal margins (css-sizing-3 § 5.1).
            let stretch = (available_width - style.margin.horizontal()).max(0.0);
            max_content.min(min_content.max(stretch))
        }
    };
    resolved.max(0.0)
}

// ---------------------------------------------------------------------------
// Group 1 — Pseudo-element helpers
// ---------------------------------------------------------------------------

pub(crate) fn resolve_content(
    items: &[ContentItem],
    attributes: &HashMap<String, String>,
    counter_state: &mut CounterState,
) -> String {
    resolve_content_with_quotes(items, attributes, counter_state, None)
}

/// Default UA quote pairs when the `quotes` property is `auto`/unset
/// (English-style typographic quotes for two nesting levels).
const DEFAULT_QUOTES: &[(&str, &str)] = &[("\u{201C}", "\u{201D}"), ("\u{2018}", "\u{2019}")];

/// Resolve a `content` value to its text, honoring the `quotes` property and
/// quote-depth nesting for open/close-quote keywords (css-content-3 §2.4.2).
///
/// `quotes` is `None` for `auto`/unset (use [`DEFAULT_QUOTES`]); `Some(&[])`
/// means `quotes: none` (open/close-quote produce no glyphs). Depth is tracked
/// within this content list, which covers nested quotes inside one declaration
/// and the common `::before { open-quote } / ::after { close-quote }` pattern.
/// `Url` items are replaced content and are skipped here.
pub(crate) fn resolve_content_with_quotes(
    items: &[ContentItem],
    attributes: &HashMap<String, String>,
    counter_state: &mut CounterState,
    quotes: Option<&[(String, String)]>,
) -> String {
    let mut result = String::new();
    let mut depth = counter_state.quote_depth;
    let open_glyph = |level: usize| -> String {
        match quotes {
            Some([]) => String::new(),
            Some(pairs) => pairs[level.min(pairs.len() - 1)].0.clone(),
            None => DEFAULT_QUOTES[level.min(DEFAULT_QUOTES.len() - 1)]
                .0
                .to_string(),
        }
    };
    let close_glyph = |level: usize| -> String {
        match quotes {
            Some([]) => String::new(),
            Some(pairs) => pairs[level.min(pairs.len() - 1)].1.clone(),
            None => DEFAULT_QUOTES[level.min(DEFAULT_QUOTES.len() - 1)]
                .1
                .to_string(),
        }
    };
    for item in items {
        match item {
            ContentItem::String(s) => {
                result.push_str(&resolve_target_attr_placeholders(s, attributes));
            }
            ContentItem::Attr(name) => {
                // Missing attribute resolves to the empty string (css-content-3 §1).
                if let Some(val) = attributes.get(name) {
                    result.push_str(val);
                }
            }
            ContentItem::Counter(name, style) => {
                result.push_str(&format_counter_value(style, counter_state.get(name)));
            }
            ContentItem::Counters(name, sep, style) => {
                result.push_str(&counter_state.get_all_styled(name, sep, style));
            }
            ContentItem::Leader(pattern) => {
                let pattern = if pattern.is_empty() {
                    "."
                } else {
                    pattern.as_str()
                };
                result.push_str(LEADER_PLACEHOLDER_START);
                result.push_str(pattern);
                result.push_str(LEADER_PLACEHOLDER_END);
            }
            ContentItem::OpenQuote => {
                result.push_str(&open_glyph(depth));
                depth += 1;
            }
            ContentItem::NoOpenQuote => {
                depth += 1;
            }
            ContentItem::CloseQuote => {
                depth = depth.saturating_sub(1);
                result.push_str(&close_glyph(depth));
            }
            ContentItem::NoCloseQuote => {
                depth = depth.saturating_sub(1);
            }
            // Replaced-element content (`url(...)`) is handled by the caller as
            // an image box, not text.
            ContentItem::Url(_) => {}
        }
    }
    counter_state.quote_depth = depth;
    result
}

fn resolve_target_attr_placeholders(s: &str, attributes: &HashMap<String, String>) -> String {
    if !s.contains(crate::style::computed::TARGET_PLACEHOLDER_START) || !s.contains("attr(") {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("attr(") {
        out.push_str(&rest[..start]);
        let arg_start = start + "attr(".len();
        let Some(end_rel) = rest[arg_start..].find(')') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = rest[arg_start..arg_start + end_rel].trim();
        if let Some(value) = attributes.get(name) {
            out.push_str(value);
        }
        rest = &rest[arg_start + end_rel + 1..];
    }
    out.push_str(rest);
    out
}

/// Returns the first `url()` image reference in a content list, if any.
pub(crate) fn content_image_url(items: &[ContentItem]) -> Option<&str> {
    items.iter().find_map(|item| match item {
        ContentItem::Url(url) => Some(url.as_str()),
        _ => None,
    })
}

/// Apply a `::first-line` pseudo style (css-pseudo-4 §2.1) to the runs of the
/// already-wrapped first line. Per the spec's restricted property set, only the
/// font/color/decoration-style typesetting properties are overlaid; the text
/// content and geometry are unchanged. Restyling happens after line breaking so
/// the first line is determined dynamically by wrapping.
pub(crate) fn apply_first_line_style(
    lines: &mut [TextLine],
    fl: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) {
    let Some(first) = lines.first_mut() else {
        return;
    };
    let family = resolve_style_font_family(fl, fonts);
    let line_height = text_run_line_height_factor(fl, fonts);
    let font_size = used_font_size(fl, fonts);
    for run in &mut first.runs {
        // Atomic inline boxes (inline-block / images) are not restyled by
        // ::first-line; only their geometry already participates in the line.
        if run.inline_box.is_some() {
            continue;
        }
        run.color = fl.color;
        run.text = apply_text_transform(&run.text, fl.text_transform);
        run.font_size = font_size * fl.font_variant_position.glyph_scale();
        run.line_height_basis = font_size;
        run.font_variant_position = fl.font_variant_position;
        run.bold = fl.font_weight == FontWeight::Bold;
        run.font_style = fl.font_style;
        run.decorations = fl.text_decorations.active(fl.color);
        run.font_family = family.clone();
        run.background_color = fl.background_color;
        run.line_height_factor = line_height;
        run.metadata = crate::layout::text::text_run_metadata(fl);
    }
    let first_height = first
        .runs
        .iter()
        .map(|run| {
            if let Some(inline) = run.inline_box.as_deref() {
                inline.height
            } else {
                run.font_size * run.line_height_factor
            }
        })
        .fold(0.0f32, f32::max);
    if first_height > 0.0 {
        first.height = first_height;
    }
}

/// Length, in bytes, of the leading `::first-letter` unit of `text`
/// (css-pseudo-4 §2.2.1): any preceding opening/other punctuation plus
/// interspersed spaces, the first Letter/Number/Symbol character, and any
/// immediately-following closing punctuation. Returns 0 when no letter unit is
/// found (the run is then left untouched).
fn first_letter_len(text: &str) -> usize {
    let mut idx = 0usize;
    let mut found_letter = false;
    let mut chars = text.char_indices().peekable();
    // Leading punctuation (Unicode P*) and interspersed spaces before the letter.
    while let Some(&(i, c)) = chars.peek() {
        if is_typographic_space(c) || is_punctuation(c) {
            idx = i + c.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    // The first Letter/Number/Symbol character unit.
    if let Some(&(i, c)) = chars.peek()
        && (c.is_alphanumeric() || is_symbol(c))
    {
        idx = i + c.len_utf8();
        found_letter = true;
        chars.next();
    }
    if !found_letter {
        return 0;
    }
    // Trailing closing punctuation directly attached to the letter.
    while let Some(&(i, c)) = chars.peek() {
        if is_punctuation(c) {
            idx = i + c.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    idx
}

fn is_typographic_space(c: char) -> bool {
    c == ' ' || c == '\u{00A0}' || c == '\t'
}

fn is_punctuation(c: char) -> bool {
    // Approximate Unicode P* using ASCII punctuation plus common typographic
    // quotes/dashes; sufficient for print fixtures without a Unicode DB.
    c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{2018}'
                | '\u{2019}'
                | '\u{201C}'
                | '\u{201D}'
                | '\u{2013}'
                | '\u{2014}'
                | '\u{00AB}'
                | '\u{00BB}'
                | '\u{00A1}'
                | '\u{00BF}'
        )
}

fn is_symbol(c: char) -> bool {
    matches!(c, '$' | '£' | '€' | '¥' | '#' | '%' | '+' | '<' | '=' | '>')
}

/// Inline geometry for a dropped initial letter.
///
/// The initial letter remains in its originating line, but its surrounding lines
/// exclude the kerned margin box. The side-bearing values are retained together
/// so line wrapping and paint positioning cannot drift apart.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DropCap {
    advance: f32,
    leading_kerning: f32,
    trailing_kerning: f32,
    span_lines: usize,
}

impl DropCap {
    fn new(advance: f32, side_bearings: GlyphSideBearings, span_lines: usize) -> Self {
        let advance = advance.max(0.0);
        let leading_kerning = side_bearings.start.clamp(0.0, advance);
        let trailing_kerning = side_bearings
            .end
            .clamp(0.0, (advance - leading_kerning).max(0.0));
        Self {
            advance,
            leading_kerning,
            trailing_kerning,
            span_lines: span_lines.max(1),
        }
    }

    /// The first line moves with the initial letter's negatively kerned margin
    /// box. Subsequent overlapping lines start after its remaining exclusion.
    pub(crate) fn line_inset(self, line_index: usize) -> f32 {
        if line_index == 0 {
            -self.leading_kerning
        } else if line_index < self.span_lines {
            self.exclusion_width()
        } else {
            0.0
        }
    }

    pub(crate) fn exclusion_width(self) -> f32 {
        (self.advance - self.leading_kerning - self.trailing_kerning).max(0.0)
    }

    pub(crate) fn spans_line(self, line_index: usize) -> bool {
        line_index < self.span_lines
    }

    fn trailing_kerning(self) -> f32 {
        self.trailing_kerning
    }
}

fn initial_letter_side_bearings(
    run: &TextRun,
    fonts: &HashMap<String, TtfFont>,
    inline_metric_size: f32,
) -> GlyphSideBearings {
    let Some(ch) = run.text.chars().find(|ch| !ch.is_whitespace()) else {
        return GlyphSideBearings::default();
    };
    let FontFamily::Custom(family) = &run.font_family else {
        return GlyphSideBearings::default();
    };
    let Some((_, font)) =
        crate::system_fonts::find_font(fonts, family, run.bold, run.font_style.is_slanted())
    else {
        return GlyphSideBearings::default();
    };
    let side_bearings = font
        .glyph_side_bearings(ch, inline_metric_size)
        .unwrap_or_default();
    GlyphSideBearings {
        start: snap_initial_letter_metric(side_bearings.start),
        end: snap_initial_letter_metric(side_bearings.end),
    }
}

/// Blink resolves a dropped initial's glyph metrics on its CSS-pixel font grid,
/// then converts the resulting exclusion box to PDF points. Keep this confined
/// to `initial-letter`: ordinary inline glyph positions intentionally retain
/// sub-pixel advances.
fn snap_initial_letter_metric(metric: f32) -> f32 {
    (metric / crate::fonts::PT_PER_CSS_PX).round() * crate::fonts::PT_PER_CSS_PX
}

/// Split off the leading `::first-letter` unit of the first text-bearing run and
/// restyle it (css-pseudo-4 §2.2). Mutates `runs` in place: the matched run is
/// replaced by an optional leading-whitespace run, the styled first-letter run,
/// and the remainder run. Applies the restricted property set (font/color/
/// decoration/transform).
///
/// A dropped `initial-letter` or floated first letter keeps its glyph in the
/// originating line while its line-box contribution is capped. The returned
/// [`DropCap`] then supplies matching exclusion geometry for the following lines.
pub(crate) fn apply_first_letter_style(
    runs: &mut Vec<TextRun>,
    fl: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
    block_line_height: f32,
    is_drop_cap: bool,
    initial_letter_inline_metric_size: Option<f32>,
) -> Option<DropCap> {
    // Find the first run carrying renderable text (skip pure-whitespace and
    // atomic-box runs, which precede the first letter, e.g. a ::before marker).
    let pos = runs
        .iter()
        .position(|r| r.inline_box.is_none() && !r.text.trim().is_empty())?;
    let split = first_letter_len(&runs[pos].text);
    if split == 0 {
        return None;
    }
    let base = runs[pos].clone();
    let first_text = base.text[..split].to_string();
    let rest_text = base.text[split..].to_string();

    let mut letter_run = base.clone();
    letter_run.text = apply_text_transform(&first_text, fl.text_transform);
    letter_run.font_size = used_font_size(fl, fonts);
    letter_run.color = fl.color;
    letter_run.bold = fl.font_weight == FontWeight::Bold;
    letter_run.font_style = fl.font_style;
    letter_run.decorations = fl.text_decorations.active(fl.color);
    letter_run.font_family = resolve_style_font_family(fl, fonts);
    letter_run.background_color = fl.background_color;
    letter_run.line_height_factor = text_run_line_height_factor(fl, fonts);
    letter_run.shaping = crate::layout::text::text_run_shaping(fl);
    letter_run.metadata = crate::layout::text::text_run_metadata(fl);
    crate::layout::text::mark_synthetic_weight_run(&mut letter_run, fl.font_weight, fonts);

    // Dropped initials do not increase their originating line's logical height.
    // Cap this run's contribution while keeping the glyph in the inline flow.
    let drop_cap = if is_drop_cap && fl.font_size > 0.0 {
        let glyph_w = snap_initial_letter_metric(estimate_word_width(
            &letter_run.text,
            letter_run.font_size,
            &letter_run.font_family,
            letter_run.bold,
            letter_run.font_style.is_slanted(),
            fonts,
        ));
        let span_lines = if fl.initial_letter > 1.0 {
            fl.initial_letter.round().max(1.0) as usize
        } else if block_line_height > 0.0 {
            (fl.font_size / block_line_height).ceil().max(1.0) as usize
        } else {
            1
        };
        // Keep the glyph inline on the first line, but cap its line-box height to
        // the surrounding line height so following lines are not pushed down.
        if block_line_height > 0.0 {
            letter_run.line_height_factor = block_line_height / fl.font_size;
        }
        letter_run.metadata.is_drop_cap = true;
        let side_bearings = if let Some(inline_metric_size) = initial_letter_inline_metric_size {
            initial_letter_side_bearings(&letter_run, fonts, inline_metric_size)
        } else {
            GlyphSideBearings::default()
        };
        Some(DropCap::new(
            glyph_w + fl.padding.right.max(0.0),
            side_bearings,
            span_lines,
        ))
    } else {
        None
    };

    // An initial letter's unbordered margin box is kerned by its side bearings.
    // Keep authored padding, then remove its trailing bearing with an explicit
    // non-painting inline advance so the first line and later exclusions agree.
    let dropcap_advance = drop_cap.map_or(0.0, |drop_cap| {
        fl.padding.right.max(0.0) - drop_cap.trailing_kerning()
    });
    let mut replacement = Vec::with_capacity(3);
    if dropcap_advance != 0.0 {
        let mut spacer = base.clone();
        spacer.text = String::new();
        spacer.background_color = None;
        spacer.vertical_align = crate::style::computed::VerticalAlign::Baseline;
        spacer.inline_box = Some(Box::new(InlineBox::advance_only(dropcap_advance)));
        replacement.push(spacer);
    }
    if !rest_text.is_empty() {
        let mut rest_run = base;
        rest_run.text = rest_text;
        // `::first-letter` keeps the initial glyph separately painted, but its
        // colour/background-only boundary must retain feasible pair positioning
        // (CSS Text 4 §8.7). Store the shaper-derived outgoing advance on the
        // first run so layout and PDF paint share it without cross-style
        // ligatures.
        letter_run.metadata.boundary.set_contextual_shaping(
            crate::text::inline_boundary_kerning_advance(&letter_run, &rest_run, fonts),
        );
        replacement.insert(0, letter_run);
        replacement.push(rest_run);
    } else {
        replacement.insert(0, letter_run);
    }
    runs.splice(pos..=pos, replacement);

    drop_cap
}

fn apply_text_transform(text: &str, transform: crate::style::computed::TextTransform) -> String {
    use crate::style::computed::TextTransform;
    match transform {
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut at_start = true;
            for c in text.chars() {
                if c.is_whitespace() {
                    at_start = true;
                    out.push(c);
                } else if at_start {
                    out.extend(c.to_uppercase());
                    at_start = false;
                } else {
                    out.push(c);
                }
            }
            out
        }
        TextTransform::None => text.to_string(),
    }
}

pub(crate) fn measure_runs_width(runs: &[TextRun], fonts: &HashMap<String, TtfFont>) -> f32 {
    let mut current = 0.0f32;
    let mut widest = 0.0f32;
    for run in runs {
        if run.text.contains('\n') {
            let mut segments = run.text.split('\n').peekable();
            let mut segment_index = 0;
            while let Some(segment) = segments.next() {
                if segment_index > 0 {
                    widest = widest.max(current);
                    current = 0.0;
                }
                if !segment.is_empty() {
                    let fragment = run.text_fragment(segment.to_owned(), segments.peek().is_none());
                    current += crate::layout::text::measure_text_run_advance(&fragment, fonts);
                }
                segment_index += 1;
            }
        } else {
            current += crate::layout::text::measure_text_run_advance(run, fonts);
        }
    }
    widest.max(current)
}

pub(crate) fn measure_lines_width(lines: &[TextLine], fonts: &HashMap<String, TtfFont>) -> f32 {
    lines
        .iter()
        .map(|line| measure_runs_width(&line.runs, fonts))
        .fold(0.0, f32::max)
}

pub(crate) fn pseudo_is_block_like(pseudo_style: &ComputedStyle) -> bool {
    matches!(pseudo_style.display, Display::Block | Display::ListItem)
        || pseudo_style.position.is_absolute()
}

pub(crate) fn append_pseudo_inline_run(
    runs: &mut Vec<TextRun>,
    pseudo_style: Option<&ComputedStyle>,
    el: &ElementNode,
    fonts: &HashMap<String, TtfFont>,
    counter_state: &mut CounterState,
) {
    if let Some(pseudo_style) = pseudo_style {
        if !pseudo_is_block_like(pseudo_style) {
            runs.push(build_pseudo_inline_run(
                pseudo_style,
                el,
                fonts,
                counter_state,
            ));
        }
    }
}

/// Geometry and document resources shared by generated block boxes.
///
/// Pseudo-elements are boxes in the originating element's formatting context,
/// not independent DOM nodes. Keeping their containing block and resource
/// resolution together prevents formatting-context-specific paths from
/// silently dropping properties such as SVG/CSS filters.
#[derive(Clone, Copy)]
pub(crate) struct PseudoBoxContext<'a> {
    available_width: f32,
    fonts: &'a HashMap<String, TtfFont>,
    filter_defs: &'a HashMap<String, ElementNode>,
    containing_block: Option<ContainingBlock>,
    positioned_ancestor_depth: usize,
}

impl<'a> PseudoBoxContext<'a> {
    pub(crate) const fn new(
        available_width: f32,
        fonts: &'a HashMap<String, TtfFont>,
        filter_defs: &'a HashMap<String, ElementNode>,
    ) -> Self {
        Self {
            available_width,
            fonts,
            filter_defs,
            containing_block: None,
            positioned_ancestor_depth: 0,
        }
    }

    pub(crate) const fn with_containing_block(
        self,
        containing_block: Option<ContainingBlock>,
    ) -> Self {
        Self {
            containing_block,
            ..self
        }
    }

    pub(crate) const fn with_positioned_ancestor_depth(
        self,
        positioned_ancestor_depth: usize,
    ) -> Self {
        Self {
            positioned_ancestor_depth,
            ..self
        }
    }
}

pub(crate) fn push_block_pseudo(
    output: &mut Vec<LayoutNode>,
    pseudo_style: Option<&ComputedStyle>,
    el: &ElementNode,
    context: PseudoBoxContext<'_>,
    counter_state: &mut CounterState,
) {
    if let Some(pseudo_style) = pseudo_style {
        if pseudo_is_block_like(pseudo_style) {
            let context = if pseudo_style.position.is_absolute() {
                context
            } else {
                context.with_containing_block(None)
            };
            output.push(build_pseudo_block(
                pseudo_style,
                el,
                context,
                counter_state,
                false,
            ));
        }
    }
}

/// Build a [`TextBlock`] for a `::before` or `::after` pseudo-element.
/// that uses `display: block` (or `position: absolute`).
pub(crate) fn build_pseudo_block(
    pseudo_style: &ComputedStyle,
    el: &ElementNode,
    context: PseudoBoxContext<'_>,
    counter_state: &mut CounterState,
    list_item_marker: bool,
) -> LayoutNode {
    let PseudoBoxContext {
        available_width,
        fonts,
        filter_defs,
        containing_block: containing_block_info,
        positioned_ancestor_depth,
    } = context;
    // Generated boxes do not pass through `flatten_element`, so resolve their
    // filter list here before constructing paint state. The resolved filter is
    // retained on the semantic box and materialized only after fragmentation,
    // exactly like an ordinary element's filter.
    let mut pseudo_style = pseudo_style.clone();
    let filter = crate::layout::filter::ResolvedFilter::from_style(&mut pseudo_style, filter_defs);
    let pseudo_style = &pseudo_style;
    let mut block_w = available_width;
    if let Some(cb) = containing_block_info
        && let Some(percent) = pseudo_style.percentage_sizing.width
    {
        block_w = cb.width * percent / 100.0;
    }
    if let Some(w) = pseudo_style.width {
        block_w = w.min(available_width);
    }
    if let Some(cb) = containing_block_info {
        if let Some(percent) = pseudo_style.percentage_sizing.min_width {
            block_w = block_w.max(cb.width * percent / 100.0);
        }
        if let Some(percent) = pseudo_style.percentage_sizing.max_width {
            block_w = block_w.min(cb.width * percent / 100.0);
        }
    }

    let inner_w = if pseudo_style.box_sizing == BoxSizing::BorderBox {
        block_w - pseudo_style.padding.horizontal() - pseudo_style.border.horizontal_width()
    } else {
        block_w - pseudo_style.padding.horizontal()
    }
    .max(0.0);

    let mut lines = Vec::new();
    let mut runs = Vec::new();
    let mut marker_hang = 0.0;
    let mut text_indent = pseudo_style.text_indent.resolve(inner_w);
    if list_item_marker {
        let marker_start = runs.len();
        let marker_text = format_list_marker(&pseudo_style.list_style_type, 0);
        let marker_font = resolve_style_font_family(pseudo_style, fonts);
        if let Some(bullet) = build_list_bullet_marker(
            &pseudo_style.list_style_type,
            used_font_size(pseudo_style, fonts),
            pseudo_style.color,
            Default::default(),
        ) {
            runs.push(TextRun {
                font_size: used_font_size(pseudo_style, fonts),
                color: pseudo_style.color,
                font_family: marker_font.clone(),
                line_height_factor: text_run_line_height_factor(pseudo_style, fonts),
                inline_box: Some(Box::new(bullet)),
                text_shadow: pseudo_style.text_shadow.clone(),
                metadata: crate::layout::text::text_run_metadata(pseudo_style),
                ..Default::default()
            });
        } else {
            push_text_run_with_fallback(
                TextRun {
                    text: marker_text,
                    font_size: used_font_size(pseudo_style, fonts),
                    bold: pseudo_style.font_weight == FontWeight::Bold,
                    font_style: pseudo_style.font_style,
                    color: pseudo_style.color,
                    font_family: marker_font,
                    line_height_factor: text_run_line_height_factor(pseudo_style, fonts),
                    text_shadow: pseudo_style.text_shadow.clone(),
                    metadata: crate::layout::text::text_run_metadata(pseudo_style),
                    ..Default::default()
                },
                &mut runs,
                fonts,
            );
        }
        marker_hang = measure_runs_width(&runs[marker_start..], fonts);
    }
    text_indent -= marker_hang;

    let generated_run = build_pseudo_inline_run(pseudo_style, el, fonts, counter_state);
    if !generated_run.text.is_empty() || generated_run.inline_box.is_some() {
        push_text_run_with_fallback(generated_run, &mut runs, fonts);
    }
    if !runs.is_empty() {
        lines = wrap_text_runs(
            runs.clone(),
            TextWrapOptions::new(
                inner_w,
                used_font_size(pseudo_style, fonts),
                text_run_line_height_factor(pseudo_style, fonts),
                pseudo_style.overflow_wrap,
            )
            .with_white_space(pseudo_style.white_space)
            .with_parent_strut(parent_line_strut(pseudo_style, fonts))
            .with_text_indent(text_indent)
            .with_rtl(pseudo_style.direction_rtl)
            .with_bidi_override(pseudo_style.bidi_override),
            fonts,
        );
    }

    if pseudo_style.position.is_absolute()
        && pseudo_style.width.is_none()
        && pseudo_style.min_width.is_none()
    {
        let content_w = measure_runs_width(&runs, fonts);
        block_w = if pseudo_style.box_sizing == BoxSizing::BorderBox {
            content_w + pseudo_style.padding.horizontal() + pseudo_style.border.horizontal_width()
        } else {
            content_w + pseudo_style.padding.horizontal()
        };
    }

    let border = LayoutBorder::from_computed(&pseudo_style.border, pseudo_style.color);
    let background_layers = BackgroundFields::from_style(pseudo_style);

    let explicit_width = if pseudo_style.position.is_absolute()
        || pseudo_style.width.is_some()
        || pseudo_style.min_width.is_some()
    {
        Some(block_w)
    } else {
        None
    };

    let preferred_height = {
        let mut height = pseudo_style.height;
        if let Some(cb) = containing_block_info
            && let Some(percent) = pseudo_style.percentage_sizing.height
        {
            height = Some(cb.height * percent / 100.0);
        }
        height
    };
    let minimum_height = pseudo_style.min_height.or_else(|| {
        containing_block_info.and_then(|cb| {
            pseudo_style
                .percentage_sizing
                .min_height
                .map(|percent| cb.height * percent / 100.0)
        })
    });
    let maximum_height = pseudo_style.max_height.or_else(|| {
        containing_block_info.and_then(|cb| {
            pseudo_style
                .percentage_sizing
                .max_height
                .map(|percent| cb.height * percent / 100.0)
        })
    });
    let height_constraints = SizeConstraints::new(minimum_height, maximum_height);
    let effective_height = preferred_height.map(|height| height_constraints.constrain(height));
    let text_height: f32 = lines.iter().map(|l| l.height).sum();
    let natural_padding_box_height = resolve_padding_box_height(
        text_height,
        effective_height,
        pseudo_style.padding,
        border.widths(),
        pseudo_style.box_sizing,
    );
    let padding_box_constraints = height_constraints.map(|height| {
        resolve_padding_box_height(
            0.0,
            Some(height),
            pseudo_style.padding,
            border.widths(),
            pseudo_style.box_sizing,
        )
    });
    let padding_box_height = if preferred_height.is_some() {
        natural_padding_box_height
    } else {
        padding_box_constraints.constrain(natural_padding_box_height)
    };
    let border_box_width = explicit_width.unwrap_or(block_w);
    let border_box_height = padding_box_height + border.vertical_width();

    // Resolve bottom/right into top/left when a containing block is present.
    // This allows pagination and rendering to only deal with top/left offsets.
    let (resolved_top, resolved_left) = if let Some(cb) = containing_block_info {
        let elem_h = padding_box_height;
        let elem_w = explicit_width.unwrap_or(block_w);
        let top_from_percent = pseudo_style
            .percentage_insets
            .top
            .map(|percent| cb.height * percent / 100.0);
        let bottom_from_percent = pseudo_style
            .percentage_insets
            .bottom
            .map(|percent| cb.height * percent / 100.0);
        let left_from_percent = pseudo_style
            .percentage_insets
            .left
            .map(|percent| cb.width * percent / 100.0);
        let right_from_percent = pseudo_style
            .percentage_insets
            .right
            .map(|percent| cb.width * percent / 100.0);

        let top = if let Some(top) = top_from_percent.or(pseudo_style.top) {
            top
        } else if let Some(bottom) = bottom_from_percent.or(pseudo_style.bottom) {
            cb.height - elem_h - bottom
        } else {
            0.0
        };
        let left = if let Some(left) = left_from_percent.or(pseudo_style.left) {
            left
        } else if let Some(right) = right_from_percent.or(pseudo_style.right) {
            cb.width - elem_w - right
        } else {
            0.0
        };
        (top, left)
    } else {
        (
            pseudo_style.top.unwrap_or(0.0),
            pseudo_style.left.unwrap_or(0.0),
        )
    };

    let mut paint = BoxPaint {
        background: BackgroundPaint {
            color: pseudo_style.background_color,
            layers: background_layers,
            blend_mode: pseudo_style.background_blend_mode,
        },
        border_radii: pseudo_style.resolve_corner_radii(border_box_width, border_box_height),
        shadows: pseudo_style.box_shadow.clone(),
        outline: OutlinePaint {
            width: pseudo_style.outline_width,
            color: pseudo_style.outline_color,
            offset: pseudo_style.outline_offset,
        },
        group: crate::layout::elements::PaintGroup::from_style(pseudo_style),
        visible: pseudo_style.visibility == Visibility::Visible,
        ..BoxPaint::default()
    };
    if filter.requires_source_surface() {
        paint.group.filter = Some(filter);
    }

    TextBlock {
        lines,
        box_model: BoxModel {
            size: LayoutSize {
                width: InlineSize::from_fixed_value(explicit_width),
                height: BlockSize::from_definite(effective_height.map(|_| padding_box_height)),
            },
            margins: BlockMargins::new(pseudo_style.margin.top, pseudo_style.margin.bottom),
            padding: pseudo_style.padding,
            border,
        },
        paint,
        flow: BlockFlow {
            float: pseudo_style.float,
            clear: pseudo_style.clear,
        },
        positioning: Positioning::from_style(pseudo_style)
            .with_resolved_insets(EdgeSizes::new(
                resolved_top,
                pseudo_style.right.unwrap_or_default(),
                pseudo_style.bottom.unwrap_or_default(),
                resolved_left,
            ))
            .with_containing_block(containing_block_info)
            .with_containing_block_depth(if pseudo_style.position.is_positioned() {
                positioned_ancestor_depth + 1
            } else {
                positioned_ancestor_depth
            }),
        fragmentation: TextFragmentation::default(),
        text: TextBlockStyle {
            alignment: pseudo_style.text_align,
            indent: text_indent,
            ..TextBlockStyle::default()
        },
        semantics: TextSemantics::default(),
        ..TextBlock::default()
    }
    .boxed()
}

/// Build a `TextRun` for an inline `::before` or `::after` pseudo-element.
pub(crate) fn build_pseudo_inline_run(
    pseudo_style: &ComputedStyle,
    el: &ElementNode,
    fonts: &HashMap<String, TtfFont>,
    counter_state: &mut CounterState,
) -> TextRun {
    let content_text = resolve_content_with_quotes(
        &pseudo_style.content,
        &el.attributes,
        counter_state,
        pseudo_style.quotes.as_deref(),
    );

    // `content: url(...)` makes the pseudo a replaced inline image
    // (css-content-3 §1). Decode it and emit an image-bearing InlineBox.
    if let Some(url) = content_image_url(&pseudo_style.content)
        && let Some(inline) = build_pseudo_image_box(pseudo_style, url)
    {
        return TextRun {
            font_size: used_font_size(pseudo_style, fonts),
            color: pseudo_style.color,
            font_family: resolve_style_font_family(pseudo_style, fonts),
            line_height_factor: text_run_line_height_factor(pseudo_style, fonts),
            inline_box: Some(Box::new(inline)),
            vertical_align: pseudo_style.vertical_align,
            text_shadow: pseudo_style.text_shadow.clone(),
            metadata: crate::layout::text::text_run_metadata(pseudo_style),
            ..Default::default()
        };
    }

    // `display: inline-block` pseudo-elements (e.g. a decorative
    // `::before { content: ""; display: inline-block; width/height/background }`)
    // are atomic boxes, not text. Emit an InlineBox so the box and any inner
    // text paint; a plain text run would drop the box entirely.
    if pseudo_style.display == Display::InlineBlock {
        let inline = build_pseudo_inline_box(pseudo_style, &content_text, fonts);
        return TextRun {
            font_size: used_font_size(pseudo_style, fonts),
            bold: pseudo_style.font_weight == FontWeight::Bold,
            font_style: pseudo_style.font_style,
            color: pseudo_style.color,
            font_family: resolve_style_font_family(pseudo_style, fonts),
            line_height_factor: text_run_line_height_factor(pseudo_style, fonts),
            inline_box: Some(Box::new(inline)),
            vertical_align: pseudo_style.vertical_align,
            text_shadow: pseudo_style.text_shadow.clone(),
            metadata: crate::layout::text::text_run_metadata(pseudo_style),
            ..Default::default()
        };
    }

    TextRun {
        text: content_text,
        font_size: used_font_size(pseudo_style, fonts),
        bold: pseudo_style.font_weight == FontWeight::Bold,
        font_style: pseudo_style.font_style,
        decorations: pseudo_style.text_decorations.active(pseudo_style.color),
        color: pseudo_style.color,
        font_family: resolve_style_font_family(pseudo_style, fonts),
        background_color: pseudo_style.background_color,
        line_height_factor: text_run_line_height_factor(pseudo_style, fonts),
        vertical_align: pseudo_style.vertical_align,
        text_shadow: pseudo_style.text_shadow.clone(),
        metadata: crate::layout::text::text_run_metadata(pseudo_style),
        ..Default::default()
    }
}

/// Build the atomic `InlineBox` for a `display: inline-block` pseudo-element.
/// Sizes follow CSS box-sizing; any text content is wrapped to one line inside
/// the content box.
fn build_pseudo_inline_box(
    pseudo_style: &ComputedStyle,
    content_text: &str,
    fonts: &HashMap<String, TtfFont>,
) -> InlineBox {
    let border = LayoutBorder::from_computed(&pseudo_style.border, pseudo_style.color);
    let pad_h = pseudo_style.padding.horizontal();
    let pad_v = pseudo_style.padding.vertical();

    // Inner text lines (empty content -> no lines).
    let lines: Vec<TextLine> = if content_text.is_empty() {
        Vec::new()
    } else {
        let run = TextRun {
            text: content_text.to_string(),
            font_size: used_font_size(pseudo_style, fonts),
            bold: pseudo_style.font_weight == FontWeight::Bold,
            font_style: pseudo_style.font_style,
            decorations: pseudo_style.text_decorations.active(pseudo_style.color),
            color: pseudo_style.color,
            font_family: resolve_style_font_family(pseudo_style, fonts),
            line_height_factor: text_run_line_height_factor(pseudo_style, fonts),
            text_shadow: pseudo_style.text_shadow.clone(),
            metadata: crate::layout::text::text_run_metadata(pseudo_style),
            ..Default::default()
        };
        wrap_text_runs(
            vec![run],
            TextWrapOptions::new(
                f32::MAX,
                used_font_size(pseudo_style, fonts),
                text_run_line_height_factor(pseudo_style, fonts),
                pseudo_style.overflow_wrap,
            )
            .with_white_space(pseudo_style.white_space)
            .with_parent_strut(parent_line_strut(pseudo_style, fonts)),
            fonts,
        )
    };

    let text_w = lines
        .iter()
        .map(|l| {
            l.runs
                .iter()
                .map(|r| {
                    estimate_word_width(
                        &r.text,
                        r.font_size,
                        &r.font_family,
                        r.bold,
                        r.font_style.is_slanted(),
                        fonts,
                    )
                })
                .sum::<f32>()
        })
        .fold(0.0f32, f32::max);
    let text_h: f32 = lines.iter().map(|l| l.height).sum();

    // Resolve the painted border-box width/height from the explicit size
    // (honoring box-sizing) or the intrinsic content size when unspecified.
    let width = match pseudo_style.width {
        Some(w) if pseudo_style.box_sizing == BoxSizing::BorderBox => w.max(0.0),
        Some(w) => (w + pad_h + border.horizontal_width()).max(0.0),
        None => text_w + pad_h + border.horizontal_width(),
    };
    let height = match pseudo_style.height {
        Some(h) if pseudo_style.box_sizing == BoxSizing::BorderBox => h.max(0.0),
        Some(h) => (h + pad_v + border.vertical_width()).max(0.0),
        None => text_h + pad_v + border.vertical_width(),
    };

    InlineBox {
        width,
        height,
        margin_left: pseudo_style.margin.left.max(0.0),
        margin_right: pseudo_style.margin.right.max(0.0),
        paint: InlineBoxPaint {
            background_color: pseudo_style.background_color,
            border,
            border_image: pseudo_style.border_image.paint(),
            border_radii: pseudo_style.resolve_corner_radii(width, height),
            ..InlineBoxPaint::default()
        },
        padding: pseudo_style.padding,
        vertical_align: pseudo_style.vertical_align,
        baseline_ascent: None,
        lines,
        ..InlineBox::default()
    }
}

/// Build a replaced-image `InlineBox` for a pseudo-element whose `content` is a
/// `url(...)` (css-content-3 §1). The box uses the explicit CSS width/height
/// when given, otherwise the image's intrinsic pixel dimensions. Returns `None`
/// if the image cannot be decoded (the pseudo then produces no box).
fn build_pseudo_image_box(pseudo_style: &ComputedStyle, url: &str) -> Option<InlineBox> {
    let image_src = crate::parser::css::extract_url_path(url).unwrap_or_else(|| url.to_string());
    let (raw, _mime) = crate::layout::images::load_resource(&image_src, None)?;
    let image = crate::layout::images::load_image_bytes(raw.to_vec())?;

    // Intrinsic image pixels map to CSS px at 1x, and CSS px → PDF points at
    // 1px = 0.75pt (96dpi). The same conversion is applied to `<img>` intrinsic
    // sizing (see layout::images). Without it the intrinsic size would be read
    // as points and the image would render ~1.33x too large.
    //
    // A pseudo-element whose `content` is a single `url()` image is a generated
    // *content* replaced box: its size is the image's intrinsic size, and the
    // `width`/`height` properties do NOT apply to it (matching Chrome). So the
    // box always paints at the intrinsic dimensions regardless of any declared
    // `width`/`height` on the pseudo rule.
    let width = image.source_width.max(1) as f32 * 0.75;
    let height = image.source_height.max(1) as f32 * 0.75;

    Some(InlineBox {
        width,
        height,
        margin_left: pseudo_style.margin.left.max(0.0),
        margin_right: pseudo_style.margin.right.max(0.0),
        paint: InlineBoxPaint {
            border: LayoutBorder::from_computed(&pseudo_style.border, pseudo_style.color),
            border_image: pseudo_style.border_image.paint(),
            border_radii: pseudo_style.resolve_corner_radii(width, height),
            ..InlineBoxPaint::default()
        },
        padding: EdgeSizes::ZERO,
        vertical_align: pseudo_style.vertical_align,
        image: Some(image),
        ..InlineBox::default()
    })
}

/// Decode a `list-style-image` value (a CSS `url(...)`, possibly a data-URI)
/// into an atomic image `InlineBox` to use as a list marker (css-lists-3 §3.1).
///
/// The marker is sized at the image's intrinsic CSS-pixel size (CSS px ==
/// intrinsic px at 1x), converted to PDF points (1px = 0.75pt) like every other
/// raster, with a small right margin so the following text does not touch it.
/// Returns `None` when the URL is absent or cannot be decoded, so the caller can
/// fall back to the `list-style-type` glyph marker.
pub(crate) fn build_list_image_marker(value: &str, gap: f32) -> Option<InlineBox> {
    let url = crate::parser::css::extract_url_path(value).unwrap_or_else(|| value.to_string());
    let (raw, _mime) = crate::layout::images::load_resource(&url, None)?;
    let image = crate::layout::images::load_image_bytes(raw.to_vec())?;
    // The InlineBox dimensions are in PDF points; the image's intrinsic size is
    // in CSS px, so convert px -> pt (1px = 0.75pt) exactly as `load_image_bytes`
    // consumers do for ordinary <img>. Without this the marker paints at its raw
    // pixel count as points (16px -> 16pt instead of 12pt), ~1.33x too large.
    const PX_TO_PT: f32 = 0.75;
    let width = image.source_width.max(1) as f32 * PX_TO_PT;
    let height = image.source_height.max(1) as f32 * PX_TO_PT;
    Some(InlineBox {
        width,
        height,
        margin_right: gap,
        padding: EdgeSizes::ZERO,
        vertical_align: VerticalAlign::Baseline,
        image: Some(image),
        ..InlineBox::default()
    })
}

// ---------------------------------------------------------------------------
// Group 2 — Background/visual helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct BackgroundFields {
    pub(crate) gradient: Option<LinearGradient>,
    pub(crate) radial_gradient: Option<RadialGradient>,
    pub(crate) conic_gradient: Option<ConicGradient>,
    pub(crate) svg: Option<crate::parser::svg::SvgTree>,
    pub(crate) blur_radius: f32,
    pub(crate) size: BackgroundSize,
    pub(crate) position: BackgroundPosition,
    pub(crate) repeat: BackgroundRepeat,
    pub(crate) origin: BackgroundOrigin,
    pub(crate) clip: BackgroundClip,
}

impl BackgroundFields {
    pub(crate) fn from_style(style: &ComputedStyle) -> Self {
        Self {
            gradient: style.background_gradient.clone(),
            radial_gradient: style.background_radial_gradient.clone(),
            conic_gradient: style.background_conic_gradient.clone(),
            svg: background_svg_for_style(style),
            // CSS `filter` belongs to the composited box, not its background
            // image layer. The post-layout filter compositor applies it once
            // to the complete SourceGraphic.
            blur_radius: 0.0,
            size: style.background_size,
            position: style.background_position,
            repeat: style.background_repeat,
            origin: style.background_origin,
            clip: style.background_clip,
        }
    }

    pub(crate) fn has_image(&self) -> bool {
        self.gradient.is_some()
            || self.radial_gradient.is_some()
            || self.conic_gradient.is_some()
            || self.svg.is_some()
    }

    /// Initial/fallback painting values inherited by an individual gradient
    /// layer when that layer did not carry its own comma-list entry.
    pub(crate) fn gradient_layer_box(&self) -> crate::style::computed::GradientLayerBox {
        crate::style::computed::GradientLayerBox {
            size: Some(self.size),
            position: Some(self.position),
            repeat: Some(self.repeat),
            origin: Some(self.origin),
            clip: Some(self.clip),
            attachment: Some(crate::style::computed::BackgroundAttachment::Scroll),
            ..Default::default()
        }
    }
}

impl Default for BackgroundFields {
    fn default() -> Self {
        Self {
            gradient: None,
            radial_gradient: None,
            conic_gradient: None,
            svg: None,
            blur_radius: 0.0,
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::Repeat,
            origin: BackgroundOrigin::Padding,
            clip: BackgroundClip::Border,
        }
    }
}

pub(crate) fn has_background_paint(style: &ComputedStyle) -> bool {
    style.background_color.is_some()
        || style.background_gradient.is_some()
        || style.background_radial_gradient.is_some()
        || style.background_conic_gradient.is_some()
        || style.background_image.is_some()
        || style.background_svg.is_some()
}

pub(crate) fn background_svg_for_style(
    style: &ComputedStyle,
) -> Option<crate::parser::svg::SvgTree> {
    style.background_svg.clone().or_else(|| {
        style
            .background_image
            .as_deref()
            .and_then(build_raster_background_tree)
    })
}

pub(crate) fn aspect_ratio_height(width: f32, style: &ComputedStyle) -> Option<f32> {
    style
        .aspect_ratio
        .filter(|ratio| *ratio > 0.0)
        .map(|ratio| width / ratio)
        .filter(|height| *height > 0.0)
}

// ---------------------------------------------------------------------------
// Group 6 — Paint order
// ---------------------------------------------------------------------------

pub(crate) fn layout_element_paint_order(
    element: &dyn LayoutElement,
) -> crate::layout::elements::StackingLevel {
    let Some(group) = element.paint_group_owner().map(|owner| owner.paint_group()) else {
        return crate::layout::elements::StackingLevel::in_flow();
    };
    group.stacking.level(
        element.positioning_owner().map(|owner| owner.positioning()),
        element.block_flow_owner().map(|owner| owner.block_flow()),
        group,
    )
}

pub(crate) fn layout_element_establishes_stacking_context(element: &dyn LayoutElement) -> bool {
    let Some(group) = element.paint_group_owner().map(|owner| owner.paint_group()) else {
        return false;
    };
    group.stacking.establishes_context(
        element.positioning_owner().map(|owner| owner.positioning()),
        group,
    )
}

// ---------------------------------------------------------------------------
// Group 6 — Heading level
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Returns the heading level (1-6) for a tag, or None if not a heading.
pub(crate) fn heading_level(tag: HtmlTag) -> Option<u8> {
    match tag {
        HtmlTag::H1 => Some(1),
        HtmlTag::H2 => Some(2),
        HtmlTag::H3 => Some(3),
        HtmlTag::H4 => Some(4),
        HtmlTag::H5 => Some(5),
        HtmlTag::H6 => Some(6),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Group 5 — Positioning helpers
// ---------------------------------------------------------------------------

/// Whether an element establishes a containing block for `position: absolute`
/// descendants (CSS Positioned Layout § 4 / CSS Transforms § 3): any positioned
/// box (relative/absolute/fixed), or — even when `position: static` — a box with
/// a `transform` or non-`none` `filter`. Other triggers (perspective,
/// will-change, contain) are not modelled here. This is what lets an absolute
/// box resolve against a transformed or filtered non-positioned ancestor.
pub(crate) fn establishes_containing_block(style: &ComputedStyle) -> bool {
    style.position.is_positioned()
        || style.transform.is_some()
        || style.filter.establishes_stacking_context
}

/// Resolve a single inset (`top`/`right`/`bottom`/`left`) to a length.
/// Prefers a percentage (resolved against `reference`, the containing block's
/// padding-box dimension on the relevant axis) over an explicit length, matching
/// how `resolve_abs_containing_block` resolves insets. Returns `None` when the
/// inset is `auto` (neither length nor percentage set).
pub(crate) fn resolve_inset(
    length: Option<f32>,
    percent: Option<f32>,
    reference: f32,
) -> Option<f32> {
    percent.map(|p| reference * p / 100.0).or(length)
}

/// Resolve the containing block for an element that is `position: absolute`.
/// If the element is absolute and `abs_cb` is `Some`, returns `abs_cb` and
/// resolves bottom/right offsets into top/left. Otherwise returns `None`
/// and leaves offsets unchanged.
pub(crate) fn resolve_abs_containing_block(
    style: &ComputedStyle,
    abs_cb: Option<ContainingBlock>,
    elem_height: f32,
    elem_width: f32,
) -> (Option<ContainingBlock>, f32, f32) {
    // `top`/`left` only shift a *positioned* box. A `position: static` element
    // ignores them entirely, so it must report a zero offset — otherwise the
    // value leaks into `offset_left`/`offset_top` and shifts the static box.
    if style.position.is_relative() {
        return (None, style.top.unwrap_or(0.0), style.left.unwrap_or(0.0));
    }
    if !style.position.is_absolute() {
        return (None, 0.0, 0.0);
    }
    let cb = match abs_cb {
        Some(cb) => cb,
        None => return (None, style.top.unwrap_or(0.0), style.left.unwrap_or(0.0)),
    };

    let top_from_percent = style.percentage_insets.top.map(|p| cb.height * p / 100.0);
    let bottom_from_percent = style
        .percentage_insets
        .bottom
        .map(|p| cb.height * p / 100.0);
    let left_from_percent = style.percentage_insets.left.map(|p| cb.width * p / 100.0);
    let right_from_percent = style.percentage_insets.right.map(|p| cb.width * p / 100.0);

    let resolved_top = if let Some(top) = top_from_percent.or(style.top) {
        top
    } else if let Some(bottom) = bottom_from_percent.or(style.bottom) {
        cb.height - elem_height - bottom
    } else {
        0.0
    };
    let resolved_left = if let Some(left) = left_from_percent.or(style.left) {
        left
    } else if let Some(right) = right_from_percent.or(style.right) {
        cb.width - elem_width - right
    } else {
        0.0
    };

    (Some(cb), resolved_top, resolved_left)
}

pub(crate) fn resolve_relative_offsets(
    style: &ComputedStyle,
    width_reference: f32,
    height_reference: f32,
) -> (f32, f32) {
    let top = resolve_inset(style.top, style.percentage_insets.top, height_reference)
        .or_else(|| {
            resolve_inset(
                style.bottom,
                style.percentage_insets.bottom,
                height_reference,
            )
            .map(|bottom| -bottom)
        })
        .unwrap_or(0.0);
    let left = resolve_inset(style.left, style.percentage_insets.left, width_reference)
        .or_else(|| {
            resolve_inset(style.right, style.percentage_insets.right, width_reference)
                .map(|right| -right)
        })
        .unwrap_or(0.0);
    (top, left)
}

/// Resolve one finalized containing block throughout a formatting-context
/// subtree.
///
/// Formatting contexts can only know their final used extent after descendant
/// layout. Absolute descendants therefore retain their authored constraints
/// and are resolved in one recursive pass. Positioned depth prevents this pass
/// from replacing geometry owned by a nearer containing block.
pub(crate) fn resolve_absolute_descendants_containing_block(
    elements: &mut [LayoutNode],
    containing_block: ContainingBlock,
) {
    fn patch(element: &mut LayoutNode, containing_block: ContainingBlock) {
        if let Some(consumer) = element.containing_block_consumer_mut() {
            consumer.resolve_containing_block(containing_block);
        }
        element.visit_child_nodes_mut(&mut |child| patch(child, containing_block));
    }

    for element in elements {
        patch(element, containing_block);
    }
}

#[cfg(test)]
mod generated_content_tests {
    use super::*;
    use crate::layout::engine::{SyntheticFontWeight, TextRun};
    use crate::layout::list_markers::BuiltInBulletSlot;
    use crate::style::computed::{ContentItem, FontStack, ListStyleType};
    use std::collections::HashMap;

    fn cs() -> CounterState {
        CounterState::default()
    }

    #[test]
    fn geometric_bullets_share_chromium_marker_slot() {
        let disc = build_list_bullet_marker(
            &ListStyleType::Disc,
            16.5,
            crate::types::Color::BLACK,
            Default::default(),
        )
        .expect("disc is a geometric marker");
        let square = build_list_bullet_marker(
            &ListStyleType::Square,
            16.5,
            crate::types::Color::BLACK,
            Default::default(),
        )
        .expect("square is a geometric marker");

        for marker in [disc, square] {
            assert!((marker.width - 5.25).abs() < f32::EPSILON);
            assert!((marker.outer_width() - 15.0).abs() < f32::EPSILON);
            assert!((marker.baseline_ascent.unwrap() - 7.5).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn standalone_inside_bullets_use_their_own_marker_slot() {
        let marker = build_list_bullet_marker(
            &ListStyleType::Disc,
            22.5,
            crate::types::Color::BLACK,
            BuiltInBulletSlot::StandaloneInside,
        )
        .expect("disc is a geometric marker");

        assert!((marker.width - 6.75).abs() < f32::EPSILON);
        assert!((marker.outer_width() - 30.0).abs() < f32::EPSILON);
        assert!((marker.baseline_ascent.unwrap() - 9.75).abs() < f32::EPSILON);
    }

    #[test]
    fn first_letter_len_basic() {
        assert_eq!(first_letter_len("Drop cap"), "D".len());
        assert_eq!(first_letter_len("hello"), "h".len());
    }

    #[test]
    fn first_letter_len_leading_punctuation() {
        // Leading quote plus the letter are one first-letter unit.
        assert_eq!(first_letter_len("\u{201C}Once"), "\u{201C}O".len());
        assert_eq!(first_letter_len("(A)"), "(A)".len());
    }

    #[test]
    fn first_letter_len_no_letter() {
        assert_eq!(first_letter_len("   "), 0);
        assert_eq!(first_letter_len(""), 0);
    }

    #[test]
    fn first_letter_preserves_the_requested_synthetic_weight() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/parity/fonts/ParitySans.ttf"),
        )
        .expect("ParitySans test font");
        let font = crate::parser::ttf::parse_ttf(bytes).expect("valid ParitySans TTF");
        let fonts = HashMap::from([("paritysans".to_string(), font)]);
        let family = FontFamily::Custom("ParitySans".to_string());
        let style = ComputedStyle {
            font_weight: FontWeight::Bold,
            font_family: family.clone(),
            font_stack: FontStack::from_family(family.clone()),
            ..Default::default()
        };
        let mut runs = vec![TextRun {
            text: "Initial".to_string(),
            font_family: family,
            ..Default::default()
        }];

        apply_first_letter_style(&mut runs, &style, &fonts, 0.0, false, None);

        assert_eq!(runs[0].font_synthesis.weight, SyntheticFontWeight::Auto);
    }

    #[test]
    fn initial_letter_metrics_snap_without_quantizing_inline_text() {
        assert_eq!(snap_initial_letter_metric(3.091_552_7), 3.0);
        assert_eq!(snap_initial_letter_metric(11.263_424), 11.25);
        assert_eq!(snap_initial_letter_metric(3.375), 3.75);
    }

    #[test]
    fn drop_cap_geometry_keeps_kerning_and_exclusion_together() {
        let drop_cap = DropCap::new(
            15.0,
            GlyphSideBearings {
                start: 4.0,
                end: 3.0,
            },
            2,
        );

        assert_eq!(drop_cap.line_inset(0), -4.0);
        assert_eq!(drop_cap.line_inset(1), 8.0);
        assert_eq!(drop_cap.line_inset(2), 0.0);
    }

    #[test]
    fn resolve_quotes_uses_declared_pairs() {
        let items = vec![ContentItem::OpenQuote];
        let pairs = [("<".to_string(), ">".to_string())];
        let mut state = cs();
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &mut state, Some(&pairs));
        assert_eq!(s, "<");
    }

    #[test]
    fn resolve_quotes_none_is_empty() {
        let items = vec![ContentItem::OpenQuote, ContentItem::CloseQuote];
        let pairs: [(String, String); 0] = [];
        let mut state = cs();
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &mut state, Some(&pairs));
        assert_eq!(s, "");
    }

    #[test]
    fn resolve_quotes_default_when_unset() {
        let items = vec![ContentItem::OpenQuote, ContentItem::CloseQuote];
        let mut state = cs();
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &mut state, None);
        assert_eq!(s, "\u{201C}\u{201D}");
    }

    #[test]
    fn resolve_quotes_nested_depth() {
        // Two pairs, nested open/open/close/close cycles through levels.
        let items = vec![
            ContentItem::OpenQuote,
            ContentItem::OpenQuote,
            ContentItem::CloseQuote,
            ContentItem::CloseQuote,
        ];
        let pairs = [
            ("A".to_string(), "a".to_string()),
            ("B".to_string(), "b".to_string()),
        ];
        let mut state = cs();
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &mut state, Some(&pairs));
        assert_eq!(s, "ABba");
    }

    #[test]
    fn resolve_quotes_carries_depth_between_calls() {
        let pairs = [
            ("A".to_string(), "a".to_string()),
            ("B".to_string(), "b".to_string()),
        ];
        let mut state = cs();

        let opened = resolve_content_with_quotes(
            &[ContentItem::OpenQuote],
            &HashMap::new(),
            &mut state,
            Some(&pairs),
        );
        assert_eq!(opened, "A");
        assert_eq!(state.quote_depth, 1);

        let closed = resolve_content_with_quotes(
            &[
                ContentItem::OpenQuote,
                ContentItem::CloseQuote,
                ContentItem::CloseQuote,
            ],
            &HashMap::new(),
            &mut state,
            Some(&pairs),
        );
        assert_eq!(closed, "Bba");
        assert_eq!(state.quote_depth, 0);
    }

    #[test]
    fn resolve_no_quote_keywords_track_depth_without_glyphs() {
        // no-open-quote increments depth but emits nothing; close then uses depth 0.
        let items = vec![ContentItem::NoOpenQuote, ContentItem::CloseQuote];
        let pairs = [("A".to_string(), "a".to_string())];
        let mut state = cs();
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &mut state, Some(&pairs));
        assert_eq!(s, "a");
    }

    #[test]
    fn resolve_close_quote_does_not_underflow() {
        // A stray close-quote at depth 0 stays at 0 (css-content-3 §2.4.2).
        let items = vec![ContentItem::CloseQuote];
        let pairs = [("A".to_string(), "a".to_string())];
        let mut state = cs();
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &mut state, Some(&pairs));
        assert_eq!(s, "a");
    }

    #[test]
    fn content_image_url_finds_url() {
        let items = vec![
            ContentItem::String("x".to_string()),
            ContentItem::Url("foo.png".to_string()),
        ];
        assert_eq!(content_image_url(&items), Some("foo.png"));
        assert_eq!(
            content_image_url(&[ContentItem::String("x".to_string())]),
            None
        );
    }

    #[test]
    fn missing_attr_resolves_to_empty() {
        let items = vec![ContentItem::Attr("data-missing".to_string())];
        let mut state = cs();
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &mut state, None);
        assert_eq!(s, "");
    }
}
