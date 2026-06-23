use crate::parser::css::{CssRule, SelectorContext};
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use crate::parser::ttf::TtfFont;
use crate::style::computed::{
    BackgroundOrigin, BackgroundPosition, BackgroundRepeat, BackgroundSize, BoxSizing,
    ComputedStyle, ConicGradient, ContentItem, Display, FontStyle, FontWeight,
    IntrinsicWidthKeyword, LinearGradient, ListStyleType, Position, RadialGradient, VerticalAlign,
    Visibility, compute_style_with_context,
};
use std::collections::HashMap;

use super::context::ContainingBlock;
use super::engine::{CounterState, InlineBox, LayoutBorder, LayoutElement, TextLine, TextRun};
use super::images::build_raster_background_tree;
use super::text::{
    TextWrapOptions, collapse_whitespace, estimate_word_width, push_text_run_with_fallback,
    resolve_style_font_family, resolved_line_height_factor, wrap_text_runs,
};

// ---------------------------------------------------------------------------
// Group 4 — Box sizing
// ---------------------------------------------------------------------------

pub(crate) fn resolve_padding_box_height(
    content_height: f32,
    specified_height: Option<f32>,
    padding_top: f32,
    padding_bottom: f32,
    border_vertical: f32,
    box_sizing: BoxSizing,
) -> f32 {
    let content_based_height = padding_top + content_height + padding_bottom;
    match specified_height {
        Some(height) => {
            // When height is explicitly set, use it (don't expand to fit content).
            // This is essential for overflow: hidden to clip correctly.
            match box_sizing {
                BoxSizing::BorderBox => (height - border_vertical).max(0.0),
                BoxSizing::ContentBox => height + padding_top + padding_bottom,
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
    padding_top: f32,
    padding_bottom: f32,
    border_vertical: f32,
    box_sizing: BoxSizing,
) -> f32 {
    let padding_box = match box_sizing {
        BoxSizing::BorderBox => specified_height - border_vertical,
        BoxSizing::ContentBox => specified_height + padding_top + padding_bottom,
    };
    (padding_box - padding_top - padding_bottom).max(0.0)
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
    children: &[LayoutElement],
    children_height: f32,
    padding_top: f32,
    padding_bottom: f32,
    border_top: f32,
    border_bottom: f32,
) -> f32 {
    let strip_top = padding_top == 0.0 && border_top == 0.0;
    let strip_bottom = padding_bottom == 0.0 && border_bottom == 0.0;
    let first_mt = if strip_top {
        children.first().map_or(0.0, outer_margin_top)
    } else {
        0.0
    };
    let last_mb = if strip_bottom {
        children.last().map_or(0.0, outer_margin_bottom)
    } else {
        0.0
    };
    (children_height - first_mt - last_mb).max(0.0)
}

pub(crate) fn outer_margin_top(el: &LayoutElement) -> f32 {
    match el {
        LayoutElement::TextBlock { margin_top, .. }
        | LayoutElement::Container { margin_top, .. }
        | LayoutElement::FlexRow { margin_top, .. }
        | LayoutElement::GridRow { margin_top, .. }
        | LayoutElement::TableRow { margin_top, .. }
        | LayoutElement::Image { margin_top, .. }
        | LayoutElement::Svg { margin_top, .. }
        | LayoutElement::MathBlock { margin_top, .. } => *margin_top,
        _ => 0.0,
    }
}

pub(crate) fn outer_margin_bottom(el: &LayoutElement) -> f32 {
    match el {
        LayoutElement::TextBlock { margin_bottom, .. }
        | LayoutElement::Container { margin_bottom, .. }
        | LayoutElement::FlexRow { margin_bottom, .. }
        | LayoutElement::GridRow { margin_bottom, .. }
        | LayoutElement::TableRow { margin_bottom, .. }
        | LayoutElement::Image { margin_bottom, .. }
        | LayoutElement::Svg { margin_bottom, .. }
        | LayoutElement::MathBlock { margin_bottom, .. } => *margin_bottom,
        _ => 0.0,
    }
}

/// True for flow-participating block children. Absolute/fixed/float elements
/// don't participate in margin collapsing.
fn is_in_flow_block(el: &LayoutElement) -> bool {
    match el {
        LayoutElement::TextBlock {
            position, float, ..
        }
        | LayoutElement::Container {
            position, float, ..
        } => *position != Position::Absolute && *float == crate::style::computed::Float::None,
        LayoutElement::FlexRow { .. }
        | LayoutElement::GridRow { .. }
        | LayoutElement::TableRow { .. }
        | LayoutElement::Image { .. }
        | LayoutElement::Svg { .. }
        | LayoutElement::MathBlock { .. } => true,
        _ => false,
    }
}

/// Return the index of the first/last in-flow child that participates in
/// margin collapsing. Skips absolute/fixed/float children.
pub(crate) fn first_in_flow_idx(children: &[LayoutElement]) -> Option<usize> {
    children.iter().position(is_in_flow_block)
}

pub(crate) fn last_in_flow_idx(children: &[LayoutElement]) -> Option<usize> {
    children.iter().rposition(is_in_flow_block)
}

/// Take the element's margin-top (and clear it), skipping elements that
/// don't participate in margin collapsing.
pub(crate) fn take_margin_top(el: &mut LayoutElement) -> f32 {
    match el {
        LayoutElement::TextBlock { margin_top, .. }
        | LayoutElement::Container { margin_top, .. }
        | LayoutElement::FlexRow { margin_top, .. }
        | LayoutElement::GridRow { margin_top, .. }
        | LayoutElement::TableRow { margin_top, .. }
        | LayoutElement::Image { margin_top, .. }
        | LayoutElement::Svg { margin_top, .. }
        | LayoutElement::MathBlock { margin_top, .. } => {
            let m = *margin_top;
            *margin_top = 0.0;
            m
        }
        _ => 0.0,
    }
}

pub(crate) fn take_margin_bottom(el: &mut LayoutElement) -> f32 {
    match el {
        LayoutElement::TextBlock { margin_bottom, .. }
        | LayoutElement::Container { margin_bottom, .. }
        | LayoutElement::FlexRow { margin_bottom, .. }
        | LayoutElement::GridRow { margin_bottom, .. }
        | LayoutElement::TableRow { margin_bottom, .. }
        | LayoutElement::Image { margin_bottom, .. }
        | LayoutElement::Svg { margin_bottom, .. }
        | LayoutElement::MathBlock { margin_bottom, .. } => {
            let m = *margin_bottom;
            *margin_bottom = 0.0;
            m
        }
        _ => 0.0,
    }
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
    children: &mut [LayoutElement],
    container_margin_top: &mut f32,
    container_margin_bottom: &mut f32,
    padding_top: f32,
    padding_bottom: f32,
    border_top: f32,
    border_bottom: f32,
    suppress_top: bool,
    suppress_bottom: bool,
) {
    if !suppress_top
        && padding_top == 0.0
        && border_top == 0.0
        && let Some(i) = first_in_flow_idx(children)
    {
        let child_mt = take_margin_top(&mut children[i]);
        *container_margin_top = collapse_margin_pair(*container_margin_top, child_mt);
    }
    if !suppress_bottom
        && padding_bottom == 0.0
        && border_bottom == 0.0
        && let Some(i) = last_in_flow_idx(children)
    {
        let child_mb = take_margin_bottom(&mut children[i]);
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
pub(crate) fn establishes_bfc(style: &ComputedStyle) -> bool {
    style.overflow.clips()
        || style.float != crate::style::computed::Float::None
        || style.position == Position::Absolute
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
    style.padding.left + style.padding.right + style.border.horizontal_width()
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
    let outer_extra = box_horizontal_extra(style) + style.margin.left + style.margin.right;
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
    let italic = style.font_style == FontStyle::Italic;

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
    let italic = style.font_style == FontStyle::Italic;
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
            let stretch = (available_width - style.margin.left - style.margin.right).max(0.0);
            max_content.min(min_content.max(stretch))
        }
    };
    resolved.max(0.0)
}

// ---------------------------------------------------------------------------
// Group 3 — List marker formatting
// ---------------------------------------------------------------------------

pub(crate) fn format_list_marker(list_style_type: ListStyleType, index: usize) -> String {
    match list_style_type {
        ListStyleType::Disc => "\u{2022} ".to_string(),
        ListStyleType::Circle => "\u{25E6} ".to_string(),
        ListStyleType::Square => "\u{25AA} ".to_string(),
        ListStyleType::Decimal => format!("{}. ", index),
        ListStyleType::DecimalLeadingZero => format!("{:02}. ", index),
        ListStyleType::LowerAlpha => format!("{}. ", to_alpha_lower(index)),
        ListStyleType::UpperAlpha => format!("{}. ", to_alpha_upper(index)),
        ListStyleType::LowerRoman => format!("{}. ", to_roman_lower(index)),
        ListStyleType::UpperRoman => format!("{}. ", to_roman_upper(index)),
        ListStyleType::None => String::new(),
    }
}
pub(crate) fn to_alpha_lower(n: usize) -> String {
    if n == 0 {
        return "a".to_string();
    }
    let mut result = String::new();
    let mut val = n;
    while val > 0 {
        val -= 1;
        result.insert(0, (b'a' + (val % 26) as u8) as char);
        val /= 26;
    }
    result
}
pub(crate) fn to_alpha_upper(n: usize) -> String {
    to_alpha_lower(n).to_uppercase()
}
pub(crate) fn to_roman_lower(n: usize) -> String {
    let vals = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut result = String::new();
    let mut remaining = n;
    for &(value, numeral) in &vals {
        while remaining >= value {
            result.push_str(numeral);
            remaining -= value;
        }
    }
    if result.is_empty() {
        "0".to_string()
    } else {
        result
    }
}
pub(crate) fn to_roman_upper(n: usize) -> String {
    to_roman_lower(n).to_uppercase()
}

/// Format a single counter value in the given list-style-type, WITHOUT any
/// marker suffix (unlike `format_list_marker`). Used by `counter()`/`counters()`
/// in the `content` property, where the CSS author supplies their own separators
/// and suffixes. Geometric markers (disc/circle/square) and `none` have no
/// numeric textual form, so they fall back to decimal — matching how browsers
/// render `counter(x, disc)` as the raw number.
pub(crate) fn format_counter_value(style: ListStyleType, value: i32) -> String {
    // Roman/alpha styles are only defined for positive integers; negative or
    // zero values fall back to decimal (CSS counter-style fallback behavior).
    if value <= 0 {
        return value.to_string();
    }
    let n = value as usize;
    match style {
        ListStyleType::DecimalLeadingZero => format!("{n:02}"),
        ListStyleType::LowerAlpha => to_alpha_lower(n),
        ListStyleType::UpperAlpha => to_alpha_upper(n),
        ListStyleType::LowerRoman => to_roman_lower(n),
        ListStyleType::UpperRoman => to_roman_upper(n),
        // decimal, disc, circle, square, none → plain decimal text.
        _ => value.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Group 1 — Pseudo-element helpers
// ---------------------------------------------------------------------------

pub(crate) fn resolve_content(
    items: &[ContentItem],
    attributes: &HashMap<String, String>,
    counter_state: &CounterState,
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
    counter_state: &CounterState,
    quotes: Option<&[(String, String)]>,
) -> String {
    let mut result = String::new();
    let mut depth: usize = 0;
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
            ContentItem::String(s) => result.push_str(s),
            ContentItem::Attr(name) => {
                // Missing attribute resolves to the empty string (css-content-3 §1).
                if let Some(val) = attributes.get(name) {
                    result.push_str(val);
                }
            }
            ContentItem::Counter(name, style) => {
                result.push_str(&format_counter_value(*style, counter_state.get(name)));
            }
            ContentItem::Counters(name, sep, style) => {
                result.push_str(&counter_state.get_all_styled(name, sep, *style));
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
    result
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
    let line_height = resolved_line_height_factor(fl, fonts);
    for run in &mut first.runs {
        // Atomic inline boxes (inline-block / images) are not restyled by
        // ::first-line; only their geometry already participates in the line.
        if run.inline_box.is_some() {
            continue;
        }
        run.color = fl.color.to_f32_rgb();
        run.bold = fl.font_weight == FontWeight::Bold;
        run.italic = fl.font_style == FontStyle::Italic;
        run.underline = fl.text_decoration_underline;
        run.line_through = fl.text_decoration_line_through;
        run.overline = fl.text_decoration_overline;
        run.font_family = family.clone();
        run.background_color = fl.background_color.map(|c| c.to_f32_rgba());
        run.line_height_factor = line_height;
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

/// Split off the leading `::first-letter` unit of the first text-bearing run and
/// restyle it (css-pseudo-4 §2.2). Mutates `runs` in place: the matched run is
/// replaced by an optional leading-whitespace run, the styled first-letter run,
/// and the remainder run. Applies the restricted property set (font/color/
/// decoration/transform). Drop-cap float reservation is not modeled, so the
/// enlarged letter renders inline at the start of the first line.
pub(crate) fn apply_first_letter_style(
    runs: &mut Vec<TextRun>,
    fl: &ComputedStyle,
    fonts: &HashMap<String, TtfFont>,
) {
    // Find the first run carrying renderable text (skip pure-whitespace and
    // atomic-box runs, which precede the first letter, e.g. a ::before marker).
    let Some(pos) = runs
        .iter()
        .position(|r| r.inline_box.is_none() && !r.text.trim().is_empty())
    else {
        return;
    };
    let split = first_letter_len(&runs[pos].text);
    if split == 0 {
        return;
    }
    let base = runs[pos].clone();
    let first_text = base.text[..split].to_string();
    let rest_text = base.text[split..].to_string();

    let mut letter_run = base.clone();
    letter_run.text = apply_text_transform(&first_text, fl.text_transform);
    letter_run.font_size = fl.font_size;
    letter_run.color = fl.color.to_f32_rgb();
    letter_run.bold = fl.font_weight == FontWeight::Bold;
    letter_run.italic = fl.font_style == FontStyle::Italic;
    letter_run.underline = fl.text_decoration_underline;
    letter_run.line_through = fl.text_decoration_line_through;
    letter_run.overline = fl.text_decoration_overline;
    letter_run.font_family = resolve_style_font_family(fl, fonts);
    letter_run.background_color = fl.background_color.map(|c| c.to_f32_rgba());
    letter_run.line_height_factor = resolved_line_height_factor(fl, fonts);

    let mut replacement = Vec::with_capacity(2);
    replacement.push(letter_run);
    if !rest_text.is_empty() {
        let mut rest_run = base;
        rest_run.text = rest_text;
        replacement.push(rest_run);
    }
    runs.splice(pos..=pos, replacement);
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
    runs.iter()
        .map(|run| {
            // Atomic inline boxes (e.g. an image list marker) carry no text but
            // occupy their outer width of inline advance.
            if let Some(inline) = run.inline_box.as_deref() {
                return inline.outer_width();
            }
            estimate_word_width(
                &run.text,
                run.font_size,
                &run.font_family,
                run.bold,
                run.italic,
                fonts,
            )
        })
        .sum()
}

pub(crate) fn pseudo_is_block_like(pseudo_style: &ComputedStyle) -> bool {
    pseudo_style.display == Display::Block || pseudo_style.position == Position::Absolute
}

pub(crate) fn append_pseudo_inline_run(
    runs: &mut Vec<TextRun>,
    pseudo_style: Option<&ComputedStyle>,
    el: &ElementNode,
    fonts: &HashMap<String, TtfFont>,
    counter_state: &CounterState,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn push_block_pseudo(
    output: &mut Vec<LayoutElement>,
    pseudo_style: Option<&ComputedStyle>,
    el: &ElementNode,
    available_width: f32,
    fonts: &HashMap<String, TtfFont>,
    containing_block_info: Option<ContainingBlock>,
    positioned_ancestor_depth: usize,
    counter_state: &CounterState,
) {
    if let Some(pseudo_style) = pseudo_style {
        if pseudo_is_block_like(pseudo_style) {
            let pseudo_cb = if pseudo_style.position == Position::Absolute {
                containing_block_info
            } else {
                None
            };
            output.push(build_pseudo_block(
                pseudo_style,
                el,
                available_width,
                fonts,
                pseudo_cb,
                positioned_ancestor_depth,
                counter_state,
            ));
        }
    }
}

/// Build a `LayoutElement::TextBlock` for a `::before` or `::after` pseudo-element
/// that uses `display: block` (or `position: absolute`).
pub(crate) fn build_pseudo_block(
    pseudo_style: &ComputedStyle,
    el: &ElementNode,
    available_width: f32,
    fonts: &HashMap<String, TtfFont>,
    containing_block_info: Option<ContainingBlock>,
    positioned_ancestor_depth: usize,
    counter_state: &CounterState,
) -> LayoutElement {
    let content_text = resolve_content_with_quotes(
        &pseudo_style.content,
        &el.attributes,
        counter_state,
        pseudo_style.quotes.as_deref(),
    );
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
        block_w
            - pseudo_style.padding.left
            - pseudo_style.padding.right
            - pseudo_style.border.horizontal_width()
    } else {
        block_w - pseudo_style.padding.left - pseudo_style.padding.right
    }
    .max(0.0);

    let mut lines = Vec::new();
    let mut runs = Vec::new();
    if !content_text.is_empty() {
        push_text_run_with_fallback(
            TextRun {
                text: content_text,
                font_size: pseudo_style.font_size,
                bold: pseudo_style.font_weight == FontWeight::Bold,
                italic: pseudo_style.font_style == FontStyle::Italic,
                underline: pseudo_style.text_decoration_underline,
                line_through: pseudo_style.text_decoration_line_through,
                overline: pseudo_style.text_decoration_overline,
                color: pseudo_style.color.to_f32_rgb(),
                link_url: None,
                font_family: resolve_style_font_family(pseudo_style, fonts),
                background_color: None,
                padding: (0.0, 0.0),
                border_radius: 0.0,
                line_height_factor: resolved_line_height_factor(pseudo_style, fonts),
                inline_box: None,
            },
            &mut runs,
            fonts,
        );
        lines = wrap_text_runs(
            runs.clone(),
            TextWrapOptions::new(
                inner_w,
                pseudo_style.font_size,
                resolved_line_height_factor(pseudo_style, fonts),
                pseudo_style.overflow_wrap,
            )
            .with_rtl(pseudo_style.direction_rtl),
            fonts,
        );
    }

    if pseudo_style.position == Position::Absolute
        && pseudo_style.width.is_none()
        && pseudo_style.min_width.is_none()
    {
        let content_w = measure_runs_width(&runs, fonts);
        block_w = if pseudo_style.box_sizing == BoxSizing::BorderBox {
            content_w
                + pseudo_style.padding.left
                + pseudo_style.padding.right
                + pseudo_style.border.horizontal_width()
        } else {
            content_w + pseudo_style.padding.left + pseudo_style.padding.right
        };
    }

    let bg = pseudo_style.background_color.map(|c| c.to_f32_rgba());
    let border = LayoutBorder::from_computed(&pseudo_style.border);
    let BackgroundFields {
        gradient: background_gradient,
        radial_gradient: background_radial_gradient,
        conic_gradient: background_conic_gradient,
        svg: background_svg,
        blur_radius: background_blur_radius,
        size: background_size,
        position: background_position,
        repeat: background_repeat,
        origin: background_origin,
    } = BackgroundFields::from_style(pseudo_style);

    let explicit_width = if pseudo_style.position == Position::Absolute
        || pseudo_style.width.is_some()
        || pseudo_style.min_width.is_some()
    {
        Some(block_w)
    } else {
        None
    };

    let effective_height = {
        let mut h = pseudo_style.height;
        if let Some(cb) = containing_block_info
            && let Some(percent) = pseudo_style.percentage_sizing.height
        {
            h = Some(cb.height * percent / 100.0);
        }
        if let Some(min_h) = pseudo_style.min_height {
            h = Some(h.map_or(min_h, |v| v.max(min_h)));
        }
        if let Some(cb) = containing_block_info
            && let Some(percent) = pseudo_style.percentage_sizing.min_height
        {
            let min_h = cb.height * percent / 100.0;
            h = Some(h.map_or(min_h, |v| v.max(min_h)));
        }
        if let Some(max_h) = pseudo_style.max_height {
            h = h.map(|v| v.min(max_h));
        }
        if let Some(cb) = containing_block_info
            && let Some(percent) = pseudo_style.percentage_sizing.max_height
        {
            let max_h = cb.height * percent / 100.0;
            h = h.map_or(Some(max_h), |v| Some(v.min(max_h)));
        }
        h
    };
    let text_height: f32 = lines.iter().map(|l| l.height).sum();
    let padding_box_height = resolve_padding_box_height(
        text_height,
        effective_height,
        pseudo_style.padding.top,
        pseudo_style.padding.bottom,
        border.vertical_width(),
        pseudo_style.box_sizing,
    );

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

    LayoutElement::TextBlock {
        lines,
        margin_top: pseudo_style.margin.top,
        margin_bottom: pseudo_style.margin.bottom,
        text_align: pseudo_style.text_align,
        background_color: bg,
        padding_top: pseudo_style.padding.top,
        padding_bottom: pseudo_style.padding.bottom,
        padding_left: pseudo_style.padding.left,
        padding_right: pseudo_style.padding.right,
        border,
        block_width: explicit_width,
        block_height: effective_height.map(|_| padding_box_height),
        opacity: pseudo_style.opacity,
        mix_blend_mode: pseudo_style.mix_blend_mode,
        background_blend_mode: pseudo_style.background_blend_mode,
        float: pseudo_style.float,
        clear: pseudo_style.clear,
        position: pseudo_style.position,
        offset_top: resolved_top,
        offset_left: resolved_left,
        offset_bottom: pseudo_style.bottom.unwrap_or(0.0),
        offset_right: pseudo_style.right.unwrap_or(0.0),
        containing_block: containing_block_info,
        box_shadow: pseudo_style.box_shadow.clone(),
        visible: pseudo_style.visibility == Visibility::Visible,
        clip_rect: None,
        transform: pseudo_style.transform,
        transform_origin: pseudo_style.transform_origin,
        border_radius: pseudo_style.border_radius,
        border_radii: pseudo_style.border_radii,
        border_radii_y: pseudo_style.border_radii_y,
        outline_offset: pseudo_style.outline_offset,
        outline_width: pseudo_style.outline_width,
        outline_color: pseudo_style.outline_color.map(|c| c.to_f32_rgb()),
        text_indent: pseudo_style.text_indent,
        letter_spacing: pseudo_style.letter_spacing,
        word_spacing: pseudo_style.word_spacing,
        vertical_align: pseudo_style.vertical_align,
        background_gradient,
        background_radial_gradient,
        background_conic_gradient,
        background_svg,
        background_blur_radius,
        background_size,
        background_position,
        background_repeat,
        background_origin,
        z_index: pseudo_style.z_index,
        repeat_on_each_page: false,
        positioned_depth: if pseudo_style.position == Position::Relative
            || pseudo_style.position == Position::Absolute
        {
            positioned_ancestor_depth + 1
        } else {
            positioned_ancestor_depth
        },
        heading_level: None,
        clip_children_count: 0,
    }
}

/// Build a `TextRun` for an inline `::before` or `::after` pseudo-element.
pub(crate) fn build_pseudo_inline_run(
    pseudo_style: &ComputedStyle,
    el: &ElementNode,
    fonts: &HashMap<String, TtfFont>,
    counter_state: &CounterState,
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
            text: String::new(),
            font_size: pseudo_style.font_size,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            overline: false,
            color: pseudo_style.color.to_f32_rgb(),
            link_url: None,
            font_family: resolve_style_font_family(pseudo_style, fonts),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: resolved_line_height_factor(pseudo_style, fonts),
            inline_box: Some(Box::new(inline)),
        };
    }

    // `display: inline-block` pseudo-elements (e.g. a decorative
    // `::before { content: ""; display: inline-block; width/height/background }`)
    // are atomic boxes, not text. Emit an InlineBox so the box and any inner
    // text paint; a plain text run would drop the box entirely.
    if pseudo_style.display == Display::InlineBlock {
        let inline = build_pseudo_inline_box(pseudo_style, &content_text, fonts);
        return TextRun {
            text: String::new(),
            font_size: pseudo_style.font_size,
            bold: pseudo_style.font_weight == FontWeight::Bold,
            italic: pseudo_style.font_style == FontStyle::Italic,
            underline: false,
            line_through: false,
            overline: false,
            color: pseudo_style.color.to_f32_rgb(),
            link_url: None,
            font_family: resolve_style_font_family(pseudo_style, fonts),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: resolved_line_height_factor(pseudo_style, fonts),
            inline_box: Some(Box::new(inline)),
        };
    }

    TextRun {
        text: content_text,
        font_size: pseudo_style.font_size,
        bold: pseudo_style.font_weight == FontWeight::Bold,
        italic: pseudo_style.font_style == FontStyle::Italic,
        underline: pseudo_style.text_decoration_underline,
        line_through: pseudo_style.text_decoration_line_through,
        overline: pseudo_style.text_decoration_overline,
        color: pseudo_style.color.to_f32_rgb(),
        link_url: None,
        font_family: resolve_style_font_family(pseudo_style, fonts),
        background_color: pseudo_style.background_color.map(|c| c.to_f32_rgba()),
        padding: (0.0, 0.0),
        border_radius: 0.0,
        line_height_factor: resolved_line_height_factor(pseudo_style, fonts),
        inline_box: None,
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
    let border = LayoutBorder::from_computed(&pseudo_style.border);
    let pad_h = pseudo_style.padding.left + pseudo_style.padding.right;
    let pad_v = pseudo_style.padding.top + pseudo_style.padding.bottom;

    // Inner text lines (empty content -> no lines).
    let lines: Vec<TextLine> = if content_text.is_empty() {
        Vec::new()
    } else {
        let run = TextRun {
            text: content_text.to_string(),
            font_size: pseudo_style.font_size,
            bold: pseudo_style.font_weight == FontWeight::Bold,
            italic: pseudo_style.font_style == FontStyle::Italic,
            underline: pseudo_style.text_decoration_underline,
            line_through: pseudo_style.text_decoration_line_through,
            overline: pseudo_style.text_decoration_overline,
            color: pseudo_style.color.to_f32_rgb(),
            link_url: None,
            font_family: resolve_style_font_family(pseudo_style, fonts),
            background_color: None,
            padding: (0.0, 0.0),
            border_radius: 0.0,
            line_height_factor: resolved_line_height_factor(pseudo_style, fonts),
            inline_box: None,
        };
        wrap_text_runs(
            vec![run],
            TextWrapOptions::new(
                f32::MAX,
                pseudo_style.font_size,
                resolved_line_height_factor(pseudo_style, fonts),
                pseudo_style.overflow_wrap,
            ),
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
                        r.italic,
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
        background_color: pseudo_style.background_color.map(|c| c.to_f32_rgba()),
        border,
        border_radius: pseudo_style.border_radius,
        padding_top: pseudo_style.padding.top,
        padding_left: pseudo_style.padding.left,
        vertical_align: pseudo_style.vertical_align,
        baseline_ascent: None,
        lines,
        image: None,
        rel_offset_x: 0.0,
        rel_offset_y: 0.0,
    }
}

/// Build a replaced-image `InlineBox` for a pseudo-element whose `content` is a
/// `url(...)` (css-content-3 §1). The box uses the explicit CSS width/height
/// when given, otherwise the image's intrinsic pixel dimensions. Returns `None`
/// if the image cannot be decoded (the pseudo then produces no box).
fn build_pseudo_image_box(pseudo_style: &ComputedStyle, url: &str) -> Option<InlineBox> {
    let image_src = crate::parser::css::extract_url_path(url).unwrap_or_else(|| url.to_string());
    let (raw, _mime) = crate::layout::images::load_src_bytes(&image_src)?;
    let image = crate::layout::images::load_image_bytes(raw)?;

    let intrinsic_w = image.source_width.max(1) as f32;
    let intrinsic_h = image.source_height.max(1) as f32;
    // Resolve the painted size: explicit dimensions win; a single explicit
    // dimension scales the other by the intrinsic aspect ratio; otherwise use
    // the intrinsic pixel size (CSS px == intrinsic px at 1x).
    let (width, height) = match (pseudo_style.width, pseudo_style.height) {
        (Some(w), Some(h)) => (w.max(0.0), h.max(0.0)),
        (Some(w), None) => (w.max(0.0), w.max(0.0) * intrinsic_h / intrinsic_w),
        (None, Some(h)) => (h.max(0.0) * intrinsic_w / intrinsic_h, h.max(0.0)),
        (None, None) => (intrinsic_w, intrinsic_h),
    };

    Some(InlineBox {
        width,
        height,
        margin_left: pseudo_style.margin.left.max(0.0),
        margin_right: pseudo_style.margin.right.max(0.0),
        background_color: None,
        border: LayoutBorder::from_computed(&pseudo_style.border),
        border_radius: pseudo_style.border_radius,
        padding_top: 0.0,
        padding_left: 0.0,
        vertical_align: pseudo_style.vertical_align,
        baseline_ascent: None,
        lines: Vec::new(),
        image: Some(image),
        rel_offset_x: 0.0,
        rel_offset_y: 0.0,
    })
}

/// Decode a `list-style-image` value (a CSS `url(...)`, possibly a data-URI)
/// into an atomic image `InlineBox` to use as a list marker (css-lists-3 §3.1).
///
/// The marker is sized at the image's intrinsic pixel size (CSS px == intrinsic
/// px at 1x), with a small right margin so the following text does not touch it.
/// Returns `None` when the URL is absent or cannot be decoded, so the caller can
/// fall back to the `list-style-type` glyph marker.
pub(crate) fn build_list_image_marker(value: &str, gap: f32) -> Option<InlineBox> {
    let url = crate::parser::css::extract_url_path(value).unwrap_or_else(|| value.to_string());
    let (raw, _mime) = crate::layout::images::load_src_bytes(&url)?;
    let image = crate::layout::images::load_image_bytes(raw)?;
    let width = image.source_width.max(1) as f32;
    let height = image.source_height.max(1) as f32;
    Some(InlineBox {
        width,
        height,
        margin_left: 0.0,
        margin_right: gap,
        background_color: None,
        border: LayoutBorder::default(),
        border_radius: 0.0,
        padding_top: 0.0,
        padding_left: 0.0,
        vertical_align: VerticalAlign::Baseline,
        baseline_ascent: None,
        lines: Vec::new(),
        image: Some(image),
        rel_offset_x: 0.0,
        rel_offset_y: 0.0,
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
}

impl BackgroundFields {
    pub(crate) fn from_style(style: &ComputedStyle) -> Self {
        Self {
            gradient: style.background_gradient.clone(),
            radial_gradient: style.background_radial_gradient.clone(),
            conic_gradient: style.background_conic_gradient.clone(),
            svg: background_svg_for_style(style),
            blur_radius: style.blur_radius,
            size: style.background_size,
            position: style.background_position,
            repeat: style.background_repeat,
            origin: style.background_origin,
        }
    }

    pub(crate) fn none() -> Self {
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

pub(crate) fn layout_element_paint_order(element: &LayoutElement) -> (i32, i32) {
    match element {
        LayoutElement::TextBlock {
            repeat_on_each_page: true,
            ..
        } => (i32::MIN, 0),
        LayoutElement::TextBlock { z_index, .. } => (0, *z_index),
        _ => (0, 0),
    }
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
/// a `transform` other than `none`. (Filter/perspective/will-change/contain also
/// establish a CB in full CSS but are not modelled here.) This is what lets an
/// absolute box resolve against a transformed non-positioned ancestor.
pub(crate) fn establishes_containing_block(style: &ComputedStyle) -> bool {
    style.position == Position::Relative
        || style.position == Position::Absolute
        || style.transform.is_some()
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
    if style.position == Position::Relative {
        return (None, style.top.unwrap_or(0.0), style.left.unwrap_or(0.0));
    }
    if style.position != Position::Absolute {
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

/// Patch absolute-positioned children in a flattened element list with
/// the parent's containing block info. This resolves bottom/right offsets
/// into top/left and sets the `containing_block` field.
pub(crate) fn patch_absolute_children_containing_block(
    elements: &mut [LayoutElement],
    cb: ContainingBlock,
) {
    for element in elements.iter_mut() {
        if let LayoutElement::TextBlock {
            position,
            containing_block,
            offset_top,
            offset_left,
            offset_bottom,
            offset_right,
            block_width,
            block_height,
            lines,
            padding_top,
            padding_bottom,
            padding_left: _,
            padding_right: _,
            border,
            ..
        } = element
        {
            if *position == Position::Absolute && containing_block.is_none() {
                // Compute element dimensions for right/bottom resolution
                let text_h: f32 = lines.iter().map(|l| l.height).sum();
                let elem_h = block_height
                    .unwrap_or(*padding_top + text_h + *padding_bottom + border.vertical_width());
                let elem_w = block_width.unwrap_or_else(|| {
                    // Estimate width from text content for right-offset resolution
                    lines
                        .iter()
                        .map(|l| {
                            l.runs
                                .iter()
                                .map(|r| {
                                    crate::fonts::str_width(
                                        &r.text,
                                        r.font_size,
                                        &r.font_family,
                                        r.bold,
                                    )
                                })
                                .sum::<f32>()
                        })
                        .fold(0.0f32, f32::max)
                });

                // Resolve right -> left
                if *offset_left == 0.0 && *offset_right > 0.0 {
                    *offset_left = cb.width - elem_w - *offset_right;
                }
                // Resolve bottom -> top
                if *offset_top == 0.0 && *offset_bottom > 0.0 {
                    *offset_top = cb.height - elem_h - *offset_bottom;
                }

                *containing_block = Some(cb);
            }
        } else if let LayoutElement::Container {
            position,
            containing_block,
            ..
        } = element
        {
            // An absolute Container (e.g. an empty `position: absolute` box with
            // a background) carries its own resolved CB from layout; only stamp
            // the parent CB when it has none, so the renderer can anchor it to
            // the correct positioned ancestor by depth. Bottom/right are already
            // resolved into offset_top/left at layout time for Containers.
            if *position == Position::Absolute && containing_block.is_none() {
                *containing_block = Some(cb);
            }
        }
    }
}

#[cfg(test)]
mod generated_content_tests {
    use super::*;
    use crate::style::computed::ContentItem;
    use std::collections::HashMap;

    fn cs() -> CounterState {
        CounterState::default()
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
    fn resolve_quotes_uses_declared_pairs() {
        let items = vec![ContentItem::OpenQuote];
        let pairs = [("<".to_string(), ">".to_string())];
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &cs(), Some(&pairs));
        assert_eq!(s, "<");
    }

    #[test]
    fn resolve_quotes_none_is_empty() {
        let items = vec![ContentItem::OpenQuote, ContentItem::CloseQuote];
        let pairs: [(String, String); 0] = [];
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &cs(), Some(&pairs));
        assert_eq!(s, "");
    }

    #[test]
    fn resolve_quotes_default_when_unset() {
        let items = vec![ContentItem::OpenQuote, ContentItem::CloseQuote];
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &cs(), None);
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
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &cs(), Some(&pairs));
        assert_eq!(s, "ABba");
    }

    #[test]
    fn resolve_no_quote_keywords_track_depth_without_glyphs() {
        // no-open-quote increments depth but emits nothing; close then uses depth 0.
        let items = vec![ContentItem::NoOpenQuote, ContentItem::CloseQuote];
        let pairs = [("A".to_string(), "a".to_string())];
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &cs(), Some(&pairs));
        assert_eq!(s, "a");
    }

    #[test]
    fn resolve_close_quote_does_not_underflow() {
        // A stray close-quote at depth 0 stays at 0 (css-content-3 §2.4.2).
        let items = vec![ContentItem::CloseQuote];
        let pairs = [("A".to_string(), "a".to_string())];
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &cs(), Some(&pairs));
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
        let s = resolve_content_with_quotes(&items, &HashMap::new(), &cs(), None);
        assert_eq!(s, "");
    }
}
