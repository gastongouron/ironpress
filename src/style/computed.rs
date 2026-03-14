use crate::parser::css::{CssRule, CssValue, StyleMap, selector_matches};
use crate::parser::dom::HtmlTag;
use crate::style::defaults::default_style;
use crate::types::{Color, EdgeSizes};

/// CSS display property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Display {
    Block,
    Inline,
    None,
}

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

/// Font family.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontFamily {
    /// Helvetica (sans-serif) — the default.
    #[default]
    Helvetica,
    /// Times Roman (serif).
    TimesRoman,
    /// Courier (monospace).
    Courier,
}

/// Fully resolved style for a node.
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub font_family: FontFamily,
    pub color: Color,
    pub background_color: Option<Color>,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub text_align: TextAlign,
    pub text_decoration_underline: bool,
    pub text_decoration_line_through: bool,
    pub line_height: f32,
    pub page_break_before: bool,
    pub page_break_after: bool,
    pub border_width: f32,
    pub border_color: Option<Color>,
    pub display: Display,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            font_size: 12.0,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            font_family: FontFamily::Helvetica,
            color: Color::BLACK,
            background_color: None,
            margin: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            text_align: TextAlign::Left,
            text_decoration_underline: false,
            text_decoration_line_through: false,
            line_height: 1.4,
            page_break_before: false,
            page_break_after: false,
            border_width: 0.0,
            border_color: None,
            display: Display::Block,
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

    // Set default display based on tag
    style.display = if tag.is_inline() {
        Display::Inline
    } else {
        Display::Block
    };

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

    if let Some(CssValue::Keyword(k)) = map.get("font-family") {
        let lower = k.to_ascii_lowercase();
        // Strip quotes from font names like "'Times New Roman'" or "\"Courier New\""
        let cleaned = lower.trim_matches(|c| c == '\'' || c == '"');
        style.font_family = match cleaned {
            // Serif → TimesRoman
            "serif" | "times" | "times new roman" | "times-roman" | "georgia" | "garamond"
            | "book antiqua" | "palatino" | "palatino linotype" | "baskerville"
            | "hoefler text" | "cambria" | "droid serif" | "noto serif" | "libre baskerville"
            | "merriweather" | "playfair display" | "lora" => FontFamily::TimesRoman,

            // Monospace → Courier
            "monospace"
            | "courier"
            | "courier new"
            | "lucida console"
            | "lucida sans typewriter"
            | "monaco"
            | "andale mono"
            | "consolas"
            | "source code pro"
            | "fira code"
            | "fira mono"
            | "jetbrains mono"
            | "ibm plex mono"
            | "roboto mono"
            | "ubuntu mono"
            | "droid sans mono"
            | "menlo"
            | "sf mono"
            | "cascadia code"
            | "cascadia mono" => FontFamily::Courier,

            // Sans-serif and everything else → Helvetica
            // Explicit sans-serif mappings: arial, helvetica, sans-serif,
            // helvetica neue, arial black, verdana, tahoma, trebuchet ms,
            // gill sans, lucida sans, lucida grande, system-ui,
            // -apple-system, segoe ui, roboto, open sans, lato, inter,
            // nunito, poppins, montserrat, raleway, ubuntu
            _ => FontFamily::Helvetica,
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
        style.text_decoration_line_through = k == "line-through";
    }

    if let Some(CssValue::Number(v)) = map.get("line-height") {
        style.line_height = *v;
    }
    if let Some(CssValue::Length(v)) = map.get("line-height") {
        style.line_height = *v / style.font_size;
    }

    if let Some(CssValue::Keyword(k)) = map.get("display") {
        style.display = match k.as_str() {
            "none" => Display::None,
            "inline" => Display::Inline,
            "block" => Display::Block,
            _ => style.display,
        };
    }

    if let Some(CssValue::Keyword(k)) = map.get("page-break-before") {
        style.page_break_before = k == "always";
    }
    if let Some(CssValue::Keyword(k)) = map.get("page-break-after") {
        style.page_break_after = k == "always";
    }

    // Border shorthand: "1px solid black"
    if let Some(CssValue::Keyword(k)) = map.get("border") {
        let parts: Vec<&str> = k.split_whitespace().collect();
        // Extract width from first token
        for part in &parts {
            if let Some(n) = part.strip_suffix("px") {
                if let Ok(v) = n.parse::<f32>() {
                    style.border_width = v * 0.75; // px to pt
                }
            } else if let Some(n) = part.strip_suffix("pt") {
                if let Ok(v) = n.parse::<f32>() {
                    style.border_width = v;
                }
            }
        }
        // Extract color from last token
        if let Some(last) = parts.last() {
            if let Some(c) = parse_border_color(last) {
                style.border_color = Some(c);
            }
        }
    }

    if let Some(CssValue::Length(v)) = map.get("border-width") {
        style.border_width = *v;
    }

    if let Some(CssValue::Color(c)) = map.get("border-color") {
        style.border_color = Some(*c);
    }
}

/// Parse a color name or hex value for border shorthand.
fn parse_border_color(val: &str) -> Option<Color> {
    let val = val.to_ascii_lowercase();
    match val.as_str() {
        "black" => Some(Color::rgb(0, 0, 0)),
        "white" => Some(Color::rgb(255, 255, 255)),
        "red" => Some(Color::rgb(255, 0, 0)),
        "green" => Some(Color::rgb(0, 128, 0)),
        "blue" => Some(Color::rgb(0, 0, 255)),
        "yellow" => Some(Color::rgb(255, 255, 0)),
        "orange" => Some(Color::rgb(255, 165, 0)),
        "purple" => Some(Color::rgb(128, 0, 128)),
        "gray" | "grey" => Some(Color::rgb(128, 128, 128)),
        _ => {
            if let Some(hex) = val.strip_prefix('#') {
                parse_hex_to_color(hex)
            } else {
                None
            }
        }
    }
}

fn parse_hex_to_color(hex: &str) -> Option<Color> {
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            Some(Color::rgb(r, g, b))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::rgb(r, g, b))
        }
        _ => None,
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
            Some(
                "margin-top: 10pt; margin-right: 20pt; margin-bottom: 30pt; margin-left: 40pt; padding-top: 5pt; padding-right: 6pt; padding-bottom: 7pt; padding-left: 8pt",
            ),
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

    #[test]
    fn text_decoration_line_through() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("text-decoration: line-through"),
            &parent,
        );
        assert!(style.text_decoration_line_through);
        assert!(!style.text_decoration_underline);
    }

    #[test]
    fn del_tag_has_line_through() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Del, None, &parent);
        assert!(style.text_decoration_line_through);
    }

    #[test]
    fn s_tag_has_line_through() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::S, None, &parent);
        assert!(style.text_decoration_line_through);
    }

    #[test]
    fn border_shorthand_parsed() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid black"), &parent);
        assert!((style.border_width - 0.75).abs() < 0.1); // 1px = 0.75pt
        assert!(style.border_color.is_some());
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_with_custom_color() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 2px solid red"), &parent);
        assert!((style.border_width - 1.5).abs() < 0.1); // 2px = 1.5pt
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_width_and_color_individual() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Div,
            Some("border-width: 3pt; border-color: blue"),
            &parent,
        );
        assert!((style.border_width - 3.0).abs() < 0.1);
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 255);
    }

    #[test]
    fn font_family_default_is_helvetica() {
        let style = ComputedStyle::default();
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_serif() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: serif"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_times_new_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("font-family: 'Times New Roman'"),
            &parent,
        );
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_monospace() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: monospace"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: courier"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_sans_serif_defaults_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: sans-serif"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_inherited() {
        let mut parent = ComputedStyle::default();
        parent.font_family = FontFamily::Courier;
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn border_shorthand_pt_unit() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 2pt solid green"), &parent);
        assert!((style.border_width - 2.0).abs() < 0.1);
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 128);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_color_variants() {
        let parent = ComputedStyle::default();
        for (name, r, g, b) in [
            ("yellow", 255, 255, 0),
            ("orange", 255, 165, 0),
            ("purple", 128, 0, 128),
            ("gray", 128, 128, 128),
            ("grey", 128, 128, 128),
            ("white", 255, 255, 255),
        ] {
            let css = format!("border: 1px solid {name}");
            let style = compute_style(HtmlTag::Div, Some(&css), &parent);
            let c = style.border_color.unwrap();
            assert_eq!((c.r, c.g, c.b), (r, g, b), "failed for {name}");
        }
    }

    #[test]
    fn border_color_hex_short() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid #f00"), &parent);
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_color_hex_long() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid #00ff00"), &parent);
        let c = style.border_color.unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 255);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn border_color_unknown_returns_none() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("border: 1px solid foobar"), &parent);
        assert!(style.border_color.is_none());
    }

    // --- Extended font-family mapping tests ---

    #[test]
    fn font_family_arial_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Arial"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_roboto_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Roboto"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_verdana_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Verdana"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_open_sans_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'Open Sans'"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_system_ui_maps_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: system-ui"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_georgia_maps_to_times_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Georgia"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_garamond_maps_to_times_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Garamond"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_merriweather_maps_to_times_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Merriweather"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_palatino_maps_to_times_roman() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Palatino"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
    }

    #[test]
    fn font_family_consolas_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Consolas"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_fira_code_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'Fira Code'"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_jetbrains_mono_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(
            HtmlTag::Span,
            Some("font-family: 'JetBrains Mono'"),
            &parent,
        );
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_menlo_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Menlo"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_sf_mono_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'SF Mono'"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_monaco_maps_to_courier() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: Monaco"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_unknown_falls_back_to_helvetica() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: 'Comic Sans MS'"), &parent);
        assert_eq!(style.font_family, FontFamily::Helvetica);
    }

    #[test]
    fn font_family_case_insensitive() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: GEORGIA"), &parent);
        assert_eq!(style.font_family, FontFamily::TimesRoman);
        let style = compute_style(HtmlTag::Span, Some("font-family: CONSOLAS"), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn font_family_double_quoted() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("font-family: \"Courier New\""), &parent);
        assert_eq!(style.font_family, FontFamily::Courier);
    }

    #[test]
    fn display_none_from_inline_style() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: none"), &parent);
        assert_eq!(style.display, Display::None);
    }

    #[test]
    fn display_block_on_inline_element() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, Some("display: block"), &parent);
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn display_inline_on_block_element() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, Some("display: inline"), &parent);
        assert_eq!(style.display, Display::Inline);
    }

    #[test]
    fn display_default_for_block_tag() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Div, None, &parent);
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn display_default_for_inline_tag() {
        let parent = ComputedStyle::default();
        let style = compute_style(HtmlTag::Span, None, &parent);
        assert_eq!(style.display, Display::Inline);
    }
}
