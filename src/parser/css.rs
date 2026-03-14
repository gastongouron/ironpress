use std::collections::HashMap;

use crate::types::Color;

/// Parsed CSS property value.
#[derive(Debug, Clone)]
pub enum CssValue {
    Length(f32),
    Color(Color),
    Keyword(String),
    Number(f32),
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
            let prop = prop.trim().to_ascii_lowercase();
            let val = val.trim();

            if let Some(css_val) = parse_value(&prop, val) {
                // Handle shorthand margin/padding
                if (prop == "margin" || prop == "padding") && !prop.contains('-') {
                    if let CssValue::Length(v) = css_val {
                        map.set(&format!("{prop}-top"), CssValue::Length(v));
                        map.set(&format!("{prop}-right"), CssValue::Length(v));
                        map.set(&format!("{prop}-bottom"), CssValue::Length(v));
                        map.set(&format!("{prop}-left"), CssValue::Length(v));
                    }
                } else {
                    map.set(&prop, css_val);
                }
            }
        }
    }

    map
}

fn parse_value(property: &str, val: &str) -> Option<CssValue> {
    let val = val.trim();

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

    // Text-align, text-decoration, display
    if property == "text-align" || property == "text-decoration" || property == "display" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Page break
    if property.starts_with("page-break") {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Length values (font-size, margin, padding, width, height, etc.)
    parse_length(val)
}

fn parse_length(val: &str) -> Option<CssValue> {
    let val = val.trim();

    if let Some(n) = val.strip_suffix("px") {
        n.trim()
            .parse::<f32>()
            .ok()
            .map(|v| CssValue::Length(v * 0.75)) // px to pt
    } else if let Some(n) = val.strip_suffix("pt") {
        n.trim().parse::<f32>().ok().map(CssValue::Length)
    } else if let Some(n) = val.strip_suffix("em") {
        // Store em as negative to distinguish from absolute values
        // Will be resolved during style computation
        n.trim().parse::<f32>().ok().map(CssValue::Number)
    } else if val.parse::<f32>().is_ok() {
        val.parse::<f32>().ok().map(CssValue::Length)
    } else {
        None
    }
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

/// A CSS rule: a selector and its declarations.
#[derive(Debug, Clone)]
pub struct CssRule {
    pub selector: String,
    pub declarations: StyleMap,
}

/// Parse a CSS stylesheet string into a list of rules.
pub fn parse_stylesheet(css: &str) -> Vec<CssRule> {
    let mut rules = Vec::new();

    for block in css.split('}') {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        if let Some((selector, declarations)) = block.split_once('{') {
            let selector = selector.trim().to_string();
            if selector.is_empty() {
                continue;
            }
            let declarations = parse_inline_style(declarations.trim());
            if !declarations.properties.is_empty() {
                rules.push(CssRule {
                    selector,
                    declarations,
                });
            }
        }
    }

    rules
}

/// Check if a CSS selector matches a given element.
pub fn selector_matches(
    selector: &str,
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
) -> bool {
    // Support comma-separated selectors: "h1, h2, h3"
    for part in selector.split(',') {
        let part = part.trim();
        if single_selector_matches(part, tag_name, classes, id) {
            return true;
        }
    }
    false
}

fn single_selector_matches(
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
}
