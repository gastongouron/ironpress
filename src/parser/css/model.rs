use super::page_descriptors::PageSheetDescriptors;
use crate::parser::dom::ElementNode;
use crate::types::{Color, EdgeSizes};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::{Bound, Deref, RangeBounds};

use super::math::{CssMathExpression, PendingMathExpression};

/// Context for evaluating CSS media queries against the target page.
#[derive(Debug, Clone, Copy)]
pub struct MediaContext {
    /// Page width in points.
    pub width: f32,
    /// Page height in points.
    pub height: f32,
}

/// Tag name and class list of each sibling of an element, in document order.
///
/// Every child's selector context, and every descendant's ancestor chain,
/// carries such lists, so copying them made deep or wide documents
/// quadratic. A parent builds its children's list once, and each child's
/// preceding and following siblings borrow a window of it.
#[derive(Debug, Clone, Default)]
pub struct SiblingNames<'a> {
    names: Cow<'a, [(String, Vec<String>)]>,
}

impl SiblingNames<'_> {
    /// The names in `range` of this list, borrowed from it. A range past the
    /// end is clamped, so the siblings after the last one are empty.
    pub(crate) fn window(&self, range: impl RangeBounds<usize>) -> SiblingNames<'_> {
        let len = self.len();
        let start = match range.start_bound() {
            Bound::Included(&start) => start,
            Bound::Excluded(&start) => start.saturating_add(1),
            Bound::Unbounded => 0,
        }
        .min(len);
        let end = match range.end_bound() {
            Bound::Included(&end) => end.saturating_add(1),
            Bound::Excluded(&end) => end,
            Bound::Unbounded => len,
        }
        .clamp(start, len);
        SiblingNames {
            names: Cow::Borrowed(self.names.get(start..end).unwrap_or_default()),
        }
    }
}

impl Deref for SiblingNames<'_> {
    type Target = [(String, Vec<String>)];

    fn deref(&self) -> &Self::Target {
        &self.names
    }
}

impl PartialEq for SiblingNames<'_> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl From<Vec<(String, Vec<String>)>> for SiblingNames<'_> {
    fn from(names: Vec<(String, Vec<String>)>) -> Self {
        Self {
            names: Cow::Owned(names),
        }
    }
}

impl<'a> From<&'a [(String, Vec<String>)]> for SiblingNames<'a> {
    fn from(names: &'a [(String, Vec<String>)]) -> Self {
        Self {
            names: Cow::Borrowed(names),
        }
    }
}

impl FromIterator<(String, Vec<String>)> for SiblingNames<'_> {
    fn from_iter<I: IntoIterator<Item = (String, Vec<String>)>>(names: I) -> Self {
        names.into_iter().collect::<Vec<_>>().into()
    }
}

/// Per-ancestor context for nth-child matching in descendant selectors.
#[derive(Debug, Clone)]
pub struct AncestorInfo<'a> {
    /// The ancestor element.
    pub element: &'a ElementNode,
    /// Zero-based index of this ancestor among its parent's children.
    pub child_index: usize,
    /// Total number of children in this ancestor's parent.
    pub sibling_count: usize,
    /// Preceding sibling elements for this ancestor within its parent.
    pub preceding_siblings: SiblingNames<'a>,
    /// Following sibling elements for this ancestor within its parent.
    pub following_siblings: SiblingNames<'a>,
    /// Whether this ancestor has no element children / non-whitespace text.
    pub is_empty: bool,
}

/// Context for advanced CSS selector matching.
#[derive(Debug, Clone, Default)]
pub struct SelectorContext<'a> {
    /// Ancestor elements from root to direct parent (outermost first).
    pub ancestors: Vec<AncestorInfo<'a>>,
    /// Zero-based index of this element among its parent's element children.
    pub child_index: usize,
    /// Total number of element children in the parent.
    pub sibling_count: usize,
    /// Preceding sibling elements (tag name, class list) in document order.
    pub preceding_siblings: SiblingNames<'a>,
    /// Following sibling elements (tag name, class list) in document order.
    /// Needed for `:last-of-type`, `:only-of-type`, `:nth-last-of-type`, and
    /// `:has(~ ...)`/`:has(+ ...)` relational matching. Defaults to empty in
    /// layout paths that don't track forward siblings.
    pub following_siblings: SiblingNames<'a>,
    /// Whether this element has no element children and no non-whitespace text
    /// (drives `:empty`). Defaults to `false` where not tracked.
    pub is_empty: bool,
}

impl<'a> SelectorContext<'a> {
    /// Preserve this element's complete selector position when descending into
    /// its children. Descendant matching must retain both sibling directions;
    /// rebuilding an ancestor with zeroed indices makes structural selectors
    /// change meaning at deeper inline-layout levels.
    pub(crate) fn as_ancestor(&self, element: &'a ElementNode) -> AncestorInfo<'a> {
        AncestorInfo {
            element,
            child_index: self.child_index,
            sibling_count: self.sibling_count,
            preceding_siblings: self.preceding_siblings.clone(),
            following_siblings: self.following_siblings.clone(),
            is_empty: self.is_empty,
        }
    }
}

/// A specified CSS color before `currentColor` has been bound to a used
/// foreground color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecifiedColor {
    Absolute(Color),
    CurrentColor,
}

impl SpecifiedColor {
    /// Resolve the CSS `currentColor` keyword against the applicable used
    /// foreground color. Callers handling the `color` property itself pass the
    /// parent's resolved color; every other property passes the element's.
    pub fn resolve(self, current_color: Color) -> Color {
        match self {
            Self::Absolute(color) => color,
            Self::CurrentColor => current_color,
        }
    }
}

impl From<Color> for SpecifiedColor {
    fn from(color: Color) -> Self {
        Self::Absolute(color)
    }
}

/// Parsed CSS property value.
#[derive(Debug, Clone)]
pub enum CssValue {
    Length(f32),
    /// Font-relative `<length>` in `em` units.
    Em(f32),
    Color(SpecifiedColor),
    Keyword(String),
    Number(f32),
    /// Percentage value (0-100 range, e.g. 50% stored as 50.0).
    Percentage(f32),
    /// `ex` unit (css-values-4 §6.1.1): a multiple of the resolved font's
    /// x-height. Stored as the raw coefficient (e.g. `4ex` -> `Ex(4.0)`),
    /// resolved against the font metrics downstream.
    Ex(f32),
    /// `ch` unit (css-values-4 §6.1.1): a multiple of the advance of the `'0'`
    /// glyph in the resolved font. Stored as the raw coefficient.
    Ch(f32),
    /// Rem value (relative to root font-size).
    Rem(f32),
    /// Viewport-width percentage.
    Vw(f32),
    /// Viewport-height percentage.
    Vh(f32),
    /// Percentage of the smaller viewport axis (css-values-4 §6.1.2.2).
    Vmin(f32),
    /// Percentage of the larger viewport axis (css-values-4 §6.1.2.2).
    Vmax(f32),
    /// Grammar-checked CSS math with a `<length-percentage>` result type.
    Math(CssMathExpression),
    /// CSS math waiting for `var()` substitution before grammar checking.
    PendingMath(PendingMathExpression),
    /// A var() reference: (variable_name, optional_fallback).
    Var(String, Option<String>),
    /// Ordered, typed sources from a comma-separated `background-image` value.
    /// This stays inside its owning `StyleMap`; no parser-global capture state is
    /// involved.
    BackgroundLayers(Vec<BackgroundLayerSource>),
}

impl CssValue {
    /// Whether resolving this value may need the font's measured `ex` or
    /// `ch`. Conservative: math, unsubstituted references and background
    /// layers may hold either unit, and so may raw text kept for later
    /// parsing, counted whenever it contains either unit's letters.
    pub(crate) fn may_use_font_metric_units(&self) -> bool {
        match self {
            Self::Ex(_)
            | Self::Ch(_)
            | Self::Math(_)
            | Self::PendingMath(_)
            | Self::Var(..)
            | Self::BackgroundLayers(_) => true,
            Self::Keyword(text) => {
                let text = text.to_ascii_lowercase();
                text.contains("ex") || text.contains("ch")
            }
            Self::Length(_)
            | Self::Em(_)
            | Self::Color(_)
            | Self::Number(_)
            | Self::Percentage(_)
            | Self::Rem(_)
            | Self::Vw(_)
            | Self::Vh(_)
            | Self::Vmin(_)
            | Self::Vmax(_) => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BackgroundLayerSource {
    Raster(String),
    Svg(String),
    Linear(String),
    Radial(String),
    Conic(String),
    None,
}

/// A map of CSS property names to values.
#[derive(Debug, Clone, Default)]
pub struct StyleMap {
    pub properties: HashMap<String, CssValue>,
    pub important: HashMap<String, bool>,
    /// Accepted declaration winners in source order. Replacing a property moves
    /// it to the new declaration position; an ignored lower-priority declaration
    /// leaves the existing position intact. This is required for order-sensitive
    /// shorthands such as `all`.
    pub(crate) declaration_order: Vec<String>,
}

impl StyleMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any declaration may resolve a length against the font's
    /// measured metrics (`ex`, `ch`); see [`CssValue::may_use_font_metric_units`].
    pub(crate) fn may_use_font_metric_units(&self) -> bool {
        self.properties
            .values()
            .any(CssValue::may_use_font_metric_units)
    }

    pub fn set(&mut self, key: &str, value: CssValue) {
        self.set_with_importance(key, value, false);
    }

    pub fn set_with_importance(&mut self, key: &str, value: CssValue, is_important: bool) {
        if self.is_important(key) && !is_important {
            return;
        }
        self.declaration_order.retain(|existing| existing != key);
        self.declaration_order.push(key.to_string());
        self.properties.insert(key.to_string(), value);
        self.important.insert(key.to_string(), is_important);
        if key == "font"
            && let Some(CssValue::Keyword(font)) = self.properties.get(key)
            && let Some(family) = font_shorthand_family(font)
        {
            self.declaration_order
                .retain(|existing| existing != "font-family");
            self.declaration_order.push("font-family".to_string());
            self.properties
                .insert("font-family".to_string(), CssValue::Keyword(family));
            self.important
                .insert("font-family".to_string(), is_important);
        }
    }

    pub fn get(&self, key: &str) -> Option<&CssValue> {
        self.properties.get(key)
    }

    pub fn remove(&mut self, key: &str) {
        self.properties.remove(key);
        self.important.remove(key);
        self.declaration_order.retain(|existing| existing != key);
    }

    pub fn is_important(&self, key: &str) -> bool {
        self.important.get(key).copied().unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn merge(&mut self, other: &StyleMap) {
        for key in &other.declaration_order {
            let Some(value) = other.properties.get(key) else {
                continue;
            };
            let is_important = other.is_important(key);
            self.set_with_importance(key, value.clone(), is_important);
        }
    }
}

fn font_shorthand_family(value: &str) -> Option<String> {
    let mut saw_size = false;
    let mut skip_line_height = false;
    let mut family = Vec::new();

    for token in value.split_whitespace() {
        if !saw_size {
            let size = token.split_once('/').map_or(token, |(size, _)| size);
            if css_font_size_token(size) {
                saw_size = true;
                skip_line_height = token.ends_with('/');
            }
            continue;
        }
        if token == "/" {
            skip_line_height = true;
            continue;
        }
        if skip_line_height {
            skip_line_height = false;
            continue;
        }
        family.push(token);
    }

    let family = family.join(" ");
    (!family.trim().is_empty()).then_some(family)
}

fn css_font_size_token(token: &str) -> bool {
    let token = token.trim();
    token
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
        || matches!(
            token.to_ascii_lowercase().as_str(),
            "xx-small"
                | "x-small"
                | "small"
                | "medium"
                | "large"
                | "x-large"
                | "xx-large"
                | "xxx-large"
                | "smaller"
                | "larger"
        )
}

/// Pseudo-element type supported by the CSS cascade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    /// The list-item marker box (`::marker`). Only a limited set of properties
    /// apply (color, font, content); see `compute_pseudo_element_style`.
    Marker,
    /// The generated in-flow reference for a GCPM `float: footnote` element.
    FootnoteCall,
    /// The generated marker at the beginning of a GCPM footnote body.
    FootnoteMarker,
    /// The first formatted line of a block container (`::first-line`).
    /// Restyles the runs that land on the first wrapped line. Per
    /// css-pseudo-4 §2.1 only a restricted property subset applies.
    FirstLine,
    /// The first typographic letter unit (plus associated leading punctuation)
    /// of the first formatted line (`::first-letter`). Per css-pseudo-4 §2.2
    /// a restricted property subset applies; enables drop-cap styling.
    FirstLetter,
}

/// A CSS rule: a selector and its declarations.
#[derive(Debug, Clone)]
pub struct CssRule {
    pub selector: String,
    pub declarations: StyleMap,
    /// If this rule targets a `::before` or `::after` pseudo-element.
    pub pseudo_element: Option<PseudoElement>,
    /// Whether the selector contains `:has()`, decided once from the selector
    /// text so matching never rescans it.
    uses_has: bool,
}

impl CssRule {
    pub(crate) fn new(
        selector: String,
        declarations: StyleMap,
        pseudo_element: Option<PseudoElement>,
    ) -> Self {
        let uses_has = selector
            .as_bytes()
            .windows(b":has(".len())
            .any(|window| window.eq_ignore_ascii_case(b":has("));
        Self {
            selector,
            declarations,
            pseudo_element,
            uses_has,
        }
    }

    /// Whether the selector contains the relational `:has()` pseudo-class.
    pub(crate) const fn uses_has(&self) -> bool {
        self.uses_has
    }

    pub(crate) fn counter_style_name(&self) -> Option<&str> {
        const AT_RULE: &str = "@counter-style";
        let selector = self.selector.trim();
        let (at_rule, rest) = selector.split_at_checked(AT_RULE.len())?;
        if !at_rule.eq_ignore_ascii_case(AT_RULE) || !rest.chars().next()?.is_whitespace() {
            return None;
        }
        let name = rest.trim();
        (!name.is_empty()).then_some(name)
    }
}

/// A source entry from an `@font-face src:` descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontFaceSource {
    /// `url(...)` source.
    Url(String),
    /// `local(...)` source.
    Local(String),
}

/// A parsed `unicode-range` interval from an `@font-face` descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnicodeRange {
    /// Inclusive first Unicode codepoint.
    pub start: u32,
    /// Inclusive last Unicode codepoint.
    pub end: u32,
}

impl UnicodeRange {
    /// Whether this interval contains `ch`.
    pub const fn contains(self, ch: char) -> bool {
        let codepoint = ch as u32;
        self.start <= codepoint && codepoint <= self.end
    }
}

/// CSS Fonts width class used by the legacy `font-stretch` alias and the
/// `@font-face` descriptor. The declaration values form the discrete part of
/// the CSS Fonts width scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStretch {
    UltraCondensed,
    ExtraCondensed,
    Condensed,
    SemiCondensed,
    #[default]
    Normal,
    SemiExpanded,
    Expanded,
    ExtraExpanded,
    UltraExpanded,
}

impl FontStretch {
    pub(crate) fn from_css(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ultra-condensed" => Some(Self::UltraCondensed),
            "extra-condensed" => Some(Self::ExtraCondensed),
            "condensed" => Some(Self::Condensed),
            "semi-condensed" => Some(Self::SemiCondensed),
            "normal" => Some(Self::Normal),
            "semi-expanded" => Some(Self::SemiExpanded),
            "expanded" => Some(Self::Expanded),
            "extra-expanded" => Some(Self::ExtraExpanded),
            "ultra-expanded" => Some(Self::UltraExpanded),
            _ => None,
        }
    }

    pub(crate) const fn key_suffix(self) -> Option<&'static str> {
        match self {
            Self::UltraCondensed => Some("ultra_condensed"),
            Self::ExtraCondensed => Some("extra_condensed"),
            Self::Condensed => Some("condensed"),
            Self::SemiCondensed => Some("semi_condensed"),
            Self::Normal => None,
            Self::SemiExpanded => Some("semi_expanded"),
            Self::Expanded => Some("expanded"),
            Self::ExtraExpanded => Some("extra_expanded"),
            Self::UltraExpanded => Some("ultra_expanded"),
        }
    }

    /// Width search order for a family with discrete faces.
    ///
    /// CSS Fonts matches width before style and weight. A request at or below
    /// normal first searches narrower widths; an expanded request first
    /// searches wider widths. The requested width remains first in either
    /// direction, so an exact face always wins.
    pub(crate) const fn matching_order(self) -> [Self; 9] {
        use FontStretch::*;
        match self {
            UltraCondensed => [
                UltraCondensed,
                ExtraCondensed,
                Condensed,
                SemiCondensed,
                Normal,
                SemiExpanded,
                Expanded,
                ExtraExpanded,
                UltraExpanded,
            ],
            ExtraCondensed => [
                ExtraCondensed,
                UltraCondensed,
                Condensed,
                SemiCondensed,
                Normal,
                SemiExpanded,
                Expanded,
                ExtraExpanded,
                UltraExpanded,
            ],
            Condensed => [
                Condensed,
                ExtraCondensed,
                UltraCondensed,
                SemiCondensed,
                Normal,
                SemiExpanded,
                Expanded,
                ExtraExpanded,
                UltraExpanded,
            ],
            SemiCondensed => [
                SemiCondensed,
                Condensed,
                ExtraCondensed,
                UltraCondensed,
                Normal,
                SemiExpanded,
                Expanded,
                ExtraExpanded,
                UltraExpanded,
            ],
            Normal => [
                Normal,
                SemiCondensed,
                Condensed,
                ExtraCondensed,
                UltraCondensed,
                SemiExpanded,
                Expanded,
                ExtraExpanded,
                UltraExpanded,
            ],
            SemiExpanded => [
                SemiExpanded,
                Expanded,
                ExtraExpanded,
                UltraExpanded,
                Normal,
                SemiCondensed,
                Condensed,
                ExtraCondensed,
                UltraCondensed,
            ],
            Expanded => [
                Expanded,
                ExtraExpanded,
                UltraExpanded,
                SemiExpanded,
                Normal,
                SemiCondensed,
                Condensed,
                ExtraCondensed,
                UltraCondensed,
            ],
            ExtraExpanded => [
                ExtraExpanded,
                UltraExpanded,
                Expanded,
                SemiExpanded,
                Normal,
                SemiCondensed,
                Condensed,
                ExtraCondensed,
                UltraCondensed,
            ],
            UltraExpanded => [
                UltraExpanded,
                ExtraExpanded,
                Expanded,
                SemiExpanded,
                Normal,
                SemiCondensed,
                Condensed,
                ExtraCondensed,
                UltraCondensed,
            ],
        }
    }
}

/// A parsed `@font-face` rule with font-family name, source list, and descriptors.
#[derive(Debug, Clone)]
pub struct FontFaceRule {
    /// The font-family name declared in the rule.
    pub font_family: String,
    /// The ordered source list from the `src:` descriptor.
    pub sources: Vec<FontFaceSource>,
    /// Whether the face descriptor declares a bold weight.
    pub font_weight_bold: bool,
    /// Whether the face descriptor declares italic/oblique style.
    pub font_style_italic: bool,
    /// Width class advertised by the face's `font-stretch` descriptor.
    pub font_stretch: FontStretch,
    /// CSS Fonts `size-adjust` descriptor as a multiplier (`normal` = 1.0).
    pub size_adjust: f32,
    /// The `unicode-range` intervals. Empty means the default full Unicode range.
    pub unicode_ranges: Vec<UnicodeRange>,
}

impl FontFaceRule {
    /// Iterate source entries as `(is_local, value)`, preserving source-list order.
    pub fn source_entries(&self) -> impl Iterator<Item = (bool, &str)> {
        self.sources.iter().map(|source| match source {
            FontFaceSource::Local(name) => (true, name.as_str()),
            FontFaceSource::Url(path) => (false, path.as_str()),
        })
    }

    /// Iterate `local(...)` source names.
    pub fn local_source_names(&self) -> impl Iterator<Item = &str> {
        self.source_entries()
            .filter_map(|(is_local, value)| is_local.then_some(value))
    }
}

/// A parsed `@import` rule with the local file path.
#[derive(Debug, Clone)]
pub struct ImportRule {
    /// The local file path to import.
    pub path: String,
}

/// The selector of an `@page` rule — the text between `@page` and `{`
/// (CSS Paged Media 3 §3 "Page selectors and the page context").
///
/// A page type name combined with zero or more page pseudo-classes.
///
/// CSS Paged Media permits compounds such as `chapter:left:first` and repeated
/// pseudo-classes contribute repeatedly to specificity. Keeping both facts in
/// one semantic value prevents parsing from discarding half of a selector.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageSelector {
    name: Option<String>,
    pseudo_classes: PagePseudoClasses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PagePseudoClasses {
    first: u32,
    blank: u32,
    left: u32,
    right: u32,
}

/// The physical-page facts against which an [`PageSelector`] is matched.
///
/// Keeping this independent from layout's `Page` type lets parsing, pagination,
/// and PDF painting share the same selector semantics without a module cycle.
#[derive(Debug, Clone, Copy)]
pub struct PageSelectorContext<'a> {
    pub page_number: usize,
    pub is_blank: bool,
    pub page_name: Option<&'a str>,
}

impl PageSelector {
    pub const fn universal() -> Self {
        Self {
            name: None,
            pseudo_classes: PagePseudoClasses {
                first: 0,
                blank: 0,
                left: 0,
                right: 0,
            },
        }
    }

    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub const fn first() -> Self {
        Self::universal().with_first()
    }

    #[cfg(test)]
    pub const fn left() -> Self {
        Self::universal().with_left()
    }

    #[cfg(test)]
    pub const fn right() -> Self {
        Self::universal().with_right()
    }

    #[cfg(test)]
    pub const fn blank() -> Self {
        Self::universal().with_blank()
    }

    pub const fn with_first(mut self) -> Self {
        self.pseudo_classes.first = self.pseudo_classes.first.saturating_add(1);
        self
    }

    pub const fn with_blank(mut self) -> Self {
        self.pseudo_classes.blank = self.pseudo_classes.blank.saturating_add(1);
        self
    }

    pub const fn with_left(mut self) -> Self {
        self.pseudo_classes.left = self.pseudo_classes.left.saturating_add(1);
        self
    }

    pub const fn with_right(mut self) -> Self {
        self.pseudo_classes.right = self.pseudo_classes.right.saturating_add(1);
        self
    }

    pub const fn is_universal(&self) -> bool {
        self.name.is_none()
            && self.pseudo_classes.first == 0
            && self.pseudo_classes.blank == 0
            && self.pseudo_classes.left == 0
            && self.pseudo_classes.right == 0
    }

    /// Whether this selector applies to a physical page.
    pub fn applies_to(&self, page: PageSelectorContext<'_>) -> bool {
        self.name
            .as_deref()
            .is_none_or(|name| page.page_name == Some(name))
            && (self.pseudo_classes.first == 0 || page.page_number == 1)
            && (self.pseudo_classes.blank == 0 || page.is_blank)
            && (self.pseudo_classes.left == 0 || page.page_number % 2 == 0)
            && (self.pseudo_classes.right == 0 || page.page_number % 2 != 0)
    }

    /// CSS Paged Media's `(f, g, h)` page-selector specificity.
    pub const fn specificity(&self) -> (u32, u32, u32) {
        (
            if self.name.is_some() { 1 } else { 0 },
            self.pseudo_classes
                .first
                .saturating_add(self.pseudo_classes.blank),
            self.pseudo_classes
                .left
                .saturating_add(self.pseudo_classes.right),
        )
    }
}

/// Declarations that participate in the inherited text style of an `@page`
/// context or a nested page-margin box.
///
/// The shared declaration map deliberately retains standard font shorthands
/// and relative values so they are resolved through the same computed-style
/// machinery as document text, after the applicable page selector is known.
#[derive(Debug, Clone, Default)]
pub struct PageTextStyle {
    pub declarations: StyleMap,
}

impl PageTextStyle {
    pub fn is_empty(&self) -> bool {
        self.declarations.properties.is_empty()
    }
}

/// The position of a page-margin box inside the `@page` context
/// (CSS Paged Media 3 §5 "Page-margin boxes"). The 16 boxes are arranged
/// around the page border: a top and bottom row (corners + left/center/right),
/// and left/right side columns (top/middle/bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginBoxPosition {
    TopLeftCorner,
    TopLeft,
    TopCenter,
    TopRight,
    TopRightCorner,
    BottomLeftCorner,
    BottomLeft,
    BottomCenter,
    BottomRight,
    BottomRightCorner,
    LeftTop,
    LeftMiddle,
    LeftBottom,
    RightTop,
    RightMiddle,
    RightBottom,
}

/// Which horizontal band (top vs bottom margin area) a margin box renders in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginBoxBand {
    Top,
    Bottom,
}

/// Horizontal alignment of a margin box within its band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginBoxAlign {
    Left,
    Center,
    Right,
}

impl MarginBoxPosition {
    /// Map an `@<ident>` margin-box at-rule name to its position.
    pub fn from_at_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "top-left-corner" => Some(Self::TopLeftCorner),
            "top-left" => Some(Self::TopLeft),
            "top-center" => Some(Self::TopCenter),
            "top-right" => Some(Self::TopRight),
            "top-right-corner" => Some(Self::TopRightCorner),
            "bottom-left-corner" => Some(Self::BottomLeftCorner),
            "bottom-left" => Some(Self::BottomLeft),
            "bottom-center" => Some(Self::BottomCenter),
            "bottom-right" => Some(Self::BottomRight),
            "bottom-right-corner" => Some(Self::BottomRightCorner),
            "left-top" => Some(Self::LeftTop),
            "left-middle" => Some(Self::LeftMiddle),
            "left-bottom" => Some(Self::LeftBottom),
            "right-top" => Some(Self::RightTop),
            "right-middle" => Some(Self::RightMiddle),
            "right-bottom" => Some(Self::RightBottom),
            _ => None,
        }
    }

    /// The horizontal band (top/bottom margin area) this box paints in, if it
    /// is a top- or bottom-row box. Side boxes (`left-*`/`right-*`) return
    /// `None` and are not rendered as running headers/footers.
    pub fn band(self) -> Option<MarginBoxBand> {
        match self {
            Self::TopLeftCorner
            | Self::TopLeft
            | Self::TopCenter
            | Self::TopRight
            | Self::TopRightCorner => Some(MarginBoxBand::Top),
            Self::BottomLeftCorner
            | Self::BottomLeft
            | Self::BottomCenter
            | Self::BottomRight
            | Self::BottomRightCorner => Some(MarginBoxBand::Bottom),
            _ => None,
        }
    }

    /// The horizontal alignment of this box within its band.
    pub fn align(self) -> MarginBoxAlign {
        match self {
            Self::TopLeftCorner | Self::BottomLeftCorner => MarginBoxAlign::Right,
            Self::TopLeft | Self::BottomLeft => MarginBoxAlign::Left,
            Self::TopCenter | Self::BottomCenter => MarginBoxAlign::Center,
            Self::TopRight | Self::BottomRight => MarginBoxAlign::Right,
            Self::TopRightCorner | Self::BottomRightCorner => MarginBoxAlign::Left,
            Self::LeftTop
            | Self::LeftMiddle
            | Self::LeftBottom
            | Self::RightTop
            | Self::RightMiddle
            | Self::RightBottom => MarginBoxAlign::Center,
        }
    }
}

/// A token in a margin-box `content` value (CSS Paged Media 3 §5.3). The
/// `content` of a running header/footer is a concatenation of string literals
/// and the page counters `counter(page)` / `counter(pages)`, e.g.
/// `content: "Page " counter(page) " of " counter(pages)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PageContentPolicy {
    #[default]
    First,
    Start,
    Last,
    FirstExcept,
}

impl PageContentPolicy {
    pub(crate) fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None => Some(Self::First),
            Some(value) if value.eq_ignore_ascii_case("first") => Some(Self::First),
            Some(value) if value.eq_ignore_ascii_case("start") => Some(Self::Start),
            Some(value) if value.eq_ignore_ascii_case("last") => Some(Self::Last),
            Some(value) if value.eq_ignore_ascii_case("first-except") => Some(Self::FirstExcept),
            Some(_) => None,
        }
    }
}

/// A prevalidated reference to page-scoped generated content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageContentReference {
    name: String,
    policy: PageContentPolicy,
}

impl PageContentReference {
    pub(crate) fn new(name: String, policy: PageContentPolicy) -> Self {
        Self { name, policy }
    }

    pub(crate) fn parse(name: &str, policy: Option<&str>) -> Option<Self> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        Some(Self::new(
            name.to_ascii_lowercase(),
            PageContentPolicy::parse(policy)?,
        ))
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) const fn policy(&self) -> PageContentPolicy {
        self.policy
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarginContentToken {
    /// A quoted string literal.
    Literal(String),
    /// `counter(page)` — resolved to the 1-based current page index.
    PageNumber,
    /// `counter(pages)` — resolved to the total page count.
    PageCount,
    /// `element(name)` — resolved to a captured `position: running(name)` box.
    Element(PageContentReference),
    /// `string(name, page-policy)` — resolved from `string-set` captures.
    NamedString(PageContentReference),
}

/// A parsed page-margin box (CSS Paged Media 3 §5): its position and the
/// resolved `content` token list rendered on every page.
#[derive(Debug, Clone)]
pub struct MarginBox {
    /// The box position within the page margin area.
    pub position: MarginBoxPosition,
    /// The page selector that owns this margin box.
    pub selector: PageSelector,
    /// Page-context controls for the implicit `page` counter.
    pub page_counter: PageCounterControl,
    pub color: Option<Color>,
    pub background_color: Option<Color>,
    /// Text declarations in this margin context. They override inherited page
    /// context values just as declarations on ordinary generated content do.
    pub text_style: PageTextStyle,
    /// The used inline size when `width` is explicitly declared, in points.
    /// `None` retains the Page 3 automatic margin-box dimension algorithm.
    pub width: Option<f32>,
    /// The `content` value parsed into a token list (literals + counters).
    pub content: Vec<MarginContentToken>,
}

/// CSS page-context operations on the implicit `page` counter.
///
/// Page contexts are established separately for every physical page. A reset
/// consequently applies anew on each page; it is not a document-global origin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageCounterControl {
    /// The value established by `counter-reset: page <integer>`.
    pub reset: Option<i32>,
    /// The value added by `counter-increment: page <integer>`.
    pub increment: Option<i32>,
}

impl PageCounterControl {
    /// Resolve the implicit `page` counter for a one-based physical page.
    ///
    /// Without a reset, the implicit counter progresses across pages. A reset
    /// belongs to each independently established page context, so it starts
    /// every page at the same value. Only an explicit increment then modifies
    /// that page-local reset.
    pub fn value_on_page(self, page_number: usize) -> usize {
        let value = match self.reset {
            Some(reset) => i128::from(reset) + i128::from(self.increment.unwrap_or(0)),
            None => {
                let page_number = i128::try_from(page_number).unwrap_or(i128::MAX);
                i128::from(self.increment.unwrap_or(1)) * page_number
            }
        };
        usize::try_from(value.max(0)).unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
mod page_counter_control_tests {
    use super::PageCounterControl;

    #[test]
    fn reset_applies_to_each_page_context() {
        let counter = PageCounterControl {
            reset: Some(7),
            ..PageCounterControl::default()
        };
        assert_eq!(counter.value_on_page(1), 7);
        assert_eq!(counter.value_on_page(2), 7);
    }

    #[test]
    fn explicit_increment_replaces_the_implicit_step() {
        let counter = PageCounterControl {
            increment: Some(2),
            ..PageCounterControl::default()
        };
        assert_eq!(counter.value_on_page(1), 2);
        assert_eq!(counter.value_on_page(3), 6);
    }

    #[test]
    fn explicit_increment_follows_a_page_local_reset() {
        let counter = PageCounterControl {
            reset: Some(7),
            increment: Some(2),
        };
        assert_eq!(counter.value_on_page(1), 9);
        assert_eq!(counter.value_on_page(3), 9);
    }
}

/// Physical edge declarations that retain whether each CSS longhand was
/// specified. This lets independently cascaded rules overlay only the edges
/// they actually declare before resolving to [`EdgeSizes`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SpecifiedEdgeSizes {
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

impl From<EdgeSizes> for SpecifiedEdgeSizes {
    fn from(edges: EdgeSizes) -> Self {
        Self {
            top: Some(edges.top),
            right: Some(edges.right),
            bottom: Some(edges.bottom),
            left: Some(edges.left),
        }
    }
}

impl SpecifiedEdgeSizes {
    /// Cascade later declarations over this set without inventing unspecified
    /// physical edges.
    pub(crate) fn cascade(&mut self, later: Self) {
        if later.top.is_some() {
            self.top = later.top;
        }
        if later.right.is_some() {
            self.right = later.right;
        }
        if later.bottom.is_some() {
            self.bottom = later.bottom;
        }
        if later.left.is_some() {
            self.left = later.left;
        }
    }

    /// Overlay the declared edges onto already-resolved values.
    pub(crate) fn apply_to(self, resolved: &mut EdgeSizes) {
        if let Some(value) = self.top {
            resolved.top = value;
        }
        if let Some(value) = self.right {
            resolved.right = value;
        }
        if let Some(value) = self.bottom {
            resolved.bottom = value;
        }
        if let Some(value) = self.left {
            resolved.left = value;
        }
    }
}

/// Declarations for the top separator between page content and footnotes.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FootnoteSeparatorStyle {
    pub width: Option<f32>,
    pub color: Option<Color>,
}

impl FootnoteSeparatorStyle {
    /// Cascade later separator declarations over this one.
    pub(crate) fn cascade(&mut self, later: Self) {
        if later.width.is_some() {
            self.width = later.width;
        }
        if later.color.is_some() {
            self.color = later.color;
        }
    }
}

/// Declarations from the GCPM `@footnote` area rule that affect pagination and
/// painting of the footnote area.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FootnoteAreaStyle {
    pub max_height: Option<f32>,
    pub padding: SpecifiedEdgeSizes,
    pub separator: FootnoteSeparatorStyle,
}

impl FootnoteAreaStyle {
    /// Cascade a later `@footnote` rule in the same page context.
    pub(crate) fn cascade(&mut self, later: Self) {
        if later.max_height.is_some() {
            self.max_height = later.max_height;
        }
        self.padding.cascade(later.padding);
        self.separator.cascade(later.separator);
    }
}

/// A parsed `@page` rule with page size and margin overrides.
#[derive(Debug, Clone, Default)]
pub struct PageRule {
    /// The page selector (`:first`/`:left`/`:right`/`:blank`/name) classified
    /// from the text between `@page` and `{`. A universal selector for an
    /// unselected `@page { }` rule that applies to every page.
    pub selector: PageSelector,
    /// Page width in points (if specified).
    pub width: Option<f32>,
    /// Page height in points (if specified).
    pub height: Option<f32>,
    /// Top margin in points (if specified).
    pub margin_top: Option<f32>,
    /// Right margin in points (if specified).
    pub margin_right: Option<f32>,
    /// Bottom margin in points (if specified).
    pub margin_bottom: Option<f32>,
    /// Left margin in points (if specified).
    pub margin_left: Option<f32>,
    /// Controls for the implicit `page` counter in this page context.
    pub page_counter: PageCounterControl,
    /// Text declarations in this page context, inherited by its page-margin
    /// boxes after the page-selector cascade has selected this physical page.
    pub text_style: PageTextStyle,
    /// Declarations from a GCPM `@footnote` area rule.
    pub(crate) footnote_area: Option<FootnoteAreaStyle>,
    /// Physical-sheet declarations parsed at the CSS boundary.
    pub(crate) sheet: PageSheetDescriptors,
    /// The raw declaration block of the `@page` rule (the text between `{` and
    /// `}`), retained verbatim so a CSS-aware parser can later extract the
    /// `@page` background (CSS Paged Media 3 §3.1 bleed-area background). Kept
    /// raw — rather than pre-split on `;` like size/margin — so data-URI values
    /// containing `;` (e.g. `;base64,`) survive intact.
    pub raw_declarations: Option<String>,
    /// Parsed page-margin boxes (CSS Paged Media 3 §5) — the `@top-center`,
    /// `@bottom-center`, etc. at-rules nested in this `@page` block, used for
    /// running headers/footers and page numbering.
    pub margin_boxes: Vec<MarginBox>,
}
