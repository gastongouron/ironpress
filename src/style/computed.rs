use crate::parser::css::{selector_matches, CssRule, CssValue, StyleMap};
use crate::parser::dom::HtmlTag;
use crate::style::defaults::default_style;
use crate::types::{Color, EdgeSizes};

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Font weight.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
}

/// Font style.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

/// Fully resolved style for a node.
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub color: Color,
    pub background_color: Option<Color>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub text_align: TextAlign,
    pub text_decoration_underline: bool,
    pub line_height: f32,
    pub page_break_before: bool,
    pub page_break_after: bool,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            font_size: 12.0,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            color: Color::BLACK,
            background_color: None,
            margin: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            text_align: TextAlign::Left,
            text_decoration_underline: false,
            line_height: 1.4,
            page_break_before: false,
            page_break_after: false,
        }
    }
}

/// Compute the style for a node given its tag, inline styles, and parent style.
pub fn compute_style(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
) -> ComputedStyle {
    compute_style_with_rules(tag, inline_style, parent, &[], "", &[], None)
}

/// Compute style with stylesheet rules, class list, and id.
pub fn compute_style_with_rules(
    tag: HtmlTag,
    inline_style: Option<&str>,
    parent: &ComputedStyle,
    rules: &[CssRule],
    tag_name: &str,
    classes: &[&str],
    id: Option<&str>,
) -> ComputedStyle {
    let mut style = parent.clone();

    // Reset block-level properties that don't inherit
    if tag.is_block() {
        style.margin = EdgeSizes::default();
        style.padding = EdgeSizes::default();
        style.background_color = None;
    }

    // Apply tag defaults
    let defaults = default_style(tag);
    apply_style_map(&mut style, &defaults);

    // Apply stylesheet rules (between defaults and inline)
    for rule in rules {
        if selector_matches(&rule.selector, tag_name, classes, id) {
            apply_style_map(&mut style, &rule.declarations);
        }
    }

    // Apply inline styles (override everything)
    if let Some(css_str) = inline_style {
        let inline = crate::parser::css::parse_inline_style(css_str);
        apply_style_map(&mut style, &inline);
    }

    style
}

fn apply_style_map(style: &mut ComputedStyle, map: &StyleMap) {
    if let Some(CssValue::Length(v)) = map.get("font-size") {
        style.font_size = *v;
    }
    if let Some(CssValue::Number(v)) = map.get("font-size") {
        // em value — multiply by current font-size
        style.font_size *= *v;
    }

    if let Some(CssValue::Keyword(k)) = map.get("font-weight") {
        style.font_weight = if k == "bold" || k == "700" || k == "800" || k == "900" {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        };
    }

    if let Some(CssValue::Keyword(k)) = map.get("font-style") {
        style.font_style = if k == "italic" || k == "oblique" {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        };
    }

    if let Some(CssValue::Color(c)) = map.get("color") {
        style.color = *c;
    }

    if let Some(CssValue::Color(c)) = map.get("background-color") {
        style.background_color = Some(*c);
    }

    if let Some(CssValue::Length(v)) = map.get("margin-top") {
        style.margin.top = *v;
    }
    if let Some(CssValue::Length(v)) = map.get("margin-right") {
        style.margin.right = *v;
    }
    if let Some(CssValue::Length(v)) = map.get("margin-bottom") {
        style.margin.bottom = *v;
    }
    if let Some(CssValue::Length(v)) = map.get("margin-left") {
        style.margin.left = *v;
    }

    if let Some(CssValue::Length(v)) = map.get("padding-top") {
        style.padding.top = *v;
    }
    if let Some(CssValue::Length(v)) = map.get("padding-right") {
        style.padding.right = *v;
    }
    if let Some(CssValue::Length(v)) = map.get("padding-bottom") {
        style.padding.bottom = *v;
    }
    if let Some(CssValue::Length(v)) = map.get("padding-left") {
        style.padding.left = *v;
    }

    if let Some(CssValue::Keyword(k)) = map.get("text-align") {
        style.text_align = match k.as_str() {
            "center" => TextAlign::Center,
            "right" => TextAlign::Right,
            _ => TextAlign::Left,
        };
    }

    if let Some(CssValue::Keyword(k)) = map.get("text-decoration") {
        style.text_decoration_underline = k == "underline";
    }

    if let Some(CssValue::Number(v)) = map.get("line-height") {
        style.line_height = *v;
    }
    if let Some(CssValue::Length(v)) = map.get("line-height") {
        style.line_height = *v / style.font_size;
    }

    if let Some(CssValue::Keyword(k)) = map.get("page-break-before") {
        style.page_break_before = k == "always";
    }
    if let Some(CssValue::Keyword(k)) = map.get("page-break-after") {
        style.page_break_after = k == "always";
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h1_defaults() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::H1, None, &parent);
        assert_eq!(style.font_size, 24.0);
        assert_eq!(style.font_weight, FontWeight::Bold);
    }

    #[test]
    fn inline_overrides_defaults() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::H1, Some("font-size: 36pt"), &parent);
        assert_eq!(style.font_size, 36.0);
        assert_eq!(style.font_weight, FontWeight::Bold); // still bold from defaults
    }

    #[test]
    fn color_inherited() {
        let mut parent = ComputedStyle::default();
        parent.color = Color::rgb(255, 0, 0);
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.color.r, 255);
    }

    #[test]
    fn bold_tag() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Strong, None, &parent);
        assert_eq!(style.font_weight, FontWeight::Bold);
    }

    #[test]
    fn italic_tag() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Em, None, &parent);
        assert_eq!(style.font_style, FontStyle::Italic);
    }

    #[test]
    fn em_font_size() {
        let parent = ComputedStyle::default(); // font_size = 12.0
        let style = compute_style(HtmlTag::Span, Some("font-size: 2em"), &parent);
        // em gets parsed as Number, then multiplied by parent font_size
        assert!((style.font_size - 24.0).abs() < 0.1);
    }

    #[test]
    fn font_weight_normal() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-weight: normal"), &parent);
        assert_eq!(style.font_weight, FontWeight::Normal);
    }

    #[test]
    fn font_style_normal() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-style: normal"), &parent);
        assert_eq!(style.font_style, FontStyle::Normal);
    }

    #[test]
    fn background_color_applied() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("background-color: red"), &parent);
        assert!(style.background_color.is_some());
        let bg = style.background_color.unwrap();
        assert_eq!(bg.r, 255);
    }

    #[test]
    fn margin_and_padding_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("margin-top: 10pt; margin-right: 20pt; margin-bottom: 30pt; margin-left: 40pt; padding-top: 5pt; padding-right: 6pt; padding-bottom: 7pt; padding-left: 8pt"),
            &parent,
        );
        assert!((style.margin.top - 10.0).abs() < 0.1);
        assert!((style.margin.right - 20.0).abs() < 0.1);
        assert!((style.margin.bottom - 30.0).abs() < 0.1);
        assert!((style.margin.left - 40.0).abs() < 0.1);
        assert!((style.padding.top - 5.0).abs() < 0.1);
        assert!((style.padding.right - 6.0).abs() < 0.1);
        assert!((style.padding.bottom - 7.0).abs() < 0.1);
        assert!((style.padding.left - 8.0).abs() < 0.1);
    }

    #[test]
    fn text_align_center_and_right() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("text-align: center"), &parent);
        assert_eq!(style.text_align, TextAlign::Center);
        let style = compute_style(HtmlTag::Div, Some("text-align: right"), &parent);
        assert_eq!(style.text_align, TextAlign::Right);
    }

    #[test]
    fn text_decoration_underline() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("text-decoration: underline"), &parent);
        assert!(style.text_decoration_underline);
    }

    #[test]
    fn line_height_number_and_length() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("line-height: 18pt"), &parent);
        // 18pt / 12.0 font-size = 1.5
        assert!((style.line_height - 1.5).abs() < 0.1);
    }

    #[test]
    fn page_break_after() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("page-break-after: always"), &parent);
        assert!(style.page_break_after);
    }

    #[test]
    fn text_align_default_fallback() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("text-align: justify"), &parent);
        // "justify" is not handled, should fall back to Left
        assert_eq!(style.text_align, TextAlign::Left);
    }

    #[test]
    fn line_height_as_number() {
        let parent = ComputedStyle::default();
        // line-height: 1.8em — em gets parsed as Number
        let style = compute_style(HtmlTag::Div, Some("line-height: 1.8em"), &parent);
        assert!((style.line_height - 1.8).abs() < 0.1);
    }
}
