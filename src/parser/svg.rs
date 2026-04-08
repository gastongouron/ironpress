//! SVG parser — converts DOM SVG elements into an SvgTree for PDF rendering.

use crate::parser::dom::ElementNode;

/// Split a style declaration string on `;`, respecting quoted strings and
/// parenthesized function arguments.
fn split_style_declarations(style: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = style.as_bytes();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut paren_depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double_quote && paren_depth > 0 => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote && paren_depth > 0 => in_double_quote = !in_double_quote,
            b'(' if !in_single_quote && !in_double_quote => paren_depth += 1,
            b')' if !in_single_quote && !in_double_quote && paren_depth > 0 => paren_depth -= 1,
            b';' if !in_single_quote && !in_double_quote && paren_depth == 0 => {
                parts.push(&style[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < style.len() {
        parts.push(&style[start..]);
    }
    parts
}

/// Inherited CSS context for SVG text rendering.
#[derive(Debug, Clone)]
pub struct SvgTextContext {
    pub font_family: String,
    pub font_size: f32,
    pub font_bold: bool,
    pub font_italic: bool,
    pub color: Option<(f32, f32, f32)>,
}

impl Default for SvgTextContext {
    fn default() -> Self {
        Self {
            font_family: "Helvetica".to_string(),
            font_size: 12.0,
            font_bold: false,
            font_italic: false,
            color: None,
        }
    }
}

/// A parsed SVG tree ready for rendering.
#[derive(Debug, Clone)]
pub struct SvgTree {
    pub width: f32,
    pub height: f32,
    pub width_attr: Option<String>,
    pub height_attr: Option<String>,
    pub view_box: Option<ViewBox>,
    pub children: Vec<SvgNode>,
    pub text_ctx: SvgTextContext,
}

#[derive(Debug, Clone)]
pub struct ViewBox {
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub enum SvgNode {
    Group {
        transform: Option<SvgTransform>,
        children: Vec<SvgNode>,
        style: SvgStyle,
    },
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        rx: f32,
        ry: f32,
        style: SvgStyle,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        style: SvgStyle,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        style: SvgStyle,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        style: SvgStyle,
    },
    Polyline {
        points: Vec<(f32, f32)>,
        style: SvgStyle,
    },
    Polygon {
        points: Vec<(f32, f32)>,
        style: SvgStyle,
    },
    Path {
        commands: Vec<PathCommand>,
        style: SvgStyle,
    },
    Text {
        x: f32,
        y: f32,
        font_size: Option<f32>,
        font_size_attr: Option<String>,
        /// True when the element explicitly set `fill` (including `none`).
        fill_specified: bool,
        fill_raw: Option<String>,
        /// Per-element font-family override (resolved PDF name, e.g. "Helvetica-Bold").
        font_family: Option<String>,
        /// Per-element font-weight override (true = bold).
        font_bold: Option<bool>,
        /// Per-element font-style override (true = italic/oblique).
        font_italic: Option<bool>,
        content: String,
        style: SvgStyle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SvgPaint {
    /// The property was not specified on this element (so it should inherit from its parent).
    Unspecified,
    /// The property was explicitly set to `none`.
    None,
    /// `currentColor` keyword (resolves to the CSS `color` property).
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
    pub fill: SvgPaint,
    pub stroke: SvgPaint,
    /// `stroke-width` is inherited in SVG.
    pub stroke_width: Option<f32>,
    // Opacity isn't wired through to PDF output yet; keep it simple until needed.
    pub opacity: f32,
}

impl Default for SvgStyle {
    fn default() -> Self {
        Self {
            fill: SvgPaint::Unspecified,
            stroke: SvgPaint::Unspecified,
            stroke_width: None,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SvgTransform {
    Matrix(f32, f32, f32, f32, f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32), // C command
    QuadTo(f32, f32, f32, f32),            // Q command
    ClosePath,
}

/// Entry point: parse an `<svg>` ElementNode into an SvgTree.
pub fn parse_svg_from_element(el: &ElementNode) -> Option<SvgTree> {
    parse_svg_from_element_with_ctx(el, SvgTextContext::default())
}

pub fn parse_svg_from_element_with_ctx(
    el: &ElementNode,
    text_ctx: SvgTextContext,
) -> Option<SvgTree> {
    let width_attr = el.attributes.get("width").cloned();
    let height_attr = el.attributes.get("height").cloned();
    let width = width_attr
        .as_deref()
        .and_then(parse_absolute_length)
        .unwrap_or(300.0);
    let height = height_attr
        .as_deref()
        .and_then(parse_absolute_length)
        .unwrap_or(150.0);
    let view_box = el.attributes.get("viewBox").and_then(|v| parse_viewbox(v));

    let mut children = Vec::new();
    let root_viewport = Some((width, height));
    for child in &el.children {
        if let crate::parser::dom::DomNode::Element(child_el) = child {
            if let Some(node) = parse_svg_node_with_viewport(child_el, root_viewport) {
                children.push(node);
            }
        }
    }

    let root_style = parse_svg_style(el);
    let root_transform = el
        .attributes
        .get("transform")
        .and_then(|v| parse_transform(v));
    if root_transform.is_some() || !svg_style_is_default(&root_style) {
        children = vec![SvgNode::Group {
            transform: root_transform,
            children,
            style: root_style,
        }];
    }

    Some(SvgTree {
        width,
        height,
        width_attr,
        height_attr,
        view_box,
        children,
        text_ctx,
    })
}

/// Parse a single SVG element node into an SvgNode.
fn parse_svg_node(el: &ElementNode) -> Option<SvgNode> {
    parse_svg_node_with_viewport(el, None)
}

fn parse_svg_node_with_viewport(
    el: &ElementNode,
    parent_viewport: Option<(f32, f32)>,
) -> Option<SvgNode> {
    let tag = el.raw_tag_name.as_str();
    match tag {
        "g" => {
            let transform = el
                .attributes
                .get("transform")
                .and_then(|v| parse_transform(v));
            let style = parse_svg_style(el);
            let mut children = Vec::new();
            for child in &el.children {
                if let crate::parser::dom::DomNode::Element(child_el) = child {
                    if let Some(node) = parse_svg_node_with_viewport(child_el, parent_viewport) {
                        children.push(node);
                    }
                }
            }
            Some(SvgNode::Group {
                transform,
                children,
                style,
            })
        }
        "svg" => {
            let child_viewport = resolve_nested_svg_viewport(el, parent_viewport);
            let transform = compose_transform(
                el.attributes
                    .get("transform")
                    .and_then(|v| parse_transform(v)),
                nested_svg_viewport_transform(el, parent_viewport),
            );
            let style = parse_svg_style(el);
            let mut children = Vec::new();
            for child in &el.children {
                if let crate::parser::dom::DomNode::Element(child_el) = child {
                    if let Some(node) = parse_svg_node_with_viewport(child_el, child_viewport) {
                        children.push(node);
                    }
                }
            }
            Some(SvgNode::Group {
                transform,
                children,
                style,
            })
        }
        "rect" => {
            let x = attr_f32(el, "x");
            let y = attr_f32(el, "y");
            let width = attr_f32(el, "width");
            let height = attr_f32(el, "height");
            let rx = attr_f32(el, "rx");
            let ry = attr_f32(el, "ry");
            let style = parse_svg_style(el);
            Some(SvgNode::Rect {
                x,
                y,
                width,
                height,
                rx,
                ry,
                style,
            })
        }
        "circle" => {
            let cx = attr_f32(el, "cx");
            let cy = attr_f32(el, "cy");
            let r = attr_f32(el, "r");
            let style = parse_svg_style(el);
            Some(SvgNode::Circle { cx, cy, r, style })
        }
        "ellipse" => {
            let cx = attr_f32(el, "cx");
            let cy = attr_f32(el, "cy");
            let rx = attr_f32(el, "rx");
            let ry = attr_f32(el, "ry");
            let style = parse_svg_style(el);
            Some(SvgNode::Ellipse {
                cx,
                cy,
                rx,
                ry,
                style,
            })
        }
        "line" => {
            let x1 = attr_f32(el, "x1");
            let y1 = attr_f32(el, "y1");
            let x2 = attr_f32(el, "x2");
            let y2 = attr_f32(el, "y2");
            let style = parse_svg_style(el);
            Some(SvgNode::Line {
                x1,
                y1,
                x2,
                y2,
                style,
            })
        }
        "polyline" => {
            let points = el
                .attributes
                .get("points")
                .map(|v| parse_points(v))
                .unwrap_or_default();
            let style = parse_svg_style(el);
            Some(SvgNode::Polyline { points, style })
        }
        "polygon" => {
            let points = el
                .attributes
                .get("points")
                .map(|v| parse_points(v))
                .unwrap_or_default();
            let style = parse_svg_style(el);
            Some(SvgNode::Polygon { points, style })
        }
        "path" => {
            let commands = el
                .attributes
                .get("d")
                .map(|v| parse_path_data(v))
                .unwrap_or_default();
            let style = parse_svg_style(el);
            Some(SvgNode::Path { commands, style })
        }
        "text" => {
            let x = attr_f32(el, "x");
            let y = attr_f32(el, "y");
            let font_size_attr = parse_font_size_attr(el);
            let font_size = font_size_attr.as_deref().and_then(parse_absolute_length);
            let fill_specified = has_fill_specified(el);
            let fill_raw = parse_fill_raw(el);
            let (font_family, font_bold, font_italic) = parse_text_font_attrs(el);
            let content = collect_text_content(el);
            let style = parse_svg_style(el);
            Some(SvgNode::Text {
                x,
                y,
                font_size,
                font_size_attr,
                fill_specified,
                fill_raw,
                font_family,
                font_bold,
                font_italic,
                content,
                style,
            })
        }
        _ => None,
    }
}

fn svg_style_is_default(style: &SvgStyle) -> bool {
    matches!(style.fill, SvgPaint::Unspecified)
        && matches!(style.stroke, SvgPaint::Unspecified)
        && style.stroke_width.is_none()
        && (style.opacity - 1.0).abs() < f32::EPSILON
}

fn compose_transform(
    outer: Option<SvgTransform>,
    inner: Option<SvgTransform>,
) -> Option<SvgTransform> {
    match (outer, inner) {
        (
            Some(SvgTransform::Matrix(a1, b1, c1, d1, e1, f1)),
            Some(SvgTransform::Matrix(a2, b2, c2, d2, e2, f2)),
        ) => Some(SvgTransform::Matrix(
            a1 * a2 + c1 * b2,
            b1 * a2 + d1 * b2,
            a1 * c2 + c1 * d2,
            b1 * c2 + d1 * d2,
            a1 * e2 + c1 * f2 + e1,
            b1 * e2 + d1 * f2 + f1,
        )),
        (Some(transform), None) | (None, Some(transform)) => Some(transform),
        (None, None) => None,
    }
}

fn resolve_nested_svg_viewport(
    el: &ElementNode,
    parent_viewport: Option<(f32, f32)>,
) -> Option<(f32, f32)> {
    let (parent_width, parent_height) = parent_viewport?;
    Some((
        resolve_svg_viewport_length(el.attributes.get("width"), Some(parent_width), 300.0),
        resolve_svg_viewport_length(el.attributes.get("height"), Some(parent_height), 150.0),
    ))
}

fn resolve_svg_viewport_length(
    attr: Option<&String>,
    parent_extent: Option<f32>,
    fallback: f32,
) -> f32 {
    match attr.map(String::as_str) {
        Some(value) => {
            let trimmed = value.trim();
            if let Some(pct) = trimmed.strip_suffix('%') {
                pct.trim()
                    .parse::<f32>()
                    .ok()
                    .and_then(|pct| parent_extent.map(|extent| extent * pct / 100.0))
                    .unwrap_or(fallback)
            } else {
                parse_absolute_length(trimmed).unwrap_or(fallback)
            }
        }
        None => parent_extent.unwrap_or(fallback),
    }
}

fn nested_svg_viewport_transform(
    el: &ElementNode,
    parent_viewport: Option<(f32, f32)>,
) -> Option<SvgTransform> {
    let x = attr_f32(el, "x");
    let y = attr_f32(el, "y");
    let view_box = el.attributes.get("viewBox").and_then(|v| parse_viewbox(v));

    if let Some(vb) = view_box {
        let (width, height) = parent_viewport
            .map(|(parent_width, parent_height)| {
                (
                    resolve_svg_viewport_length(
                        el.attributes.get("width"),
                        Some(parent_width),
                        300.0,
                    ),
                    resolve_svg_viewport_length(
                        el.attributes.get("height"),
                        Some(parent_height),
                        150.0,
                    ),
                )
            })
            .unwrap_or((
                resolve_svg_viewport_length(el.attributes.get("width"), None, 300.0),
                resolve_svg_viewport_length(el.attributes.get("height"), None, 150.0),
            ));
        if vb.width > 0.0 && vb.height > 0.0 {
            let scale_x = width / vb.width;
            let scale_y = height / vb.height;
            return Some(SvgTransform::Matrix(
                scale_x,
                0.0,
                0.0,
                scale_y,
                x - vb.min_x * scale_x,
                y - vb.min_y * scale_y,
            ));
        }
    }

    if x != 0.0 || y != 0.0 {
        Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, x, y))
    } else {
        None
    }
}

/// Get a float attribute, defaulting to 0.0.
fn attr_f32(el: &ElementNode, name: &str) -> f32 {
    el.attributes
        .get(name)
        .and_then(|v| parse_length(v))
        .unwrap_or(0.0)
}

/// Parse a length value (strip px/em/etc suffix, parse number).
pub(crate) fn parse_length(val: &str) -> Option<f32> {
    let trimmed = val.trim();
    let num_str = trimmed.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
    num_str.trim().parse::<f32>().ok()
}

fn parse_absolute_length(val: &str) -> Option<f32> {
    let trimmed = val.trim();
    if trimmed.ends_with('%') {
        return None;
    }
    parse_length(trimmed)
}

/// Parse a viewBox attribute: "min-x min-y width height".
fn parse_viewbox(val: &str) -> Option<ViewBox> {
    let parts: Vec<f32> = val
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    if parts.len() == 4 {
        Some(ViewBox {
            min_x: parts[0],
            min_y: parts[1],
            width: parts[2],
            height: parts[3],
        })
    } else {
        None
    }
}

/// Parse fill, stroke, stroke-width, opacity from element attributes.
fn parse_svg_style(el: &ElementNode) -> SvgStyle {
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
        for part in split_style_declarations(style_val) {
            let part = part.trim();
            if let Some((prop, val)) = part.split_once(':') {
                match prop.trim() {
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
                    "stroke-width" => {
                        if let Ok(v) = val.trim().parse::<f32>() {
                            if v >= 0.0 {
                                stroke_width = Some(v);
                            }
                        }
                    }
                    "opacity" => opacity = val.trim().parse().ok().unwrap_or(opacity),
                    _ => {}
                }
            }
        }
    }

    SvgStyle {
        fill,
        stroke,
        stroke_width,
        opacity,
    }
}

/// Extract the raw `font-size` value from a `<text>` element.
///
/// Checks the `font-size` attribute first, then falls back to parsing
/// `font-size:` from the inline `style` attribute.
fn parse_font_size_attr(el: &ElementNode) -> Option<String> {
    if let Some(val) = el.attributes.get("font-size") {
        return Some(val.trim().to_string());
    }
    // 2) Inline style: style="font-size:20px"
    if let Some(style_val) = el.attributes.get("style") {
        for part in split_style_declarations(style_val) {
            let part = part.trim();
            if let Some((prop, val)) = part.split_once(':') {
                if prop.trim() == "font-size" {
                    return Some(val.trim().to_string());
                }
            }
        }
    }
    None
}

/// Parse per-element font-family, font-weight, and font-style from a `<text>` element.
///
/// Checks both XML attributes (`font-family`, `font-weight`, `font-style`) and
/// properties inside the `style` attribute.  Returns `(font_family, font_bold, font_italic)`,
/// each `None` when no explicit value was found on the element.
fn parse_text_font_attrs(el: &ElementNode) -> (Option<String>, Option<bool>, Option<bool>) {
    let mut family: Option<String> = None;
    let mut bold: Option<bool> = None;
    let mut italic: Option<bool> = None;

    // 1) Direct XML attributes
    if let Some(val) = el.attributes.get("font-family") {
        let val = val.trim().trim_matches(|c| c == '\'' || c == '"');
        if !val.is_empty() {
            family = Some(val.to_string());
        }
    }
    if let Some(val) = el.attributes.get("font-weight") {
        bold = Some(is_bold_value(val.trim()));
    }
    if let Some(val) = el.attributes.get("font-style") {
        italic = Some(is_italic_value(val.trim()));
    }

    // 2) Inline `style` attribute (overrides XML attributes when present)
    if let Some(style_val) = el.attributes.get("style") {
        for part in split_style_declarations(style_val) {
            let part = part.trim();
            if let Some((prop, val)) = part.split_once(':') {
                let prop = prop.trim();
                let val = val.trim();
                if prop == "font-family" {
                    let val = val.trim_matches(|c| c == '\'' || c == '"');
                    if !val.is_empty() {
                        family = Some(val.to_string());
                    }
                } else if prop == "font-weight" {
                    bold = Some(is_bold_value(val));
                } else if prop == "font-style" {
                    italic = Some(is_italic_value(val));
                }
            }
        }
    }

    // Resolve family to a PDF base name (sans bold/italic suffix -- that's applied at render time)
    let family = family.map(|f| resolve_svg_font_family(&f));

    (family, bold, italic)
}

fn has_fill_specified(el: &ElementNode) -> bool {
    if el.attributes.contains_key("fill") {
        return true;
    }
    if let Some(style_val) = el.attributes.get("style") {
        for part in split_style_declarations(style_val) {
            let part = part.trim();
            if let Some((prop, _val)) = part.split_once(':') {
                if prop.trim() == "fill" {
                    return true;
                }
            }
        }
    }
    false
}

fn parse_fill_raw(el: &ElementNode) -> Option<String> {
    if let Some(style_val) = el.attributes.get("style") {
        for part in split_style_declarations(style_val) {
            let part = part.trim();
            if let Some((prop, val)) = part.split_once(':') {
                if prop.trim() == "fill" {
                    let raw = val.trim();
                    if !raw.is_empty() {
                        return Some(raw.to_string());
                    }
                }
            }
        }
    }
    if let Some(val) = el.attributes.get("fill") {
        return Some(val.trim().to_string());
    }
    None
}

/// Map a CSS font-family value to a PDF base-font family name.
fn resolve_svg_font_family(css_family: &str) -> String {
    let lower = css_family.to_ascii_lowercase();
    if lower.contains("times") || lower == "serif" {
        "Times-Roman".to_string()
    } else if lower.contains("courier") || lower == "monospace" {
        "Courier".to_string()
    } else {
        // Default to Helvetica for sans-serif / Arial / Helvetica / anything else
        "Helvetica".to_string()
    }
}

fn is_bold_value(val: &str) -> bool {
    let lower = val.to_ascii_lowercase();
    lower == "bold" || lower == "bolder" || lower.parse::<u32>().map_or(false, |w| w >= 700)
}

fn is_italic_value(val: &str) -> bool {
    let lower = val.to_ascii_lowercase();
    lower == "italic" || lower == "oblique"
}

/// Collect all text content from a `<text>` element, including `<tspan>` children.
fn collect_text_content(el: &ElementNode) -> String {
    let mut text = String::new();
    for child in &el.children {
        match child {
            crate::parser::dom::DomNode::Text(s) => text.push_str(s),
            crate::parser::dom::DomNode::Element(child_el) => {
                if child_el.raw_tag_name == "tspan" {
                    text.push_str(&collect_text_content(child_el));
                }
            }
        }
    }
    text
}

/// Parse common SVG colors: named, hex (#rgb / #rrggbb), rgb(r,g,b), or "none".
pub fn parse_svg_color(val: &str) -> Option<(f32, f32, f32)> {
    let val = val.trim();
    if val.eq_ignore_ascii_case("none") {
        return None;
    }

    // Named colors
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

    // Hex colors
    if let Some(hex) = val.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    // rgb(r, g, b)
    if let Some(inner) = val.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3 {
            let r = parts[0].trim().parse::<f32>().ok()?;
            let g = parts[1].trim().parse::<f32>().ok()?;
            let b = parts[2].trim().parse::<f32>().ok()?;
            return Some((r / 255.0, g / 255.0, b / 255.0));
        }
    }

    None
}

/// Parse a hex color string (without the #).
fn parse_hex_color(hex: &str) -> Option<(f32, f32, f32)> {
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
            Some((
                (r * 17) as f32 / 255.0,
                (g * 17) as f32 / 255.0,
                (b * 17) as f32 / 255.0,
            ))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
        }
        _ => None,
    }
}

/// Parse SVG path `d` attribute data into PathCommands.
/// Supports: M/m, L/l, H/h, V/v, C/c, S/s, Q/q, T/t, Z/z.
pub fn parse_path_data(d: &str) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    let mut cur_x: f32 = 0.0;
    let mut cur_y: f32 = 0.0;
    let mut last_ctrl_x: f32 = 0.0;
    let mut last_ctrl_y: f32 = 0.0;
    let mut last_cmd: char = ' ';

    let tokens = tokenize_path(d);
    let mut i = 0;

    while i < tokens.len() {
        let token = &tokens[i];

        // Determine if this token is a command letter
        let cmd_char = if token.len() == 1 && token.as_bytes()[0].is_ascii_alphabetic() {
            let c = token.chars().next().unwrap();
            i += 1;
            c
        } else {
            // Implicit repeat of last command (L after M)
            match last_cmd {
                'M' => 'L',
                'm' => 'l',
                c => c,
            }
        };

        match cmd_char {
            'M' => {
                if let Some((x, y)) = read_pair(&tokens, &mut i) {
                    cur_x = x;
                    cur_y = y;
                    commands.push(PathCommand::MoveTo(cur_x, cur_y));
                    last_cmd = 'M';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'm' => {
                if let Some((dx, dy)) = read_pair(&tokens, &mut i) {
                    cur_x += dx;
                    cur_y += dy;
                    commands.push(PathCommand::MoveTo(cur_x, cur_y));
                    last_cmd = 'm';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'L' => {
                if let Some((x, y)) = read_pair(&tokens, &mut i) {
                    cur_x = x;
                    cur_y = y;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'L';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'l' => {
                if let Some((dx, dy)) = read_pair(&tokens, &mut i) {
                    cur_x += dx;
                    cur_y += dy;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'l';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'H' => {
                if let Some(x) = read_number(&tokens, &mut i) {
                    cur_x = x;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'H';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'h' => {
                if let Some(dx) = read_number(&tokens, &mut i) {
                    cur_x += dx;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'h';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'V' => {
                if let Some(y) = read_number(&tokens, &mut i) {
                    cur_y = y;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'V';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'v' => {
                if let Some(dy) = read_number(&tokens, &mut i) {
                    cur_y += dy;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'v';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'C' => {
                if let Some((x1, y1, x2, y2, x, y)) = read_six(&tokens, &mut i) {
                    commands.push(PathCommand::CubicTo(x1, y1, x2, y2, x, y));
                    last_ctrl_x = x2;
                    last_ctrl_y = y2;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'C';
                }
            }
            'c' => {
                if let Some((dx1, dy1, dx2, dy2, dx, dy)) = read_six(&tokens, &mut i) {
                    let x1 = cur_x + dx1;
                    let y1 = cur_y + dy1;
                    let x2 = cur_x + dx2;
                    let y2 = cur_y + dy2;
                    let x = cur_x + dx;
                    let y = cur_y + dy;
                    commands.push(PathCommand::CubicTo(x1, y1, x2, y2, x, y));
                    last_ctrl_x = x2;
                    last_ctrl_y = y2;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'c';
                }
            }
            'S' => {
                if let Some((x2, y2, x, y)) = read_four(&tokens, &mut i) {
                    // Reflect previous control point
                    let x1 = 2.0 * cur_x - last_ctrl_x;
                    let y1 = 2.0 * cur_y - last_ctrl_y;
                    commands.push(PathCommand::CubicTo(x1, y1, x2, y2, x, y));
                    last_ctrl_x = x2;
                    last_ctrl_y = y2;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'S';
                }
            }
            's' => {
                if let Some((dx2, dy2, dx, dy)) = read_four(&tokens, &mut i) {
                    let x1 = 2.0 * cur_x - last_ctrl_x;
                    let y1 = 2.0 * cur_y - last_ctrl_y;
                    let x2 = cur_x + dx2;
                    let y2 = cur_y + dy2;
                    let x = cur_x + dx;
                    let y = cur_y + dy;
                    commands.push(PathCommand::CubicTo(x1, y1, x2, y2, x, y));
                    last_ctrl_x = x2;
                    last_ctrl_y = y2;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 's';
                }
            }
            'Q' => {
                if let Some((x1, y1, x, y)) = read_four(&tokens, &mut i) {
                    commands.push(PathCommand::QuadTo(x1, y1, x, y));
                    last_ctrl_x = x1;
                    last_ctrl_y = y1;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'Q';
                }
            }
            'q' => {
                if let Some((dx1, dy1, dx, dy)) = read_four(&tokens, &mut i) {
                    let x1 = cur_x + dx1;
                    let y1 = cur_y + dy1;
                    let x = cur_x + dx;
                    let y = cur_y + dy;
                    commands.push(PathCommand::QuadTo(x1, y1, x, y));
                    last_ctrl_x = x1;
                    last_ctrl_y = y1;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'q';
                }
            }
            'T' => {
                if let Some((x, y)) = read_pair(&tokens, &mut i) {
                    let x1 = 2.0 * cur_x - last_ctrl_x;
                    let y1 = 2.0 * cur_y - last_ctrl_y;
                    commands.push(PathCommand::QuadTo(x1, y1, x, y));
                    last_ctrl_x = x1;
                    last_ctrl_y = y1;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'T';
                }
            }
            't' => {
                if let Some((dx, dy)) = read_pair(&tokens, &mut i) {
                    let x1 = 2.0 * cur_x - last_ctrl_x;
                    let y1 = 2.0 * cur_y - last_ctrl_y;
                    let x = cur_x + dx;
                    let y = cur_y + dy;
                    commands.push(PathCommand::QuadTo(x1, y1, x, y));
                    last_ctrl_x = x1;
                    last_ctrl_y = y1;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 't';
                }
            }
            'Z' | 'z' => {
                commands.push(PathCommand::ClosePath);
                last_cmd = 'Z';
            }
            _ => {
                // Unknown command, skip
                i += 1;
            }
        }
    }

    commands
}

/// Tokenize a path data string into numbers and command letters.
fn tokenize_path(d: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = d.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c.is_ascii_alphabetic() {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            tokens.push(c.to_string());
            i += 1;
        } else if c == '-' {
            // Minus could be start of negative number or separator
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            current.push(c);
            i += 1;
        } else if c == '.' {
            // Dot could start a new number if we already have a dot
            if current.contains('.') {
                tokens.push(current.clone());
                current.clear();
            }
            current.push(c);
            i += 1;
        } else if c.is_ascii_digit() {
            current.push(c);
            i += 1;
        } else {
            // Whitespace or comma — separator
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            i += 1;
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Read a single number from tokens.
fn read_number(tokens: &[String], i: &mut usize) -> Option<f32> {
    if *i < tokens.len() {
        let val = tokens[*i].parse::<f32>().ok()?;
        *i += 1;
        Some(val)
    } else {
        None
    }
}

/// Read a pair of numbers from tokens.
fn read_pair(tokens: &[String], i: &mut usize) -> Option<(f32, f32)> {
    let x = read_number(tokens, i)?;
    let y = read_number(tokens, i)?;
    Some((x, y))
}

/// Read four numbers from tokens.
fn read_four(tokens: &[String], i: &mut usize) -> Option<(f32, f32, f32, f32)> {
    let a = read_number(tokens, i)?;
    let b = read_number(tokens, i)?;
    let c = read_number(tokens, i)?;
    let d = read_number(tokens, i)?;
    Some((a, b, c, d))
}

/// Read six numbers from tokens.
fn read_six(tokens: &[String], i: &mut usize) -> Option<(f32, f32, f32, f32, f32, f32)> {
    let a = read_number(tokens, i)?;
    let b = read_number(tokens, i)?;
    let c = read_number(tokens, i)?;
    let d = read_number(tokens, i)?;
    let e = read_number(tokens, i)?;
    let f = read_number(tokens, i)?;
    Some((a, b, c, d, e, f))
}

/// Parse polyline/polygon points attribute: "x1,y1 x2,y2 ..."
pub fn parse_points(val: &str) -> Vec<(f32, f32)> {
    let mut points = Vec::new();
    let numbers: Vec<f32> = val
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();

    let mut i = 0;
    while i + 1 < numbers.len() {
        points.push((numbers[i], numbers[i + 1]));
        i += 2;
    }

    points
}

/// Parse the transform attribute and convert to a Matrix.
/// Supports: translate, scale, rotate, matrix.
pub fn parse_transform(val: &str) -> Option<SvgTransform> {
    let val = val.trim();

    if let Some(inner) = extract_func_args(val, "matrix") {
        let nums = parse_num_list(&inner);
        if nums.len() == 6 {
            return Some(SvgTransform::Matrix(
                nums[0], nums[1], nums[2], nums[3], nums[4], nums[5],
            ));
        }
    }

    if let Some(inner) = extract_func_args(val, "translate") {
        let nums = parse_num_list(&inner);
        let tx = nums.first().copied().unwrap_or(0.0);
        let ty = nums.get(1).copied().unwrap_or(0.0);
        return Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, tx, ty));
    }

    if let Some(inner) = extract_func_args(val, "scale") {
        let nums = parse_num_list(&inner);
        let sx = nums.first().copied().unwrap_or(1.0);
        let sy = nums.get(1).copied().unwrap_or(sx);
        return Some(SvgTransform::Matrix(sx, 0.0, 0.0, sy, 0.0, 0.0));
    }

    if let Some(inner) = extract_func_args(val, "rotate") {
        let nums = parse_num_list(&inner);
        let angle_deg = nums.first().copied().unwrap_or(0.0);
        let angle = angle_deg.to_radians();
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        if nums.len() >= 3 {
            // rotate(angle, cx, cy) — rotate around a point
            let cx = nums[1];
            let cy = nums[2];
            let tx = cx - cos_a * cx + sin_a * cy;
            let ty = cy - sin_a * cx - cos_a * cy;
            return Some(SvgTransform::Matrix(cos_a, sin_a, -sin_a, cos_a, tx, ty));
        }

        return Some(SvgTransform::Matrix(cos_a, sin_a, -sin_a, cos_a, 0.0, 0.0));
    }

    None
}

/// Extract the arguments string from a function call like "translate(10, 20)".
fn extract_func_args(val: &str, func_name: &str) -> Option<String> {
    let lower = val.to_ascii_lowercase();
    if let Some(start) = lower.find(func_name) {
        let after = &val[start + func_name.len()..];
        if let Some(open) = after.find('(') {
            if let Some(close) = after.find(')') {
                return Some(after[open + 1..close].to_string());
            }
        }
    }
    None
}

/// Parse a comma/space-separated list of numbers.
fn parse_num_list(s: &str) -> Vec<f32> {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_path_data_move_and_line() {
        let cmds = parse_path_data("M 0 0 L 10 10");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        assert_eq!(cmds[1], PathCommand::LineTo(10.0, 10.0));
    }

    #[test]
    fn parse_path_data_cubic() {
        let cmds = parse_path_data("M 0 0 C 10 0 10 10 0 10");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        assert_eq!(
            cmds[1],
            PathCommand::CubicTo(10.0, 0.0, 10.0, 10.0, 0.0, 10.0)
        );
    }

    #[test]
    fn parse_path_data_close() {
        let cmds = parse_path_data("M 0 0 L 10 0 L 10 10 Z");
        assert_eq!(cmds.len(), 4);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        assert_eq!(cmds[1], PathCommand::LineTo(10.0, 0.0));
        assert_eq!(cmds[2], PathCommand::LineTo(10.0, 10.0));
        assert_eq!(cmds[3], PathCommand::ClosePath);
    }

    #[test]
    fn parse_path_data_relative() {
        let cmds = parse_path_data("M 0 0 l 10 10");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        assert_eq!(cmds[1], PathCommand::LineTo(10.0, 10.0));
    }

    #[test]
    fn parse_path_data_horizontal_vertical() {
        let cmds = parse_path_data("M 0 0 H 10 V 10");
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        assert_eq!(cmds[1], PathCommand::LineTo(10.0, 0.0));
        assert_eq!(cmds[2], PathCommand::LineTo(10.0, 10.0));
    }

    #[test]
    fn parse_svg_color_hex() {
        let color = parse_svg_color("#ff0000");
        assert_eq!(color, Some((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_named() {
        let color = parse_svg_color("red");
        assert_eq!(color, Some((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_none() {
        let color = parse_svg_color("none");
        assert_eq!(color, None);
    }

    #[test]
    fn parse_svg_style_unparseable_style_fill_does_not_override_attribute() {
        let el = make_el("rect", vec![("fill", "red"), ("style", "fill: ???;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::Color((1.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_style_style_fill_none_overrides_attribute() {
        let el = make_el("rect", vec![("fill", "red"), ("style", "fill: none;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::None);
    }

    #[test]
    fn parse_svg_style_unparseable_style_stroke_does_not_override_attribute() {
        let el = make_el(
            "rect",
            vec![("stroke", "blue"), ("style", "stroke: not-a-color;")],
        );
        let style = parse_svg_style(&el);
        assert_eq!(style.stroke, SvgPaint::Color((0.0, 0.0, 1.0)));
    }

    #[test]
    fn parse_svg_paint_current_color_keyword() {
        let el = make_el("rect", vec![("fill", "currentColor")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::CurrentColor);
    }

    #[test]
    fn parse_points_basic() {
        let pts = parse_points("10,20 30,40");
        assert_eq!(pts, vec![(10.0, 20.0), (30.0, 40.0)]);
    }

    #[test]
    fn parse_transform_translate() {
        let t = parse_transform("translate(10, 20)").unwrap();
        match t {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                assert!((a - 1.0).abs() < 0.001);
                assert!((b - 0.0).abs() < 0.001);
                assert!((c - 0.0).abs() < 0.001);
                assert!((d - 1.0).abs() < 0.001);
                assert!((e - 10.0).abs() < 0.001);
                assert!((f - 20.0).abs() < 0.001);
            }
        }
    }

    #[test]
    fn parse_transform_scale() {
        let t = parse_transform("scale(2)").unwrap();
        match t {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                assert!((a - 2.0).abs() < 0.001);
                assert!((b - 0.0).abs() < 0.001);
                assert!((c - 0.0).abs() < 0.001);
                assert!((d - 2.0).abs() < 0.001);
                assert!((e - 0.0).abs() < 0.001);
                assert!((f - 0.0).abs() < 0.001);
            }
        }
    }

    #[test]
    fn parse_transform_rotate() {
        let t = parse_transform("rotate(45)").unwrap();
        match t {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                let cos45 = 45.0_f32.to_radians().cos();
                let sin45 = 45.0_f32.to_radians().sin();
                assert!((a - cos45).abs() < 0.001);
                assert!((b - sin45).abs() < 0.001);
                assert!((c - (-sin45)).abs() < 0.001);
                assert!((d - cos45).abs() < 0.001);
                assert!((e - 0.0).abs() < 0.001);
                assert!((f - 0.0).abs() < 0.001);
            }
        }
    }

    #[test]
    fn parse_transform_matrix() {
        let t = parse_transform("matrix(1,0,0,1,10,20)").unwrap();
        match t {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                assert!((a - 1.0).abs() < 0.001);
                assert!((b - 0.0).abs() < 0.001);
                assert!((c - 0.0).abs() < 0.001);
                assert!((d - 1.0).abs() < 0.001);
                assert!((e - 10.0).abs() < 0.001);
                assert!((f - 20.0).abs() < 0.001);
            }
        }
    }

    // ── Helper to build ElementNode for tests ──────────────────────────

    use crate::parser::dom::{DomNode, HtmlTag};
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

    fn make_svg_el(attrs: Vec<(&str, &str)>, children: Vec<ElementNode>) -> ElementNode {
        let mut attributes = HashMap::new();
        for (k, v) in attrs {
            attributes.insert(k.to_string(), v.to_string());
        }
        ElementNode {
            tag: HtmlTag::Svg,
            raw_tag_name: "svg".to_string(),
            attributes,
            children: children.into_iter().map(DomNode::Element).collect(),
        }
    }

    // ── parse_length edge cases ────────────────────────────────────────

    #[test]
    fn parse_length_plain_number() {
        assert_eq!(parse_length("42"), Some(42.0));
    }

    #[test]
    fn parse_length_with_px_suffix() {
        assert_eq!(parse_length("100px"), Some(100.0));
    }

    #[test]
    fn parse_length_with_em_suffix() {
        assert_eq!(parse_length("1.5em"), Some(1.5));
    }

    #[test]
    fn parse_length_with_percent() {
        assert_eq!(parse_length("50%"), Some(50.0));
    }

    #[test]
    fn parse_length_with_whitespace() {
        assert_eq!(parse_length("  200  "), Some(200.0));
    }

    #[test]
    fn parse_length_invalid() {
        assert_eq!(parse_length("abc"), None);
    }

    #[test]
    fn parse_length_empty() {
        assert_eq!(parse_length(""), None);
    }

    // ── parse_viewbox edge cases ───────────────────────────────────────

    #[test]
    fn parse_viewbox_comma_separated() {
        let vb = parse_viewbox("0,0,100,200").unwrap();
        assert_eq!(
            (vb.min_x, vb.min_y, vb.width, vb.height),
            (0.0, 0.0, 100.0, 200.0)
        );
    }

    #[test]
    fn parse_viewbox_space_separated() {
        let vb = parse_viewbox("10 20 300 400").unwrap();
        assert_eq!(
            (vb.min_x, vb.min_y, vb.width, vb.height),
            (10.0, 20.0, 300.0, 400.0)
        );
    }

    #[test]
    fn parse_viewbox_mixed_separators() {
        let vb = parse_viewbox("5, 10  200, 300").unwrap();
        assert_eq!(
            (vb.min_x, vb.min_y, vb.width, vb.height),
            (5.0, 10.0, 200.0, 300.0)
        );
    }

    #[test]
    fn parse_viewbox_too_few_values() {
        assert!(parse_viewbox("0 0 100").is_none());
    }

    #[test]
    fn parse_viewbox_too_many_values() {
        assert!(parse_viewbox("0 0 100 200 300").is_none());
    }

    #[test]
    fn parse_viewbox_invalid_number() {
        assert!(parse_viewbox("0 abc 100 200").is_none());
    }

    // ── parse_svg_color edge cases ─────────────────────────────────────

    #[test]
    fn parse_svg_color_hex_3_char() {
        let c = parse_svg_color("#f00").unwrap();
        assert_eq!(c, (1.0, 0.0, 0.0));
    }

    #[test]
    fn parse_svg_color_hex_3_char_white() {
        let c = parse_svg_color("#fff").unwrap();
        assert_eq!(c, (1.0, 1.0, 1.0));
    }

    #[test]
    fn parse_svg_color_hex_invalid_length() {
        assert!(parse_svg_color("#abcd").is_none());
    }

    #[test]
    fn parse_svg_color_rgb_func() {
        let c = parse_svg_color("rgb(255, 0, 128)").unwrap();
        assert!((c.0 - 1.0).abs() < 0.01);
        assert!((c.1 - 0.0).abs() < 0.01);
        assert!((c.2 - 128.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn parse_svg_color_rgb_func_with_spaces() {
        let c = parse_svg_color("rgb( 0 , 128 , 255 )").unwrap();
        assert!((c.0 - 0.0).abs() < 0.01);
        assert!((c.1 - 128.0 / 255.0).abs() < 0.01);
        assert!((c.2 - 1.0).abs() < 0.01);
    }

    #[test]
    fn parse_svg_color_rgb_invalid_components() {
        // Only 2 components
        assert!(parse_svg_color("rgb(255, 0)").is_none());
    }

    #[test]
    fn parse_svg_color_rgb_non_numeric() {
        assert!(parse_svg_color("rgb(a, b, c)").is_none());
    }

    #[test]
    fn parse_svg_color_named_black() {
        assert_eq!(parse_svg_color("black"), Some((0.0, 0.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_named_white() {
        assert_eq!(parse_svg_color("white"), Some((1.0, 1.0, 1.0)));
    }

    #[test]
    fn parse_svg_color_named_green() {
        assert_eq!(parse_svg_color("green"), Some((0.0, 128.0 / 255.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_named_blue() {
        assert_eq!(parse_svg_color("blue"), Some((0.0, 0.0, 1.0)));
    }

    #[test]
    fn parse_svg_color_named_yellow() {
        assert_eq!(parse_svg_color("yellow"), Some((1.0, 1.0, 0.0)));
    }

    #[test]
    fn parse_svg_color_named_cyan() {
        assert_eq!(parse_svg_color("cyan"), Some((0.0, 1.0, 1.0)));
    }

    #[test]
    fn parse_svg_color_named_magenta() {
        assert_eq!(parse_svg_color("magenta"), Some((1.0, 0.0, 1.0)));
    }

    #[test]
    fn parse_svg_color_named_gray() {
        let expected = (128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0);
        assert_eq!(parse_svg_color("gray"), Some(expected));
        assert_eq!(parse_svg_color("grey"), Some(expected));
    }

    #[test]
    fn parse_svg_color_named_orange() {
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
    fn parse_svg_color_with_leading_trailing_spaces() {
        assert_eq!(parse_svg_color("  red  "), Some((1.0, 0.0, 0.0)));
    }

    // ── parse_points edge cases ────────────────────────────────────────

    #[test]
    fn parse_points_space_only() {
        let pts = parse_points("10 20 30 40");
        assert_eq!(pts, vec![(10.0, 20.0), (30.0, 40.0)]);
    }

    #[test]
    fn parse_points_odd_count() {
        // Odd number of values: last unpaired value is ignored
        let pts = parse_points("10,20,30");
        assert_eq!(pts, vec![(10.0, 20.0)]);
    }

    #[test]
    fn parse_points_empty() {
        let pts = parse_points("");
        assert!(pts.is_empty());
    }

    #[test]
    fn parse_points_extra_whitespace() {
        let pts = parse_points("  1 , 2  ,  3 , 4  ");
        assert_eq!(pts, vec![(1.0, 2.0), (3.0, 4.0)]);
    }

    // ── Path command edge cases ────────────────────────────────────────

    #[test]
    fn parse_path_relative_move() {
        let cmds = parse_path_data("m 5 10 l 3 4");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], PathCommand::MoveTo(5.0, 10.0));
        assert_eq!(cmds[1], PathCommand::LineTo(8.0, 14.0));
    }

    #[test]
    fn parse_path_relative_h_v() {
        let cmds = parse_path_data("M 10 20 h 5 v 10");
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0], PathCommand::MoveTo(10.0, 20.0));
        assert_eq!(cmds[1], PathCommand::LineTo(15.0, 20.0));
        assert_eq!(cmds[2], PathCommand::LineTo(15.0, 30.0));
    }

    #[test]
    fn parse_path_relative_cubic() {
        let cmds = parse_path_data("M 10 10 c 5 0 5 5 0 5");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], PathCommand::MoveTo(10.0, 10.0));
        assert_eq!(
            cmds[1],
            PathCommand::CubicTo(15.0, 10.0, 15.0, 15.0, 10.0, 15.0)
        );
    }

    #[test]
    fn parse_path_smooth_cubic_s() {
        // S command: reflects previous control point
        let cmds = parse_path_data("M 0 0 C 10 0 20 10 20 20 S 30 40 20 40");
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        assert_eq!(
            cmds[1],
            PathCommand::CubicTo(10.0, 0.0, 20.0, 10.0, 20.0, 20.0)
        );
        // Reflected control: 2*20 - 20 = 20, 2*20 - 10 = 30
        assert_eq!(
            cmds[2],
            PathCommand::CubicTo(20.0, 30.0, 30.0, 40.0, 20.0, 40.0)
        );
    }

    #[test]
    fn parse_path_smooth_cubic_s_relative() {
        let cmds = parse_path_data("M 10 10 C 15 10 20 15 20 20 s 5 10 0 10");
        assert_eq!(cmds.len(), 3);
        // After C: cur=(20,20), last_ctrl=(20,15)
        // Reflected: (2*20-20, 2*20-15) = (20, 25)
        // s relative: x2=20+5=25, y2=20+10=30, x=20+0=20, y=20+10=30
        assert_eq!(
            cmds[2],
            PathCommand::CubicTo(20.0, 25.0, 25.0, 30.0, 20.0, 30.0)
        );
    }

    #[test]
    fn parse_path_quad_q() {
        let cmds = parse_path_data("M 0 0 Q 10 20 30 40");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[1], PathCommand::QuadTo(10.0, 20.0, 30.0, 40.0));
    }

    #[test]
    fn parse_path_quad_relative_q() {
        let cmds = parse_path_data("M 10 10 q 5 10 15 20");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[1], PathCommand::QuadTo(15.0, 20.0, 25.0, 30.0));
    }

    #[test]
    fn parse_path_smooth_quad_t() {
        let cmds = parse_path_data("M 0 0 Q 10 20 20 20 T 40 0");
        assert_eq!(cmds.len(), 3);
        // After Q: cur=(20,20), last_ctrl=(10,20)
        // T reflected: (2*20-10, 2*20-20) = (30, 20)
        assert_eq!(cmds[2], PathCommand::QuadTo(30.0, 20.0, 40.0, 0.0));
    }

    #[test]
    fn parse_path_smooth_quad_t_relative() {
        let cmds = parse_path_data("M 0 0 Q 5 10 10 10 t 10 0");
        assert_eq!(cmds.len(), 3);
        // After Q: cur=(10,10), last_ctrl=(5,10)
        // t reflected: (2*10-5, 2*10-10) = (15, 10)
        // t relative endpoint: (10+10, 10+0) = (20, 10)
        assert_eq!(cmds[2], PathCommand::QuadTo(15.0, 10.0, 20.0, 10.0));
    }

    #[test]
    fn parse_path_lowercase_z() {
        let cmds = parse_path_data("M 0 0 L 10 0 z");
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[2], PathCommand::ClosePath);
    }

    #[test]
    fn parse_path_implicit_lineto_after_move() {
        // After M, implicit repeated coordinates become L
        let cmds = parse_path_data("M 0 0 10 10 20 20");
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        assert_eq!(cmds[1], PathCommand::LineTo(10.0, 10.0));
        assert_eq!(cmds[2], PathCommand::LineTo(20.0, 20.0));
    }

    #[test]
    fn parse_path_implicit_lineto_after_relative_move() {
        let cmds = parse_path_data("m 0 0 10 10");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        // implicit 'l' after 'm': relative
        assert_eq!(cmds[1], PathCommand::LineTo(10.0, 10.0));
    }

    #[test]
    fn parse_path_negative_numbers() {
        let cmds = parse_path_data("M -5 -10 L -20 -30");
        assert_eq!(cmds[0], PathCommand::MoveTo(-5.0, -10.0));
        assert_eq!(cmds[1], PathCommand::LineTo(-20.0, -30.0));
    }

    #[test]
    fn parse_path_numbers_without_space() {
        // Negative sign acts as separator
        let cmds = parse_path_data("M10-20L30-40");
        assert_eq!(cmds[0], PathCommand::MoveTo(10.0, -20.0));
        assert_eq!(cmds[1], PathCommand::LineTo(30.0, -40.0));
    }

    #[test]
    fn parse_path_decimal_without_leading_zero() {
        let cmds = parse_path_data("M .5 .5 L 1.5 1.5");
        assert_eq!(cmds[0], PathCommand::MoveTo(0.5, 0.5));
        assert_eq!(cmds[1], PathCommand::LineTo(1.5, 1.5));
    }

    #[test]
    fn parse_path_consecutive_decimals() {
        // Two decimals separated by dot: "0.5.5" should be 0.5 and .5
        let cmds = parse_path_data("M 0.5.5 1.5.5");
        assert_eq!(cmds[0], PathCommand::MoveTo(0.5, 0.5));
        assert_eq!(cmds[1], PathCommand::LineTo(1.5, 0.5));
    }

    #[test]
    fn parse_path_empty() {
        let cmds = parse_path_data("");
        assert!(cmds.is_empty());
    }

    #[test]
    fn parse_path_unknown_command_skipped() {
        // 'A' (arc) is not supported; it should be skipped
        let cmds = parse_path_data("M 0 0 A 1 1 0 0 1 10 10 L 20 20");
        // M produces MoveTo, then A is unknown and chars get skipped,
        // eventually L 20 20 is parsed
        assert!(cmds.iter().any(|c| *c == PathCommand::MoveTo(0.0, 0.0)));
    }

    // ── parse_transform edge cases ─────────────────────────────────────

    #[test]
    fn parse_transform_rotate_with_center() {
        let t = parse_transform("rotate(90, 50, 50)").unwrap();
        match t {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                let cos90 = 90.0_f32.to_radians().cos();
                let sin90 = 90.0_f32.to_radians().sin();
                assert!((a - cos90).abs() < 0.01);
                assert!((b - sin90).abs() < 0.01);
                assert!((c - (-sin90)).abs() < 0.01);
                assert!((d - cos90).abs() < 0.01);
                // tx = cx - cos*cx + sin*cy = 50 - cos90*50 + sin90*50
                let tx = 50.0 - cos90 * 50.0 + sin90 * 50.0;
                let ty = 50.0 - sin90 * 50.0 - cos90 * 50.0;
                assert!((e - tx).abs() < 0.01);
                assert!((f - ty).abs() < 0.01);
            }
        }
    }

    #[test]
    fn parse_transform_scale_xy() {
        let t = parse_transform("scale(2, 3)").unwrap();
        match t {
            SvgTransform::Matrix(a, _b, _c, d, _e, _f) => {
                assert!((a - 2.0).abs() < 0.001);
                assert!((d - 3.0).abs() < 0.001);
            }
        }
    }

    #[test]
    fn parse_transform_translate_single_value() {
        // translate with one value: ty defaults to 0
        let t = parse_transform("translate(10)").unwrap();
        match t {
            SvgTransform::Matrix(_a, _b, _c, _d, e, f) => {
                assert!((e - 10.0).abs() < 0.001);
                assert!((f - 0.0).abs() < 0.001);
            }
        }
    }

    #[test]
    fn parse_transform_unknown() {
        assert!(parse_transform("skewX(30)").is_none());
    }

    #[test]
    fn parse_transform_empty() {
        assert!(parse_transform("").is_none());
    }

    // ── parse_svg_node for element types ───────────────────────────────

    #[test]
    fn parse_node_rect() {
        let el = make_el(
            "rect",
            vec![
                ("x", "10"),
                ("y", "20"),
                ("width", "100"),
                ("height", "50"),
                ("rx", "5"),
                ("ry", "3"),
            ],
        );
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Rect {
                x,
                y,
                width,
                height,
                rx,
                ry,
                ..
            } => {
                assert_eq!(
                    (x, y, width, height, rx, ry),
                    (10.0, 20.0, 100.0, 50.0, 5.0, 3.0)
                );
            }
            _ => panic!("Expected Rect"),
        }
    }

    #[test]
    fn parse_node_circle() {
        let el = make_el("circle", vec![("cx", "50"), ("cy", "50"), ("r", "25")]);
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Circle { cx, cy, r, .. } => {
                assert_eq!((cx, cy, r), (50.0, 50.0, 25.0));
            }
            _ => panic!("Expected Circle"),
        }
    }

    #[test]
    fn parse_node_ellipse() {
        let el = make_el(
            "ellipse",
            vec![("cx", "50"), ("cy", "50"), ("rx", "30"), ("ry", "20")],
        );
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Ellipse { cx, cy, rx, ry, .. } => {
                assert_eq!((cx, cy, rx, ry), (50.0, 50.0, 30.0, 20.0));
            }
            _ => panic!("Expected Ellipse"),
        }
    }

    #[test]
    fn parse_node_line() {
        let el = make_el(
            "line",
            vec![("x1", "0"), ("y1", "0"), ("x2", "100"), ("y2", "100")],
        );
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Line { x1, y1, x2, y2, .. } => {
                assert_eq!((x1, y1, x2, y2), (0.0, 0.0, 100.0, 100.0));
            }
            _ => panic!("Expected Line"),
        }
    }

    #[test]
    fn parse_node_polyline() {
        let el = make_el("polyline", vec![("points", "0,0 10,20 30,40")]);
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Polyline { points, .. } => {
                assert_eq!(points, vec![(0.0, 0.0), (10.0, 20.0), (30.0, 40.0)]);
            }
            _ => panic!("Expected Polyline"),
        }
    }

    #[test]
    fn parse_node_polyline_no_points() {
        let el = make_el("polyline", vec![]);
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Polyline { points, .. } => {
                assert!(points.is_empty());
            }
            _ => panic!("Expected Polyline"),
        }
    }

    #[test]
    fn parse_node_polygon() {
        let el = make_el("polygon", vec![("points", "0,0 50,0 50,50 0,50")]);
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Polygon { points, .. } => {
                assert_eq!(points.len(), 4);
            }
            _ => panic!("Expected Polygon"),
        }
    }

    #[test]
    fn parse_node_path() {
        let el = make_el("path", vec![("d", "M 0 0 L 10 10 Z")]);
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Path { commands, .. } => {
                assert_eq!(commands.len(), 3);
            }
            _ => panic!("Expected Path"),
        }
    }

    #[test]
    fn parse_node_path_no_d_attr() {
        let el = make_el("path", vec![]);
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Path { commands, .. } => {
                assert!(commands.is_empty());
            }
            _ => panic!("Expected Path"),
        }
    }

    #[test]
    fn parse_node_group() {
        let child = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut group = make_el("g", vec![("transform", "translate(5,5)")]);
        group.children.push(DomNode::Element(child));
        let node = parse_svg_node(&group).unwrap();
        match node {
            SvgNode::Group {
                transform,
                children,
                ..
            } => {
                assert!(transform.is_some());
                assert_eq!(children.len(), 1);
            }
            _ => panic!("Expected Group"),
        }
    }

    #[test]
    fn parse_node_group_with_text_child_ignored() {
        let mut group = make_el("g", vec![]);
        group.children.push(DomNode::Text("hello".to_string()));
        let node = parse_svg_node(&group).unwrap();
        match node {
            SvgNode::Group { children, .. } => {
                assert!(children.is_empty());
            }
            _ => panic!("Expected Group"),
        }
    }

    #[test]
    fn parse_node_unknown_tag_returns_none() {
        let el = make_el("defs", vec![]);
        assert!(parse_svg_node(&el).is_none());
    }

    // ── parse_svg_style ────────────────────────────────────────────────

    #[test]
    fn parse_style_defaults() {
        let el = make_el("rect", vec![]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::Unspecified);
        assert_eq!(style.stroke, SvgPaint::Unspecified);
        assert_eq!(style.stroke_width, None);
        assert_eq!(style.opacity, 1.0);
    }

    #[test]
    fn parse_style_with_fill_stroke() {
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
    fn parse_style_fill_none() {
        let el = make_el("rect", vec![("fill", "none")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::None);
    }

    #[test]
    fn parse_style_stroke_none() {
        let el = make_el("rect", vec![("stroke", "none")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.stroke, SvgPaint::None);
    }

    #[test]
    fn parse_style_from_style_attribute() {
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

    // ── parse_svg_from_element ─────────────────────────────────────────

    #[test]
    fn parse_svg_from_element_basic() {
        let rect = make_el("rect", vec![("width", "50"), ("height", "30")]);
        let svg = make_svg_el(
            vec![
                ("width", "200"),
                ("height", "100"),
                ("viewBox", "0 0 200 100"),
            ],
            vec![rect],
        );
        let tree = parse_svg_from_element(&svg).unwrap();
        assert_eq!(tree.width, 200.0);
        assert_eq!(tree.height, 100.0);
        assert!(tree.view_box.is_some());
        assert_eq!(tree.children.len(), 1);
    }

    #[test]
    fn parse_svg_from_element_defaults() {
        let svg = make_svg_el(vec![], vec![]);
        let tree = parse_svg_from_element(&svg).unwrap();
        assert_eq!(tree.width, 300.0);
        assert_eq!(tree.height, 150.0);
        assert!(tree.view_box.is_none());
        assert!(tree.children.is_empty());
    }

    #[test]
    fn parse_svg_from_element_wraps_root_style_and_transform() {
        let rect = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let svg = make_svg_el(
            vec![("fill", "red"), ("transform", "translate(5, 6)")],
            vec![rect],
        );
        let tree = parse_svg_from_element(&svg).unwrap();
        assert_eq!(tree.children.len(), 1);
        match &tree.children[0] {
            SvgNode::Group {
                transform,
                children,
                style,
            } => {
                assert!(matches!(
                    transform,
                    Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 5.0, 6.0))
                ));
                assert!(matches!(style.fill, SvgPaint::Color((1.0, 0.0, 0.0))));
                assert_eq!(children.len(), 1);
            }
            other => panic!("expected wrapped root group, got {other:?}"),
        }
    }

    #[test]
    fn parse_text_style_ignores_font_size_adjust_prefix() {
        let text = make_el(
            "text",
            vec![("style", "font-size-adjust: 0.5; font-size: 20px")],
        );
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![text]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text {
                font_size_attr,
                font_size,
                ..
            } => {
                assert_eq!(font_size_attr.as_deref(), Some("20px"));
                assert_eq!(*font_size, Some(20.0));
            }
            other => panic!("expected text node, got {other:?}"),
        }
    }

    #[test]
    fn parse_text_style_ignores_fill_opacity_prefix() {
        let text = make_el(
            "text",
            vec![("style", "fill-opacity: 0.5; fill: currentColor")],
        );
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![text]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text {
                fill_raw,
                fill_specified,
                ..
            } => {
                assert!(*fill_specified);
                assert_eq!(fill_raw.as_deref(), Some("currentColor"));
            }
            other => panic!("expected text node, got {other:?}"),
        }
    }

    #[test]
    fn parse_text_raw_fill_prefers_inline_style_over_attribute() {
        let text = make_el(
            "text",
            vec![("fill", "none"), ("style", "fill: currentColor")],
        );
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![text]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { fill_raw, .. } => {
                assert_eq!(fill_raw.as_deref(), Some("currentColor"));
            }
            other => panic!("expected text node, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_from_element_text_children_ignored() {
        let mut svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![]);
        svg.children.push(DomNode::Text("some text".to_string()));
        let tree = parse_svg_from_element(&svg).unwrap();
        assert!(tree.children.is_empty());
    }

    #[test]
    fn parse_svg_from_element_unknown_child_skipped() {
        let defs_el = make_el("defs", vec![]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs_el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        assert!(tree.children.is_empty());
    }

    // ── attr_f32 ───────────────────────────────────────────────────────

    #[test]
    fn attr_f32_present() {
        let el = make_el("rect", vec![("x", "42px")]);
        assert_eq!(attr_f32(&el, "x"), 42.0);
    }

    #[test]
    fn attr_f32_missing() {
        let el = make_el("rect", vec![]);
        assert_eq!(attr_f32(&el, "x"), 0.0);
    }

    // ── tokenize_path edge cases ───────────────────────────────────────

    #[test]
    fn tokenize_path_commas_and_spaces() {
        let tokens = tokenize_path("M10,20 L30,40");
        assert_eq!(
            tokens,
            vec!["M", "10", "20", "L", "30", "40"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn tokenize_path_negative_after_number() {
        let tokens = tokenize_path("M10-20");
        assert_eq!(
            tokens,
            vec!["M", "10", "-20"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn tokenize_path_double_dot() {
        let tokens = tokenize_path("0.5.5");
        assert_eq!(
            tokens,
            vec!["0.5", ".5"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    // ── read_number / read_pair / read_four / read_six edge cases ──────

    #[test]
    fn read_number_past_end() {
        let tokens: Vec<String> = vec![];
        let mut i = 0;
        assert!(read_number(&tokens, &mut i).is_none());
    }

    #[test]
    fn read_number_non_numeric() {
        let tokens = vec!["abc".to_string()];
        let mut i = 0;
        assert!(read_number(&tokens, &mut i).is_none());
    }

    #[test]
    fn read_pair_insufficient_tokens() {
        let tokens = vec!["5".to_string()];
        let mut i = 0;
        assert!(read_pair(&tokens, &mut i).is_none());
    }

    #[test]
    fn read_four_insufficient_tokens() {
        let tokens = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        let mut i = 0;
        assert!(read_four(&tokens, &mut i).is_none());
    }

    #[test]
    fn read_six_insufficient_tokens() {
        let tokens = vec![
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
            "4".to_string(),
            "5".to_string(),
        ];
        let mut i = 0;
        assert!(read_six(&tokens, &mut i).is_none());
    }

    // ── extract_func_args / parse_num_list ─────────────────────────────

    #[test]
    fn extract_func_args_basic() {
        assert_eq!(
            extract_func_args("translate(10, 20)", "translate"),
            Some("10, 20".to_string())
        );
    }

    #[test]
    fn extract_func_args_not_found() {
        assert_eq!(extract_func_args("translate(10, 20)", "rotate"), None);
    }

    #[test]
    fn extract_func_args_no_parens() {
        assert_eq!(extract_func_args("translate", "translate"), None);
    }

    #[test]
    fn parse_num_list_basic() {
        let nums = parse_num_list("1, 2.5, 3");
        assert_eq!(nums, vec![1.0, 2.5, 3.0]);
    }

    #[test]
    fn parse_num_list_empty() {
        let nums = parse_num_list("");
        assert!(nums.is_empty());
    }

    #[test]
    fn parse_num_list_with_invalid() {
        // Invalid entries are skipped by filter_map
        let nums = parse_num_list("1, abc, 3");
        assert_eq!(nums, vec![1.0, 3.0]);
    }

    // ── Nested SVG element in group ────────────────────────────────────

    #[test]
    fn parse_node_nested_svg_acts_as_group() {
        let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut svg_inner = make_el("svg", vec![]);
        svg_inner.children.push(DomNode::Element(inner));
        let node = parse_svg_node(&svg_inner).unwrap();
        match node {
            SvgNode::Group { children, .. } => {
                assert_eq!(children.len(), 1);
            }
            _ => panic!("Expected Group for inner svg"),
        }
    }

    #[test]
    fn parse_node_nested_svg_applies_viewport_transform() {
        let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut svg_inner = make_el(
            "svg",
            vec![
                ("x", "10"),
                ("y", "20"),
                ("width", "100"),
                ("height", "50"),
                ("viewBox", "0 0 10 5"),
            ],
        );
        svg_inner.children.push(DomNode::Element(inner));
        let node = parse_svg_node(&svg_inner).unwrap();
        match node {
            SvgNode::Group { transform, .. } => {
                assert!(matches!(
                    transform,
                    Some(SvgTransform::Matrix(10.0, 0.0, 0.0, 10.0, 10.0, 20.0))
                ));
            }
            other => panic!("expected nested svg group, got {other:?}"),
        }
    }

    #[test]
    fn parse_node_nested_svg_percent_viewport_uses_parent_size() {
        let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut svg_inner = make_el(
            "svg",
            vec![
                ("width", "100%"),
                ("height", "50%"),
                ("viewBox", "0 0 20 10"),
            ],
        );
        svg_inner.children.push(DomNode::Element(inner));
        let outer = make_svg_el(vec![("width", "200"), ("height", "100")], vec![svg_inner]);
        let tree = parse_svg_from_element(&outer).unwrap();
        match &tree.children[0] {
            SvgNode::Group { transform, .. } => {
                assert!(matches!(
                    transform,
                    Some(SvgTransform::Matrix(10.0, 0.0, 0.0, 5.0, 0.0, 0.0))
                ));
            }
            other => panic!("expected nested svg group, got {other:?}"),
        }
    }

    // ── Polygon without points ─────────────────────────────────────────

    #[test]
    fn parse_node_polygon_no_points() {
        let el = make_el("polygon", vec![]);
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Polygon { points, .. } => {
                assert!(points.is_empty());
            }
            _ => panic!("Expected Polygon"),
        }
    }

    // ── Rect with missing attributes defaults to 0 ─────────────────────

    #[test]
    fn parse_node_rect_defaults() {
        let el = make_el("rect", vec![]);
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Rect {
                x,
                y,
                width,
                height,
                rx,
                ry,
                ..
            } => {
                assert_eq!(
                    (x, y, width, height, rx, ry),
                    (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
                );
            }
            _ => panic!("Expected Rect"),
        }
    }

    // ── Group without transform ────────────────────────────────────────

    #[test]
    fn parse_node_group_no_transform() {
        let group = make_el("g", vec![]);
        let node = parse_svg_node(&group).unwrap();
        match node {
            SvgNode::Group { transform, .. } => {
                assert!(transform.is_none());
            }
            _ => panic!("Expected Group"),
        }
    }
}
