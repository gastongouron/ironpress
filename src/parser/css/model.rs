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
    /// Font-relative length in em units.
    Em(f32),
    /// Percentage value (0-100).
    Percent(f32),
    /// Value in rem units.
    Rem(f32),
    /// Value in vw units.
    Vw(f32),
    /// Value in vh units.
    Vh(f32),
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
    /// Rem value (relative to root font-size).
    Rem(f32),
    /// Viewport-width percentage.
    Vw(f32),
    /// Viewport-height percentage.
    Vh(f32),
    /// A calc() expression as a list of tokens.
    Calc(Vec<CalcToken>),
    /// A var() reference: (variable_name, optional_fallback).
    Var(String, Option<String>),
}

/// A map of CSS property names to values.
#[derive(Debug, Clone, Default)]
pub struct StyleMap {
    pub properties: HashMap<String, CssValue>,
    pub important: HashMap<String, bool>,
    pub declarations: Vec<(String, CssValue, bool)>,
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
        let key = key.to_string();
        self.properties.insert(key.clone(), value.clone());
        self.important.insert(key.clone(), is_important);
        self.declarations.push((key, value, is_important));
    }

    pub fn get(&self, key: &str) -> Option<&CssValue> {
        self.properties.get(key)
    }

    pub fn is_important(&self, key: &str) -> bool {
        self.important.get(key).copied().unwrap_or(false)
    }

    #[allow(dead_code)]
    pub fn merge(&mut self, other: &StyleMap) {
        for (key, value, is_important) in &other.declarations {
            self.set_with_importance(key, value.clone(), *is_important);
        }
    }
}

/// Pseudo-element type for `::before` and `::after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
}

/// A CSS rule: a selector and its declarations.
#[derive(Debug, Clone)]
pub struct CssRule {
    pub selector: String,
    pub declarations: StyleMap,
    /// If this rule targets a `::before` or `::after` pseudo-element.
    pub pseudo_element: Option<PseudoElement>,
}

/// A parsed `@font-face` rule with font-family name and source path.
#[derive(Debug, Clone)]
pub struct FontFaceRule {
    /// The font-family name declared in the rule.
    pub font_family: String,
    /// The local file path from the `src: url(...)` declaration.
    pub src_path: String,
}

/// A parsed `@import` rule with the local file path.
#[derive(Debug, Clone)]
pub struct ImportRule {
    /// The local file path to import.
    pub path: String,
}

/// A parsed `@page` rule with page size and margin overrides.
#[derive(Debug, Clone, Default)]
pub struct PageRule {
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
}
