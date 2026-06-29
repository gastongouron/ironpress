use std::collections::HashMap;

use crate::parser::dom::ElementNode;
use crate::types::Color;

/// Context for evaluating CSS media queries against the target page.
#[derive(Debug, Clone, Copy)]
pub struct MediaContext {
    /// Page width in points.
    pub width: f32,
    /// Page height in points.
    pub height: f32,
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
    pub preceding_siblings: Vec<(String, Vec<String>)>,
    /// Following sibling elements for this ancestor within its parent.
    pub following_siblings: Vec<(String, Vec<String>)>,
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
    pub preceding_siblings: Vec<(String, Vec<String>)>,
    /// Following sibling elements (tag name, class list) in document order.
    /// Needed for `:last-of-type`, `:only-of-type`, `:nth-last-of-type`, and
    /// `:has(~ ...)`/`:has(+ ...)` relational matching. Defaults to empty in
    /// layout paths that don't track forward siblings.
    pub following_siblings: Vec<(String, Vec<String>)>,
    /// Whether this element has no element children and no non-whitespace text
    /// (drives `:empty`). Defaults to `false` where not tracked.
    pub is_empty: bool,
}

/// An operator in a calc() expression.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CalcOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// A token in a calc() expression.
#[derive(Debug, Clone)]
pub enum CalcToken {
    /// Absolute length in points.
    Length(f32),
    /// Percentage value (0-100).
    Percent(f32),
    /// Value in em units.
    Em(f32),
    /// Value in rem units.
    Rem(f32),
    /// Value in vw units.
    Vw(f32),
    /// Value in vh units.
    Vh(f32),
    /// Value in vmin units (1% of the smaller viewport axis).
    Vmin(f32),
    /// Value in vmax units (1% of the larger viewport axis).
    Vmax(f32),
    /// An operator.
    Op(CalcOp),
}

/// Parsed CSS property value.
#[derive(Debug, Clone)]
pub enum CssValue {
    Length(f32),
    Color(Color),
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
    /// A calc() expression as a list of tokens.
    Calc(Vec<CalcToken>),
    /// A clamp(min, preferred, max) expression. Each operand is itself a
    /// length-like value (length, percentage, calc, …) resolved lazily so the
    /// percentage basis is known. Resolves to `max(min, min(preferred, max))`.
    Clamp(Box<CssValue>, Box<CssValue>, Box<CssValue>),
    /// A var() reference: (variable_name, optional_fallback).
    Var(String, Option<String>),
}

/// A map of CSS property names to values.
#[derive(Debug, Clone, Default)]
pub struct StyleMap {
    pub properties: HashMap<String, CssValue>,
    pub important: HashMap<String, bool>,
}

impl StyleMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: &str, value: CssValue) {
        self.set_with_importance(key, value, false);
    }

    pub fn set_with_importance(&mut self, key: &str, value: CssValue, is_important: bool) {
        if self.is_important(key) && !is_important {
            return;
        }
        self.properties.insert(key.to_string(), value);
        self.important.insert(key.to_string(), is_important);
    }

    pub fn get(&self, key: &str) -> Option<&CssValue> {
        self.properties.get(key)
    }

    pub fn remove(&mut self, key: &str) {
        self.properties.remove(key);
        self.important.remove(key);
    }

    pub fn is_important(&self, key: &str) -> bool {
        self.important.get(key).copied().unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn merge(&mut self, other: &StyleMap) {
        for (key, value) in &other.properties {
            self.set_with_importance(key, value.clone(), other.is_important(key));
        }
    }
}

/// Pseudo-element type for `::before`, `::after`, and `::marker`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    /// The list-item marker box (`::marker`). Only a limited set of properties
    /// apply (color, font, content); see `compute_pseudo_element_style`.
    Marker,
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
/// `@page { }` (no selector) is [`PageSelector::None`] and applies to every
/// page; the pseudo-class / named variants override per page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PageSelector {
    /// `@page { }` — the default rule, applies to all pages.
    #[default]
    None,
    /// `@page :first { }` — the first page of the document.
    First,
    /// `@page :left { }` — verso (left) pages.
    Left,
    /// `@page :right { }` — recto (right) pages.
    Right,
    /// `@page :blank { }` — intentionally-blank pages.
    Blank,
    /// `@page <name> { }` — a named page targeted by the `page` property.
    Named(String),
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
            Self::TopLeftCorner | Self::TopLeft | Self::BottomLeftCorner | Self::BottomLeft => {
                MarginBoxAlign::Left
            }
            Self::TopCenter | Self::BottomCenter => MarginBoxAlign::Center,
            _ => MarginBoxAlign::Right,
        }
    }
}

/// A token in a margin-box `content` value (CSS Paged Media 3 §5.3). The
/// `content` of a running header/footer is a concatenation of string literals
/// and the page counters `counter(page)` / `counter(pages)`, e.g.
/// `content: "Page " counter(page) " of " counter(pages)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarginContentToken {
    /// A quoted string literal.
    Literal(String),
    /// `counter(page)` — resolved to the 1-based current page index.
    PageNumber,
    /// `counter(pages)` — resolved to the total page count.
    PageCount,
    /// `element(name)` — resolved to a captured `position: running(name)` box.
    Element(String),
}

/// A parsed page-margin box (CSS Paged Media 3 §5): its position and the
/// resolved `content` token list rendered on every page.
#[derive(Debug, Clone)]
pub struct MarginBox {
    /// The box position within the page margin area.
    pub position: MarginBoxPosition,
    /// The `content` value parsed into a token list (literals + counters).
    pub content: Vec<MarginContentToken>,
}

/// A parsed `@page` rule with page size and margin overrides.
#[derive(Debug, Clone, Default)]
pub struct PageRule {
    /// The page selector (`:first`/`:left`/`:right`/`:blank`/name) classified
    /// from the text between `@page` and `{`. [`PageSelector::None`] for an
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
