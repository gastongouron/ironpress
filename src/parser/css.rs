use std::collections::HashMap;

use crate::parser::dom::ElementNode;
use crate::types::Color;

/// Context for advanced CSS selector matching (descendant, child, pseudo-class).
#[derive(Debug, Clone, Default)]
pub struct SelectorContext<'a> {
    /// Ancestor elements from root to direct parent (outermost first).
    pub ancestors: Vec<&'a ElementNode>,
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
}

impl StyleMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: &str, value: CssValue) {
        self.properties.insert(key.to_string(), value);
    }

    pub fn get(&self, key: &str) -> Option<&CssValue> {
        self.properties.get(key)
    }

    #[allow(dead_code)]
    pub fn merge(&mut self, other: &StyleMap) {
        for (k, v) in &other.properties {
            self.properties.insert(k.clone(), v.clone());
        }
    }
}

/// Parse an inline CSS style string (e.g. "color: red; font-size: 14px").
pub fn parse_inline_style(style: &str) -> StyleMap {
    let mut map = StyleMap::new();

    for declaration in style.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }

        if let Some((prop, val)) = declaration.split_once(':') {
            let raw_prop = prop.trim();
            let val = val.trim();

            // Handle CSS custom properties (--*)
            if raw_prop.starts_with("--") {
                map.set(raw_prop, CssValue::Keyword(val.to_string()));
                continue;
            }

            let prop = raw_prop.to_ascii_lowercase();

            if (prop == "margin" || prop == "padding") && !prop.contains('-') {
                let parts: Vec<&str> = val.split_whitespace().collect();
                if parts.len() > 1 {
                    let (top, right, bottom, left) = match parts.len() {
                        2 => (parts[0], parts[1], parts[0], parts[1]),
                        3 => (parts[0], parts[1], parts[2], parts[1]),
                        4 => (parts[0], parts[1], parts[2], parts[3]),
                        _ => continue,
                    };
                    for (side, token) in [
                        ("top", top),
                        ("right", right),
                        ("bottom", bottom),
                        ("left", left),
                    ] {
                        let key = format!("{prop}-{side}");
                        if token == "auto" {
                            map.set(&key, CssValue::Keyword("auto".to_string()));
                        } else if let Some(len) = parse_length(token) {
                            map.set(&key, len);
                        }
                    }
                } else if val.trim() == "auto" {
                    for side in &["top", "right", "bottom", "left"] {
                        map.set(
                            &format!("{prop}-{side}"),
                            CssValue::Keyword("auto".to_string()),
                        );
                    }
                } else if let Some(CssValue::Length(v)) = parse_value(&prop, val) {
                    map.set(&format!("{prop}-top"), CssValue::Length(v));
                    map.set(&format!("{prop}-right"), CssValue::Length(v));
                    map.set(&format!("{prop}-bottom"), CssValue::Length(v));
                    map.set(&format!("{prop}-left"), CssValue::Length(v));
                }
            } else if (prop == "margin-left" || prop == "margin-right") && val.trim() == "auto" {
                map.set(&prop, CssValue::Keyword("auto".to_string()));
            } else if (prop == "background" || prop == "background-image")
                && val.trim_start().starts_with("linear-gradient(")
            {
                // Store the full gradient function string for later parsing
                map.set(
                    "background-gradient",
                    CssValue::Keyword(val.trim().to_string()),
                );
            } else if (prop == "background" || prop == "background-image")
                && val.trim_start().starts_with("radial-gradient(")
            {
                map.set(
                    "background-radial-gradient",
                    CssValue::Keyword(val.trim().to_string()),
                );
            } else if let Some(css_val) = parse_value(&prop, val) {
                map.set(&prop, css_val);
            }
        }
    }

    map
}

fn parse_value(property: &str, val: &str) -> Option<CssValue> {
    let val = val.trim();

    // Handle var() references for any property
    if let Some(var_val) = parse_var_function(val) {
        return Some(var_val);
    }

    // Handle calc() expressions for any property that accepts lengths
    if val.starts_with("calc(") {
        if let Some(calc_val) = parse_calc_expression(val) {
            return Some(calc_val);
        }
    }

    // Handle inherit, initial, unset keywords for any property
    {
        let lower = val.to_ascii_lowercase();
        if lower == "inherit" || lower == "initial" || lower == "unset" {
            return Some(CssValue::Keyword(lower));
        }
    }

    // Color properties
    if property.contains("color") {
        return parse_color(val);
    }

    // Font-weight
    if property == "font-weight" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Font-style
    if property == "font-style" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Font-family — store the first font name (before any comma fallback list)
    if property == "font-family" {
        let first = val.split(',').next().unwrap_or(val).trim();
        return Some(CssValue::Keyword(first.to_string()));
    }

    // Text-align, text-decoration, display
    if property == "text-align" || property == "text-decoration" || property == "display" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Page break
    if property.starts_with("page-break") {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Border shorthand and individual border properties
    if property == "border" || property == "border-style" {
        return Some(CssValue::Keyword(val.to_string()));
    }
    if property == "border-width" {
        return parse_length(val);
    }
    if property == "border-color" {
        return parse_color(val);
    }

    // z-index — integer value
    if property == "z-index" {
        if val == "auto" {
            return Some(CssValue::Keyword("auto".to_string()));
        }
        if let Ok(n) = val.parse::<i32>() {
            return Some(CssValue::Number(n as f32));
        }
        return None;
    }

    // Float, clear, position — keyword properties
    if property == "float" || property == "clear" || property == "position" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Flex properties — keyword values
    if property == "flex-direction"
        || property == "justify-content"
        || property == "align-items"
        || property == "flex-wrap"
    {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Gap — length value
    if property == "gap" {
        return parse_length(val);
    }

    // Content property (for ::before / ::after pseudo-elements)
    if property == "content" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Counter properties
    if property == "counter-reset" || property == "counter-increment" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // List style properties
    if property == "list-style-type"
        || property == "list-style-position"
        || property == "list-style"
    {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Overflow, visibility — keyword properties
    if property == "overflow" || property == "visibility" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Transform — store as keyword (full function string)
    if property == "transform" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Grid template columns — store as keyword for later parsing
    if property == "grid-template-columns" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Grid gap — parse as length
    if property == "grid-gap" {
        return parse_length(val);
    }

    // Box-shadow — store as keyword (full shorthand string)
    if property == "box-shadow" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Border-radius — parse as length (single value shorthand)
    if property == "border-radius" {
        return parse_length(val);
    }

    // Outline shorthand — store as keyword (full shorthand string)
    if property == "outline" {
        return Some(CssValue::Keyword(val.to_string()));
    }
    if property == "outline-width" {
        return parse_length(val);
    }
    if property == "outline-color" {
        return parse_color(val);
    }

    // Box-sizing — keyword
    if property == "box-sizing" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Text-overflow — keyword
    if property == "text-overflow" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Border-collapse — keyword
    if property == "border-collapse" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Border-spacing — length value (single or two values; store first)
    if property == "border-spacing" {
        let parts: Vec<&str> = val.split_whitespace().collect();
        // Use the first value (horizontal); vertical is the same for single value
        return parse_length(parts.first().unwrap_or(&val));
    }

    // Background-size — keyword or explicit values
    if property == "background-size" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Background-repeat — keyword
    if property == "background-repeat" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Background-position — keyword/values
    if property == "background-position" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // White-space — keyword
    if property == "white-space" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Text-transform — keyword
    if property == "text-transform" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Length values (font-size, margin, padding, width, height, top, left, etc.)
    parse_length(val)
}

/// Preprocess CSS to handle @media queries.
/// - `@media print { ... }` => extract inner rules (we are a print renderer)
/// - `@media screen { ... }` => skip entirely
/// - Other @media blocks => skip
fn preprocess_media_queries(css: &str) -> String {
    let mut result = String::new();
    let mut chars = css.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch == '@' {
            // Collect the @-rule up to '{'
            let mut at_rule = String::new();
            while let Some(&c) = chars.peek() {
                if c == '{' {
                    break;
                }
                at_rule.push(c);
                chars.next();
            }

            let at_rule_lower = at_rule.trim().to_ascii_lowercase();

            if at_rule_lower.starts_with("@media") {
                // Consume the opening '{'
                if chars.peek() == Some(&'{') {
                    chars.next();
                }

                // Extract the content inside the @media block, handling nested braces
                let inner = extract_braced_content(&mut chars);

                let media_type = at_rule_lower.trim_start_matches("@media").trim();
                if media_type == "print" {
                    // Include inner rules for print media
                    result.push_str(&inner);
                    result.push(' ');
                }
                // For "screen" and any other media type, skip the inner rules
            } else if at_rule_lower.starts_with("@page") || at_rule_lower.starts_with("@font-face")
            {
                // Pass through @page and @font-face rules with their braces and content
                result.push_str(&at_rule);
                if chars.peek() == Some(&'{') {
                    result.push('{');
                    chars.next();
                    let inner = extract_braced_content(&mut chars);
                    result.push_str(&inner);
                    result.push('}');
                }
            } else if at_rule_lower.starts_with("@import") {
                // Pass through @import rules as-is (they end at ';')
                result.push_str(&at_rule);
                // Consume up to and including the ';'
                while let Some(&c) = chars.peek() {
                    result.push(c);
                    chars.next();
                    if c == ';' {
                        break;
                    }
                }
            } else {
                // Non-media @-rules: pass through as-is
                result.push_str(&at_rule);
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }

    result
}

/// Extract content inside braces, handling nested brace pairs.
/// Assumes the opening '{' has already been consumed.
fn extract_braced_content(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut content = String::new();
    let mut depth = 1;

    for c in chars.by_ref() {
        if c == '{' {
            depth += 1;
            content.push(c);
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                break;
            }
            content.push(c);
        } else {
            content.push(c);
        }
    }

    content
}

fn parse_length(val: &str) -> Option<CssValue> {
    let val = val.trim();

    // Handle var() references
    if let Some(var_val) = parse_var_function(val) {
        return Some(var_val);
    }

    // Handle calc() expressions
    if val.starts_with("calc(") {
        return parse_calc_expression(val);
    }

    if let Some(n) = val.strip_suffix("px") {
        n.trim()
            .parse::<f32>()
            .ok()
            .map(|v| CssValue::Length(v * 0.75)) // px to pt
    } else if let Some(n) = val.strip_suffix("pt") {
        n.trim().parse::<f32>().ok().map(CssValue::Length)
    } else if let Some(n) = val.strip_suffix("rem") {
        n.trim().parse::<f32>().ok().map(CssValue::Rem)
    } else if let Some(n) = val.strip_suffix("em") {
        // Store em as Number to distinguish from absolute values
        // Will be resolved during style computation
        n.trim().parse::<f32>().ok().map(CssValue::Number)
    } else if let Some(n) = val.strip_suffix("vw") {
        n.trim().parse::<f32>().ok().map(CssValue::Vw)
    } else if let Some(n) = val.strip_suffix("vh") {
        n.trim().parse::<f32>().ok().map(CssValue::Vh)
    } else if let Some(n) = val.strip_suffix('%') {
        n.trim().parse::<f32>().ok().map(CssValue::Percentage)
    } else if val.parse::<f32>().is_ok() {
        val.parse::<f32>().ok().map(CssValue::Length)
    } else {
        None
    }
}

/// Parse a `var(--name)` or `var(--name, fallback)` function.
fn parse_var_function(val: &str) -> Option<CssValue> {
    let val = val.trim();
    let inner = val.strip_prefix("var(")?.strip_suffix(')')?;
    // Split on first comma to get variable name and optional fallback
    if let Some((name, fallback)) = inner.split_once(',') {
        let name = name.trim().to_string();
        let fallback = fallback.trim().to_string();
        if name.starts_with("--") {
            Some(CssValue::Var(name, Some(fallback)))
        } else {
            None
        }
    } else {
        let name = inner.trim().to_string();
        if name.starts_with("--") {
            Some(CssValue::Var(name, None))
        } else {
            None
        }
    }
}

/// Parse a calc() expression into a CssValue::Calc.
fn parse_calc_expression(val: &str) -> Option<CssValue> {
    let inner = val.trim().strip_prefix("calc(")?.strip_suffix(')')?;
    let tokens = tokenize_calc(inner)?;
    if tokens.is_empty() {
        return None;
    }
    Some(CssValue::Calc(tokens))
}

/// Tokenize the inside of a calc() expression into CalcTokens.
fn tokenize_calc(expr: &str) -> Option<Vec<CalcToken>> {
    let mut tokens = Vec::new();
    let mut chars = expr.chars().peekable();

    while chars.peek().is_some() {
        // Skip whitespace
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }

        let ch = *chars.peek()?;

        // Check for operator (must be surrounded by spaces per CSS spec, but be lenient)
        if (ch == '+' || ch == '-') && !tokens.is_empty() {
            // Only treat as operator if previous token is not an operator
            let is_op = matches!(
                tokens.last(),
                Some(CalcToken::Length(_))
                    | Some(CalcToken::Percent(_))
                    | Some(CalcToken::Rem(_))
                    | Some(CalcToken::Vw(_))
                    | Some(CalcToken::Vh(_))
            );
            if is_op {
                let op = match ch {
                    '+' => CalcOp::Add,
                    '-' => CalcOp::Sub,
                    _ => unreachable!(),
                };
                tokens.push(CalcToken::Op(op));
                chars.next();
                continue;
            }
        }
        if ch == '*' {
            tokens.push(CalcToken::Op(CalcOp::Mul));
            chars.next();
            continue;
        }
        if ch == '/' {
            tokens.push(CalcToken::Op(CalcOp::Div));
            chars.next();
            continue;
        }

        // Parse a number with optional unit
        let mut num_str = String::new();
        if ch == '-' || ch == '+' {
            num_str.push(ch);
            chars.next();
        }
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                num_str.push(c);
                chars.next();
            } else {
                break;
            }
        }

        // Parse unit suffix
        let mut unit = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphabetic() || c == '%' {
                unit.push(c);
                chars.next();
            } else {
                break;
            }
        }

        if num_str.is_empty() || num_str == "-" || num_str == "+" {
            return None;
        }
        let num: f32 = num_str.parse().ok()?;
        let token = match unit.as_str() {
            "px" => CalcToken::Length(num * 0.75),
            "pt" => CalcToken::Length(num),
            "em" => CalcToken::Length(num * 12.0), // approximate: resolved at compute time
            "rem" => CalcToken::Rem(num),
            "%" => CalcToken::Percent(num),
            "vw" => CalcToken::Vw(num),
            "vh" => CalcToken::Vh(num),
            "" => CalcToken::Length(num),
            _ => return None,
        };
        tokens.push(token);
    }

    Some(tokens)
}

fn parse_color(val: &str) -> Option<CssValue> {
    let val = val.trim().to_ascii_lowercase();

    // Named colors
    let color = match val.as_str() {
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "orange" => Color::rgb(255, 165, 0),
        "purple" => Color::rgb(128, 0, 128),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "maroon" => Color::rgb(128, 0, 0),
        "navy" => Color::rgb(0, 0, 128),
        "teal" => Color::rgb(0, 128, 128),
        "aqua" | "cyan" => Color::rgb(0, 255, 255),
        "fuchsia" | "magenta" => Color::rgb(255, 0, 255),
        "lime" => Color::rgb(0, 255, 0),
        _ => {
            // Hex color
            if let Some(hex) = val.strip_prefix('#') {
                return parse_hex_color(hex);
            }
            // rgb() function
            if let Some(inner) = val.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
                return parse_rgb_function(inner);
            }
            return None;
        }
    };

    Some(CssValue::Color(color))
}

fn parse_hex_color(hex: &str) -> Option<CssValue> {
    let hex = hex.trim();
    let (r, g, b) = match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            (r, g, b)
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b)
        }
        _ => return None,
    };
    Some(CssValue::Color(Color::rgb(r, g, b)))
}

fn parse_rgb_function(inner: &str) -> Option<CssValue> {
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].trim().parse::<u8>().ok()?;
    let g = parts[1].trim().parse::<u8>().ok()?;
    let b = parts[2].trim().parse::<u8>().ok()?;
    Some(CssValue::Color(Color::rgb(r, g, b)))
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

/// Parse a CSS stylesheet string into a list of rules.
///
/// Handles `@media print { ... }` (rules are applied since we generate PDFs)
/// and `@media screen { ... }` (rules are ignored).
pub fn parse_stylesheet(css: &str) -> Vec<CssRule> {
    let mut rules = Vec::new();
    let preprocessed = preprocess_media_queries(css);
    parse_rules_from(&preprocessed, &mut rules);
    rules
}

/// Parse a CSS stylesheet and extract `@page` rules.
pub fn parse_page_rules(css: &str) -> Vec<PageRule> {
    let preprocessed = preprocess_media_queries(css);
    extract_page_rules(&preprocessed)
}

/// Parse a CSS stylesheet and extract `@font-face` rules.
///
/// Only local file paths are supported in `src: url(...)`. Remote URLs
/// (http/https) are rejected for security reasons.
pub fn parse_font_face_rules(css: &str) -> Vec<FontFaceRule> {
    let preprocessed = preprocess_media_queries(css);
    extract_font_face_rules(&preprocessed)
}

/// Extract @font-face rules from preprocessed CSS.
fn extract_font_face_rules(css: &str) -> Vec<FontFaceRule> {
    let mut rules = Vec::new();
    let mut remaining = css;

    while let Some(at_pos) = remaining.to_ascii_lowercase().find("@font-face") {
        let after_at = &remaining[at_pos + 10..];
        if let Some(brace_pos) = after_at.find('{') {
            let after_brace = &after_at[brace_pos + 1..];
            if let Some(close_pos) = after_brace.find('}') {
                let declarations = &after_brace[..close_pos];
                if let Some(rule) = parse_font_face_declarations(declarations) {
                    rules.push(rule);
                }
                remaining = &after_brace[close_pos + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    rules
}

/// Parse the declarations inside an @font-face block.
fn parse_font_face_declarations(decls: &str) -> Option<FontFaceRule> {
    let mut font_family: Option<String> = None;
    let mut src_path: Option<String> = None;

    for declaration in decls.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }

        if let Some((prop, val)) = declaration.split_once(':') {
            let prop = prop.trim().to_ascii_lowercase();
            let val = val.trim();

            match prop.as_str() {
                "font-family" => {
                    // Strip quotes: "MyFont" or 'MyFont' => MyFont
                    let name = val.trim_matches('"').trim_matches('\'').trim().to_string();
                    if !name.is_empty() {
                        font_family = Some(name);
                    }
                }
                "src" => {
                    // Parse url("path") or url('path') or url(path)
                    if let Some(path) = extract_url_path(val) {
                        // Security: reject remote URLs
                        if !path.starts_with("http://") && !path.starts_with("https://") {
                            src_path = Some(path);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    match (font_family, src_path) {
        (Some(family), Some(path)) => Some(FontFaceRule {
            font_family: family,
            src_path: path,
        }),
        _ => None,
    }
}

/// Extract a file path from a CSS `url(...)` value.
///
/// Handles: `url("path")`, `url('path')`, `url(path)`
fn extract_url_path(val: &str) -> Option<String> {
    let val = val.trim();
    // Look for url(...) pattern
    let lower = val.to_ascii_lowercase();
    if let Some(start) = lower.find("url(") {
        let after_url = &val[start + 4..];
        if let Some(end) = after_url.find(')') {
            let inner = after_url[..end].trim();
            let path = inner
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }
    None
}

/// Parse `@import` rules from a CSS string.
///
/// Returns the list of import rules found. Only local file paths are
/// supported; remote URLs (http/https) are rejected for security.
pub fn parse_import_rules(css: &str) -> Vec<ImportRule> {
    let preprocessed = preprocess_media_queries(css);
    extract_import_rules(&preprocessed)
}

/// Extract @import rules from preprocessed CSS.
fn extract_import_rules(css: &str) -> Vec<ImportRule> {
    let mut rules = Vec::new();

    for line in css.split(';') {
        let line = line.trim();
        let lower = line.to_ascii_lowercase();
        if !lower.starts_with("@import") {
            continue;
        }
        let after_import = line[7..].trim();
        let path = if after_import.to_ascii_lowercase().starts_with("url(") {
            // @import url("path") or @import url('path')
            extract_url_path(after_import)
        } else {
            // @import "path" or @import 'path'
            let trimmed = after_import
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        };

        if let Some(path) = path {
            // Security: reject remote URLs
            if !path.starts_with("http://") && !path.starts_with("https://") {
                rules.push(ImportRule { path });
            }
        }
    }

    rules
}

/// Maximum recursion depth for @import processing.
pub const MAX_IMPORT_DEPTH: usize = 10;

/// Resolve `@import` rules in a CSS string by reading and inlining local files.
///
/// The `base_dir` is the directory relative to which import paths are resolved.
/// Recursion is limited to [`MAX_IMPORT_DEPTH`] levels to prevent infinite loops.
pub fn resolve_imports(css: &str, base_dir: &std::path::Path, depth: usize) -> String {
    if depth >= MAX_IMPORT_DEPTH {
        return css.to_string();
    }

    let import_rules = parse_import_rules(css);
    if import_rules.is_empty() {
        return css.to_string();
    }

    let mut result = String::new();

    // Prepend imported content
    for import in &import_rules {
        let path = base_dir.join(&import.path);
        if let Ok(imported_css) = std::fs::read_to_string(&path) {
            // Determine base dir for the imported file
            let imported_base = path.parent().unwrap_or(base_dir);
            let resolved = resolve_imports(&imported_css, imported_base, depth + 1);
            result.push_str(&resolved);
            result.push('\n');
        }
    }

    // Strip the @import lines from original CSS and append the rest
    result.push_str(&strip_import_rules(css));
    result
}

/// Remove @import rules from CSS text, leaving all other content intact.
fn strip_import_rules(css: &str) -> String {
    let mut result = String::new();
    let mut remaining = css;

    while !remaining.is_empty() {
        let trimmed = remaining.trim_start();
        if trimmed.to_ascii_lowercase().starts_with("@import") {
            // Skip past the semicolon
            if let Some(semi_pos) = trimmed.find(';') {
                remaining = &trimmed[semi_pos + 1..];
            } else {
                // Malformed @import with no semicolon — skip the rest
                break;
            }
        } else {
            // Copy characters until we might hit another @import
            if let Some(at_pos) = remaining.find('@') {
                result.push_str(&remaining[..at_pos]);
                remaining = &remaining[at_pos..];
                if !remaining.to_ascii_lowercase().starts_with("@import") {
                    // Not an @import, copy the '@' and continue
                    result.push('@');
                    remaining = &remaining[1..];
                }
            } else {
                result.push_str(remaining);
                break;
            }
        }
    }

    result
}

/// Extract @page rules from preprocessed CSS.
fn extract_page_rules(css: &str) -> Vec<PageRule> {
    let mut page_rules = Vec::new();
    let mut remaining = css;

    while let Some(at_pos) = remaining.find("@page") {
        let after_at = &remaining[at_pos + 5..];
        if let Some(brace_pos) = after_at.find('{') {
            let after_brace = &after_at[brace_pos + 1..];
            if let Some(close_pos) = after_brace.find('}') {
                let declarations = &after_brace[..close_pos];
                if let Some(rule) = parse_page_declarations(declarations) {
                    page_rules.push(rule);
                }
                remaining = &after_brace[close_pos + 1..];
            } else {
                break;
            }
        } else {
            break;
        }
    }

    page_rules
}

/// Parse the declarations inside an @page block.
fn parse_page_declarations(decls: &str) -> Option<PageRule> {
    let mut rule = PageRule::default();
    let mut has_any = false;

    for declaration in decls.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }

        if let Some((prop, val)) = declaration.split_once(':') {
            let prop = prop.trim().to_ascii_lowercase();
            let val = val.trim().to_ascii_lowercase();

            match prop.as_str() {
                "size" => {
                    if let Some((w, h)) = parse_page_size(&val) {
                        rule.width = Some(w);
                        rule.height = Some(h);
                        has_any = true;
                    }
                }
                "margin" => {
                    let parts: Vec<&str> = val.split_whitespace().collect();
                    match parts.len() {
                        1 => {
                            if let Some(v) = parse_page_length(parts[0]) {
                                rule.margin_top = Some(v);
                                rule.margin_right = Some(v);
                                rule.margin_bottom = Some(v);
                                rule.margin_left = Some(v);
                                has_any = true;
                            }
                        }
                        2 => {
                            if let (Some(tb), Some(lr)) =
                                (parse_page_length(parts[0]), parse_page_length(parts[1]))
                            {
                                rule.margin_top = Some(tb);
                                rule.margin_bottom = Some(tb);
                                rule.margin_right = Some(lr);
                                rule.margin_left = Some(lr);
                                has_any = true;
                            }
                        }
                        4 => {
                            if let (Some(t), Some(r), Some(b), Some(l)) = (
                                parse_page_length(parts[0]),
                                parse_page_length(parts[1]),
                                parse_page_length(parts[2]),
                                parse_page_length(parts[3]),
                            ) {
                                rule.margin_top = Some(t);
                                rule.margin_right = Some(r);
                                rule.margin_bottom = Some(b);
                                rule.margin_left = Some(l);
                                has_any = true;
                            }
                        }
                        _ => {}
                    }
                }
                "margin-top" => {
                    if let Some(v) = parse_page_length(&val) {
                        rule.margin_top = Some(v);
                        has_any = true;
                    }
                }
                "margin-right" => {
                    if let Some(v) = parse_page_length(&val) {
                        rule.margin_right = Some(v);
                        has_any = true;
                    }
                }
                "margin-bottom" => {
                    if let Some(v) = parse_page_length(&val) {
                        rule.margin_bottom = Some(v);
                        has_any = true;
                    }
                }
                "margin-left" => {
                    if let Some(v) = parse_page_length(&val) {
                        rule.margin_left = Some(v);
                        has_any = true;
                    }
                }
                _ => {}
            }
        }
    }

    if has_any { Some(rule) } else { None }
}

/// Parse a page size value. Returns (width, height) in points.
fn parse_page_size(val: &str) -> Option<(f32, f32)> {
    let val = val.trim();
    // Named sizes
    match val {
        "a4" => return Some((595.28, 841.89)),
        "a3" => return Some((841.89, 1190.55)),
        "a5" => return Some((419.53, 595.28)),
        "letter" => return Some((612.0, 792.0)),
        "legal" => return Some((612.0, 1008.0)),
        "b5" => return Some((498.9, 708.66)),
        _ => {}
    }

    // Two-value form: "210mm 297mm" or "8.5in 11in"
    let parts: Vec<&str> = val.split_whitespace().collect();
    if parts.len() == 2 {
        if let (Some(w), Some(h)) = (parse_page_length(parts[0]), parse_page_length(parts[1])) {
            return Some((w, h));
        }
    }

    // Single value with landscape/portrait (e.g., "a4 landscape")
    if parts.len() == 2 {
        let (size_name, orientation) = (parts[0], parts[1]);
        if let Some((w, h)) = parse_page_size(size_name) {
            return match orientation {
                "landscape" => Some((h, w)),
                _ => Some((w, h)),
            };
        }
    }

    None
}

/// Parse a length value for @page rules (supports mm, in, cm, pt, px).
fn parse_page_length(val: &str) -> Option<f32> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix("mm") {
        n.trim().parse::<f32>().ok().map(|v| v * 2.83465) // mm to pt
    } else if let Some(n) = val.strip_suffix("cm") {
        n.trim().parse::<f32>().ok().map(|v| v * 28.3465) // cm to pt
    } else if let Some(n) = val.strip_suffix("in") {
        n.trim().parse::<f32>().ok().map(|v| v * 72.0) // in to pt
    } else if let Some(n) = val.strip_suffix("pt") {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = val.strip_suffix("px") {
        n.trim().parse::<f32>().ok().map(|v| v * 0.75) // px to pt
    } else {
        val.parse::<f32>().ok() // bare number as pt
    }
}

fn parse_rules_from(css: &str, rules: &mut Vec<CssRule>) {
    for block in css.split('}') {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        if let Some((selector, declarations)) = block.split_once('{') {
            let raw_selector = selector.trim().to_string();
            if raw_selector.is_empty()
                || raw_selector.starts_with("@page")
                || raw_selector.starts_with("@font-face")
            {
                continue;
            }
            let declarations = parse_inline_style(declarations.trim());
            if !declarations.properties.is_empty() {
                let (clean_selector, pseudo_element) = extract_pseudo_element(&raw_selector);
                rules.push(CssRule {
                    selector: clean_selector,
                    declarations,
                    pseudo_element,
                });
            }
        }
    }
}

/// Extract `::before` or `::after` from a selector, returning the base selector
/// and the pseudo-element (if any). Handles both `::before` and `:before` syntax.
fn extract_pseudo_element(selector: &str) -> (String, Option<PseudoElement>) {
    for (suffix, pe) in [
        ("::before", PseudoElement::Before),
        ("::after", PseudoElement::After),
        (":before", PseudoElement::Before),
        (":after", PseudoElement::After),
    ] {
        if let Some(base) = selector.strip_suffix(suffix) {
            let base = base.trim();
            let base = if base.is_empty() {
                "*".to_string()
            } else {
                base.to_string()
            };
            return (base, Some(pe));
        }
    }
    (selector.to_string(), None)
}

/// Check if a CSS selector matches a given element (backward-compatible, no context).
pub fn selector_matches(
    selector: &str,
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
) -> bool {
    let ctx = SelectorContext::default();
    selector_matches_with_context(selector, tag_name, classes, id, &HashMap::new(), &ctx)
}

/// Check if a CSS selector matches a given element with full context for
/// advanced selectors (descendant, child, attribute, pseudo-class).
pub fn selector_matches_with_context(
    selector: &str,
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    // Support comma-separated selectors: "h1, h2, h3"
    for part in selector.split(',') {
        let part = part.trim();
        if compound_selector_matches(part, tag_name, classes, id, attributes, ctx) {
            return true;
        }
    }
    false
}

/// Match a single (non-comma-separated) selector which may contain
/// child combinators (`>`), descendant combinators (space), or be a simple selector.
fn compound_selector_matches(
    selector: &str,
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    if selector.is_empty() {
        return false;
    }

    // Check for general sibling combinator: "h1 ~ p"
    if let Some(pos) = selector.rfind(" ~ ") {
        let sibling_sel = selector[..pos].trim();
        let current_sel = &selector[pos + 3..].trim();
        if !simple_selector_matches(current_sel, tag_name, classes, id, attributes, ctx) {
            return false;
        }
        // Check if any preceding sibling matches sibling_sel
        for (sib_tag, sib_classes) in &ctx.preceding_siblings {
            let sib_class_refs: Vec<&str> = sib_classes.iter().map(|s| s.as_str()).collect();
            if simple_selector_matches(
                sibling_sel,
                sib_tag,
                &sib_class_refs,
                None,
                &HashMap::new(),
                &SelectorContext::default(),
            ) {
                return true;
            }
        }
        return false;
    }

    // Check for adjacent sibling combinator: "h1 + p"
    if let Some(pos) = selector.rfind(" + ") {
        let sibling_sel = selector[..pos].trim();
        let current_sel = &selector[pos + 3..].trim();
        if !simple_selector_matches(current_sel, tag_name, classes, id, attributes, ctx) {
            return false;
        }
        // Check if the immediately preceding sibling matches sibling_sel
        if let Some((sib_tag, sib_classes)) = ctx.preceding_siblings.last() {
            let sib_class_refs: Vec<&str> = sib_classes.iter().map(|s| s.as_str()).collect();
            return simple_selector_matches(
                sibling_sel,
                sib_tag,
                &sib_class_refs,
                None,
                &HashMap::new(),
                &SelectorContext::default(),
            );
        }
        return false;
    }

    // Check for child combinator: "div > p"
    // We need to be careful to split on " > " (with spaces) to avoid matching inside selectors
    if let Some(pos) = selector.rfind(" > ") {
        let parent_sel = selector[..pos].trim();
        let child_sel = &selector[pos + 3..].trim();
        // The child selector must match the current element
        if !simple_selector_matches(child_sel, tag_name, classes, id, attributes, ctx) {
            return false;
        }
        // The parent selector must match the direct parent
        if let Some(parent) = ctx.ancestors.last() {
            let parent_classes = parent.class_list();
            let parent_attrs = &parent.attributes;
            // Build a context for the parent (its ancestors are our ancestors minus the last)
            let parent_ctx = SelectorContext {
                ancestors: ctx.ancestors[..ctx.ancestors.len() - 1].to_vec(),
                child_index: 0,
                sibling_count: 0,
                preceding_siblings: Vec::new(),
            };
            return compound_selector_matches(
                parent_sel,
                parent.tag_name(),
                &parent_classes,
                parent.id(),
                parent_attrs,
                &parent_ctx,
            );
        }
        return false;
    }

    // Check for descendant combinator: "div p" (space-separated, no `>`)
    // Find the last space that is NOT inside brackets
    if let Some(pos) = rfind_descendant_space(selector) {
        let ancestor_sel = selector[..pos].trim();
        let descendant_sel = selector[pos + 1..].trim();
        // The descendant selector must match the current element
        if !simple_selector_matches(descendant_sel, tag_name, classes, id, attributes, ctx) {
            return false;
        }
        // The ancestor selector must match some ancestor in the chain
        for (i, ancestor) in ctx.ancestors.iter().enumerate() {
            let anc_classes = ancestor.class_list();
            let anc_attrs = &ancestor.attributes;
            let anc_ctx = SelectorContext {
                ancestors: ctx.ancestors[..i].to_vec(),
                child_index: 0,
                sibling_count: 0,
                preceding_siblings: Vec::new(),
            };
            if compound_selector_matches(
                ancestor_sel,
                ancestor.tag_name(),
                &anc_classes,
                ancestor.id(),
                anc_attrs,
                &anc_ctx,
            ) {
                return true;
            }
        }
        return false;
    }

    // Simple selector (no combinators)
    simple_selector_matches(selector, tag_name, classes, id, attributes, ctx)
}

/// Find the last space in a selector that represents a descendant combinator,
/// ignoring spaces inside attribute selectors `[...]`.
fn rfind_descendant_space(selector: &str) -> Option<usize> {
    let bytes = selector.as_bytes();
    let mut bracket_depth = 0;
    let mut paren_depth = 0;
    let mut last_space = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'[' => bracket_depth += 1,
            b']' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                }
            }
            b'(' => paren_depth += 1,
            b')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            b' ' if bracket_depth == 0 && paren_depth == 0 => {
                last_space = Some(i);
            }
            _ => {}
        }
    }
    last_space
}

/// Match a simple selector (no combinators): tag, .class, #id, [attr], :pseudo, or combinations.
fn simple_selector_matches(
    selector: &str,
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
    attributes: &HashMap<String, String>,
    ctx: &SelectorContext,
) -> bool {
    if selector.is_empty() {
        return false;
    }

    // Split off pseudo-classes first (e.g., "p:first-child" -> "p" + ":first-child")
    let (base, pseudo) = split_pseudo_class(selector);

    // Check pseudo-class if present
    if let Some(pseudo_str) = pseudo {
        // Handle :not() pseudo-class
        if let Some(inner) = pseudo_str
            .strip_prefix("not(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let inner = inner.trim();
            // The inner selector must NOT match
            if simple_selector_core_matches(inner, tag_name, classes, id) {
                return false;
            }
            if base.is_empty() {
                return true;
            }
        } else {
            if !pseudo_class_matches(pseudo_str, ctx) {
                return false;
            }
            if base.is_empty() {
                // Selector is just a pseudo-class like ":first-child"
                return true;
            }
        }
    }

    // Check attribute selectors: tag[attr] or tag[attr="val"]
    if let Some(bracket_start) = base.find('[') {
        let tag_part = &base[..bracket_start];
        let rest = &base[bracket_start..];
        // Verify tag part matches (if non-empty)
        if !tag_part.is_empty() && tag_part != tag_name {
            // Could be a class or id selector before the bracket
            if !simple_selector_core_matches(tag_part, tag_name, classes, id) {
                return false;
            }
        }
        return attribute_selector_matches(rest, attributes);
    }

    // Core selector matching (tag, .class, #id)
    simple_selector_core_matches(base, tag_name, classes, id)
}

/// Match the core part of a simple selector: tag, .class, #id, or combined (tag.class, tag#id).
fn simple_selector_core_matches(
    selector: &str,
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
) -> bool {
    if selector.is_empty() {
        return false;
    }

    // ID selector: #foo or tag#foo
    if let Some(pos) = selector.find('#') {
        let tag_part = &selector[..pos];
        let id_part = &selector[pos + 1..];
        if !tag_part.is_empty() && tag_part != tag_name {
            return false;
        }
        return id == Some(id_part);
    }

    // Class selector: .foo or tag.foo
    if let Some(pos) = selector.find('.') {
        let tag_part = &selector[..pos];
        let class_part = &selector[pos + 1..];
        if !tag_part.is_empty() && tag_part != tag_name {
            return false;
        }
        return classes.contains(&class_part);
    }

    // Tag selector
    selector == tag_name
}

/// Split a selector into (base, pseudo-class) at the first `:` that is not inside brackets
/// or parentheses. Handles pseudo-classes with arguments like `:nth-child(2n+1)` and `:not(.class)`.
fn split_pseudo_class(selector: &str) -> (&str, Option<&str>) {
    let bytes = selector.as_bytes();
    let mut bracket_depth = 0;
    let mut paren_depth = 0;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'[' => bracket_depth += 1,
            b']' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                }
            }
            b'(' => paren_depth += 1,
            b')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            b':' if bracket_depth == 0 && paren_depth == 0 => {
                return (&selector[..i], Some(&selector[i + 1..]));
            }
            _ => {}
        }
    }
    (selector, None)
}

/// Check if a pseudo-class matches given the context.
fn pseudo_class_matches(pseudo: &str, ctx: &SelectorContext) -> bool {
    match pseudo {
        "first-child" => ctx.sibling_count > 0 && ctx.child_index == 0,
        "last-child" => ctx.sibling_count > 0 && ctx.child_index == ctx.sibling_count - 1,
        _ => {
            // :nth-child(...)
            if let Some(arg) = pseudo
                .strip_prefix("nth-child(")
                .and_then(|s| s.strip_suffix(')'))
            {
                return nth_child_matches(arg.trim(), ctx.child_index);
            }
            false
        }
    }
}

/// Check if child_index (0-based) satisfies an :nth-child() argument.
fn nth_child_matches(arg: &str, child_index: usize) -> bool {
    let n = child_index + 1; // 1-based position
    let arg = arg.trim().to_ascii_lowercase();

    if arg == "odd" {
        return n % 2 == 1;
    }
    if arg == "even" {
        return n % 2 == 0;
    }

    // Try plain number
    if let Ok(val) = arg.parse::<usize>() {
        return n == val;
    }

    // an+b formula: e.g. "2n+1", "3n", "n+2", "-n+3", "2n-1"
    if let Some((a, b)) = parse_an_plus_b(&arg) {
        if a == 0 {
            return n as i64 == b;
        }
        // n must satisfy: a*k + b == n  for some k >= 0
        // => k = (n - b) / a, must be non-negative integer
        let diff = n as i64 - b;
        if a > 0 {
            diff >= 0 && diff % a == 0
        } else {
            // a < 0: k = diff / a, diff and a both negative or diff <= 0
            diff <= 0 && diff % a == 0
        }
    } else {
        false
    }
}

/// Parse an "an+b" formula. Returns (a, b) as i64.
fn parse_an_plus_b(s: &str) -> Option<(i64, i64)> {
    let s = s.replace(" ", "");
    // Find 'n' position
    let n_pos = s.find('n')?;

    // Parse 'a' part (before 'n')
    let a_str = &s[..n_pos];
    let a = if a_str.is_empty() || a_str == "+" {
        1
    } else if a_str == "-" {
        -1
    } else {
        a_str.parse::<i64>().ok()?
    };

    // Parse 'b' part (after 'n')
    let after_n = &s[n_pos + 1..];
    let b = if after_n.is_empty() {
        0
    } else {
        after_n.parse::<i64>().ok()?
    };

    Some((a, b))
}

/// Check if an attribute selector like `[href]` or `[type="text"]` matches.
/// The input includes the brackets.
fn attribute_selector_matches(selector: &str, attributes: &HashMap<String, String>) -> bool {
    // May have multiple attribute selectors: [a][b]
    let mut rest = selector;
    while let Some(start) = rest.find('[') {
        let end = match rest[start..].find(']') {
            Some(e) => start + e,
            None => return false,
        };
        let inner = &rest[start + 1..end];
        if !single_attribute_matches(inner, attributes) {
            return false;
        }
        rest = &rest[end + 1..];
    }
    true
}

/// Match a single attribute expression (without brackets), e.g. `href` or `type="text"`.
fn single_attribute_matches(expr: &str, attributes: &HashMap<String, String>) -> bool {
    if let Some((attr_name, attr_val)) = expr.split_once('=') {
        let attr_name = attr_name.trim();
        let attr_val = attr_val.trim().trim_matches('"').trim_matches('\'');
        match attributes.get(attr_name) {
            Some(v) => v == attr_val,
            None => false,
        }
    } else {
        // Presence check: [href]
        attributes.contains_key(expr.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_font_size_px() {
        let style = parse_inline_style("font-size: 16px");
        match style.get("font-size") {
            Some(CssValue::Length(v)) => assert!((v - 12.0).abs() < 0.1), // 16px * 0.75 = 12pt
            other => panic!("Expected Length, got {:?}", other),
        }
    }

    #[test]
    fn parse_color_named() {
        let style = parse_inline_style("color: red");
        match style.get("color") {
            Some(CssValue::Color(c)) => {
                assert_eq!(c.r, 255);
                assert_eq!(c.g, 0);
                assert_eq!(c.b, 0);
            }
            other => panic!("Expected Color, got {:?}", other),
        }
    }

    #[test]
    fn parse_color_hex() {
        let style = parse_inline_style("color: #ff0000");
        match style.get("color") {
            Some(CssValue::Color(c)) => assert_eq!(c.r, 255),
            other => panic!("Expected Color, got {:?}", other),
        }
    }

    #[test]
    fn parse_color_hex_short() {
        let style = parse_inline_style("color: #f00");
        match style.get("color") {
            Some(CssValue::Color(c)) => {
                assert_eq!(c.r, 255);
                assert_eq!(c.g, 0);
            }
            other => panic!("Expected Color, got {:?}", other),
        }
    }

    #[test]
    fn parse_color_rgb() {
        let style = parse_inline_style("color: rgb(128, 64, 32)");
        match style.get("color") {
            Some(CssValue::Color(c)) => {
                assert_eq!(c.r, 128);
                assert_eq!(c.g, 64);
                assert_eq!(c.b, 32);
            }
            other => panic!("Expected Color, got {:?}", other),
        }
    }

    #[test]
    fn parse_margin_shorthand() {
        let style = parse_inline_style("margin: 10px");
        match style.get("margin-top") {
            Some(CssValue::Length(v)) => assert!((v - 7.5).abs() < 0.1),
            other => panic!("Expected Length, got {:?}", other),
        }
        assert!(style.get("margin-bottom").is_some());
        assert!(style.get("margin-left").is_some());
        assert!(style.get("margin-right").is_some());
    }

    #[test]
    fn parse_multiple_properties() {
        let style = parse_inline_style("font-size: 14pt; color: blue; text-align: center");
        assert!(style.get("font-size").is_some());
        assert!(style.get("color").is_some());
        assert!(style.get("text-align").is_some());
    }

    #[test]
    fn parse_empty_style() {
        let style = parse_inline_style("");
        assert!(style.properties.is_empty());
    }

    #[test]
    fn parse_font_weight() {
        let style = parse_inline_style("font-weight: bold");
        match style.get("font-weight") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "bold"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_font_style() {
        let style = parse_inline_style("font-style: italic");
        match style.get("font-style") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "italic"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_pt_length() {
        let style = parse_inline_style("font-size: 14pt");
        match style.get("font-size") {
            Some(CssValue::Length(v)) => assert!((v - 14.0).abs() < 0.1),
            other => panic!("Expected Length, got {:?}", other),
        }
    }

    #[test]
    fn parse_em_unit() {
        let style = parse_inline_style("font-size: 1.5em");
        match style.get("font-size") {
            Some(CssValue::Number(v)) => assert!((v - 1.5).abs() < 0.01),
            other => panic!("Expected Number for em, got {:?}", other),
        }
    }

    #[test]
    fn parse_bare_number_length() {
        let style = parse_inline_style("line-height: 1.6");
        match style.get("line-height") {
            Some(CssValue::Length(v)) => assert!((v - 1.6).abs() < 0.01),
            other => panic!("Expected Length, got {:?}", other),
        }
    }

    #[test]
    fn parse_invalid_length_returns_none() {
        let style = parse_inline_style("font-size: abc");
        assert!(style.get("font-size").is_none());
    }

    #[test]
    fn parse_page_break() {
        let style = parse_inline_style("page-break-before: always");
        match style.get("page-break-before") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "always"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_text_decoration() {
        let style = parse_inline_style("text-decoration: underline");
        match style.get("text-decoration") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "underline"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn style_map_merge() {
        let mut a = StyleMap::new();
        a.set("font-size", CssValue::Length(12.0));
        let mut b = StyleMap::new();
        b.set("font-size", CssValue::Length(16.0));
        b.set("color", CssValue::Keyword("red".into()));
        a.merge(&b);
        match a.get("font-size") {
            Some(CssValue::Length(v)) => assert!((v - 16.0).abs() < 0.01),
            other => panic!("Expected overridden length, got {:?}", other),
        }
        assert!(a.get("color").is_some());
    }

    #[test]
    fn parse_invalid_hex_length() {
        let style = parse_inline_style("color: #12345");
        assert!(style.get("color").is_none());
    }

    #[test]
    fn parse_rgb_invalid_parts() {
        let style = parse_inline_style("color: rgb(1,2)");
        assert!(style.get("color").is_none());
    }

    #[test]
    fn parse_stylesheet_basic() {
        let rules = parse_stylesheet("p { color: red; font-size: 14pt } h1 { font-weight: bold }");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, "p");
        assert!(rules[0].declarations.get("color").is_some());
        assert_eq!(rules[1].selector, "h1");
    }

    #[test]
    fn parse_stylesheet_class_and_id() {
        let rules = parse_stylesheet(".highlight { font-weight: bold } #main { color: blue }");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, ".highlight");
        assert_eq!(rules[1].selector, "#main");
    }

    #[test]
    fn selector_matches_tag() {
        assert!(selector_matches("p", "p", &[], None));
        assert!(!selector_matches("p", "h1", &[], None));
    }

    #[test]
    fn selector_matches_class() {
        assert!(selector_matches(".foo", "p", &["foo", "bar"], None));
        assert!(!selector_matches(".baz", "p", &["foo"], None));
        assert!(selector_matches("p.foo", "p", &["foo"], None));
        assert!(!selector_matches("h1.foo", "p", &["foo"], None));
    }

    #[test]
    fn selector_matches_id() {
        assert!(selector_matches("#main", "div", &[], Some("main")));
        assert!(!selector_matches("#main", "div", &[], Some("other")));
        assert!(selector_matches("div#main", "div", &[], Some("main")));
        assert!(!selector_matches("p#main", "div", &[], Some("main")));
    }

    #[test]
    fn selector_matches_comma_separated() {
        assert!(selector_matches("h1, h2, h3", "h2", &[], None));
        assert!(!selector_matches("h1, h2, h3", "p", &[], None));
    }

    #[test]
    fn selector_empty_no_match() {
        assert!(!selector_matches("", "p", &[], None));
    }

    #[test]
    fn parse_padding_shorthand() {
        let style = parse_inline_style("padding: 8px");
        assert!(style.get("padding-top").is_some());
        assert!(style.get("padding-right").is_some());
        assert!(style.get("padding-bottom").is_some());
        assert!(style.get("padding-left").is_some());
    }

    #[test]
    fn parse_color_unknown_returns_none() {
        // Line 156: unknown color name with no hex/rgb prefix returns None
        let style = parse_inline_style("color: nonexistentcolor");
        assert!(style.get("color").is_none());
    }

    #[test]
    fn parse_stylesheet_empty_selector_skipped() {
        // Line 213: empty selector after split is skipped
        let rules = parse_stylesheet("{ color: red }");
        assert_eq!(rules.len(), 0);
    }

    #[test]
    fn parse_stylesheet_empty_declarations_skipped() {
        // A rule with an empty declarations block is skipped
        let rules = parse_stylesheet("p { }");
        assert_eq!(rules.len(), 0);
    }

    #[test]
    fn parse_display_property() {
        let style = parse_inline_style("display: none");
        match style.get("display") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "none"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_color_rgb_function() {
        // Exercises the rgb() branch in parse_color (line 153-154)
        let style = parse_inline_style("color: rgb(10, 20, 30)");
        match style.get("color") {
            Some(CssValue::Color(c)) => {
                assert_eq!(c.r, 10);
                assert_eq!(c.g, 20);
                assert_eq!(c.b, 30);
            }
            other => panic!("Expected Color, got {:?}", other),
        }
    }

    #[test]
    fn parse_border_shorthand() {
        let style = parse_inline_style("border: 1px solid black");
        match style.get("border") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "1px solid black"),
            other => panic!("Expected Keyword for border, got {:?}", other),
        }
    }

    #[test]
    fn parse_border_width_property() {
        let style = parse_inline_style("border-width: 2pt");
        match style.get("border-width") {
            Some(CssValue::Length(v)) => assert!((v - 2.0).abs() < 0.1),
            other => panic!("Expected Length for border-width, got {:?}", other),
        }
    }

    #[test]
    fn parse_border_color_property() {
        let style = parse_inline_style("border-color: red");
        match style.get("border-color") {
            Some(CssValue::Color(c)) => {
                assert_eq!(c.r, 255);
                assert_eq!(c.g, 0);
                assert_eq!(c.b, 0);
            }
            other => panic!("Expected Color for border-color, got {:?}", other),
        }
    }

    #[test]
    fn parse_border_style_property() {
        let style = parse_inline_style("border-style: dashed");
        match style.get("border-style") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "dashed"),
            other => panic!("Expected Keyword for border-style, got {:?}", other),
        }
    }

    #[test]
    fn parse_font_family_serif() {
        let style = parse_inline_style("font-family: serif");
        match style.get("font-family") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "serif"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_font_family_monospace() {
        let style = parse_inline_style("font-family: monospace");
        match style.get("font-family") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "monospace"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_font_family_with_fallback() {
        let style = parse_inline_style("font-family: 'Times New Roman', serif");
        match style.get("font-family") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "'Times New Roman'"),
            other => panic!("Expected Keyword with first font name, got {:?}", other),
        }
    }

    #[test]
    fn parse_font_family_courier_new() {
        let style = parse_inline_style("font-family: 'Courier New'");
        match style.get("font-family") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "'Courier New'"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_stylesheet_media_print_applied() {
        let css = "@media print { p { color: red } }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, "p");
        assert!(rules[0].declarations.get("color").is_some());
    }

    #[test]
    fn parse_stylesheet_media_screen_ignored() {
        let css = "@media screen { p { color: red } }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 0);
    }

    #[test]
    fn parse_stylesheet_media_print_with_regular_rules() {
        let css =
            "h1 { font-size: 24pt } @media print { p { color: blue } } h2 { font-size: 18pt }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].selector, "h1");
        assert_eq!(rules[1].selector, "p");
        assert_eq!(rules[2].selector, "h2");
    }

    #[test]
    fn parse_stylesheet_media_screen_with_regular_rules() {
        let css =
            "h1 { font-size: 24pt } @media screen { p { color: blue } } h2 { font-size: 18pt }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, "h1");
        assert_eq!(rules[1].selector, "h2");
    }

    #[test]
    fn parse_stylesheet_media_print_multiple_rules() {
        let css = "@media print { h1 { font-size: 20pt } p { color: black } }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, "h1");
        assert_eq!(rules[1].selector, "p");
    }

    // --- Advanced selector tests ---

    use crate::parser::dom::HtmlTag;

    fn make_element(tag: &str) -> ElementNode {
        ElementNode::new(HtmlTag::from_tag_name(tag))
    }

    fn make_element_with_class(tag: &str, class: &str) -> ElementNode {
        let mut el = ElementNode::new(HtmlTag::from_tag_name(tag));
        el.attributes.insert("class".to_string(), class.to_string());
        el
    }

    fn make_element_with_attr(tag: &str, attr: &str, val: &str) -> ElementNode {
        let mut el = ElementNode::new(HtmlTag::from_tag_name(tag));
        el.attributes.insert(attr.to_string(), val.to_string());
        el
    }

    #[test]
    fn descendant_selector_matches() {
        // "div p" should match <div><p>
        let div = make_element("div");
        let ctx = SelectorContext {
            ancestors: vec![&div],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "div p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn descendant_selector_no_match_without_ancestor() {
        // "div p" should NOT match <p> alone (no div ancestor)
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            "div p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn descendant_selector_deep_nesting() {
        // "div p" should match <div><section><p>
        let div = make_element("div");
        let section = make_element("section");
        let ctx = SelectorContext {
            ancestors: vec![&div, &section],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "div p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn child_selector_matches_direct_parent() {
        // "div > p" should match when div is direct parent
        let div = make_element("div");
        let ctx = SelectorContext {
            ancestors: vec![&div],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "div > p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn child_selector_no_match_indirect() {
        // "div > p" should NOT match <div><section><p>
        let div = make_element("div");
        let section = make_element("section");
        let ctx = SelectorContext {
            ancestors: vec![&div, &section],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            "div > p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn attribute_presence_selector() {
        // "[href]" matches elements with href attribute
        let mut attrs = HashMap::new();
        attrs.insert("href".to_string(), "https://example.com".to_string());
        let ctx = SelectorContext::default();
        assert!(selector_matches_with_context(
            "[href]",
            "a",
            &[],
            None,
            &attrs,
            &ctx,
        ));
    }

    #[test]
    fn attribute_presence_selector_no_match() {
        // "[href]" does NOT match elements without href
        let attrs = HashMap::new();
        let ctx = SelectorContext::default();
        assert!(!selector_matches_with_context(
            "[href]",
            "a",
            &[],
            None,
            &attrs,
            &ctx,
        ));
    }

    #[test]
    fn attribute_value_selector() {
        // [type="text"] matches elements with type="text"
        let mut attrs = HashMap::new();
        attrs.insert("type".to_string(), "text".to_string());
        let ctx = SelectorContext::default();
        assert!(selector_matches_with_context(
            "[type=\"text\"]",
            "input",
            &[],
            None,
            &attrs,
            &ctx,
        ));
    }

    #[test]
    fn attribute_value_selector_wrong_value() {
        // [type="text"] does NOT match type="password"
        let mut attrs = HashMap::new();
        attrs.insert("type".to_string(), "password".to_string());
        let ctx = SelectorContext::default();
        assert!(!selector_matches_with_context(
            "[type=\"text\"]",
            "input",
            &[],
            None,
            &attrs,
            &ctx,
        ));
    }

    #[test]
    fn attribute_selector_with_tag() {
        // "a[href]" matches <a href="...">
        let mut attrs = HashMap::new();
        attrs.insert("href".to_string(), "https://example.com".to_string());
        let ctx = SelectorContext::default();
        assert!(selector_matches_with_context(
            "a[href]",
            "a",
            &[],
            None,
            &attrs,
            &ctx,
        ));
        // "a[href]" does NOT match <div href="...">
        assert!(!selector_matches_with_context(
            "a[href]",
            "div",
            &[],
            None,
            &attrs,
            &ctx,
        ));
    }

    #[test]
    fn pseudo_class_first_child() {
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 0,
            sibling_count: 3,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            ":first-child",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
        assert!(selector_matches_with_context(
            "p:first-child",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
        // Not first child
        let ctx2 = SelectorContext {
            ancestors: vec![],
            child_index: 1,
            sibling_count: 3,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            ":first-child",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx2,
        ));
    }

    #[test]
    fn pseudo_class_last_child() {
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 2,
            sibling_count: 3,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            ":last-child",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
        assert!(selector_matches_with_context(
            "p:last-child",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
        // Not last child
        let ctx2 = SelectorContext {
            ancestors: vec![],
            child_index: 0,
            sibling_count: 3,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            ":last-child",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx2,
        ));
    }

    #[test]
    fn pseudo_class_first_child_with_tag_mismatch() {
        // "h1:first-child" should NOT match a <p> even if it's first child
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 0,
            sibling_count: 3,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            "h1:first-child",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn descendant_with_class_selector() {
        // ".container p" should match <div class="container"><p>
        let container = make_element_with_class("div", "container");
        let ctx = SelectorContext {
            ancestors: vec![&container],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            ".container p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn child_selector_with_class() {
        // ".wrap > span" should match <div class="wrap"><span>
        let wrap = make_element_with_class("div", "wrap");
        let ctx = SelectorContext {
            ancestors: vec![&wrap],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            ".wrap > span",
            "span",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn comma_separated_with_descendant() {
        // "div p, span" should match <span> alone
        let ctx = SelectorContext::default();
        assert!(selector_matches_with_context(
            "div p, span",
            "span",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn backward_compat_simple_selectors_still_work() {
        // Ensure the old selector_matches API still works
        assert!(selector_matches("p", "p", &[], None));
        assert!(!selector_matches("p", "h1", &[], None));
        assert!(selector_matches(".foo", "p", &["foo", "bar"], None));
        assert!(selector_matches("#main", "div", &[], Some("main")));
        assert!(selector_matches("h1, h2, h3", "h2", &[], None));
    }

    #[test]
    fn descendant_selector_with_attribute_ancestor() {
        // "a[href] span" — <a href="x"><span>
        let a_el = make_element_with_attr("a", "href", "https://example.com");
        let ctx = SelectorContext {
            ancestors: vec![&a_el],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "a[href] span",
            "span",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    // --- Float / Clear / Position / Box-shadow CSS parsing tests ---

    #[test]
    fn parse_float_property() {
        let style = parse_inline_style("float: left");
        match style.get("float") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "left"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_clear_property() {
        let style = parse_inline_style("clear: both");
        match style.get("clear") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "both"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_position_property() {
        let style = parse_inline_style("position: absolute");
        match style.get("position") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "absolute"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn parse_top_and_left_properties() {
        let style = parse_inline_style("top: 10pt; left: 20px");
        match style.get("top") {
            Some(CssValue::Length(v)) => assert!((v - 10.0).abs() < 0.1),
            other => panic!("Expected Length for top, got {:?}", other),
        }
        match style.get("left") {
            Some(CssValue::Length(v)) => assert!((v - 15.0).abs() < 0.1), // 20px * 0.75
            other => panic!("Expected Length for left, got {:?}", other),
        }
    }

    #[test]
    fn parse_box_shadow_property() {
        let style = parse_inline_style("box-shadow: 2px 2px 4px black");
        match style.get("box-shadow") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "2px 2px 4px black"),
            other => panic!("Expected Keyword for box-shadow, got {:?}", other),
        }
    }

    #[test]
    fn parse_box_shadow_none() {
        let style = parse_inline_style("box-shadow: none");
        match style.get("box-shadow") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "none"),
            other => panic!("Expected Keyword for box-shadow, got {:?}", other),
        }
    }

    #[test]
    fn parse_margin_0_auto_shorthand() {
        let style = parse_inline_style("margin: 0 auto");
        match style.get("margin-top") {
            Some(CssValue::Length(v)) => assert!((*v - 0.0).abs() < 0.01),
            other => panic!("Expected Length(0) for margin-top, got {:?}", other),
        }
        match style.get("margin-bottom") {
            Some(CssValue::Length(v)) => assert!((*v - 0.0).abs() < 0.01),
            other => panic!("Expected Length(0) for margin-bottom, got {:?}", other),
        }
        match style.get("margin-left") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "auto"),
            other => panic!("Expected Keyword(auto) for margin-left, got {:?}", other),
        }
        match style.get("margin-right") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "auto"),
            other => panic!("Expected Keyword(auto) for margin-right, got {:?}", other),
        }
    }

    #[test]
    fn parse_margin_left_auto() {
        let style = parse_inline_style("margin-left: auto");
        match style.get("margin-left") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "auto"),
            other => panic!("Expected Keyword(auto) for margin-left, got {:?}", other),
        }
    }

    #[test]
    fn parse_margin_right_auto() {
        let style = parse_inline_style("margin-right: auto");
        match style.get("margin-right") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "auto"),
            other => panic!("Expected Keyword(auto) for margin-right, got {:?}", other),
        }
    }

    #[test]
    fn parse_margin_4_values_with_auto() {
        let style = parse_inline_style("margin: 10pt auto 20pt auto");
        match style.get("margin-top") {
            Some(CssValue::Length(v)) => assert!((*v - 10.0).abs() < 0.01),
            other => panic!("Expected Length(10) for margin-top, got {:?}", other),
        }
        match style.get("margin-right") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "auto"),
            other => panic!("Expected Keyword(auto) for margin-right, got {:?}", other),
        }
        match style.get("margin-bottom") {
            Some(CssValue::Length(v)) => assert!((*v - 20.0).abs() < 0.01),
            other => panic!("Expected Length(20) for margin-bottom, got {:?}", other),
        }
        match style.get("margin-left") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "auto"),
            other => panic!("Expected Keyword(auto) for margin-left, got {:?}", other),
        }
    }

    #[test]
    fn parse_padding_multi_value() {
        let style = parse_inline_style("padding: 10pt 20pt");
        match style.get("padding-top") {
            Some(CssValue::Length(v)) => assert!((*v - 10.0).abs() < 0.01),
            other => panic!("Expected Length(10) for padding-top, got {:?}", other),
        }
        match style.get("padding-right") {
            Some(CssValue::Length(v)) => assert!((*v - 20.0).abs() < 0.01),
            other => panic!("Expected Length(20) for padding-right, got {:?}", other),
        }
        match style.get("padding-bottom") {
            Some(CssValue::Length(v)) => assert!((*v - 10.0).abs() < 0.01),
            other => panic!("Expected Length(10) for padding-bottom, got {:?}", other),
        }
        match style.get("padding-left") {
            Some(CssValue::Length(v)) => assert!((*v - 20.0).abs() < 0.01),
            other => panic!("Expected Length(20) for padding-left, got {:?}", other),
        }
    }

    #[test]
    fn page_rule_size_a4() {
        let css = "@page { size: A4; }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        assert!((rules[0].width.unwrap() - 595.28).abs() < 0.01);
        assert!((rules[0].height.unwrap() - 841.89).abs() < 0.01);
    }

    #[test]
    fn page_rule_size_letter() {
        let css = "@page { size: letter; }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        assert!((rules[0].width.unwrap() - 612.0).abs() < 0.01);
        assert!((rules[0].height.unwrap() - 792.0).abs() < 0.01);
    }

    #[test]
    fn page_rule_margin_uniform() {
        let css = "@page { margin: 1in; }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        assert!((rules[0].margin_top.unwrap() - 72.0).abs() < 0.01);
        assert!((rules[0].margin_right.unwrap() - 72.0).abs() < 0.01);
        assert!((rules[0].margin_bottom.unwrap() - 72.0).abs() < 0.01);
        assert!((rules[0].margin_left.unwrap() - 72.0).abs() < 0.01);
    }

    #[test]
    fn page_rule_margin_two_values() {
        let css = "@page { margin: 1in 0.5in; }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        assert!((rules[0].margin_top.unwrap() - 72.0).abs() < 0.01);
        assert!((rules[0].margin_right.unwrap() - 36.0).abs() < 0.01);
        assert!((rules[0].margin_bottom.unwrap() - 72.0).abs() < 0.01);
        assert!((rules[0].margin_left.unwrap() - 36.0).abs() < 0.01);
    }

    #[test]
    fn page_rule_size_mm() {
        let css = "@page { size: 210mm 297mm; }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        assert!((rules[0].width.unwrap() - 595.28).abs() < 1.0); // ~A4
        assert!((rules[0].height.unwrap() - 841.89).abs() < 1.0);
    }

    #[test]
    fn page_rule_combined() {
        let css = "@page { size: letter; margin: 0.5in; }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        assert!((rules[0].width.unwrap() - 612.0).abs() < 0.01);
        assert!((rules[0].margin_top.unwrap() - 36.0).abs() < 0.01);
    }

    #[test]
    fn page_rule_not_parsed_as_regular_rule() {
        let css = "@page { size: A4; margin: 1in; } .foo { color: red; }";
        let rules = parse_stylesheet(css);
        // @page should not appear as a regular rule
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ".foo");
    }

    #[test]
    fn page_rule_individual_margins() {
        let css = "@page { margin-top: 2cm; margin-left: 1cm; }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        assert!((rules[0].margin_top.unwrap() - 56.693).abs() < 0.1);
        assert!((rules[0].margin_left.unwrap() - 28.3465).abs() < 0.1);
        assert!(rules[0].margin_right.is_none());
    }

    #[test]
    fn gradient_in_background_property() {
        let style = parse_inline_style("background: linear-gradient(to right, red, blue)");
        assert!(style.get("background-gradient").is_some());
    }

    #[test]
    fn gradient_in_background_image_property() {
        let style =
            parse_inline_style("background-image: linear-gradient(45deg, #ff0000, #0000ff)");
        assert!(style.get("background-gradient").is_some());
    }

    #[test]
    fn radial_gradient_in_background() {
        let style = parse_inline_style("background: radial-gradient(red, blue)");
        assert!(style.get("background-radial-gradient").is_some());
    }

    #[test]
    fn page_rule_landscape() {
        let css = "@page { size: a4 landscape; }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        // Landscape swaps width and height
        assert!((rules[0].width.unwrap() - 841.89).abs() < 0.01);
        assert!((rules[0].height.unwrap() - 595.28).abs() < 0.01);
    }

    #[test]
    fn nth_child_number_matches_second() {
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 1,
            sibling_count: 3,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            ":nth-child(2)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx
        ));
    }

    #[test]
    fn nth_child_odd_even() {
        for (idx, odd_m, even_m) in [(0usize, true, false), (1, false, true), (2, true, false)] {
            let ctx = SelectorContext {
                ancestors: vec![],
                child_index: idx,
                sibling_count: 4,
                preceding_siblings: Vec::new(),
            };
            assert_eq!(
                selector_matches_with_context(
                    ":nth-child(odd)",
                    "p",
                    &[],
                    None,
                    &HashMap::new(),
                    &ctx
                ),
                odd_m
            );
            assert_eq!(
                selector_matches_with_context(
                    ":nth-child(even)",
                    "p",
                    &[],
                    None,
                    &HashMap::new(),
                    &ctx
                ),
                even_m
            );
        }
    }

    #[test]
    fn nth_child_formula() {
        for (idx, expected) in [(0usize, true), (1, false), (2, true), (3, false)] {
            let ctx = SelectorContext {
                ancestors: vec![],
                child_index: idx,
                sibling_count: 5,
                preceding_siblings: Vec::new(),
            };
            assert_eq!(
                selector_matches_with_context(
                    ":nth-child(2n+1)",
                    "p",
                    &[],
                    None,
                    &HashMap::new(),
                    &ctx
                ),
                expected
            );
        }
    }

    #[test]
    fn not_class_excludes() {
        let ctx = SelectorContext::default();
        assert!(selector_matches_with_context(
            ":not(.hidden)",
            "p",
            &["visible"],
            None,
            &HashMap::new(),
            &ctx
        ));
        assert!(!selector_matches_with_context(
            ":not(.hidden)",
            "p",
            &["hidden"],
            None,
            &HashMap::new(),
            &ctx
        ));
    }

    #[test]
    fn not_tag_excludes() {
        let ctx = SelectorContext::default();
        assert!(selector_matches_with_context(
            ":not(div)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx
        ));
        assert!(!selector_matches_with_context(
            ":not(div)",
            "div",
            &[],
            None,
            &HashMap::new(),
            &ctx
        ));
    }

    #[test]
    fn adjacent_sibling_match() {
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 1,
            sibling_count: 3,
            preceding_siblings: vec![("h1".into(), vec![])],
        };
        assert!(selector_matches_with_context(
            "h1 + p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx
        ));
    }

    #[test]
    fn adjacent_sibling_mismatch() {
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 1,
            sibling_count: 3,
            preceding_siblings: vec![("h2".into(), vec![])],
        };
        assert!(!selector_matches_with_context(
            "h1 + p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx
        ));
    }

    #[test]
    fn general_sibling_match() {
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 2,
            sibling_count: 4,
            preceding_siblings: vec![("h1".into(), vec![]), ("div".into(), vec![])],
        };
        assert!(selector_matches_with_context(
            "h1 ~ p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx
        ));
    }

    #[test]
    fn general_sibling_mismatch() {
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 1,
            sibling_count: 3,
            preceding_siblings: vec![("h2".into(), vec![])],
        };
        assert!(!selector_matches_with_context(
            "h1 ~ p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx
        ));
    }

    #[test]
    fn parse_inherit_kw() {
        let s = parse_inline_style("color: inherit");
        assert!(matches!(s.get("color"), Some(CssValue::Keyword(k)) if k == "inherit"));
    }

    #[test]
    fn parse_initial_kw() {
        let s = parse_inline_style("margin-top: initial");
        assert!(matches!(s.get("margin-top"), Some(CssValue::Keyword(k)) if k == "initial"));
    }

    #[test]
    fn parse_unset_kw() {
        let s = parse_inline_style("font-size: unset");
        assert!(matches!(s.get("font-size"), Some(CssValue::Keyword(k)) if k == "unset"));
    }

    #[test]
    fn parse_border_radius() {
        let style = parse_inline_style("border-radius: 10pt");
        match style.get("border-radius") {
            Some(CssValue::Length(v)) => assert!((*v - 10.0).abs() < 0.01),
            other => panic!("Expected Length for border-radius, got {:?}", other),
        }
    }

    #[test]
    fn parse_border_radius_px() {
        let style = parse_inline_style("border-radius: 20px");
        match style.get("border-radius") {
            Some(CssValue::Length(v)) => assert!((*v - 15.0).abs() < 0.01), // 20 * 0.75
            other => panic!("Expected Length for border-radius, got {:?}", other),
        }
    }

    #[test]
    fn parse_outline_shorthand() {
        let style = parse_inline_style("outline: 2px solid red");
        match style.get("outline") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "2px solid red"),
            other => panic!("Expected Keyword for outline, got {:?}", other),
        }
    }

    #[test]
    fn parse_outline_width() {
        let style = parse_inline_style("outline-width: 3pt");
        match style.get("outline-width") {
            Some(CssValue::Length(v)) => assert!((*v - 3.0).abs() < 0.01),
            other => panic!("Expected Length for outline-width, got {:?}", other),
        }
    }

    #[test]
    fn parse_box_sizing_border_box() {
        let style = parse_inline_style("box-sizing: border-box");
        match style.get("box-sizing") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "border-box"),
            other => panic!("Expected Keyword for box-sizing, got {:?}", other),
        }
    }

    #[test]
    fn parse_box_sizing_content_box() {
        let style = parse_inline_style("box-sizing: content-box");
        match style.get("box-sizing") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "content-box"),
            other => panic!("Expected Keyword for box-sizing, got {:?}", other),
        }
    }

    // --- Coverage tests for uncovered lines ---

    #[test]
    fn margin_shorthand_three_values() {
        // Line 73: 3-value margin shorthand => top, right/left, bottom
        let style = parse_inline_style("margin: 10px 20px 30px");
        match style.get("margin-top") {
            Some(CssValue::Length(v)) => assert!((*v - 7.5).abs() < 0.1), // 10px * 0.75
            other => panic!("Expected Length for margin-top, got {:?}", other),
        }
        match style.get("margin-right") {
            Some(CssValue::Length(v)) => assert!((*v - 15.0).abs() < 0.1), // 20px * 0.75
            other => panic!("Expected Length for margin-right, got {:?}", other),
        }
        match style.get("margin-bottom") {
            Some(CssValue::Length(v)) => assert!((*v - 22.5).abs() < 0.1), // 30px * 0.75
            other => panic!("Expected Length for margin-bottom, got {:?}", other),
        }
        match style.get("margin-left") {
            Some(CssValue::Length(v)) => assert!((*v - 15.0).abs() < 0.1), // 20px * 0.75 (same as right)
            other => panic!("Expected Length for margin-left, got {:?}", other),
        }
    }

    #[test]
    fn margin_shorthand_five_values_skipped() {
        // Line 75: 5+ values => continue (skip)
        let style = parse_inline_style("margin: 1px 2px 3px 4px 5px");
        assert!(style.get("margin-top").is_none());
    }

    #[test]
    fn margin_single_auto() {
        // Lines 91-94: margin: auto (single value)
        let style = parse_inline_style("margin: auto");
        for side in &["margin-top", "margin-right", "margin-bottom", "margin-left"] {
            match style.get(side) {
                Some(CssValue::Keyword(k)) => assert_eq!(k, "auto"),
                other => panic!("Expected Keyword 'auto' for {}, got {:?}", side, other),
            }
        }
    }

    #[test]
    fn parse_border_color() {
        // Line 179: border-color property
        let style = parse_inline_style("border-color: red");
        match style.get("border-color") {
            Some(CssValue::Color(c)) => {
                assert_eq!(c.r, 255);
                assert_eq!(c.g, 0);
                assert_eq!(c.b, 0);
            }
            other => panic!("Expected Color for border-color, got {:?}", other),
        }
    }

    #[test]
    fn parse_outline_color() {
        // Line 239: outline-color property
        let style = parse_inline_style("outline-color: blue");
        match style.get("outline-color") {
            Some(CssValue::Color(c)) => {
                assert_eq!(c.r, 0);
                assert_eq!(c.g, 0);
                assert_eq!(c.b, 255);
            }
            other => panic!("Expected Color for outline-color, got {:?}", other),
        }
    }

    #[test]
    fn preprocess_non_media_at_rule() {
        // Line 301: non-media, non-page @-rules pass through as-is
        // Use @font-face which has braces (but starts with @f, not @media or @page)
        // The preprocessor collects the @-rule text up to '{', and since it's
        // neither @media nor @page, it just pushes the at_rule string.
        // We just verify the preprocessor doesn't panic and produces some output.
        let css = "@import url('foo.css')";
        let rules = parse_stylesheet(css);
        // No actual CSS rules, just verifying the @-rule path is exercised
        assert!(rules.is_empty());
    }

    #[test]
    fn page_rule_margin_four_values() {
        // Lines 539-549: @page margin with 4 values
        let css = "@page { margin: 10mm 20mm 30mm 40mm }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert!(rule.margin_top.is_some());
        assert!(rule.margin_right.is_some());
        assert!(rule.margin_bottom.is_some());
        assert!(rule.margin_left.is_some());
        // 10mm * 2.83465 ~= 28.3465
        assert!((rule.margin_top.unwrap() - 28.3465).abs() < 0.1);
        // 20mm * 2.83465 ~= 56.693
        assert!((rule.margin_right.unwrap() - 56.693).abs() < 0.1);
        // 30mm * 2.83465 ~= 85.0395
        assert!((rule.margin_bottom.unwrap() - 85.0395).abs() < 0.1);
        // 40mm * 2.83465 ~= 113.386
        assert!((rule.margin_left.unwrap() - 113.386).abs() < 0.1);
    }

    #[test]
    fn page_rule_margin_three_values_ignored() {
        // Line 552: 3-value margin in @page => no match, falls through
        let css = "@page { margin: 10mm 20mm 30mm }";
        let rules = parse_page_rules(css);
        // 3-value margin is not handled, so no page rule produced
        assert!(rules.is_empty());
    }

    #[test]
    fn page_rule_individual_margins_right_bottom() {
        // Lines 562-564, 568-570: margin-right and margin-bottom
        let css = "@page { margin-right: 15mm; margin-bottom: 25mm }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert!((rule.margin_right.unwrap() - 15.0 * 2.83465).abs() < 0.1);
        assert!((rule.margin_bottom.unwrap() - 25.0 * 2.83465).abs() < 0.1);
    }

    #[test]
    fn page_rule_unknown_property() {
        // Line 579: unknown property in @page => no rule
        let css = "@page { unknown-prop: 10mm }";
        let rules = parse_page_rules(css);
        assert!(rules.is_empty());
    }

    #[test]
    fn page_size_landscape() {
        // Line 615: a4 landscape
        let css = "@page { size: a4 landscape }";
        let rules = parse_page_rules(css);
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        // A4 landscape: width=841.89, height=595.28 (swapped)
        assert!((rule.width.unwrap() - 841.89).abs() < 0.1);
        assert!((rule.height.unwrap() - 595.28).abs() < 0.1);
    }

    #[test]
    fn page_size_unknown_returns_none() {
        // Line 620: unknown page size
        let css = "@page { size: unknown-size }";
        let rules = parse_page_rules(css);
        assert!(rules.is_empty());
    }

    #[test]
    fn page_length_pt_and_px() {
        // Lines 633, 635: parse_page_length with pt and px
        let css_pt = "@page { margin-top: 72pt }";
        let rules_pt = parse_page_rules(css_pt);
        assert_eq!(rules_pt.len(), 1);
        assert!((rules_pt[0].margin_top.unwrap() - 72.0).abs() < 0.01);

        let css_px = "@page { margin-top: 100px }";
        let rules_px = parse_page_rules(css_px);
        assert_eq!(rules_px.len(), 1);
        assert!((rules_px[0].margin_top.unwrap() - 75.0).abs() < 0.01); // 100 * 0.75
    }

    #[test]
    fn page_rule_missing_close_brace() {
        // Line 482: @page with { but no }
        // Call extract_page_rules directly to bypass preprocessor
        let css = "@page { margin-top: 10mm";
        let rules = extract_page_rules(css);
        assert!(rules.is_empty());
    }

    #[test]
    fn page_rule_missing_open_brace() {
        // Line 485: @page with no opening brace
        let css = "@page margin-top: 10mm";
        let rules = extract_page_rules(css);
        assert!(rules.is_empty());
    }

    #[test]
    fn general_sibling_combinator() {
        // Line 713: h1 ~ p
        let ctx = SelectorContext {
            ancestors: Vec::new(),
            child_index: 2,
            sibling_count: 3,
            preceding_siblings: vec![("h1".to_string(), vec![]), ("span".to_string(), vec![])],
        };
        assert!(selector_matches_with_context(
            "h1 ~ p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
        // No matching preceding sibling
        let ctx2 = SelectorContext {
            ancestors: Vec::new(),
            child_index: 1,
            sibling_count: 2,
            preceding_siblings: vec![("div".to_string(), vec![])],
        };
        assert!(!selector_matches_with_context(
            "h1 ~ p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx2,
        ));
        // Current element doesn't match
        assert!(!selector_matches_with_context(
            "h1 ~ p",
            "span",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn adjacent_sibling_combinator() {
        // Lines 737, 751: h1 + p
        let ctx = SelectorContext {
            ancestors: Vec::new(),
            child_index: 1,
            sibling_count: 2,
            preceding_siblings: vec![("h1".to_string(), vec![])],
        };
        assert!(selector_matches_with_context(
            "h1 + p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
        // No preceding sibling at all => false (line 751)
        let ctx_empty = SelectorContext {
            ancestors: Vec::new(),
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: vec![],
        };
        assert!(!selector_matches_with_context(
            "h1 + p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx_empty,
        ));
        // Current element doesn't match
        assert!(!selector_matches_with_context(
            "h1 + p",
            "span",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn child_combinator_no_parent() {
        // Line 783: child combinator with no ancestors
        let ctx = SelectorContext {
            ancestors: Vec::new(),
            child_index: 0,
            sibling_count: 0,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            "div > p",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn child_combinator_current_no_match() {
        // Line 761: child combinator where current element doesn't match
        let parent = ElementNode::new(crate::parser::dom::HtmlTag::Div);
        let ctx = SelectorContext {
            ancestors: vec![&parent],
            child_index: 0,
            sibling_count: 1,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            "div > p",
            "span",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn simple_selector_empty() {
        // Line 863: empty selector
        assert!(!selector_matches("", "p", &[], None));
    }

    #[test]
    fn simple_selector_core_empty() {
        // Line 921: empty core selector via :not() with empty base
        // Using :not(.foo) which has an empty base => should return true if element lacks .foo
        assert!(selector_matches(":not(.foo)", "p", &[], None));
        // And false if element has .foo
        assert!(!selector_matches(":not(.foo)", "p", &["foo"], None));
    }

    #[test]
    fn split_pseudo_class_with_parens() {
        // Lines 962, 964-965: parentheses depth tracking in split_pseudo_class
        // This is tested implicitly through :nth-child and :not
        // Test a selector that has parens inside brackets
        let attrs = HashMap::from([("data-x".to_string(), "a(b)".to_string())]);
        assert!(selector_matches_with_context(
            "p[data-x=\"a(b)\"]",
            "p",
            &[],
            None,
            &attrs,
            &SelectorContext::default(),
        ));
    }

    #[test]
    fn pseudo_class_unknown() {
        // Line 990: unknown pseudo-class => false
        assert!(!selector_matches(":hover", "p", &[], None));
    }

    #[test]
    fn nth_child_formula_a_zero() {
        // Line 1015: a==0, so check n==b (0n+3 means 3rd child only)
        let ctx = SelectorContext {
            ancestors: Vec::new(),
            child_index: 2, // 0-based, so 3rd child
            sibling_count: 5,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "p:nth-child(0n+3)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
        let ctx2 = SelectorContext {
            ancestors: Vec::new(),
            child_index: 0,
            sibling_count: 5,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            "p:nth-child(0n+3)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx2,
        ));
    }

    #[test]
    fn nth_child_formula_negative_a() {
        // Line 1024: a < 0 => matches first few children
        // -n+3 means children 1, 2, 3
        let make_ctx = |idx: usize| SelectorContext {
            ancestors: Vec::new(),
            child_index: idx,
            sibling_count: 5,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "p:nth-child(-n+3)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &make_ctx(0),
        ));
        assert!(selector_matches_with_context(
            "p:nth-child(-n+3)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &make_ctx(2),
        ));
        assert!(!selector_matches_with_context(
            "p:nth-child(-n+3)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &make_ctx(3),
        ));
    }

    #[test]
    fn nth_child_invalid_formula() {
        // Line 1027: unparseable formula => false
        let ctx = SelectorContext {
            ancestors: Vec::new(),
            child_index: 0,
            sibling_count: 5,
            preceding_siblings: Vec::new(),
        };
        assert!(!selector_matches_with_context(
            "p:nth-child(abc)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx,
        ));
    }

    #[test]
    fn parse_an_plus_b_edge_cases() {
        // Lines 1040, 1042, 1050: +n => a=1, -n => a=-1, n alone => b=0
        // n+2: a=1, b=2
        let ctx1 = SelectorContext {
            ancestors: Vec::new(),
            child_index: 1, // 2nd child
            sibling_count: 5,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "p:nth-child(n+2)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx1,
        ));
        // +n+1 should also work (a=1, b=1 => matches all)
        let ctx0 = SelectorContext {
            ancestors: Vec::new(),
            child_index: 0,
            sibling_count: 5,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "p:nth-child(+n+1)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx0,
        ));
        // 3n: a=3, b=0 => matches children 3, 6, 9...
        let ctx2 = SelectorContext {
            ancestors: Vec::new(),
            child_index: 2, // 3rd child (1-based: 3)
            sibling_count: 10,
            preceding_siblings: Vec::new(),
        };
        assert!(selector_matches_with_context(
            "p:nth-child(3n)",
            "p",
            &[],
            None,
            &HashMap::new(),
            &ctx2,
        ));
    }

    #[test]
    fn attribute_selector_unclosed_bracket() {
        // Line 1066: unclosed bracket returns false
        let attrs = HashMap::from([("href".to_string(), "foo".to_string())]);
        assert!(!selector_matches_with_context(
            "a[href",
            "a",
            &[],
            None,
            &attrs,
            &SelectorContext::default(),
        ));
    }

    #[test]
    fn attribute_selector_value_not_present() {
        // Line 1084: attr=val where attribute is not in the map
        let attrs = HashMap::new();
        assert!(!selector_matches_with_context(
            "input[type=\"text\"]",
            "input",
            &[],
            None,
            &attrs,
            &SelectorContext::default(),
        ));
    }

    #[test]
    fn not_pseudo_class_selector() {
        // :not() selector tests
        // :not(div) on a p element => should match
        assert!(selector_matches(":not(div)", "p", &[], None));
        // :not(p) on a p element => should NOT match
        assert!(!selector_matches(":not(p)", "p", &[], None));
        // p:not(.active) on p without .active => should match
        assert!(selector_matches("p:not(.active)", "p", &[], None));
        // p:not(.active) on p with .active => should NOT match
        assert!(!selector_matches("p:not(.active)", "p", &["active"], None));
    }

    // ==================== New feature tests ====================

    #[test]
    fn parse_percentage_unit() {
        let map = parse_inline_style("width: 50%");
        match map.get("width") {
            Some(CssValue::Percentage(v)) => assert!((*v - 50.0).abs() < 0.01),
            other => panic!("Expected Percentage, got {:?}", other),
        }
    }

    #[test]
    fn parse_rem_unit() {
        let map = parse_inline_style("font-size: 2rem");
        match map.get("font-size") {
            Some(CssValue::Rem(v)) => assert!((*v - 2.0).abs() < 0.01),
            other => panic!("Expected Rem, got {:?}", other),
        }
    }

    #[test]
    fn parse_vw_unit() {
        let map = parse_inline_style("width: 100vw");
        match map.get("width") {
            Some(CssValue::Vw(v)) => assert!((*v - 100.0).abs() < 0.01),
            other => panic!("Expected Vw, got {:?}", other),
        }
    }

    #[test]
    fn parse_vh_unit() {
        let map = parse_inline_style("height: 50vh");
        match map.get("height") {
            Some(CssValue::Vh(v)) => assert!((*v - 50.0).abs() < 0.01),
            other => panic!("Expected Vh, got {:?}", other),
        }
    }

    #[test]
    fn parse_calc_basic() {
        let map = parse_inline_style("width: calc(100% - 20pt)");
        match map.get("width") {
            Some(CssValue::Calc(tokens)) => {
                assert_eq!(tokens.len(), 3);
                assert!(matches!(&tokens[0], CalcToken::Percent(v) if (*v - 100.0).abs() < 0.01));
                assert!(matches!(&tokens[1], CalcToken::Op(CalcOp::Sub)));
                assert!(matches!(&tokens[2], CalcToken::Length(v) if (*v - 20.0).abs() < 0.01));
            }
            other => panic!("Expected Calc, got {:?}", other),
        }
    }

    #[test]
    fn parse_calc_addition() {
        let map = parse_inline_style("width: calc(50% + 10px)");
        match map.get("width") {
            Some(CssValue::Calc(tokens)) => {
                assert_eq!(tokens.len(), 3);
                assert!(matches!(&tokens[0], CalcToken::Percent(v) if (*v - 50.0).abs() < 0.01));
                assert!(matches!(&tokens[1], CalcToken::Op(CalcOp::Add)));
                // 10px = 7.5pt
                assert!(matches!(&tokens[2], CalcToken::Length(v) if (*v - 7.5).abs() < 0.01));
            }
            other => panic!("Expected Calc, got {:?}", other),
        }
    }

    #[test]
    fn parse_calc_multiply() {
        let map = parse_inline_style("width: calc(10pt * 3)");
        match map.get("width") {
            Some(CssValue::Calc(tokens)) => {
                assert_eq!(tokens.len(), 3);
                assert!(matches!(&tokens[0], CalcToken::Length(v) if (*v - 10.0).abs() < 0.01));
                assert!(matches!(&tokens[1], CalcToken::Op(CalcOp::Mul)));
                assert!(matches!(&tokens[2], CalcToken::Length(v) if (*v - 3.0).abs() < 0.01));
            }
            other => panic!("Expected Calc, got {:?}", other),
        }
    }

    #[test]
    fn parse_calc_divide() {
        let map = parse_inline_style("width: calc(100pt / 2)");
        match map.get("width") {
            Some(CssValue::Calc(tokens)) => {
                assert_eq!(tokens.len(), 3);
                assert!(matches!(&tokens[0], CalcToken::Length(v) if (*v - 100.0).abs() < 0.01));
                assert!(matches!(&tokens[1], CalcToken::Op(CalcOp::Div)));
                assert!(matches!(&tokens[2], CalcToken::Length(v) if (*v - 2.0).abs() < 0.01));
            }
            other => panic!("Expected Calc, got {:?}", other),
        }
    }

    #[test]
    fn parse_calc_with_vw() {
        let map = parse_inline_style("width: calc(100vw - 20pt)");
        match map.get("width") {
            Some(CssValue::Calc(tokens)) => {
                assert_eq!(tokens.len(), 3);
                assert!(matches!(&tokens[0], CalcToken::Vw(v) if (*v - 100.0).abs() < 0.01));
                assert!(matches!(&tokens[1], CalcToken::Op(CalcOp::Sub)));
                assert!(matches!(&tokens[2], CalcToken::Length(v) if (*v - 20.0).abs() < 0.01));
            }
            other => panic!("Expected Calc, got {:?}", other),
        }
    }

    #[test]
    fn parse_var_simple() {
        let map = parse_inline_style("width: var(--my-width)");
        match map.get("width") {
            Some(CssValue::Var(name, fallback)) => {
                assert_eq!(name, "--my-width");
                assert!(fallback.is_none());
            }
            other => panic!("Expected Var, got {:?}", other),
        }
    }

    #[test]
    fn parse_var_with_fallback() {
        let map = parse_inline_style("color: var(--text-color, red)");
        match map.get("color") {
            Some(CssValue::Var(name, fallback)) => {
                assert_eq!(name, "--text-color");
                assert_eq!(fallback.as_deref(), Some("red"));
            }
            other => panic!("Expected Var, got {:?}", other),
        }
    }

    #[test]
    fn parse_custom_property_declaration() {
        let map = parse_inline_style("--my-var: 10pt");
        match map.get("--my-var") {
            Some(CssValue::Keyword(v)) => assert_eq!(v, "10pt"),
            other => panic!("Expected Keyword with raw value, got {:?}", other),
        }
    }

    #[test]
    fn parse_z_index_positive() {
        let map = parse_inline_style("z-index: 5");
        match map.get("z-index") {
            Some(CssValue::Number(v)) => assert!((*v - 5.0).abs() < 0.01),
            other => panic!("Expected Number(5), got {:?}", other),
        }
    }

    #[test]
    fn parse_z_index_negative() {
        let map = parse_inline_style("z-index: -1");
        match map.get("z-index") {
            Some(CssValue::Number(v)) => assert!((*v - (-1.0)).abs() < 0.01),
            other => panic!("Expected Number(-1), got {:?}", other),
        }
    }

    #[test]
    fn parse_z_index_auto() {
        let map = parse_inline_style("z-index: auto");
        match map.get("z-index") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "auto"),
            other => panic!("Expected Keyword(auto), got {:?}", other),
        }
    }

    #[test]
    fn parse_stylesheet_custom_properties() {
        let css = ":root { --main-color: #ff0000; --spacing: 10pt; } .box { color: var(--main-color); padding-left: var(--spacing); }";
        let rules = parse_stylesheet(css);
        assert!(rules.len() >= 2);

        // The :root rule should contain --main-color and --spacing
        let root_rule = &rules[0];
        assert!(root_rule.declarations.get("--main-color").is_some());
        assert!(root_rule.declarations.get("--spacing").is_some());

        // The .box rule should have var() references
        let box_rule = &rules[1];
        assert!(matches!(
            box_rule.declarations.get("color"),
            Some(CssValue::Var(_, _))
        ));
    }

    #[test]
    fn parse_calc_in_stylesheet() {
        let css = ".container { width: calc(100% - 40pt); }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert!(matches!(
            rules[0].declarations.get("width"),
            Some(CssValue::Calc(_))
        ));
    }

    #[test]
    fn parse_rem_in_stylesheet() {
        let css = "p { font-size: 1.5rem; }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert!(matches!(
            rules[0].declarations.get("font-size"),
            Some(CssValue::Rem(v)) if (*v - 1.5).abs() < 0.01
        ));
    }

    #[test]
    fn parse_calc_with_rem() {
        let map = parse_inline_style("margin-top: calc(2rem + 5pt)");
        match map.get("margin-top") {
            Some(CssValue::Calc(tokens)) => {
                assert_eq!(tokens.len(), 3);
                assert!(matches!(&tokens[0], CalcToken::Rem(v) if (*v - 2.0).abs() < 0.01));
                assert!(matches!(&tokens[1], CalcToken::Op(CalcOp::Add)));
                assert!(matches!(&tokens[2], CalcToken::Length(v) if (*v - 5.0).abs() < 0.01));
            }
            other => panic!("Expected Calc, got {:?}", other),
        }
    }

    // ===== @font-face parsing tests =====

    #[test]
    fn parse_font_face_basic() {
        let css = r#"
            @font-face {
                font-family: "MyFont";
                src: url("fonts/MyFont.ttf");
            }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].font_family, "MyFont");
        assert_eq!(rules[0].src_path, "fonts/MyFont.ttf");
    }

    #[test]
    fn parse_font_face_single_quotes() {
        let css = r#"
            @font-face {
                font-family: 'CustomFont';
                src: url('assets/custom.ttf');
            }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].font_family, "CustomFont");
        assert_eq!(rules[0].src_path, "assets/custom.ttf");
    }

    #[test]
    fn parse_font_face_no_quotes_in_url() {
        let css = r#"
            @font-face {
                font-family: "NoQuotes";
                src: url(path/to/font.ttf);
            }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].src_path, "path/to/font.ttf");
    }

    #[test]
    fn parse_font_face_multiple() {
        let css = r#"
            @font-face {
                font-family: "FontA";
                src: url("a.ttf");
            }
            @font-face {
                font-family: "FontB";
                src: url("b.ttf");
            }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].font_family, "FontA");
        assert_eq!(rules[1].font_family, "FontB");
    }

    #[test]
    fn parse_font_face_rejects_http_url() {
        let css = r#"
            @font-face {
                font-family: "Remote";
                src: url("https://example.com/font.ttf");
            }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 0, "Remote URLs should be rejected");
    }

    #[test]
    fn parse_font_face_rejects_http_url_no_s() {
        let css = r#"
            @font-face {
                font-family: "Remote";
                src: url("http://example.com/font.ttf");
            }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 0, "Remote HTTP URLs should be rejected");
    }

    #[test]
    fn parse_font_face_missing_family() {
        let css = r#"
            @font-face {
                src: url("font.ttf");
            }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 0, "Missing font-family should skip rule");
    }

    #[test]
    fn parse_font_face_missing_src() {
        let css = r#"
            @font-face {
                font-family: "NoSrc";
            }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 0, "Missing src should skip rule");
    }

    #[test]
    fn parse_font_face_with_other_rules() {
        let css = r#"
            body { color: black; }
            @font-face {
                font-family: "Mixed";
                src: url("mixed.ttf");
            }
            p { font-size: 14px; }
        "#;
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].font_family, "Mixed");

        // Regular rules should not include font-face
        let style_rules = parse_stylesheet(css);
        for rule in &style_rules {
            assert!(
                !rule.selector.contains("@font-face"),
                "font-face should not appear in regular rules"
            );
        }
    }

    #[test]
    fn parse_font_face_in_media_print() {
        let css = r#"
            @media print {
                @font-face {
                    font-family: "PrintFont";
                    src: url("print.ttf");
                }
            }
        "#;
        // @font-face inside @media print should be extracted
        // (media print is inlined by preprocessor)
        let rules = parse_font_face_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].font_family, "PrintFont");
    }

    // ===== @import parsing tests =====

    #[test]
    fn parse_import_quoted_string() {
        let css = r#"@import "styles.css";"#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "styles.css");
    }

    #[test]
    fn parse_import_single_quoted() {
        let css = "@import 'other.css';";
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "other.css");
    }

    #[test]
    fn parse_import_url_function() {
        let css = r#"@import url("path/to/styles.css");"#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "path/to/styles.css");
    }

    #[test]
    fn parse_import_url_single_quotes() {
        let css = "@import url('path/to/styles.css');";
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "path/to/styles.css");
    }

    #[test]
    fn parse_import_url_no_quotes() {
        let css = "@import url(styles.css);";
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "styles.css");
    }

    #[test]
    fn parse_import_multiple() {
        let css = r#"
            @import "a.css";
            @import url("b.css");
            body { color: black; }
        "#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].path, "a.css");
        assert_eq!(rules[1].path, "b.css");
    }

    #[test]
    fn parse_import_rejects_https() {
        let css = r#"@import "https://example.com/styles.css";"#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 0, "Remote HTTPS URLs should be rejected");
    }

    #[test]
    fn parse_import_rejects_http() {
        let css = r#"@import url("http://example.com/styles.css");"#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 0, "Remote HTTP URLs should be rejected");
    }

    #[test]
    fn parse_import_no_rules_for_regular_css() {
        let css = "body { color: red; } p { font-size: 14px; }";
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 0);
    }

    // ===== strip_import_rules tests =====

    #[test]
    fn strip_import_preserves_regular_rules() {
        let css = r#"@import "a.css"; body { color: red; }"#;
        let stripped = strip_import_rules(css);
        assert!(stripped.contains("body"));
        assert!(stripped.contains("color: red"));
        assert!(!stripped.contains("@import"));
    }

    #[test]
    fn strip_import_multiple() {
        let css = r#"@import "a.css"; @import "b.css"; p { margin: 0; }"#;
        let stripped = strip_import_rules(css);
        assert!(!stripped.contains("@import"));
        assert!(stripped.contains("margin: 0"));
    }

    // ===== resolve_imports tests =====

    #[test]
    fn resolve_imports_no_imports() {
        let css = "body { color: red; }";
        let resolved = resolve_imports(css, std::path::Path::new("/tmp"), 0);
        assert_eq!(resolved.trim(), css);
    }

    #[test]
    fn resolve_imports_depth_limit() {
        let css = r#"@import "a.css"; body { color: red; }"#;
        // At max depth, imports should not be resolved
        let resolved = resolve_imports(css, std::path::Path::new("/tmp"), MAX_IMPORT_DEPTH);
        assert!(resolved.contains("@import"));
    }

    #[test]
    fn resolve_imports_missing_file() {
        let css = r#"@import "nonexistent.css"; body { color: red; }"#;
        // Missing files should be silently ignored
        let resolved = resolve_imports(css, std::path::Path::new("/tmp/nonexistent"), 0);
        assert!(resolved.contains("body"));
    }

    // ===== extract_url_path tests =====

    #[test]
    fn extract_url_path_double_quotes() {
        assert_eq!(
            extract_url_path(r#"url("fonts/test.ttf")"#),
            Some("fonts/test.ttf".to_string())
        );
    }

    #[test]
    fn extract_url_path_single_quotes() {
        assert_eq!(
            extract_url_path("url('fonts/test.ttf')"),
            Some("fonts/test.ttf".to_string())
        );
    }

    #[test]
    fn extract_url_path_no_quotes() {
        assert_eq!(
            extract_url_path("url(fonts/test.ttf)"),
            Some("fonts/test.ttf".to_string())
        );
    }

    #[test]
    fn extract_url_path_empty() {
        assert_eq!(extract_url_path("url()"), None);
    }

    #[test]
    fn extract_url_path_no_url_function() {
        assert_eq!(extract_url_path("fonts/test.ttf"), None);
    }

    // --- Pseudo-element parsing tests ---
    #[test]
    fn parse_before_pseudo_element() {
        let css = r#"p::before { content: "Hello" }"#;
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pseudo_element, Some(PseudoElement::Before));
        assert_eq!(rules[0].selector, "p");
    }

    #[test]
    fn parse_after_pseudo_element() {
        let css = r#"p::after { content: "!" }"#;
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pseudo_element, Some(PseudoElement::After));
        assert_eq!(rules[0].selector, "p");
    }

    #[test]
    fn parse_single_colon_before() {
        let css = r#"p:before { content: "Hey" }"#;
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pseudo_element, Some(PseudoElement::Before));
        assert_eq!(rules[0].selector, "p");
    }

    #[test]
    fn parse_single_colon_after() {
        let css = r#"p:after { content: "!" }"#;
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pseudo_element, Some(PseudoElement::After));
        assert_eq!(rules[0].selector, "p");
    }

    #[test]
    fn parse_class_with_pseudo_element() {
        let css = r#".note::before { content: "Note: " }"#;
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pseudo_element, Some(PseudoElement::Before));
        assert_eq!(rules[0].selector, ".note");
    }

    #[test]
    fn parse_rule_without_pseudo_element() {
        let css = "p { color: red }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pseudo_element, None);
        assert_eq!(rules[0].selector, "p");
    }

    #[test]
    fn parse_content_property_as_keyword() {
        let map = parse_inline_style(r#"content: "hello world""#);
        assert!(map.get("content").is_some());
        if let Some(CssValue::Keyword(k)) = map.get("content") {
            assert_eq!(k, "\"hello world\"");
        }
    }

    #[test]
    fn parse_counter_reset_as_keyword() {
        let map = parse_inline_style("counter-reset: section 0");
        assert!(map.get("counter-reset").is_some());
        if let Some(CssValue::Keyword(k)) = map.get("counter-reset") {
            assert_eq!(k, "section 0");
        }
    }

    #[test]
    fn parse_counter_increment_as_keyword() {
        let map = parse_inline_style("counter-increment: section");
        assert!(map.get("counter-increment").is_some());
        if let Some(CssValue::Keyword(k)) = map.get("counter-increment") {
            assert_eq!(k, "section");
        }
    }

    #[test]
    fn parse_list_style_type_as_keyword() {
        let map = parse_inline_style("list-style-type: circle");
        assert!(map.get("list-style-type").is_some());
        if let Some(CssValue::Keyword(k)) = map.get("list-style-type") {
            assert_eq!(k, "circle");
        }
    }

    #[test]
    fn parse_list_style_position_as_keyword() {
        let map = parse_inline_style("list-style-position: inside");
        assert!(map.get("list-style-position").is_some());
        if let Some(CssValue::Keyword(k)) = map.get("list-style-position") {
            assert_eq!(k, "inside");
        }
    }

    #[test]
    fn parse_list_style_shorthand_as_keyword() {
        let map = parse_inline_style("list-style: circle inside");
        assert!(map.get("list-style").is_some());
        if let Some(CssValue::Keyword(k)) = map.get("list-style") {
            assert_eq!(k, "circle inside");
        }
    }

    // --- Coverage tests for uncovered lines ---

    #[test]
    fn parse_border_color_as_color() {
        // Covers line 238: border-color -> parse_color(val)
        let map = parse_inline_style("border-color: #ff0000");
        match map.get("border-color") {
            Some(CssValue::Color(c)) => assert_eq!(c.r, 255),
            other => panic!("Expected Color, got {:?}", other),
        }
    }

    #[test]
    fn parse_z_index_invalid_returns_none() {
        // Covers line 249: z-index with non-integer, non-auto value
        let map = parse_inline_style("z-index: abc");
        assert!(map.get("z-index").is_none());
    }

    #[test]
    fn parse_outline_color_as_color() {
        // Covers line 327: outline-color -> parse_color(val)
        let map = parse_inline_style("outline-color: blue");
        match map.get("outline-color") {
            Some(CssValue::Color(c)) => {
                assert_eq!(c.r, 0);
                assert_eq!(c.g, 0);
                assert_eq!(c.b, 255);
            }
            other => panic!("Expected Color, got {:?}", other),
        }
    }

    #[test]
    fn parse_text_transform_keyword() {
        // Covers line 374: text-transform as keyword
        let map = parse_inline_style("text-transform: uppercase");
        match map.get("text-transform") {
            Some(CssValue::Keyword(k)) => assert_eq!(k, "uppercase"),
            other => panic!("Expected Keyword, got {:?}", other),
        }
    }

    #[test]
    fn preprocess_non_media_at_rule_passthrough() {
        // Covers line 443: non-media @-rules pass through
        let css = "@charset \"utf-8\"; p { color: red; }";
        let result = preprocess_media_queries(css);
        assert!(result.contains("@charset"));
        assert!(result.contains("p { color: red; }"));
    }

    #[test]
    fn parse_length_with_var_function() {
        // Covers line 483: parse_length delegates to parse_var_function
        let map = parse_inline_style("width: var(--my-width)");
        match map.get("width") {
            Some(CssValue::Var(name, fallback)) => {
                assert_eq!(name, "--my-width");
                assert!(fallback.is_none());
            }
            other => panic!("Expected Var, got {:?}", other),
        }
    }

    #[test]
    fn parse_length_with_calc_expression() {
        // Covers line 488: parse_length delegates to parse_calc_expression
        let map = parse_inline_style("width: calc(100px - 20px)");
        match map.get("width") {
            Some(CssValue::Calc(tokens)) => {
                assert!(!tokens.is_empty());
            }
            other => panic!("Expected Calc, got {:?}", other),
        }
    }

    #[test]
    fn parse_var_with_invalid_name_and_fallback() {
        // Covers line 528: var() with non-"--" name and fallback returns None
        let result = parse_var_function("var(invalid, fallback)");
        assert!(result.is_none());
    }

    #[test]
    fn parse_var_with_invalid_name_no_fallback() {
        // Covers line 535: var() without fallback, invalid name
        let result = parse_var_function("var(invalid)");
        assert!(result.is_none());
    }

    #[test]
    fn parse_calc_empty_expression() {
        // Covers line 545: calc() with empty content
        let result = parse_calc_expression("calc()");
        assert!(result.is_none());
    }

    #[test]
    fn tokenize_calc_trailing_whitespace() {
        // Covers line 561: tokenize_calc with trailing whitespace
        let tokens = tokenize_calc("10px   ").unwrap();
        assert_eq!(tokens.len(), 1);
    }

    #[test]
    fn tokenize_calc_with_sign_prefix() {
        // Covers lines 602-603: negative/positive sign on value
        let tokens = tokenize_calc("-5px + 10px").unwrap();
        assert!(tokens.len() >= 3);
    }

    #[test]
    fn tokenize_calc_invalid_bare_sign() {
        // Covers line 626: bare sign with no digits returns None
        let result = tokenize_calc("+");
        assert!(result.is_none());
    }

    #[test]
    fn tokenize_calc_unknown_unit() {
        // Covers line 638: unknown unit returns None
        let result = tokenize_calc("10xyz");
        assert!(result.is_none());
    }

    #[test]
    fn parse_font_face_malformed_no_brace() {
        // Covers line 808: @font-face with no opening brace
        let rules = parse_font_face_rules("@font-face no-brace-here");
        assert!(rules.is_empty());
    }

    #[test]
    fn parse_font_face_malformed_no_close_brace() {
        // Covers line 805: extract_font_face_rules with opening brace but no close
        // (preprocess_media_queries adds closing braces, so we test the inner function directly)
        let rules = extract_font_face_rules("@font-face { font-family: test; src: url(test.ttf);");
        assert!(rules.is_empty());
    }

    #[test]
    fn parse_font_face_unknown_property_ignored() {
        // Covers line 847: unknown properties in @font-face are ignored
        let rules = parse_font_face_rules(
            r#"@font-face {
                font-family: "Test";
                src: url("test.ttf");
                font-weight: bold;
            }"#,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].font_family, "Test");
    }

    #[test]
    fn parse_import_rules_empty_path() {
        // Covers line 916: @import with empty quoted path
        let rules = parse_import_rules("@import \"\";");
        assert!(rules.is_empty());
    }

    #[test]
    fn strip_import_rules_malformed_no_semicolon() {
        // Covers line 982: @import with no semicolon
        let result = strip_import_rules("@import url(test.css)");
        // Should handle gracefully (skip the rest)
        assert!(result.is_empty());
    }

    #[test]
    fn strip_import_rules_non_import_at_rule() {
        // Covers lines 987-989, 991-992: non-@import @ rules pass through
        let result = strip_import_rules("@charset 'utf-8'; p { color: red; }");
        assert!(result.contains("@charset"));
        assert!(result.contains("p { color: red; }"));
    }

    #[test]
    fn parse_page_size_landscape_unknown_name() {
        // Covers line 1153: landscape with unknown named size
        let result = parse_page_size("unknown landscape");
        assert!(result.is_none());
    }

    #[test]
    fn extract_pseudo_element_bare_before() {
        // Covers line 1218: ::before with empty base becomes "*"
        let (base, pe) = extract_pseudo_element("::before");
        assert_eq!(base, "*");
        assert!(matches!(pe, Some(PseudoElement::Before)));
    }

    #[test]
    fn selector_matches_empty_selector() {
        // Covers line 1428: empty selector returns false
        let ctx = SelectorContext {
            ancestors: vec![],
            child_index: 0,
            sibling_count: 0,
            preceding_siblings: vec![],
        };
        let attrs = std::collections::HashMap::new();
        let result = selector_matches_with_context("", "p", &[], None, &attrs, &ctx);
        assert!(!result);
    }

    #[test]
    fn simple_selector_core_matches_empty_string() {
        // Covers line 1486: empty selector returns false
        let result = simple_selector_core_matches("", "div", &[], None);
        assert!(!result);
    }

    #[test]
    fn resolve_imports_with_real_file() {
        // Covers lines 957-960: resolve_imports loads a real file
        let dir = std::env::temp_dir().join("ironpress_css_test");
        std::fs::create_dir_all(&dir).unwrap();
        let imported_file = dir.join("imported.css");
        std::fs::write(&imported_file, "body { color: green; }").unwrap();
        let css = "@import \"imported.css\";\np { font-size: 12pt; }";
        let result = resolve_imports(css, &dir, 0);
        assert!(result.contains("body { color: green; }"));
        assert!(result.contains("p { font-size: 12pt; }"));
        std::fs::remove_file(&imported_file).ok();
        std::fs::remove_dir(&dir).ok();
    }
}
