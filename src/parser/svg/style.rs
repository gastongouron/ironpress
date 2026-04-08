use crate::parser::dom::ElementNode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SvgPaint {
    /// The property was not specified on this element (so it should inherit from its parent).
    Unspecified,
    /// The property was explicitly set to `none`.
    None,
    /// `currentColor` keyword (resolves to the inherited CSS `color` property).
    CurrentColor,
    /// An explicit sRGB color (0.0-1.0 per channel).
    Color((f32, f32, f32)),
}

impl Default for SvgPaint {
    fn default() -> Self {
        Self::Unspecified
    }
}

#[derive(Debug, Clone)]
pub struct SvgStyle {
    pub color: Option<(f32, f32, f32)>,
    pub fill: SvgPaint,
    pub stroke: SvgPaint,
    /// `stroke-width` is an inherited property in SVG; keep it optional so a child element can
    /// inherit a group `<g>` value when it doesn't specify its own.
    pub stroke_width: Option<f32>,
    // Opacity isn't wired through to PDF output yet; keep it simple until needed.
    pub opacity: f32,
}

impl Default for SvgStyle {
    fn default() -> Self {
        Self {
            color: None,
            fill: SvgPaint::Unspecified,
            stroke: SvgPaint::Unspecified,
            stroke_width: None,
            opacity: 1.0,
        }
    }
}

pub(super) fn parse_svg_style(el: &ElementNode) -> SvgStyle {
    fn parse_svg_paint(val: &str) -> Option<SvgPaint> {
        let val = val.trim();
        if val.eq_ignore_ascii_case("none") {
            return Some(SvgPaint::None);
        }
        if val.eq_ignore_ascii_case("currentColor") {
            return Some(SvgPaint::CurrentColor);
        }
        parse_svg_color(val).map(SvgPaint::Color)
    }

    let mut color = el.attributes.get("color").and_then(|v| parse_svg_color(v));
    let mut fill = el
        .attributes
        .get("fill")
        .and_then(|v| parse_svg_paint(v))
        .unwrap_or(SvgPaint::Unspecified);
    let mut stroke = el
        .attributes
        .get("stroke")
        .and_then(|v| parse_svg_paint(v))
        .unwrap_or(SvgPaint::Unspecified);
    let mut stroke_width = el
        .attributes
        .get("stroke-width")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v >= 0.0);
    let mut opacity = el
        .attributes
        .get("opacity")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);

    if let Some(style_val) = el.attributes.get("style") {
        for declaration in style_val
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            if let Some((prop, val)) = declaration.split_once(':') {
                match prop.trim() {
                    // Only override when the declaration is valid; invalid CSS declarations are
                    // ignored so they do not wipe a valid presentation attribute.
                    "fill" => {
                        if let Some(paint) = parse_svg_paint(val) {
                            fill = paint;
                        }
                    }
                    "stroke" => {
                        if let Some(paint) = parse_svg_paint(val) {
                            stroke = paint;
                        }
                    }
                    "color" => {
                        if let Some(parsed) = parse_svg_color(val) {
                            color = Some(parsed);
                        }
                    }
                    "stroke-width" => {
                        if let Ok(parsed) = val.trim().parse::<f32>() {
                            if parsed >= 0.0 {
                                stroke_width = Some(parsed);
                            }
                        }
                    }
                    "opacity" => {
                        if let Ok(parsed) = val.trim().parse::<f32>() {
                            opacity = parsed;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    SvgStyle {
        color,
        fill,
        stroke,
        stroke_width,
        opacity,
    }
}

pub(super) fn parse_svg_color(val: &str) -> Option<(f32, f32, f32)> {
    let val = val.trim();
    if val.eq_ignore_ascii_case("none") {
        return None;
    }

    match val.to_ascii_lowercase().as_str() {
        "black" => return Some((0.0, 0.0, 0.0)),
        "white" => return Some((1.0, 1.0, 1.0)),
        "red" => return Some((1.0, 0.0, 0.0)),
        "green" => return Some((0.0, 128.0 / 255.0, 0.0)),
        "blue" => return Some((0.0, 0.0, 1.0)),
        "yellow" => return Some((1.0, 1.0, 0.0)),
        "cyan" => return Some((0.0, 1.0, 1.0)),
        "magenta" => return Some((1.0, 0.0, 1.0)),
        "gray" | "grey" => return Some((128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0)),
        "orange" => return Some((1.0, 165.0 / 255.0, 0.0)),
        _ => {}
    }

    if let Some(hex) = val.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    if let Some(inner) = val
        .to_ascii_lowercase()
        .strip_prefix("rgb(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_rgb_color(inner);
    }

    None
}

fn parse_rgb_color(inner: &str) -> Option<(f32, f32, f32)> {
    let (r, rest) = inner.split_once(',')?;
    let (g, b) = rest.split_once(',')?;
    if b.contains(',') {
        return None;
    }

    let r = r.trim().parse::<f32>().ok()?;
    let g = g.trim().parse::<f32>().ok()?;
    let b = b.trim().parse::<f32>().ok()?;
    Some((r / 255.0, g / 255.0, b / 255.0))
}

fn parse_hex_color(hex: &str) -> Option<(f32, f32, f32)> {
    fn hex_digit(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    fn hex_pair(hi: u8, lo: u8) -> Option<u8> {
        Some(hex_digit(hi)? * 16 + hex_digit(lo)?)
    }

    match hex.as_bytes() {
        [r, g, b] => Some((
            (hex_digit(*r)? * 17) as f32 / 255.0,
            (hex_digit(*g)? * 17) as f32 / 255.0,
            (hex_digit(*b)? * 17) as f32 / 255.0,
        )),
        [r1, r2, g1, g2, b1, b2] => Some((
            hex_pair(*r1, *r2)? as f32 / 255.0,
            hex_pair(*g1, *g2)? as f32 / 255.0,
            hex_pair(*b1, *b2)? as f32 / 255.0,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::dom::HtmlTag;
    use std::collections::HashMap;

    fn make_el(raw_tag: &str, attrs: Vec<(&str, &str)>) -> ElementNode {
        let mut attributes = HashMap::new();
        for (k, v) in attrs {
            attributes.insert(k.to_string(), v.to_string());
        }
        ElementNode {
            tag: HtmlTag::Unknown,
            raw_tag_name: raw_tag.to_string(),
            attributes,
            children: Vec::new(),
        }
    }

    #[test]
    fn parse_svg_color_hex() {
        assert_eq!(parse_svg_color("#ff0000"), Some((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_hex_3_char() {
        assert_eq!(parse_svg_color("#f00"), Some((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_hex_3_char_white() {
        assert_eq!(parse_svg_color("#fff"), Some((1.0, 1.0, 1.0)));
    }

    #[test]
    fn parse_svg_color_hex_invalid_length() {
        assert!(parse_svg_color("#abcd").is_none());
    }

    #[test]
    fn parse_svg_color_rgb() {
        assert_eq!(
            parse_svg_color("rgb(255, 0, 128)"),
            Some((1.0, 0.0, 128.0 / 255.0))
        );
    }

    #[test]
    fn parse_svg_color_rgb_with_spaces() {
        assert_eq!(
            parse_svg_color("rgb( 0 , 128 , 255 )"),
            Some((0.0, 128.0 / 255.0, 1.0))
        );
    }

    #[test]
    fn parse_svg_color_rgb_invalid_components() {
        assert!(parse_svg_color("rgb(255, 0)").is_none());
    }

    #[test]
    fn parse_svg_color_rgb_non_numeric() {
        assert!(parse_svg_color("rgb(a, b, c)").is_none());
    }

    #[test]
    fn parse_svg_color_named() {
        assert_eq!(parse_svg_color("red"), Some((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_named_palette() {
        let gray = (128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0);
        assert_eq!(parse_svg_color("black"), Some((0.0, 0.0, 0.0)));
        assert_eq!(parse_svg_color("white"), Some((1.0, 1.0, 1.0)));
        assert_eq!(parse_svg_color("green"), Some((0.0, 128.0 / 255.0, 0.0)));
        assert_eq!(parse_svg_color("blue"), Some((0.0, 0.0, 1.0)));
        assert_eq!(parse_svg_color("yellow"), Some((1.0, 1.0, 0.0)));
        assert_eq!(parse_svg_color("cyan"), Some((0.0, 1.0, 1.0)));
        assert_eq!(parse_svg_color("magenta"), Some((1.0, 0.0, 1.0)));
        assert_eq!(parse_svg_color("gray"), Some(gray));
        assert_eq!(parse_svg_color("grey"), Some(gray));
        assert_eq!(parse_svg_color("orange"), Some((1.0, 165.0 / 255.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_unknown_name() {
        assert!(parse_svg_color("papayawhip").is_none());
    }

    #[test]
    fn parse_svg_color_none_case_insensitive() {
        assert_eq!(parse_svg_color("None"), None);
        assert_eq!(parse_svg_color("NONE"), None);
    }

    #[test]
    fn parse_svg_color_with_whitespace() {
        assert_eq!(parse_svg_color("  red  "), Some((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_style_current_color() {
        let el = make_el("rect", vec![("fill", "currentColor")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::CurrentColor);
    }

    #[test]
    fn parse_svg_style_defaults() {
        let style = parse_svg_style(&make_el("rect", vec![]));
        assert_eq!(style.fill, SvgPaint::Unspecified);
        assert_eq!(style.stroke, SvgPaint::Unspecified);
        assert_eq!(style.stroke_width, None);
        assert_eq!(style.opacity, 1.0);
    }

    #[test]
    fn parse_svg_style_with_fill_stroke() {
        let el = make_el(
            "rect",
            vec![
                ("fill", "#ff0000"),
                ("stroke", "blue"),
                ("stroke-width", "2.5"),
                ("opacity", "0.5"),
            ],
        );
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::Color((1.0, 0.0, 0.0)));
        assert_eq!(style.stroke, SvgPaint::Color((0.0, 0.0, 1.0)));
        assert_eq!(style.stroke_width, Some(2.5));
        assert!((style.opacity - 0.5).abs() < 0.001);
    }

    #[test]
    fn parse_svg_style_fill_none() {
        let style = parse_svg_style(&make_el("rect", vec![("fill", "none")]));
        assert_eq!(style.fill, SvgPaint::None);
    }

    #[test]
    fn parse_svg_style_stroke_none() {
        let style = parse_svg_style(&make_el("rect", vec![("stroke", "none")]));
        assert_eq!(style.stroke, SvgPaint::None);
    }

    #[test]
    fn parse_svg_style_from_style_attribute() {
        let el = make_el(
            "rect",
            vec![(
                "style",
                "fill: #00ff00; stroke: rgb(0,0,255); stroke-width: 3; opacity: 0.25;",
            )],
        );
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::Color((0.0, 1.0, 0.0)));
        assert_eq!(style.stroke, SvgPaint::Color((0.0, 0.0, 1.0)));
        assert_eq!(style.stroke_width, Some(3.0));
        assert!((style.opacity - 0.25).abs() < 0.001);
    }

    #[test]
    fn parse_svg_style_unparseable_style_fill_does_not_override_attribute() {
        let el = make_el("rect", vec![("fill", "red"), ("style", "fill: bogus;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::Color((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_style_style_fill_none_overrides_attribute() {
        let el = make_el("rect", vec![("fill", "red"), ("style", "fill: none")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::None);
    }

    #[test]
    fn parse_svg_style_unparseable_style_stroke_does_not_override_attribute() {
        let el = make_el("rect", vec![("stroke", "blue"), ("style", "stroke: ???;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.stroke, SvgPaint::Color((0.0, 0.0, 1.0)));
    }

    #[test]
    fn parse_svg_style_declares_color_and_fill() {
        let el = make_el(
            "rect",
            vec![
                ("color", "#123456"),
                ("style", "fill: #ff0000; color: rgb(0, 255, 0)"),
            ],
        );
        let style = parse_svg_style(&el);
        assert_eq!(style.color, Some((0.0, 1.0, 0.0)));
        assert_eq!(style.fill, SvgPaint::Color((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_style_color_property() {
        let el = make_el(
            "rect",
            vec![("color", "#123456"), ("style", "color: rgb(255, 0, 0);")],
        );
        let style = parse_svg_style(&el);
        assert_eq!(style.color, Some((1.0, 0.0, 0.0)));
    }
}
