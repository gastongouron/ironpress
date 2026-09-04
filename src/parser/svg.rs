//! SVG parser — converts DOM SVG elements into an SvgTree for PDF rendering.

use crate::parser::css::{CssValue, parse_inline_style, parse_property_value};
use crate::parser::dom::{DomNode, ElementNode};
use crate::types::Color;
use std::collections::HashMap;

mod letter_spacing;
pub use letter_spacing::SvgLetterSpacing;

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

fn style_property_value<'a>(el: &'a ElementNode, name: &str) -> Option<&'a str> {
    let style_val = el.attributes.get("style")?;
    let mut value = None;

    for part in split_style_declarations(style_val) {
        let part = part.trim();
        if let Some((prop, val)) = part.split_once(':') {
            if prop.trim() == name {
                value = Some(val.trim());
            }
        }
    }

    value
}

/// Inherited CSS context for SVG text rendering.
#[derive(Debug, Clone)]
pub struct SvgTextContext {
    pub font_family: String,
    pub font_size: f32,
    pub font_bold: bool,
    pub font_italic: bool,
    pub color: Option<Color>,
}

const SVG_INITIAL_FONT_FAMILY: &str = "Helvetica";
const SVG_INITIAL_FONT_SIZE: f32 = 12.0;

impl Default for SvgTextContext {
    fn default() -> Self {
        Self {
            font_family: SVG_INITIAL_FONT_FAMILY.to_string(),
            font_size: SVG_INITIAL_FONT_SIZE,
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
    pub preserve_aspect_ratio: SvgPreserveAspectRatio,
    pub view_box: Option<ViewBox>,
    pub defs: SvgDefs,
    pub children: Vec<SvgNode>,
    pub text_ctx: SvgTextContext,
    pub source_markup: Option<String>,
}

impl SvgTree {
    pub fn with_source_markup(mut self, source_markup: impl Into<String>) -> Self {
        self.source_markup = Some(source_markup.into());
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ViewBox {
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SvgDefs {
    pub gradients: std::collections::HashMap<String, SvgLinearGradient>,
    pub radial_gradients: std::collections::HashMap<String, SvgRadialGradient>,
    pub clip_paths: std::collections::HashMap<String, SvgClipPath>,
    pub masks: std::collections::HashMap<String, SvgMask>,
}

impl SvgDefs {
    fn append(&mut self, defs: Self) {
        self.gradients.extend(defs.gradients);
        self.radial_gradients.extend(defs.radial_gradients);
        self.clip_paths.extend(defs.clip_paths);
        self.masks.extend(defs.masks);
    }
}

#[derive(Debug, Clone)]
pub struct SvgLinearGradient {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub gradient_units: SvgGradientUnits,
    pub gradient_transform: Option<SvgTransform>,
    pub stops: Vec<SvgGradientStop>,
}

#[derive(Debug, Clone)]
pub struct SvgRadialGradient {
    pub cx: f32,
    pub cy: f32,
    pub r: f32,
    pub fx: f32,
    pub fy: f32,
    pub gradient_units: SvgGradientUnits,
    pub gradient_transform: Option<SvgTransform>,
    pub stops: Vec<SvgGradientStop>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvgGradientUnits {
    UserSpaceOnUse,
    #[default]
    ObjectBoundingBox,
}

#[derive(Debug, Clone, Copy)]
pub struct SvgGradientStop {
    pub offset: f32,
    pub color: Color,
    pub opacity: f32,
}

#[derive(Debug, Clone)]
pub struct SvgClipPath {
    pub clip_path_units: SvgClipPathUnits,
    pub transform: Option<SvgTransform>,
    pub children: Vec<SvgNode>,
}

#[derive(Debug, Clone)]
pub struct SvgMask {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub mask_type: SvgMaskType,
    pub children: Vec<SvgNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvgMaskType {
    Alpha,
    #[default]
    Luminance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvgClipPathUnits {
    #[default]
    UserSpaceOnUse,
    ObjectBoundingBox,
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
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        href: String,
        preserve_aspect_ratio: SvgPreserveAspectRatio,
        style: SvgStyle,
    },
    Text {
        x: f32,
        y: f32,
        /// True when the element explicitly set `fill` (including `none`).
        fill_specified: bool,
        fill_raw: Option<String>,
        /// Per-element font-family override (resolved PDF name, e.g. "Helvetica-Bold").
        font_family: Option<String>,
        /// Per-element font-weight override (true = bold).
        font_bold: Option<bool>,
        /// Per-element font-style override (true = italic/oblique).
        font_italic: Option<bool>,
        /// SVG text-anchor: "start" (default), "middle", or "end".
        text_anchor: SvgTextAnchor,
        content: String,
        style: SvgStyle,
    },
}

/// SVG text-anchor property.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SvgTextAnchor {
    #[default]
    Start,
    Middle,
    End,
}

/// A parsed SVG `font-size` whose unit conversion remains tied to its parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvgFontSize(SvgFontSizeValue);

#[derive(Debug, Clone, Copy, PartialEq)]
enum SvgFontSizeValue {
    UserUnits(f32),
    ParentFactor(f32),
}

impl SvgFontSize {
    const fn initial() -> Self {
        Self(SvgFontSizeValue::UserUnits(SVG_INITIAL_FONT_SIZE))
    }

    fn from_css_value(value: &CssValue, presentation_attribute: bool) -> Option<Self> {
        match value {
            CssValue::Length(points) => Self::user_units(points * 4.0 / 3.0),
            CssValue::Em(factor) => Self::parent_factor(*factor),
            CssValue::Percentage(percentage) => Self::parent_factor(percentage / 100.0),
            CssValue::Number(value) if presentation_attribute => Self::user_units(*value),
            _ => None,
        }
    }

    pub(crate) fn user_units(value: f32) -> Option<Self> {
        (value.is_finite() && value >= 0.0).then_some(Self(SvgFontSizeValue::UserUnits(value)))
    }

    pub(crate) fn parent_factor(value: f32) -> Option<Self> {
        (value.is_finite() && value >= 0.0).then_some(Self(SvgFontSizeValue::ParentFactor(value)))
    }

    pub(crate) fn resolve_user_units(self, inherited_size: f32) -> f32 {
        match self.0 {
            SvgFontSizeValue::UserUnits(value) => value,
            SvgFontSizeValue::ParentFactor(factor) => inherited_size * factor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SvgPaint {
    /// The property was not specified on this element (so it should inherit from its parent).
    #[default]
    Unspecified,
    /// The property was explicitly set to `none`.
    None,
    /// `currentColor` keyword (resolves to the CSS `color` property).
    CurrentColor,
    /// `url(#id)` paint server reference.
    Url(String),
    /// An explicit sRGB color.
    Color(Color),
}

#[derive(Debug, Clone)]
pub struct SvgStyle {
    pub color: Option<Color>,
    pub fill: SvgPaint,
    pub stroke: SvgPaint,
    pub clip_path: Option<String>,
    pub clip_rule: SvgClipRule,
    /// `stroke-width` is inherited in SVG.
    pub stroke_width: Option<f32>,
    /// Inherited SVG font-family, resolved to a PDF base family name.
    pub font_family: Option<String>,
    /// Inherited SVG font-weight.
    pub font_bold: Option<bool>,
    /// Inherited SVG font-style.
    pub font_italic: Option<bool>,
    /// Local inherited `font-size`; relative values resolve against the parent.
    pub font_size: Option<SvgFontSize>,
    /// Local inherited `letter-spacing`; descendants inherit its computed length.
    pub letter_spacing: Option<SvgLetterSpacing>,
    // Opacity isn't wired through to PDF output yet; keep it simple until needed.
    pub opacity: f32,
}

impl Default for SvgStyle {
    fn default() -> Self {
        Self {
            color: None,
            fill: SvgPaint::Unspecified,
            stroke: SvgPaint::Unspecified,
            clip_path: None,
            clip_rule: SvgClipRule::NonZero,
            stroke_width: None,
            font_family: None,
            font_bold: None,
            font_italic: None,
            font_size: None,
            letter_spacing: None,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgPreserveAspectRatio {
    None,
    Align {
        align: SvgAlign,
        meet_or_slice: SvgMeetOrSlice,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvgClipRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl Default for SvgPreserveAspectRatio {
    fn default() -> Self {
        Self::Align {
            align: SvgAlign::Center,
            meet_or_slice: SvgMeetOrSlice::Meet,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgAlign {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgMeetOrSlice {
    Meet,
    Slice,
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
    parse_svg_from_element_with_ctx_and_viewport(el, SvgTextContext::default(), None)
}

/// Parse an SVG tree from raw SVG markup.
pub fn parse_svg_from_string(svg_text: &str) -> Option<SvgTree> {
    parse_svg_from_string_with_viewport(svg_text, None)
}

/// Parse standalone SVG markup for a concrete viewport supplied by its host.
pub(crate) fn parse_svg_from_string_with_viewport(
    svg_text: &str,
    viewport: Option<(f32, f32)>,
) -> Option<SvgTree> {
    let nodes = crate::parser::html::parse_html(svg_text).ok()?;
    nodes.iter().find_map(|node| {
        if let DomNode::Element(el) = node {
            (el.tag == crate::parser::dom::HtmlTag::Svg)
                .then_some(el)
                .and_then(|element| {
                    parse_svg_from_element_with_ctx_and_viewport(
                        element,
                        SvgTextContext::default(),
                        viewport,
                    )
                })
                .map(|tree| tree.with_source_markup(svg_text))
        } else {
            None
        }
    })
}

pub fn parse_svg_from_element_with_viewport(
    el: &ElementNode,
    root_viewport_override: Option<(f32, f32)>,
) -> Option<SvgTree> {
    parse_svg_from_element_with_ctx_and_viewport(
        el,
        SvgTextContext::default(),
        root_viewport_override,
    )
}

pub fn parse_svg_from_element_with_ctx(
    el: &ElementNode,
    text_ctx: SvgTextContext,
) -> Option<SvgTree> {
    parse_svg_from_element_with_ctx_and_viewport(el, text_ctx, None)
}

pub fn parse_svg_from_element_with_ctx_and_viewport(
    el: &ElementNode,
    text_ctx: SvgTextContext,
    root_viewport_override: Option<(f32, f32)>,
) -> Option<SvgTree> {
    let width_attr = el.attributes.get("width").cloned();
    let height_attr = el.attributes.get("height").cloned();
    let parsed_width = width_attr
        .as_deref()
        .and_then(parse_absolute_length)
        .unwrap_or(300.0);
    let parsed_height = height_attr
        .as_deref()
        .and_then(parse_absolute_length)
        .unwrap_or(150.0);
    let (width, height) = root_viewport_override.unwrap_or((parsed_width, parsed_height));
    let view_box = el.attributes.get("viewBox").and_then(|v| parse_viewbox(v));
    let preserve_aspect_ratio = parse_svg_preserve_aspect_ratio(el);

    let defs_raw = collect_svg_defs(&el.children);
    let defs = parse_svg_defs(&defs_raw);

    let root_style = parse_svg_style(el);
    let root_transform = el
        .attributes
        .get("transform")
        .and_then(|v| parse_transform(v));
    let root_viewport = Some((width, height));
    let mut ctx = SvgParseContext::new(&defs_raw);
    let children = parse_svg_children(&el.children, root_viewport, &mut ctx);
    let children = if root_transform.is_some() || !svg_style_is_default(&root_style) {
        vec![SvgNode::Group {
            transform: root_transform,
            children,
            style: root_style,
        }]
    } else {
        children
    };

    Some(SvgTree {
        width,
        height,
        width_attr,
        height_attr,
        preserve_aspect_ratio,
        view_box,
        defs,
        children,
        text_ctx,
        source_markup: None,
    })
}

/// Parse a single SVG element node into an SvgNode.
fn parse_svg_node(el: &ElementNode) -> Option<SvgNode> {
    let defs_raw = HashMap::new();
    let mut ctx = SvgParseContext::new(&defs_raw);
    parse_svg_node_with_viewport(el, None, &mut ctx)
}

#[derive(Clone, Copy)]
struct SvgParseContext<'a> {
    defs_raw: &'a HashMap<String, ElementNode>,
    ref_stack_len: usize,
}

impl<'a> SvgParseContext<'a> {
    fn new(defs_raw: &'a HashMap<String, ElementNode>) -> Self {
        Self {
            defs_raw,
            ref_stack_len: 0,
        }
    }

    fn push_ref(&mut self) {
        self.ref_stack_len += 1;
    }

    fn pop_ref(&mut self) {
        self.ref_stack_len = self.ref_stack_len.saturating_sub(1);
    }

    fn ref_depth(&self) -> usize {
        self.ref_stack_len
    }
}

fn collect_svg_defs(children: &[DomNode]) -> HashMap<String, ElementNode> {
    let mut defs = HashMap::new();
    for child in children {
        if let DomNode::Element(child_el) = child {
            collect_svg_defs_from_element(child_el, &mut defs);
        }
    }
    defs
}

fn collect_svg_defs_from_element(el: &ElementNode, defs: &mut HashMap<String, ElementNode>) {
    if el.raw_tag_name.eq_ignore_ascii_case("defs") {
        for child in &el.children {
            if let DomNode::Element(child_el) = child {
                collect_svg_defs_from_element(child_el, defs);
            }
        }
        return;
    }

    if let Some(id) = el.attributes.get("id") {
        defs.entry(id.clone()).or_insert_with(|| el.clone());
    }

    for child in &el.children {
        if let DomNode::Element(child_el) = child {
            collect_svg_defs_from_element(child_el, defs);
        }
    }
}

fn parse_svg_defs(defs_raw: &HashMap<String, ElementNode>) -> SvgDefs {
    let mut defs = SvgDefs::default();
    for el in defs_raw.values() {
        match el.raw_tag_name.to_ascii_lowercase().as_str() {
            "lineargradient" => {
                if let Some(id) = el.attributes.get("id").cloned()
                    && let Some(gradient) = parse_svg_linear_gradient(el)
                {
                    defs.gradients.insert(id, gradient);
                }
            }
            "radialgradient" => {
                if let Some(id) = el.attributes.get("id").cloned()
                    && let Some(gradient) = parse_svg_radial_gradient(el)
                {
                    defs.radial_gradients.insert(id, gradient);
                }
            }
            "clippath" => {
                if let Some(id) = el.attributes.get("id").cloned()
                    && let Some(clip_path) = parse_svg_clip_path(el, defs_raw)
                {
                    defs.clip_paths.insert(id, clip_path);
                }
            }
            "mask" => {
                if let Some(id) = el.attributes.get("id").cloned()
                    && let Some(mask) = parse_svg_mask(el, defs_raw)
                {
                    defs.masks.insert(id, mask);
                }
            }
            _ => {}
        }
    }
    defs
}

/// Collect fragment-addressable SVG resources owned by one HTML document.
///
/// This walk is independent of box generation: an inline SVG used only as a
/// `<defs>` container may be `display: none` or otherwise produce no painted
/// layout element, but its fragment identifiers still belong to this document.
pub(crate) fn collect_document_svg_defs(nodes: &[DomNode]) -> SvgDefs {
    fn collect(nodes: &[DomNode], document_defs: &mut SvgDefs) {
        for node in nodes {
            let DomNode::Element(element) = node else {
                continue;
            };
            if element.raw_tag_name.eq_ignore_ascii_case("svg") {
                document_defs.append(parse_svg_defs(&collect_svg_defs(&element.children)));
            } else {
                collect(&element.children, document_defs);
            }
        }
    }

    let mut defs = SvgDefs::default();
    collect(nodes, &mut defs);
    defs
}

fn parse_svg_children(
    children: &[DomNode],
    viewport: Option<(f32, f32)>,
    ctx: &mut SvgParseContext<'_>,
) -> Vec<SvgNode> {
    children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(child_el) => {
                if child_el.raw_tag_name.eq_ignore_ascii_case("defs") {
                    None
                } else {
                    parse_svg_node_with_viewport(child_el, viewport, ctx)
                }
            }
            _ => None,
        })
        .collect()
}

fn parse_svg_node_with_viewport(
    el: &ElementNode,
    parent_viewport: Option<(f32, f32)>,
    ctx: &mut SvgParseContext<'_>,
) -> Option<SvgNode> {
    let tag = el.raw_tag_name.as_str();
    let node = match tag {
        "g" => {
            let style = parse_svg_style(el);
            let children = parse_svg_children(&el.children, parent_viewport, ctx);
            Some(SvgNode::Group {
                transform: None,
                children,
                style,
            })
        }
        "svg" => {
            let child_viewport = resolve_nested_svg_viewport(el, parent_viewport);
            let style = parse_svg_style(el);
            let children = parse_svg_children(&el.children, child_viewport, ctx);
            Some(SvgNode::Group {
                transform: nested_svg_viewport_transform(el, child_viewport),
                children,
                style,
            })
        }
        "use" => {
            let href = parse_svg_reference_id(
                el.attributes
                    .get("href")
                    .or_else(|| el.attributes.get("xlink:href"))?,
            )?;
            let referenced = parse_svg_referenced_node(&href, parent_viewport, ctx)?;
            let style = parse_svg_style(el);
            Some(SvgNode::Group {
                transform: svg_translate_from_use(el),
                children: vec![referenced],
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
        "image" => {
            let href = parse_svg_image_href(el)?;
            let x = resolve_svg_viewport_length(
                el.attributes.get("x"),
                parent_viewport.map(|(w, _)| w),
                0.0,
            );
            let y = resolve_svg_viewport_length(
                el.attributes.get("y"),
                parent_viewport.map(|(_, h)| h),
                0.0,
            );
            let width = resolve_svg_viewport_length(
                el.attributes.get("width"),
                parent_viewport.map(|(w, _)| w),
                0.0,
            )
            .max(0.0);
            let height = resolve_svg_viewport_length(
                el.attributes.get("height"),
                parent_viewport.map(|(_, h)| h),
                0.0,
            )
            .max(0.0);
            let preserve_aspect_ratio = parse_svg_preserve_aspect_ratio(el);
            let style = parse_svg_style(el);
            Some(SvgNode::Image {
                x,
                y,
                width,
                height,
                href,
                preserve_aspect_ratio,
                style,
            })
        }
        "text" => {
            let (x, y) = resolve_text_position(el, parent_viewport);
            let fill_specified = has_fill_specified(el);
            let fill_raw = parse_fill_raw(el);
            let (font_family, font_bold, font_italic) = parse_svg_font_attrs(el);
            let content = collect_text_content(el);
            let style = parse_svg_style(el);
            let text_anchor = match el.attributes.get("text-anchor").map(|s| s.as_str()) {
                Some("middle") => SvgTextAnchor::Middle,
                Some("end") => SvgTextAnchor::End,
                _ => SvgTextAnchor::Start,
            };
            Some(SvgNode::Text {
                x,
                y,
                fill_specified,
                fill_raw,
                font_family,
                font_bold,
                font_italic,
                text_anchor,
                content,
                style,
            })
        }
        _ => None,
    }?;

    Some(apply_svg_element_transform(el, node))
}

/// Apply `transform` uniformly to every supported SVG graphics element.
///
/// A transform is an element-level property in SVG, not a group-only feature.
/// Leaf nodes stay focused on their own geometry and are wrapped in a neutral
/// group, while group-like nodes compose their intrinsic viewport/use mapping
/// with the explicit element transform.
fn apply_svg_element_transform(el: &ElementNode, node: SvgNode) -> SvgNode {
    let Some(element_transform) = el
        .attributes
        .get("transform")
        .and_then(|value| parse_transform(value))
    else {
        return node;
    };

    match node {
        SvgNode::Group {
            transform,
            children,
            style,
        } => SvgNode::Group {
            transform: compose_transform(Some(element_transform), transform),
            children,
            style,
        },
        leaf => SvgNode::Group {
            transform: Some(element_transform),
            children: vec![leaf],
            style: SvgStyle::default(),
        },
    }
}

fn parse_svg_referenced_node(
    id: &str,
    parent_viewport: Option<(f32, f32)>,
    ctx: &mut SvgParseContext<'_>,
) -> Option<SvgNode> {
    ctx.defs_raw.get(id)?;

    if ctx.ref_depth() > 16 {
        return None;
    }

    let def = ctx.defs_raw.get(id)?.clone();
    ctx.push_ref();
    let parsed = parse_svg_node_with_viewport(&def, parent_viewport, ctx);
    ctx.pop_ref();
    parsed
}

fn parse_svg_linear_gradient(el: &ElementNode) -> Option<SvgLinearGradient> {
    let x1 = parse_svg_gradient_coordinate(el.attributes.get("x1"), 0.0);
    let y1 = parse_svg_gradient_coordinate(el.attributes.get("y1"), 0.0);
    let x2 = parse_svg_gradient_coordinate(el.attributes.get("x2"), 1.0);
    let y2 = parse_svg_gradient_coordinate(el.attributes.get("y2"), 0.0);
    let gradient_units = match el.attributes.get("gradientUnits").map(String::as_str) {
        Some(val) if val.eq_ignore_ascii_case("userSpaceOnUse") => SvgGradientUnits::UserSpaceOnUse,
        Some(val) if val.eq_ignore_ascii_case("objectBoundingBox") => {
            SvgGradientUnits::ObjectBoundingBox
        }
        _ => SvgGradientUnits::default(),
    };
    let gradient_transform = el
        .attributes
        .get("gradientTransform")
        .and_then(|v| parse_transform(v));
    let stops = el
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(stop) if stop.raw_tag_name.eq_ignore_ascii_case("stop") => {
                parse_svg_gradient_stop(stop)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    if stops.len() < 2 {
        return None;
    }

    Some(SvgLinearGradient {
        x1,
        y1,
        x2,
        y2,
        gradient_units,
        gradient_transform,
        stops,
    })
}

fn parse_svg_radial_gradient(el: &ElementNode) -> Option<SvgRadialGradient> {
    let cx = parse_svg_gradient_coordinate(el.attributes.get("cx"), 0.5);
    let cy = parse_svg_gradient_coordinate(el.attributes.get("cy"), 0.5);
    let r = parse_svg_gradient_coordinate(el.attributes.get("r"), 0.5);
    let fx = parse_svg_gradient_coordinate(el.attributes.get("fx"), cx);
    let fy = parse_svg_gradient_coordinate(el.attributes.get("fy"), cy);
    let gradient_units = match el.attributes.get("gradientUnits").map(String::as_str) {
        Some(val) if val.eq_ignore_ascii_case("userSpaceOnUse") => SvgGradientUnits::UserSpaceOnUse,
        Some(val) if val.eq_ignore_ascii_case("objectBoundingBox") => {
            SvgGradientUnits::ObjectBoundingBox
        }
        _ => SvgGradientUnits::default(),
    };
    let gradient_transform = el
        .attributes
        .get("gradientTransform")
        .and_then(|v| parse_transform(v));
    let stops = el
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(stop) if stop.raw_tag_name.eq_ignore_ascii_case("stop") => {
                parse_svg_gradient_stop(stop)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    if stops.len() < 2 {
        return None;
    }

    Some(SvgRadialGradient {
        cx,
        cy,
        r,
        fx,
        fy,
        gradient_units,
        gradient_transform,
        stops,
    })
}

fn parse_svg_gradient_stop(el: &ElementNode) -> Option<SvgGradientStop> {
    let offset = el
        .attributes
        .get("offset")
        .and_then(|v| parse_svg_gradient_offset(v))?;
    let stop_color = el
        .attributes
        .get("stop-color")
        .map(String::as_str)
        .or_else(|| style_property_value(el, "stop-color"))?;
    let color = parse_svg_color(stop_color)?;
    let opacity = el
        .attributes
        .get("stop-opacity")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .or_else(|| {
            style_property_value(el, "stop-opacity").and_then(|v| v.trim().parse::<f32>().ok())
        })
        .unwrap_or(1.0);

    Some(SvgGradientStop {
        offset,
        color,
        opacity,
    })
}

fn parse_svg_clip_path(
    el: &ElementNode,
    defs_raw: &HashMap<String, ElementNode>,
) -> Option<SvgClipPath> {
    let clip_path_units = match el.attributes.get("clipPathUnits").map(String::as_str) {
        Some(val) if val.eq_ignore_ascii_case("userSpaceOnUse") => SvgClipPathUnits::UserSpaceOnUse,
        Some(val) if val.eq_ignore_ascii_case("objectBoundingBox") => {
            SvgClipPathUnits::ObjectBoundingBox
        }
        _ => SvgClipPathUnits::default(),
    };
    let transform = el
        .attributes
        .get("transform")
        .and_then(|v| parse_transform(v));
    let mut ctx = SvgParseContext::new(defs_raw);
    let children = parse_svg_children(&el.children, None, &mut ctx);
    if children.is_empty() {
        return None;
    }

    Some(SvgClipPath {
        clip_path_units,
        transform,
        children,
    })
}

fn parse_svg_mask(el: &ElementNode, defs_raw: &HashMap<String, ElementNode>) -> Option<SvgMask> {
    let mask_type = el
        .attributes
        .get("mask-type")
        .map(String::as_str)
        .or_else(|| style_property_value(el, "mask-type"))
        .map(|v| {
            if v.trim().eq_ignore_ascii_case("alpha") {
                SvgMaskType::Alpha
            } else {
                SvgMaskType::Luminance
            }
        })
        .unwrap_or_default();
    let x = attr_f32(el, "x");
    let y = attr_f32(el, "y");
    let width = el
        .attributes
        .get("width")
        .and_then(|v| parse_absolute_length(v))
        .unwrap_or(0.0);
    let height = el
        .attributes
        .get("height")
        .and_then(|v| parse_absolute_length(v))
        .unwrap_or(0.0);
    let mut ctx = SvgParseContext::new(defs_raw);
    let children = parse_svg_children(&el.children, Some((width, height)), &mut ctx);
    if children.is_empty() {
        return None;
    }
    Some(SvgMask {
        x,
        y,
        width,
        height,
        mask_type,
        children,
    })
}

fn parse_svg_gradient_coordinate(attr: Option<&String>, fallback: f32) -> f32 {
    let Some(value) = attr.map(String::as_str) else {
        return fallback;
    };
    let trimmed = value.trim();
    if let Some(pct) = trimmed.strip_suffix('%') {
        return pct
            .trim()
            .parse::<f32>()
            .ok()
            .map_or(fallback, |pct| pct / 100.0);
    }
    parse_absolute_length(trimmed).unwrap_or(fallback)
}

fn parse_svg_gradient_offset(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    if let Some(pct) = trimmed.strip_suffix('%') {
        return pct.trim().parse::<f32>().ok().map(|pct| pct / 100.0);
    }
    trimmed.parse::<f32>().ok()
}

fn parse_svg_paint_server_reference(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix("url(")
        .and_then(|s| s.strip_suffix(')'))?
        .trim()
        .trim_matches(|c| c == '\'' || c == '"');
    inner.strip_prefix('#').map(|id| id.to_string())
}

fn parse_svg_reference_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if let Some(id) = trimmed.strip_prefix('#') {
        return Some(id.to_string());
    }
    parse_svg_paint_server_reference(trimmed)
}

fn parse_svg_clip_rule(raw: &str) -> SvgClipRule {
    if raw.trim().eq_ignore_ascii_case("evenodd") {
        SvgClipRule::EvenOdd
    } else {
        SvgClipRule::NonZero
    }
}

/// One parsed SVG filter whose region and color space remain attached to its
/// ordered primitive list.
#[derive(Debug, Clone)]
pub(crate) struct SvgFilterDefinition {
    pub(crate) operations: Vec<crate::style::computed::FilterOperation>,
    pub(crate) linear_rgb: bool,
    pub(crate) region: crate::style::computed::NormalizedFilterRegion,
}

/// Parse a CSS `filter: url(#id)` target into one semantic SVG filter.
///
/// Supported primitives (SVG filter-effects §15.9 feColorMatrix):
/// - `type="saturate"`  → `Saturate(values)`   (default amount 1)
/// - `type="matrix"`    → decoded to the closest [`FilterOperation`] when the
///   20-value matrix is a recognizable luminance/identity form; otherwise the
///   raw matrix is applied verbatim via [`FilterOperation`] is not possible, so it
///   is skipped.
/// - `type="luminanceToAlpha"` is skipped (alpha-only; no RGB color change).
///
/// `linear_rgb` reflects `color-interpolation-filters`, whose SVG initial value
/// is `linearRGB`. The returned region is the hard filter clip in normalized
/// object-bounding-box coordinates.
pub(crate) fn filter_element_definition(filter_el: &ElementNode) -> Option<SvgFilterDefinition> {
    use crate::style::computed::FilterOperation;
    if !filter_el.raw_tag_name.eq_ignore_ascii_case("filter") {
        return None;
    }
    let region = filter_region(filter_el);
    let mut operations = Vec::new();
    // Default linearRGB; honor `color-interpolation-filters` on the <filter>
    // (presentation attribute or inline style). A per-primitive override is read
    // below too; the most specific wins for the primitive that supplies the op.
    let filter_space_srgb = color_interpolation_filters_is_srgb(filter_el);
    let mut use_linear_rgb = !filter_space_srgb;
    let filter_current_color = parse_svg_style(filter_el).color.unwrap_or(Color::BLACK);
    let mut flood_color: Option<crate::types::Color> = None;
    let element_children: Vec<&ElementNode> = filter_el
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(el) => Some(el),
            _ => None,
        })
        .collect();
    for (idx, el) in element_children.iter().enumerate() {
        // SVG `color` is inherited. Resolve paint-like `currentColor` values at
        // the primitive's used-value boundary, using a local override when one
        // exists and otherwise the filter's inherited foreground.
        let current_color = parse_svg_style(el).color.unwrap_or(filter_current_color);
        // A primitive may override the filter's color space; an explicit sRGB on
        // any contributing primitive disables linearRGB for the whole recolor
        // (we apply ops as a single composite, so the box uses one space).
        if color_interpolation_filters_is_srgb(el) {
            use_linear_rgb = false;
        }
        if el.raw_tag_name.eq_ignore_ascii_case("feFlood") {
            flood_color = Some(svg_flood_color(el, current_color));
            continue;
        }
        if el.raw_tag_name.eq_ignore_ascii_case("feGaussianBlur") {
            let std_dev = attr_f32(el, "stdDeviation");
            if std_dev > 0.0 {
                operations.push(FilterOperation::Blur(std_dev * 0.75));
            }
            continue;
        }
        if el.raw_tag_name.eq_ignore_ascii_case("feDropShadow") {
            let color = svg_flood_color(el, current_color);
            operations.push(FilterOperation::DropShadow(
                crate::style::computed::DropShadow {
                    dx: attr_f32(el, "dx") * 0.75,
                    dy: attr_f32(el, "dy") * 0.75,
                    blur: attr_f32(el, "stdDeviation") * 0.75,
                    color,
                },
            ));
            continue;
        }
        if el.raw_tag_name.eq_ignore_ascii_case("feOffset") {
            let keep_source = element_children
                .get(idx + 1)
                .is_some_and(|next| next.raw_tag_name.eq_ignore_ascii_case("feMerge"));
            operations.push(FilterOperation::Offset {
                dx: attr_f32(el, "dx") * 0.75,
                dy: attr_f32(el, "dy") * 0.75,
                keep_source,
                region,
            });
            continue;
        }
        if el.raw_tag_name.eq_ignore_ascii_case("feMorphology")
            && el
                .attributes
                .get("operator")
                .is_none_or(|v| v.eq_ignore_ascii_case("dilate"))
        {
            let radius = el
                .attributes
                .get("radius")
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            if radius > 0.0 {
                operations.push(FilterOperation::MorphologyDilate(radius * 0.75));
            }
            continue;
        }
        if el.raw_tag_name.eq_ignore_ascii_case("feBlend")
            && el
                .attributes
                .get("mode")
                .is_some_and(|v| v.eq_ignore_ascii_case("multiply"))
            && let Some(color) = flood_color
        {
            operations.push(FilterOperation::BlendWithFlood {
                color,
                mode: crate::style::computed::BlendMode::Multiply,
                region,
            });
            continue;
        }
        if el.raw_tag_name.eq_ignore_ascii_case("feComposite")
            && el
                .attributes
                .get("operator")
                .is_some_and(|v| v.eq_ignore_ascii_case("in"))
            && let Some(color) = flood_color
        {
            let (r, g, b, a) = color.to_f32_rgba();
            let (r, g, b) = filter_rgb_constants(r, g, b, use_linear_rgb);
            operations.push(FilterOperation::Matrix([
                0.0, 0.0, 0.0, 0.0, r, 0.0, 0.0, 0.0, 0.0, g, 0.0, 0.0, 0.0, 0.0, b, 0.0, 0.0, 0.0,
                a, 0.0,
            ]));
            continue;
        }
        if el.raw_tag_name.eq_ignore_ascii_case("feComponentTransfer") {
            if let Some(matrix) = component_transfer_matrix(el) {
                operations.push(FilterOperation::Matrix(matrix));
            }
            continue;
        }
        if !el.raw_tag_name.eq_ignore_ascii_case("feColorMatrix") {
            continue;
        }
        let kind = el
            .attributes
            .get("type")
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "matrix".to_string());
        let values = el.attributes.get("values").map(|s| s.as_str());
        match kind.as_str() {
            // saturate: a single value (default 1). saturate(0) == grayscale(1).
            "saturate" => {
                let amount = values
                    .and_then(|v| v.split_whitespace().next())
                    .and_then(|t| t.parse::<f32>().ok())
                    .unwrap_or(1.0)
                    .max(0.0);
                operations.push(FilterOperation::Saturate(amount));
            }
            // hueRotate: a single value in degrees (default 0).
            "huerotate" => {
                let deg = values
                    .and_then(|v| v.split_whitespace().next())
                    .and_then(|t| t.parse::<f32>().ok())
                    .unwrap_or(0.0);
                operations.push(FilterOperation::HueRotate(deg));
            }
            // matrix: 20 values. Recognize the common saturate/grayscale form
            // so we can route it through the existing FilterOperation math.
            "matrix" => {
                if let Some(v) = values
                    && let Some(op) = color_matrix_to_op(v)
                {
                    operations.push(op);
                }
            }
            "luminancetoalpha" => {
                operations.push(FilterOperation::Matrix([
                    0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                    0.2125, 0.7154, 0.0721, 0.0, 0.0,
                ]));
            }
            _ => {}
        }
    }
    Some(SvgFilterDefinition {
        operations,
        linear_rgb: use_linear_rgb,
        region,
    })
}

/// True when `color-interpolation-filters` resolves to `sRGB` on this element
/// (presentation attribute or inline `style`). Absent/`auto`/`linearRGB` →
/// `false` (linearRGB is the initial value, SVG filter-effects §13.5).
fn color_interpolation_filters_is_srgb(el: &ElementNode) -> bool {
    if let Some(v) = el.attributes.get("color-interpolation-filters")
        && v.trim().eq_ignore_ascii_case("srgb")
    {
        return true;
    }
    if let Some(style) = el.attributes.get("style") {
        for decl in split_style_declarations(style) {
            if let Some((prop, val)) = decl.split_once(':')
                && prop
                    .trim()
                    .eq_ignore_ascii_case("color-interpolation-filters")
                && val.trim().eq_ignore_ascii_case("srgb")
            {
                return true;
            }
        }
    }
    false
}

/// Decode a 20-value SVG `feColorMatrix type="matrix"` (filter-effects §15.9)
/// into a [`FilterOperation`] when it matches a recognizable luminance-blend form
/// (identity, grayscale, or saturate). Returns `None` for matrices that cannot
/// be expressed as one of the existing ops (e.g. arbitrary channel mixes).
fn color_matrix_to_op(values: &str) -> Option<crate::style::computed::FilterOperation> {
    use crate::style::computed::FilterOperation;
    let m: Vec<f32> = values
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|t| t.parse::<f32>().ok())
        .collect();
    if m.len() != 20 {
        return None;
    }
    let matrix: [f32; 20] = m.clone().try_into().ok()?;
    // The saturate(s) matrix (filter-effects §15.9) has the form:
    //   row R: 0.213+0.787s  0.715-0.715s  0.072-0.072s  0 0
    //   row G: 0.213-0.213s  0.715+0.285s  0.072-0.072s  0 0
    //   row B: 0.213-0.213s  0.715-0.715s  0.072+0.928s  0 0
    // Recover s from m[0] = 0.213 + 0.787 s and verify the rest is consistent.
    let s = (m[0] - 0.213) / 0.787;
    let approx = |a: f32, b: f32| (a - b).abs() < 1e-3;
    let saturate_ok = approx(m[0], 0.213 + 0.787 * s)
        && approx(m[1], 0.715 - 0.715 * s)
        && approx(m[2], 0.072 - 0.072 * s)
        && approx(m[5], 0.213 - 0.213 * s)
        && approx(m[6], 0.715 + 0.285 * s)
        && approx(m[7], 0.072 - 0.072 * s)
        && approx(m[10], 0.213 - 0.213 * s)
        && approx(m[11], 0.715 - 0.715 * s)
        && approx(m[12], 0.072 + 0.928 * s)
        // alpha row is pass-through and offsets are zero
        && approx(m[18], 1.0)
        && approx(m[4], 0.0)
        && approx(m[9], 0.0)
        && approx(m[14], 0.0)
        && approx(m[19], 0.0);
    if saturate_ok {
        return Some(FilterOperation::Saturate(s.max(0.0)));
    }
    Some(FilterOperation::Matrix(matrix))
}

fn svg_flood_color(el: &ElementNode, current_color: Color) -> Color {
    let parse_specified = |raw: &str| {
        crate::parser::css::parse_color(raw).and_then(|value| match value {
            crate::parser::css::CssValue::Color(color) => Some(color),
            _ => None,
        })
    };
    let mut specified = el
        .attributes
        .get("flood-color")
        .and_then(|raw| parse_specified(raw));
    if let Some(color) = style_property_value(el, "flood-color").and_then(parse_specified) {
        specified = Some(color);
    }
    let mut color = specified
        .map(|color| color.resolve(current_color))
        .unwrap_or(Color::BLACK);
    let mut opacity = el
        .attributes
        .get("flood-opacity")
        .and_then(|value| value.trim().parse::<f32>().ok());
    if let Some(style_opacity) =
        style_property_value(el, "flood-opacity").and_then(|value| value.trim().parse::<f32>().ok())
    {
        opacity = Some(style_opacity);
    }
    if let Some(opacity) = opacity {
        color.a *= opacity.clamp(0.0, 1.0);
    }
    color
}

fn filter_rgb_constants(r: f32, g: f32, b: f32, linear_rgb: bool) -> (f32, f32, f32) {
    if linear_rgb {
        (
            svg_srgb_to_linear(r),
            svg_srgb_to_linear(g),
            svg_srgb_to_linear(b),
        )
    } else {
        (r, g, b)
    }
}

fn svg_srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn component_transfer_matrix(el: &ElementNode) -> Option<[f32; 20]> {
    let mut slopes = [1.0_f32; 4];
    let mut intercepts = [0.0_f32; 4];
    for child in &el.children {
        let DomNode::Element(func) = child else {
            continue;
        };
        let channel = match func.raw_tag_name.to_ascii_lowercase().as_str() {
            "fefuncr" => 0,
            "fefuncg" => 1,
            "fefuncb" => 2,
            "fefunca" => 3,
            _ => continue,
        };
        let kind = func
            .attributes
            .get("type")
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "identity".to_string());
        match kind.as_str() {
            "identity" => {}
            "table" => {
                let values: Vec<f32> = func
                    .attributes
                    .get("tableValues")?
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .filter_map(|v| v.parse::<f32>().ok())
                    .collect();
                if values.len() >= 2 {
                    intercepts[channel] = values[0];
                    slopes[channel] = values[values.len() - 1] - values[0];
                }
            }
            "linear" => {
                slopes[channel] = func
                    .attributes
                    .get("slope")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(1.0);
                intercepts[channel] = func
                    .attributes
                    .get("intercept")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.0);
            }
            _ => return None,
        }
    }
    Some([
        slopes[0],
        0.0,
        0.0,
        0.0,
        intercepts[0],
        0.0,
        slopes[1],
        0.0,
        0.0,
        intercepts[1],
        0.0,
        0.0,
        slopes[2],
        0.0,
        intercepts[2],
        0.0,
        0.0,
        0.0,
        slopes[3],
        intercepts[3],
    ])
}

fn svg_translate_from_use(el: &ElementNode) -> Option<SvgTransform> {
    let x = attr_f32(el, "x");
    let y = attr_f32(el, "y");
    if x != 0.0 || y != 0.0 {
        Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, x, y))
    } else {
        None
    }
}

fn svg_style_is_default(style: &SvgStyle) -> bool {
    style.color.is_none()
        && matches!(style.fill, SvgPaint::Unspecified)
        && matches!(style.stroke, SvgPaint::Unspecified)
        && style.clip_path.is_none()
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
    viewport: Option<(f32, f32)>,
) -> Option<SvgTransform> {
    let x = attr_f32(el, "x");
    let y = attr_f32(el, "y");
    let view_box = el.attributes.get("viewBox").and_then(|v| parse_viewbox(v));

    if let Some(vb) = view_box {
        let (width, height) = viewport.unwrap_or_else(|| {
            (
                resolve_svg_viewport_length(el.attributes.get("width"), None, 300.0),
                resolve_svg_viewport_length(el.attributes.get("height"), None, 150.0),
            )
        });
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

fn filter_region_attr(el: &ElementNode, name: &str, default: f32) -> f32 {
    el.attributes.get(name).map_or(default, |raw| {
        let value = raw.trim();
        if let Some(pct) = value.strip_suffix('%') {
            pct.trim()
                .parse::<f32>()
                .map(|v| v / 100.0)
                .unwrap_or(default)
        } else {
            value.parse::<f32>().unwrap_or(default)
        }
    })
}

fn filter_region(el: &ElementNode) -> crate::style::computed::NormalizedFilterRegion {
    crate::style::computed::NormalizedFilterRegion::new(
        filter_region_attr(el, "x", -0.10),
        filter_region_attr(el, "y", -0.10),
        filter_region_attr(el, "width", 1.20),
        filter_region_attr(el, "height", 1.20),
    )
    .unwrap_or_default()
}

fn resolve_text_position(el: &ElementNode, viewport: Option<(f32, f32)>) -> (f32, f32) {
    let x = resolve_svg_text_coordinate(
        el.attributes.get("x").map(String::as_str),
        viewport.map(|(w, _)| w),
    );
    let y = resolve_svg_text_coordinate(
        el.attributes.get("y").map(String::as_str),
        viewport.map(|(_, h)| h),
    );
    (x, y)
}

fn resolve_svg_text_coordinate(value: Option<&str>, viewport: Option<f32>) -> f32 {
    let Some(raw) = value else {
        return 0.0;
    };
    let trimmed = raw.trim();
    if let Some(pct) = trimmed.strip_suffix('%') {
        let Ok(percent) = pct.trim().parse::<f32>() else {
            return 0.0;
        };
        return viewport.unwrap_or(0.0) * percent / 100.0;
    }
    parse_length(trimmed).unwrap_or(0.0)
}

/// Parse a length value (strip px/em/etc suffix, parse number).
pub(crate) fn parse_length(val: &str) -> Option<f32> {
    let trimmed = val.trim();
    let num_str = trimmed.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
    num_str.trim().parse::<f32>().ok()
}

pub(crate) fn parse_absolute_length(val: &str) -> Option<f32> {
    let trimmed = val.trim();
    if trimmed.ends_with('%') {
        return None;
    }
    parse_length(trimmed)
}

/// Parse a viewBox attribute: "min-x min-y width height".
pub(crate) fn parse_viewbox(val: &str) -> Option<ViewBox> {
    let mut parts = val
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok());
    let view_box = ViewBox {
        min_x: parts.next()?,
        min_y: parts.next()?,
        width: parts.next()?,
        height: parts.next()?,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(view_box)
}

/// Parse color, fill, stroke, stroke-width, font, opacity from element attributes.
fn parse_svg_style(el: &ElementNode) -> SvgStyle {
    fn parse_svg_paint(val: &str) -> Option<SvgPaint> {
        let val = val.trim();
        if val.eq_ignore_ascii_case("none") || val.eq_ignore_ascii_case("transparent") {
            return Some(SvgPaint::None);
        }
        if val.eq_ignore_ascii_case("inherit") {
            return Some(SvgPaint::Unspecified);
        }
        if val.eq_ignore_ascii_case("currentColor") {
            return Some(SvgPaint::CurrentColor);
        }
        if let Some(url) = parse_svg_paint_server_reference(val) {
            return Some(SvgPaint::Url(url));
        }
        parse_svg_color(val).map(SvgPaint::Color)
    }

    fn parse_svg_color_property(val: &str) -> Option<Option<Color>> {
        let val = val.trim();
        if val.eq_ignore_ascii_case("inherit") {
            return Some(None);
        }
        parse_svg_color(val).map(Some)
    }

    let mut color = el
        .attributes
        .get("color")
        .and_then(|v| parse_svg_color_property(v))
        .flatten();
    if let Some(val) = style_property_value(el, "color") {
        if let Some(parsed) = parse_svg_color_property(val) {
            color = parsed;
        }
    }
    let mut fill = el
        .attributes
        .get("fill")
        .and_then(|v| parse_svg_paint(v))
        .unwrap_or(SvgPaint::Unspecified);
    if let Some(paint) = style_property_value(el, "fill").and_then(parse_svg_paint) {
        fill = paint;
    }
    let mut stroke = el
        .attributes
        .get("stroke")
        .and_then(|v| parse_svg_paint(v))
        .unwrap_or(SvgPaint::Unspecified);
    if let Some(paint) = style_property_value(el, "stroke").and_then(parse_svg_paint) {
        stroke = paint;
    }
    let mut clip_path = el
        .attributes
        .get("clip-path")
        .and_then(|v| parse_svg_reference_id(v));
    if let Some(path) = style_property_value(el, "clip-path").and_then(parse_svg_reference_id) {
        clip_path = Some(path);
    }
    let mut clip_rule = el
        .attributes
        .get("clip-rule")
        .map(|v| parse_svg_clip_rule(v))
        .unwrap_or_default();
    if let Some(rule) = style_property_value(el, "clip-rule").map(parse_svg_clip_rule) {
        clip_rule = rule;
    }
    let mut stroke_width = el
        .attributes
        .get("stroke-width")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v >= 0.0);
    if let Some(width) = style_property_value(el, "stroke-width")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| *v >= 0.0)
    {
        stroke_width = Some(width);
    }
    let mut opacity = el
        .attributes
        .get("opacity")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    if let Some(val) = style_property_value(el, "opacity") {
        opacity = val.trim().parse().ok().unwrap_or(opacity);
    }
    let (font_family, font_bold, font_italic) = parse_svg_font_attrs(el);
    let font_size = parse_svg_font_size(el);
    let letter_spacing = parse_svg_letter_spacing(el);

    SvgStyle {
        color,
        fill,
        stroke,
        clip_path,
        clip_rule,
        stroke_width,
        font_family,
        font_bold,
        font_italic,
        font_size,
        letter_spacing,
        opacity,
    }
}

enum SvgInheritedProperty<T> {
    Inherit,
    Value(T),
}

impl<T> SvgInheritedProperty<T> {
    fn into_local_value(self) -> Option<T> {
        match self {
            Self::Inherit => None,
            Self::Value(value) => Some(value),
        }
    }
}

fn cascade_inherited_property<T>(
    presentation: Option<SvgInheritedProperty<T>>,
    inline: Option<SvgInheritedProperty<T>>,
) -> Option<T> {
    inline.or(presentation)?.into_local_value()
}

fn parse_svg_font_size(el: &ElementNode) -> Option<SvgFontSize> {
    fn css_wide_keyword(raw: &str) -> Option<SvgInheritedProperty<SvgFontSize>> {
        if raw.eq_ignore_ascii_case("inherit") || raw.eq_ignore_ascii_case("unset") {
            return Some(SvgInheritedProperty::Inherit);
        }
        raw.eq_ignore_ascii_case("initial")
            .then_some(SvgInheritedProperty::Value(SvgFontSize::initial()))
    }

    fn presentation(raw: &str) -> Option<SvgInheritedProperty<SvgFontSize>> {
        let raw = raw.trim();
        if let Some(keyword) = css_wide_keyword(raw) {
            return Some(keyword);
        }
        let value = parse_property_value("font-size", raw)?;
        SvgFontSize::from_css_value(&value, true).map(SvgInheritedProperty::Value)
    }

    fn inline(value: &CssValue) -> Option<SvgInheritedProperty<SvgFontSize>> {
        if let CssValue::Keyword(keyword) = value
            && let Some(keyword) = css_wide_keyword(keyword)
        {
            return Some(keyword);
        }
        SvgFontSize::from_css_value(value, false).map(SvgInheritedProperty::Value)
    }

    let presentation = el
        .attributes
        .get("font-size")
        .and_then(|raw| presentation(raw));
    let inline = el
        .attributes
        .get("style")
        .and_then(|style| parse_inline_style(style).get("font-size").and_then(inline));
    cascade_inherited_property(presentation, inline)
}

fn parse_svg_font_family_declaration(val: &str) -> Option<SvgInheritedProperty<String>> {
    let val = val.trim();
    if val.eq_ignore_ascii_case("inherit") || val.eq_ignore_ascii_case("unset") {
        return Some(SvgInheritedProperty::Inherit);
    }
    if val.eq_ignore_ascii_case("initial") {
        return Some(SvgInheritedProperty::Value(
            SVG_INITIAL_FONT_FAMILY.to_string(),
        ));
    }
    crate::style::font_family::CssFontFamilyList::parse(val)
        .map(|_| SvgInheritedProperty::Value(val.to_string()))
}

fn parse_svg_font_family_css_value(value: &CssValue) -> Option<SvgInheritedProperty<String>> {
    let CssValue::Keyword(value) = value else {
        return None;
    };
    parse_svg_font_family_declaration(value)
}

fn parse_svg_font_weight_value(val: &str) -> Option<bool> {
    let val = val.trim();
    if val.eq_ignore_ascii_case("inherit") {
        return None;
    }
    Some(is_bold_value(val))
}

fn parse_svg_font_style_value(val: &str) -> Option<bool> {
    let val = val.trim();
    if val.eq_ignore_ascii_case("inherit") {
        return None;
    }
    Some(is_italic_value(val))
}

fn parse_svg_letter_spacing(el: &ElementNode) -> Option<SvgLetterSpacing> {
    fn presentation(raw: &str) -> Option<SvgInheritedProperty<SvgLetterSpacing>> {
        let raw = raw.trim();
        if raw.eq_ignore_ascii_case("inherit") || raw.eq_ignore_ascii_case("unset") {
            return Some(SvgInheritedProperty::Inherit);
        }
        if raw.eq_ignore_ascii_case("initial") {
            return Some(SvgInheritedProperty::Value(SvgLetterSpacing::normal()));
        }
        SvgLetterSpacing::parse_presentation_attribute(raw).map(SvgInheritedProperty::Value)
    }

    fn inline(value: &CssValue) -> Option<SvgInheritedProperty<SvgLetterSpacing>> {
        if let CssValue::Keyword(keyword) = value {
            if keyword.eq_ignore_ascii_case("inherit") || keyword.eq_ignore_ascii_case("unset") {
                return Some(SvgInheritedProperty::Inherit);
            }
            if keyword.eq_ignore_ascii_case("initial") {
                return Some(SvgInheritedProperty::Value(SvgLetterSpacing::normal()));
            }
        }
        SvgLetterSpacing::from_css_value(value).map(SvgInheritedProperty::Value)
    }

    let presentation = el
        .attributes
        .get("letter-spacing")
        .and_then(|raw| presentation(raw));
    let inline = el.attributes.get("style").and_then(|style| {
        parse_inline_style(style)
            .get("letter-spacing")
            .and_then(inline)
    });
    cascade_inherited_property(presentation, inline)
}

fn parse_svg_font_attrs(el: &ElementNode) -> (Option<String>, Option<bool>, Option<bool>) {
    let mut bold: Option<bool> = None;
    let mut italic: Option<bool> = None;

    let presentation_family = el
        .attributes
        .get("font-family")
        .and_then(|value| parse_svg_font_family_declaration(value));
    let inline_family = el.attributes.get("style").and_then(|style| {
        parse_inline_style(style)
            .get("font-family")
            .and_then(parse_svg_font_family_css_value)
    });
    let family = cascade_inherited_property(presentation_family, inline_family);
    if let Some(val) = el.attributes.get("font-weight") {
        bold = parse_svg_font_weight_value(val);
    }
    if let Some(val) = el.attributes.get("font-style") {
        italic = parse_svg_font_style_value(val);
    }

    if let Some(val) = style_property_value(el, "font-weight") {
        bold = parse_svg_font_weight_value(val);
    }
    if let Some(val) = style_property_value(el, "font-style") {
        italic = parse_svg_font_style_value(val);
    }

    (family, bold, italic)
}

fn parse_svg_image_href(el: &ElementNode) -> Option<String> {
    el.attributes
        .get("href")
        .or_else(|| el.attributes.get("xlink:href"))
        .map(|href| href.trim())
        .filter(|href| !href.is_empty())
        .map(|href| href.to_string())
}

fn parse_svg_preserve_aspect_ratio(el: &ElementNode) -> SvgPreserveAspectRatio {
    let Some(raw) = el.attributes.get("preserveAspectRatio") else {
        return SvgPreserveAspectRatio::default();
    };
    parse_svg_preserve_aspect_ratio_value(raw).unwrap_or_default()
}

fn parse_svg_preserve_aspect_ratio_value(raw: &str) -> Option<SvgPreserveAspectRatio> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("none") {
        return Some(SvgPreserveAspectRatio::None);
    }

    let mut parts = raw.split_whitespace();
    let align = parse_svg_align(parts.next()?)?;
    let meet_or_slice = match parts.next() {
        Some(value) if value.eq_ignore_ascii_case("slice") => SvgMeetOrSlice::Slice,
        Some(value) if value.eq_ignore_ascii_case("meet") => SvgMeetOrSlice::Meet,
        Some(_) => return None,
        None => SvgMeetOrSlice::Meet,
    };
    if parts.next().is_some() {
        return None;
    }

    Some(SvgPreserveAspectRatio::Align {
        align,
        meet_or_slice,
    })
}

fn parse_svg_align(raw: &str) -> Option<SvgAlign> {
    match raw {
        "xMinYMin" => Some(SvgAlign::TopLeft),
        "xMidYMin" => Some(SvgAlign::TopCenter),
        "xMaxYMin" => Some(SvgAlign::TopRight),
        "xMinYMid" => Some(SvgAlign::CenterLeft),
        "xMidYMid" => Some(SvgAlign::Center),
        "xMaxYMid" => Some(SvgAlign::CenterRight),
        "xMinYMax" => Some(SvgAlign::BottomLeft),
        "xMidYMax" => Some(SvgAlign::BottomCenter),
        "xMaxYMax" => Some(SvgAlign::BottomRight),
        _ => None,
    }
}

fn has_fill_specified(el: &ElementNode) -> bool {
    el.attributes.contains_key("fill") || style_property_value(el, "fill").is_some()
}

fn parse_fill_raw(el: &ElementNode) -> Option<String> {
    if let Some(raw) = style_property_value(el, "fill") {
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
    }
    if let Some(val) = el.attributes.get("fill") {
        return Some(val.trim().to_string());
    }
    None
}

/// Map a CSS font-family value to a PDF base-font family name.
fn is_bold_value(val: &str) -> bool {
    let lower = val.to_ascii_lowercase();
    lower == "bold" || lower == "bolder" || lower.parse::<u32>().is_ok_and(|w| w >= 700)
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

/// Parse an absolute SVG color through the canonical CSS color parser.
///
/// Paint-specific keywords such as `none` and `currentColor` are represented by
/// [`SvgPaint`] before this function is called. Keeping absolute color syntax in
/// one parser prevents SVG and CSS from accepting subtly different color sets.
pub fn parse_svg_color(val: &str) -> Option<Color> {
    match crate::parser::css::parse_color(val)? {
        crate::parser::css::CssValue::Color(crate::parser::css::SpecifiedColor::Absolute(
            color,
        )) => Some(color),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default)]
enum SmoothControl {
    #[default]
    None,
    Cubic(f32, f32),
    Quadratic(f32, f32),
}

impl SmoothControl {
    fn cubic(self, current: (f32, f32)) -> (f32, f32) {
        match self {
            Self::Cubic(x, y) => (2.0 * current.0 - x, 2.0 * current.1 - y),
            _ => current,
        }
    }

    fn quadratic(self, current: (f32, f32)) -> (f32, f32) {
        match self {
            Self::Quadratic(x, y) => (2.0 * current.0 - x, 2.0 * current.1 - y),
            _ => current,
        }
    }
}

/// Parse SVG path `d` attribute data into PathCommands.
/// Supports: M/m, L/l, H/h, V/v, C/c, S/s, Q/q, T/t, Z/z.
pub fn parse_path_data(d: &str) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    let mut cur_x: f32 = 0.0;
    let mut cur_y: f32 = 0.0;
    let mut subpath_start = (0.0, 0.0);
    let mut smooth_control = SmoothControl::None;
    let mut last_cmd: char = ' ';

    let tokens = tokenize_path(d);
    let mut i = 0;

    while i < tokens.len() {
        let token = &tokens[i];

        // Determine if this token is a command letter
        let cmd_char = match token.as_bytes() {
            [b] if b.is_ascii_alphabetic() => {
                i += 1;
                *b as char
            }
            _ => {
                // Implicit repeat of last command (L after M)
                match last_cmd {
                    'M' => 'L',
                    'm' => 'l',
                    c => c,
                }
            }
        };

        // Smooth controls are eligible for exactly one following segment. A
        // curve branch below installs the control that its successor may use;
        // every other command leaves the state cleared.
        let preceding_control = std::mem::take(&mut smooth_control);
        match cmd_char {
            'M' => {
                if let Some((x, y)) = read_pair(&tokens, &mut i) {
                    cur_x = x;
                    cur_y = y;
                    subpath_start = (x, y);
                    commands.push(PathCommand::MoveTo(cur_x, cur_y));
                    last_cmd = 'M';
                }
            }
            'm' => {
                if let Some((dx, dy)) = read_pair(&tokens, &mut i) {
                    cur_x += dx;
                    cur_y += dy;
                    subpath_start = (cur_x, cur_y);
                    commands.push(PathCommand::MoveTo(cur_x, cur_y));
                    last_cmd = 'm';
                }
            }
            'L' => {
                if let Some((x, y)) = read_pair(&tokens, &mut i) {
                    cur_x = x;
                    cur_y = y;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'L';
                }
            }
            'l' => {
                if let Some((dx, dy)) = read_pair(&tokens, &mut i) {
                    cur_x += dx;
                    cur_y += dy;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'l';
                }
            }
            'H' => {
                if let Some(x) = read_number(&tokens, &mut i) {
                    cur_x = x;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'H';
                }
            }
            'h' => {
                if let Some(dx) = read_number(&tokens, &mut i) {
                    cur_x += dx;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'h';
                }
            }
            'V' => {
                if let Some(y) = read_number(&tokens, &mut i) {
                    cur_y = y;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'V';
                }
            }
            'v' => {
                if let Some(dy) = read_number(&tokens, &mut i) {
                    cur_y += dy;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'v';
                }
            }
            'C' => {
                if let Some((x1, y1, x2, y2, x, y)) = read_six(&tokens, &mut i) {
                    commands.push(PathCommand::CubicTo(x1, y1, x2, y2, x, y));
                    smooth_control = SmoothControl::Cubic(x2, y2);
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
                    smooth_control = SmoothControl::Cubic(x2, y2);
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'c';
                }
            }
            'S' => {
                if let Some((x2, y2, x, y)) = read_four(&tokens, &mut i) {
                    let (x1, y1) = preceding_control.cubic((cur_x, cur_y));
                    commands.push(PathCommand::CubicTo(x1, y1, x2, y2, x, y));
                    smooth_control = SmoothControl::Cubic(x2, y2);
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'S';
                }
            }
            's' => {
                if let Some((dx2, dy2, dx, dy)) = read_four(&tokens, &mut i) {
                    let (x1, y1) = preceding_control.cubic((cur_x, cur_y));
                    let x2 = cur_x + dx2;
                    let y2 = cur_y + dy2;
                    let x = cur_x + dx;
                    let y = cur_y + dy;
                    commands.push(PathCommand::CubicTo(x1, y1, x2, y2, x, y));
                    smooth_control = SmoothControl::Cubic(x2, y2);
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 's';
                }
            }
            'Q' => {
                if let Some((x1, y1, x, y)) = read_four(&tokens, &mut i) {
                    commands.push(PathCommand::QuadTo(x1, y1, x, y));
                    smooth_control = SmoothControl::Quadratic(x1, y1);
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
                    smooth_control = SmoothControl::Quadratic(x1, y1);
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'q';
                }
            }
            'T' => {
                if let Some((x, y)) = read_pair(&tokens, &mut i) {
                    let (x1, y1) = preceding_control.quadratic((cur_x, cur_y));
                    commands.push(PathCommand::QuadTo(x1, y1, x, y));
                    smooth_control = SmoothControl::Quadratic(x1, y1);
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'T';
                }
            }
            't' => {
                if let Some((dx, dy)) = read_pair(&tokens, &mut i) {
                    let (x1, y1) = preceding_control.quadratic((cur_x, cur_y));
                    let x = cur_x + dx;
                    let y = cur_y + dy;
                    commands.push(PathCommand::QuadTo(x1, y1, x, y));
                    smooth_control = SmoothControl::Quadratic(x1, y1);
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 't';
                }
            }
            'A' => {
                if let Some((rx, ry, x_axis_rotation, large_arc, sweep, x, y)) =
                    read_arc(&tokens, &mut i)
                {
                    append_arc(
                        &mut commands,
                        ArcEndpoint::new(
                            (cur_x, cur_y),
                            (x, y),
                            (rx, ry),
                            x_axis_rotation,
                            large_arc,
                            sweep,
                        ),
                    );
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'A';
                }
            }
            'a' => {
                if let Some((rx, ry, x_axis_rotation, large_arc, sweep, dx, dy)) =
                    read_arc(&tokens, &mut i)
                {
                    let x = cur_x + dx;
                    let y = cur_y + dy;
                    append_arc(
                        &mut commands,
                        ArcEndpoint::new(
                            (cur_x, cur_y),
                            (x, y),
                            (rx, ry),
                            x_axis_rotation,
                            large_arc,
                            sweep,
                        ),
                    );
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'a';
                }
            }
            'Z' | 'z' => {
                commands.push(PathCommand::ClosePath);
                (cur_x, cur_y) = subpath_start;
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

fn read_flag(tokens: &[String], i: &mut usize) -> Option<bool> {
    let value = read_number(tokens, i)?;
    Some(value != 0.0)
}

fn read_arc(tokens: &[String], i: &mut usize) -> Option<(f32, f32, f32, bool, bool, f32, f32)> {
    let rx = read_number(tokens, i)?;
    let ry = read_number(tokens, i)?;
    let x_axis_rotation = read_number(tokens, i)?;
    let large_arc = read_flag(tokens, i)?;
    let sweep = read_flag(tokens, i)?;
    let x = read_number(tokens, i)?;
    let y = read_number(tokens, i)?;
    Some((rx, ry, x_axis_rotation, large_arc, sweep, x, y))
}

#[derive(Debug, Clone, Copy)]
struct ArcPoint {
    x: f64,
    y: f64,
}

impl ArcPoint {
    fn from_f32((x, y): (f32, f32)) -> Self {
        Self {
            x: f64::from(x),
            y: f64::from(y),
        }
    }

    fn as_f32(self) -> (f32, f32) {
        (self.x as f32, self.y as f32)
    }
}

#[derive(Debug, Clone, Copy)]
struct ArcEndpoint {
    start: ArcPoint,
    end: ArcPoint,
    radii: ArcPoint,
    x_axis_rotation: f64,
    large_arc: bool,
    sweep: bool,
}

impl ArcEndpoint {
    fn new(
        start: (f32, f32),
        end: (f32, f32),
        radii: (f32, f32),
        x_axis_rotation: f32,
        large_arc: bool,
        sweep: bool,
    ) -> Self {
        Self {
            start: ArcPoint::from_f32(start),
            end: ArcPoint::from_f32(end),
            radii: ArcPoint::from_f32(radii),
            x_axis_rotation: f64::from(x_axis_rotation),
            large_arc,
            sweep,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ArcTransform {
    cos_phi: f64,
    sin_phi: f64,
    radii: ArcPoint,
    center: ArcPoint,
}

#[derive(Debug, Clone, Copy)]
struct CubicArcSegment {
    control_1: ArcPoint,
    control_2: ArcPoint,
    end: ArcPoint,
}

enum ConvertedArc {
    Omit,
    Line,
    Cubics(Box<[CubicArcSegment]>),
}

fn append_arc(commands: &mut Vec<PathCommand>, arc: ArcEndpoint) {
    let end = arc.end.as_f32();
    match convert_arc(arc) {
        ConvertedArc::Omit => {}
        ConvertedArc::Line => {
            commands.push(PathCommand::LineTo(end.0, end.1));
        }
        ConvertedArc::Cubics(segments) => {
            for segment in &segments {
                let control_1 = segment.control_1.as_f32();
                let control_2 = segment.control_2.as_f32();
                let segment_end = segment.end.as_f32();
                commands.push(PathCommand::CubicTo(
                    control_1.0,
                    control_1.1,
                    control_2.0,
                    control_2.1,
                    segment_end.0,
                    segment_end.1,
                ));
            }
        }
    }
}

fn convert_arc(arc: ArcEndpoint) -> ConvertedArc {
    if arc.start.x == arc.end.x && arc.start.y == arc.end.y {
        return ConvertedArc::Omit;
    }

    let mut radii = ArcPoint {
        x: arc.radii.x.abs(),
        y: arc.radii.y.abs(),
    };
    if radii.x == 0.0 || radii.y == 0.0 {
        return ConvertedArc::Line;
    }

    let phi = arc
        .x_axis_rotation
        .to_radians()
        .rem_euclid(std::f64::consts::TAU);
    let cos_phi = phi.cos();
    let sin_phi = phi.sin();
    let dx2 = (arc.start.x - arc.end.x) * 0.5;
    let dy2 = (arc.start.y - arc.end.y) * 0.5;
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    let lambda = (x1p * x1p) / (radii.x * radii.x) + (y1p * y1p) / (radii.y * radii.y);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        radii.x *= scale;
        radii.y *= scale;
    }

    let rx_sq = radii.x * radii.x;
    let ry_sq = radii.y * radii.y;
    let x1p_sq = x1p * x1p;
    let y1p_sq = y1p * y1p;

    let numerator = (rx_sq * ry_sq) - (rx_sq * y1p_sq) - (ry_sq * x1p_sq);
    let denominator = (rx_sq * y1p_sq) + (ry_sq * x1p_sq);
    if denominator == 0.0 {
        return ConvertedArc::Line;
    }
    let sign = if arc.large_arc == arc.sweep {
        -1.0
    } else {
        1.0
    };
    let coeff = sign * (numerator / denominator).max(0.0).sqrt();
    let cxp = coeff * ((radii.x * y1p) / radii.y);
    let cyp = coeff * (-(radii.y * x1p) / radii.x);

    let center = ArcPoint {
        x: cos_phi * cxp - sin_phi * cyp + (arc.start.x + arc.end.x) * 0.5,
        y: sin_phi * cxp + cos_phi * cyp + (arc.start.y + arc.end.y) * 0.5,
    };
    let transform = ArcTransform {
        cos_phi,
        sin_phi,
        radii,
        center,
    };

    let theta1 = unit_vector_angle((1.0, 0.0), ((x1p - cxp) / radii.x, (y1p - cyp) / radii.y));
    let mut delta_theta = unit_vector_angle(
        ((x1p - cxp) / radii.x, (y1p - cyp) / radii.y),
        ((-x1p - cxp) / radii.x, (-y1p - cyp) / radii.y),
    );
    if !arc.sweep && delta_theta > 0.0 {
        delta_theta -= std::f64::consts::TAU;
    } else if arc.sweep && delta_theta < 0.0 {
        delta_theta += std::f64::consts::TAU;
    }

    let segments = (delta_theta.abs() / std::f64::consts::FRAC_PI_2)
        .ceil()
        .max(1.0) as usize;
    let step = delta_theta / segments as f64;
    let mut curves = Vec::with_capacity(segments);

    for segment_idx in 0..segments {
        let start_theta = theta1 + segment_idx as f64 * step;
        let end_theta = start_theta + step;
        let alpha = (4.0 / 3.0) * ((end_theta - start_theta) * 0.25).tan();

        let (sin_start, cos_start) = start_theta.sin_cos();
        let (sin_end, cos_end) = end_theta.sin_cos();

        let c1 = map_arc_point(
            transform,
            ArcPoint {
                x: cos_start - alpha * sin_start,
                y: sin_start + alpha * cos_start,
            },
        );
        let c2 = map_arc_point(
            transform,
            ArcPoint {
                x: cos_end + alpha * sin_end,
                y: sin_end - alpha * cos_end,
            },
        );
        let end = if segment_idx + 1 == segments {
            arc.end
        } else {
            map_arc_point(
                transform,
                ArcPoint {
                    x: cos_end,
                    y: sin_end,
                },
            )
        };
        curves.push(CubicArcSegment {
            control_1: c1,
            control_2: c2,
            end,
        });
    }

    ConvertedArc::Cubics(curves.into_boxed_slice())
}

fn map_arc_point(transform: ArcTransform, unit_point: ArcPoint) -> ArcPoint {
    ArcPoint {
        x: transform.center.x + transform.cos_phi * transform.radii.x * unit_point.x
            - transform.sin_phi * transform.radii.y * unit_point.y,
        y: transform.center.y
            + transform.sin_phi * transform.radii.x * unit_point.x
            + transform.cos_phi * transform.radii.y * unit_point.y,
    }
}

fn unit_vector_angle(u: (f64, f64), v: (f64, f64)) -> f64 {
    let dot = u.0 * v.0 + u.1 * v.1;
    let cross = u.0 * v.1 - u.1 * v.0;
    cross.atan2(dot)
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

/// Parse the transform attribute and convert it to a single composed matrix.
/// Supports transform lists containing: translate, scale, rotate, matrix.
pub fn parse_transform(val: &str) -> Option<SvgTransform> {
    let mut rest = val.trim();
    let mut combined = None;

    while !rest.is_empty() {
        let (name, args, next) = extract_next_transform_call(rest)?;
        let current = parse_single_transform(name, args)?;
        combined = compose_transform(combined, Some(current));
        rest = next;
    }

    combined
}

fn parse_single_transform(name: &str, args: &str) -> Option<SvgTransform> {
    let nums = parse_num_list(args);

    if name.eq_ignore_ascii_case("matrix") {
        let [a, b, c, d, e, f] = nums.as_slice() else {
            return None;
        };
        return Some(SvgTransform::Matrix(*a, *b, *c, *d, *e, *f));
    }

    if name.eq_ignore_ascii_case("translate") {
        let tx = nums.first().copied().unwrap_or(0.0);
        let ty = nums.get(1).copied().unwrap_or(0.0);
        return Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, tx, ty));
    }

    if name.eq_ignore_ascii_case("scale") {
        let sx = nums.first().copied().unwrap_or(1.0);
        let sy = nums.get(1).copied().unwrap_or(sx);
        return Some(SvgTransform::Matrix(sx, 0.0, 0.0, sy, 0.0, 0.0));
    }

    if name.eq_ignore_ascii_case("rotate") {
        let angle_deg = nums.first().copied().unwrap_or(0.0);
        let angle = angle_deg.to_radians();
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        if let [_, cx, cy, ..] = nums.as_slice() {
            let tx = cx - cos_a * cx + sin_a * cy;
            let ty = cy - sin_a * cx - cos_a * cy;
            return Some(SvgTransform::Matrix(cos_a, sin_a, -sin_a, cos_a, tx, ty));
        }

        return Some(SvgTransform::Matrix(cos_a, sin_a, -sin_a, cos_a, 0.0, 0.0));
    }

    None
}

fn extract_next_transform_call(input: &str) -> Option<(&str, &str, &str)> {
    let trimmed = input.trim_start();
    let open = trimmed.find('(')?;
    let name = trimmed.get(..open)?.trim();
    if name.is_empty() {
        return None;
    }

    let mut depth = 0usize;
    let mut close = None;
    for (idx, ch) in trimmed.char_indices().skip(open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    close = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }

    let close = close?;
    let args = trimmed.get(open + 1..close)?;
    let rest = trimmed
        .get(close + 1..)?
        .trim_start_matches(|ch: char| ch.is_ascii_whitespace() || ch == ',');
    Some((name, args, rest))
}

/// Parse a comma/space-separated list of numbers.
fn parse_num_list(s: &str) -> Vec<f32> {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
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
        assert_eq!(color, Some(Color::rgb(255, 0, 0)));
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
        assert_eq!(style.fill, SvgPaint::Color(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn parse_svg_style_style_fill_none_overrides_attribute() {
        let el = make_el("rect", vec![("fill", "red"), ("style", "fill: none;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::None);
    }

    #[test]
    fn parse_svg_style_style_fill_inherit_overrides_attribute() {
        let el = make_el("rect", vec![("fill", "red"), ("style", "fill: inherit;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.fill, SvgPaint::Unspecified);
    }

    #[test]
    fn parse_svg_style_unparseable_style_stroke_does_not_override_attribute() {
        let el = make_el(
            "rect",
            vec![("stroke", "blue"), ("style", "stroke: not-a-color;")],
        );
        let style = parse_svg_style(&el);
        assert_eq!(style.stroke, SvgPaint::Color(Color::rgb(0, 0, 255)));
    }

    #[test]
    fn parse_svg_style_style_stroke_inherit_overrides_attribute() {
        let el = make_el(
            "rect",
            vec![("stroke", "blue"), ("style", "stroke: inherit;")],
        );
        let style = parse_svg_style(&el);
        assert_eq!(style.stroke, SvgPaint::Unspecified);
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

    // ── filter: url(#id) -> feColorMatrix -> FilterOperation ─────────────

    fn filter_with(child: ElementNode) -> ElementNode {
        let mut f = make_el("filter", vec![("id", "f")]);
        f.children = vec![DomNode::Element(child)];
        f
    }

    fn parsed_filter(filter: &ElementNode) -> SvgFilterDefinition {
        filter_element_definition(filter).expect("the test node is an SVG filter")
    }

    #[test]
    fn fecolormatrix_saturate_maps_to_op_and_defaults_to_linear_rgb() {
        use crate::style::computed::FilterOperation;
        let f = filter_with(make_el(
            "feColorMatrix",
            vec![("type", "saturate"), ("values", "0")],
        ));
        let definition = parsed_filter(&f);
        assert_eq!(definition.operations, vec![FilterOperation::Saturate(0.0)]);
        // SVG <filter> default color space is linearRGB.
        assert!(definition.linear_rgb);
    }

    #[test]
    fn fecolormatrix_srgb_color_space_override() {
        let mut prim = make_el("feColorMatrix", vec![("type", "saturate"), ("values", "0")]);
        prim.attributes
            .insert("color-interpolation-filters".into(), "sRGB".into());
        assert!(!parsed_filter(&filter_with(prim)).linear_rgb);
    }

    #[test]
    fn filter_flood_current_color_resolves_from_filter_foreground() {
        use crate::style::computed::FilterOperation;
        let mut filter = filter_with(make_el(
            "feDropShadow",
            vec![("flood-color", "currentColor"), ("flood-opacity", "0.5")],
        ));
        filter.attributes.insert("color".into(), "red".into());

        let definition = parsed_filter(&filter);
        assert!(matches!(
            definition.operations.as_slice(),
            [FilterOperation::DropShadow(shadow)]
                if shadow.color == crate::types::Color::from_srgb(1.0, 0.0, 0.0, 0.5)
        ));
    }

    #[test]
    fn svg_flood_keeps_fractional_rgb_precision_until_the_filter_backend() {
        let flood = make_el("feFlood", vec![("flood-color", "rgb(80% 20% 10%)")]);

        let color = svg_flood_color(&flood, Color::BLACK);

        let (r, g, b, a) = color.to_f32_rgba();
        assert!((r - 0.8).abs() < 0.000_001);
        assert!((g - 0.2).abs() < 0.000_001);
        assert!((b - 0.1).abs() < 0.000_001);
        assert_eq!(a, 1.0);
    }

    #[test]
    fn feblend_retains_its_flood_input_and_filter_region() {
        use crate::style::computed::{BlendMode, FilterOperation, NormalizedFilterRegion};
        use crate::types::Color;

        let mut filter = make_el(
            "filter",
            vec![
                ("id", "f"),
                ("x", "-20%"),
                ("y", "-30%"),
                ("width", "140%"),
                ("height", "160%"),
            ],
        );
        filter.children = vec![
            DomNode::Element(make_el("feFlood", vec![("flood-color", "#1565c0")])),
            DomNode::Element(make_el(
                "feBlend",
                vec![
                    ("in", "SourceGraphic"),
                    ("in2", "blue"),
                    ("mode", "multiply"),
                ],
            )),
        ];

        let definition = parsed_filter(&filter);
        let expected_region = NormalizedFilterRegion::new(-0.2, -0.3, 1.4, 1.6)
            .expect("the expected region is valid");

        assert!(definition.linear_rgb);
        assert_eq!(definition.region, expected_region);
        assert_eq!(
            definition.operations,
            vec![FilterOperation::BlendWithFlood {
                color: Color::rgb(21, 101, 192),
                mode: BlendMode::Multiply,
                region: expected_region,
            }]
        );
    }

    #[test]
    fn gaussian_blur_retains_the_filter_hard_clip() {
        use crate::style::computed::{FilterOperation, NormalizedFilterRegion};

        let mut filter = make_el(
            "filter",
            vec![
                ("x", "-30%"),
                ("y", "-40%"),
                ("width", "160%"),
                ("height", "180%"),
            ],
        );
        filter.children = vec![DomNode::Element(make_el(
            "feGaussianBlur",
            vec![("stdDeviation", "6")],
        ))];

        let definition = parsed_filter(&filter);

        assert_eq!(definition.operations, vec![FilterOperation::Blur(4.5)]);
        assert_eq!(
            definition.region,
            NormalizedFilterRegion::new(-0.3, -0.4, 1.6, 1.8)
                .expect("the expected normalized region is valid")
        );
    }

    #[test]
    fn fecolormatrix_matrix_form_decodes_to_saturate() {
        use crate::style::computed::FilterOperation;
        // The 20-value saturate(0) matrix == grayscale luminance blend.
        let m = "0.213 0.715 0.072 0 0 \
                 0.213 0.715 0.072 0 0 \
                 0.213 0.715 0.072 0 0 \
                 0 0 0 1 0";
        let f = filter_with(make_el(
            "feColorMatrix",
            vec![("type", "matrix"), ("values", m)],
        ));
        let definition = parsed_filter(&f);
        assert!(matches!(
            definition.operations.as_slice(),
            [FilterOperation::Saturate(s)] if s.abs() < 1e-3
        ));
    }

    #[test]
    fn non_filter_element_yields_no_ops() {
        assert!(filter_element_definition(&make_el("g", vec![])).is_none());
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
        assert_eq!(c, Color::rgb(255, 0, 0));
    }

    #[test]
    fn parse_svg_color_hex_3_char_white() {
        let c = parse_svg_color("#fff").unwrap();
        assert_eq!(c, Color::WHITE);
    }

    #[test]
    fn parse_svg_color_hex_4_char_rgba() {
        assert_eq!(
            parse_svg_color("#abcd"),
            Some(Color::rgba8(0xaa, 0xbb, 0xcc, 0xdd))
        );
    }

    #[test]
    fn parse_svg_color_rgb_func() {
        let c = parse_svg_color("rgb(255, 0, 128)").unwrap();
        assert_eq!(c, Color::rgb(255, 0, 128));
    }

    #[test]
    fn parse_svg_color_rgb_func_with_spaces() {
        let c = parse_svg_color("rgb( 0 , 128 , 255 )").unwrap();
        assert_eq!(c, Color::rgb(0, 128, 255));
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
    fn parse_svg_color_none_case_insensitive() {
        assert_eq!(parse_svg_color("None"), None);
        assert_eq!(parse_svg_color("NONE"), None);
    }

    #[test]
    fn parse_svg_color_with_leading_trailing_spaces() {
        assert_eq!(parse_svg_color("  red  "), Some(Color::rgb(255, 0, 0)));
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

    #[derive(Clone, Copy)]
    struct SmoothPredecessor {
        command: &'static str,
        suffix: &'static str,
        current: (f32, f32),
        cubic_control: Option<(f32, f32)>,
        quadratic_control: Option<(f32, f32)>,
    }

    impl SmoothPredecessor {
        const fn none(command: &'static str, suffix: &'static str, current: (f32, f32)) -> Self {
            Self {
                command,
                suffix,
                current,
                cubic_control: None,
                quadratic_control: None,
            }
        }

        const fn cubic(command: &'static str, suffix: &'static str, control: (f32, f32)) -> Self {
            Self {
                command,
                suffix,
                current: (20.0, 20.0),
                cubic_control: Some(control),
                quadratic_control: None,
            }
        }

        const fn quadratic(
            command: &'static str,
            suffix: &'static str,
            control: (f32, f32),
        ) -> Self {
            Self {
                command,
                suffix,
                current: (20.0, 20.0),
                cubic_control: None,
                quadratic_control: Some(control),
            }
        }
    }

    fn smooth_predecessors() -> [SmoothPredecessor; 20] {
        use SmoothPredecessor as P;

        [
            P::none("M", "M 20 20", (20.0, 20.0)),
            P::none("m", "m 17 17", (20.0, 20.0)),
            P::none("L", "L 20 20", (20.0, 20.0)),
            P::none("l", "l 17 17", (20.0, 20.0)),
            P::none("H", "H 20", (20.0, 3.0)),
            P::none("h", "h 17", (20.0, 3.0)),
            P::none("V", "V 20", (3.0, 20.0)),
            P::none("v", "v 17", (3.0, 20.0)),
            P::cubic("C", "C 10 10 18 19 20 20", (18.0, 19.0)),
            P::cubic("c", "c 7 7 15 16 17 17", (18.0, 19.0)),
            P::cubic("S", "S 18 19 20 20", (18.0, 19.0)),
            P::cubic("s", "s 15 16 17 17", (18.0, 19.0)),
            P::quadratic("Q", "Q 18 19 20 20", (18.0, 19.0)),
            P::quadratic("q", "q 15 16 17 17", (18.0, 19.0)),
            P::quadratic("T", "T 20 20", (5.0, 5.0)),
            P::quadratic("t", "t 17 17", (5.0, 5.0)),
            P::none("A", "A 10 10 0 0 1 20 20", (20.0, 20.0)),
            P::none("a", "a 10 10 0 0 1 17 17", (20.0, 20.0)),
            P::none("Z", "Z", (0.0, 0.0)),
            P::none("z", "z", (0.0, 0.0)),
        ]
    }

    fn reflected(current: (f32, f32), control: Option<(f32, f32)>) -> (f32, f32) {
        control.map_or(current, |control| {
            (2.0 * current.0 - control.0, 2.0 * current.1 - control.1)
        })
    }

    #[test]
    fn smooth_cubic_reflects_only_cubic_predecessors() {
        const CUBIC_SEED: &str = "M 0 0 C 1 1 2 2 3 3";

        for predecessor in smooth_predecessors() {
            let expected = reflected(predecessor.current, predecessor.cubic_control);
            for target in ["S 30 30 40 40", "s 10 10 20 20"] {
                let path = format!("{CUBIC_SEED} {} {target}", predecessor.suffix);
                let Some(PathCommand::CubicTo(x, y, ..)) = parse_path_data(&path).last().cloned()
                else {
                    panic!(
                        "{target} after {} did not produce a cubic",
                        predecessor.command
                    );
                };
                assert_eq!((x, y), expected, "{target} after {}", predecessor.command);
            }
        }
    }

    #[test]
    fn smooth_quadratic_reflects_only_quadratic_predecessors() {
        const QUADRATIC_SEED: &str = "M 0 0 Q 1 1 3 3";

        for predecessor in smooth_predecessors() {
            let expected = reflected(predecessor.current, predecessor.quadratic_control);
            for target in ["T 30 30", "t 10 10"] {
                let path = format!("{QUADRATIC_SEED} {} {target}", predecessor.suffix);
                let Some(PathCommand::QuadTo(x, y, ..)) = parse_path_data(&path).last().cloned()
                else {
                    panic!(
                        "{target} after {} did not produce a quadratic",
                        predecessor.command
                    );
                };
                assert_eq!((x, y), expected, "{target} after {}", predecessor.command);
            }
        }
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
    fn parse_path_absolute_arc_converts_to_cubic_segments() {
        let cmds = parse_path_data("M 0 0 A 10 10 0 0 1 10 10");
        assert!(matches!(cmds.first(), Some(PathCommand::MoveTo(0.0, 0.0))));
        assert!(
            cmds.iter()
                .any(|cmd| matches!(cmd, PathCommand::CubicTo(..))),
            "Expected arc to become cubic segment(s)"
        );
    }

    #[test]
    fn parse_path_relative_arc_compact_syntax() {
        let cmds = parse_path_data("M0 0a2 2 0 1 0-3 2l5 0");
        assert!(matches!(cmds.first(), Some(PathCommand::MoveTo(0.0, 0.0))));
        assert!(
            cmds.iter()
                .any(|cmd| matches!(cmd, PathCommand::CubicTo(..))),
            "Expected relative arc to become cubic segment(s)"
        );
        assert!(matches!(cmds.last(), Some(PathCommand::LineTo(2.0, 2.0))));
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
    fn parse_transform_list_composes_in_order() {
        let t = parse_transform("translate(0, 300) scale(0.1, -0.1)").unwrap();
        match t {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                assert!((a - 0.1).abs() < 0.001);
                assert!((b - 0.0).abs() < 0.001);
                assert!((c - 0.0).abs() < 0.001);
                assert!((d + 0.1).abs() < 0.001);
                assert!((e - 0.0).abs() < 0.001);
                assert!((f - 300.0).abs() < 0.001);
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
    fn parse_node_path_preserves_element_transform() {
        let el = make_el(
            "path",
            vec![
                ("d", "M 0 7.5 L 84.5 7.5"),
                ("transform", "matrix(-0.85 0 0 -0.85 337.5 50.3)"),
            ],
        );

        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Group {
                transform: Some(SvgTransform::Matrix(a, b, c, d, e, f)),
                children,
                ..
            } => {
                assert_eq!((a, b, c, d, e, f), (-0.85, 0.0, 0.0, -0.85, 337.5, 50.3));
                assert!(matches!(children.as_slice(), [SvgNode::Path { .. }]));
            }
            other => panic!("expected transformed path group, got {other:?}"),
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
        assert_eq!(style.color, None);
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
        assert_eq!(style.fill, SvgPaint::Color(Color::rgb(255, 0, 0)));
        assert_eq!(style.stroke, SvgPaint::Color(Color::rgb(0, 0, 255)));
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
    fn parse_style_color_inherited_property() {
        let el = make_el("g", vec![("style", "color: red;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.color, Some(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn parse_style_invalid_color_does_not_override_attribute() {
        let el = make_el("g", vec![("color", "red"), ("style", "color: ???;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.color, Some(Color::rgb(255, 0, 0)));
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
        assert_eq!(style.fill, SvgPaint::Color(Color::rgb(0, 255, 0)));
        assert_eq!(style.stroke, SvgPaint::Color(Color::rgb(0, 0, 255)));
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
    fn parse_svg_from_element_collects_defs_and_use_references() {
        let stop0 = make_el("stop", vec![("offset", "0"), ("stop-color", "#000000")]);
        let stop1 = make_el("stop", vec![("offset", "1"), ("stop-color", "#ffffff")]);
        let mut gradient = make_el(
            "linearGradient",
            vec![
                ("id", "grad"),
                ("x1", "0"),
                ("y1", "0"),
                ("x2", "10"),
                ("y2", "0"),
                ("gradientUnits", "userSpaceOnUse"),
            ],
        );
        gradient.children = vec![DomNode::Element(stop0), DomNode::Element(stop1)];

        let mut clip_path = make_el("clipPath", vec![("id", "clip")]);
        clip_path.children = vec![DomNode::Element(make_el(
            "path",
            vec![("d", "M0 0 L10 0 L10 10 Z")],
        ))];

        let shape = make_el("path", vec![("id", "shape"), ("d", "M0 0 L10 0")]);
        let mut defs = make_el("defs", vec![]);
        defs.children = vec![
            DomNode::Element(shape),
            DomNode::Element(gradient),
            DomNode::Element(clip_path),
        ];

        let use_el = make_el("use", vec![("href", "#shape"), ("x", "5"), ("y", "7")]);
        let rect = make_el(
            "rect",
            vec![
                ("width", "12"),
                ("height", "8"),
                ("fill", "url(#grad)"),
                ("clip-path", "#clip"),
            ],
        );

        let svg = make_svg_el(
            vec![
                ("width", "200"),
                ("height", "100"),
                ("viewBox", "0 0 200 100"),
            ],
            vec![defs, use_el, rect],
        );
        let tree = parse_svg_from_element(&svg).unwrap();

        assert!(tree.defs.gradients.contains_key("grad"));
        assert!(tree.defs.clip_paths.contains_key("clip"));
        assert_eq!(tree.children.len(), 2);

        match &tree.children[0] {
            SvgNode::Group {
                transform,
                children,
                ..
            } => {
                assert!(matches!(
                    transform,
                    Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 5.0, 7.0))
                ));
                assert_eq!(children.len(), 1);
                assert!(matches!(children[0], SvgNode::Path { .. }));
            }
            other => panic!("expected translated <use> group, got {other:?}"),
        }

        match &tree.children[1] {
            SvgNode::Rect { style, .. } => {
                assert!(matches!(style.fill, SvgPaint::Url(ref id) if id == "grad"));
                assert_eq!(style.clip_path.as_deref(), Some("clip"));
            }
            other => panic!("expected rect, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_group_transform_list_is_composed() {
        let path = make_el("path", vec![("d", "M0 0 L10 0")]);
        let mut group = make_el(
            "g",
            vec![("transform", "translate(0, 300) scale(0.1, -0.1)")],
        );
        group.children.push(DomNode::Element(path));

        let svg = make_svg_el(vec![("width", "10"), ("height", "10")], vec![group]);
        let tree = parse_svg_from_element(&svg).unwrap();

        match &tree.children[0] {
            SvgNode::Group {
                transform: Some(SvgTransform::Matrix(a, b, c, d, e, f)),
                ..
            } => {
                assert!((*a - 0.1).abs() < 0.001);
                assert!((*b - 0.0).abs() < 0.001);
                assert!((*c - 0.0).abs() < 0.001);
                assert!((*d + 0.1).abs() < 0.001);
                assert!((*e - 0.0).abs() < 0.001);
                assert!((*f - 300.0).abs() < 0.001);
            }
            other => panic!("expected transformed group, got {other:?}"),
        }
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
                assert_eq!(style.fill, SvgPaint::Color(Color::rgb(255, 0, 0)));
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
            SvgNode::Text { style, .. } => {
                let font_size = style.font_size.expect("parsed font-size");
                assert_eq!(font_size.resolve_user_units(12.0), 20.0);
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
    fn parse_text_percentage_coordinates_use_viewport() {
        let text = make_el("text", vec![("x", "50%"), ("y", "25%")]);
        let svg = make_svg_el(vec![("width", "200"), ("height", "100")], vec![text]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { x, y, .. } => {
                assert_eq!(*x, 100.0);
                assert_eq!(*y, 25.0);
            }
            other => panic!("expected text node, got {other:?}"),
        }
    }

    #[test]
    fn parse_image_percentage_coordinates_use_viewport_and_parse_href() {
        let image = make_el(
            "image",
            vec![
                ("x", "10%"),
                ("y", "20%"),
                ("width", "50%"),
                ("height", "25%"),
                ("preserveAspectRatio", "none"),
                ("xlink:href", "assets/qr.png"),
            ],
        );
        let svg = make_svg_el(vec![("width", "200"), ("height", "100")], vec![image]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Image {
                x,
                y,
                width,
                height,
                href,
                preserve_aspect_ratio,
                ..
            } => {
                assert_eq!((*x, *y, *width, *height), (20.0, 20.0, 100.0, 25.0));
                assert_eq!(href, "assets/qr.png");
                assert!(matches!(
                    preserve_aspect_ratio,
                    SvgPreserveAspectRatio::None
                ));
            }
            other => panic!("expected image node, got {other:?}"),
        }
    }

    #[test]
    fn parse_image_negative_coordinates_are_preserved() {
        let image = make_el(
            "image",
            vec![
                ("x", "-10"),
                ("y", "-20"),
                ("width", "50"),
                ("height", "25"),
                ("href", "assets/qr.png"),
            ],
        );
        let svg = make_svg_el(vec![("width", "200"), ("height", "100")], vec![image]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Image {
                x,
                y,
                width,
                height,
                ..
            } => assert_eq!((*x, *y, *width, *height), (-10.0, -20.0, 50.0, 25.0)),
            other => panic!("expected image node, got {other:?}"),
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

    // ── transform parsing helpers / parse_num_list ─────────────────────

    #[test]
    fn extract_next_transform_call_basic() {
        assert_eq!(
            extract_next_transform_call("translate(10, 20)"),
            Some(("translate", "10, 20", ""))
        );
    }

    #[test]
    fn extract_next_transform_call_no_parens() {
        assert_eq!(extract_next_transform_call("translate"), None);
    }

    #[test]
    fn extract_next_transform_call_preserves_remaining_list() {
        assert_eq!(
            extract_next_transform_call("translate(10, 20) rotate(30)"),
            Some(("translate", "10, 20", "rotate(30)"))
        );
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

    #[test]
    fn parse_svg_with_viewport_override_resolves_nested_percentages() {
        let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut svg_inner = make_el(
            "svg",
            vec![
                ("width", "50%"),
                ("height", "50%"),
                ("viewBox", "0 0 20 10"),
            ],
        );
        svg_inner.children.push(DomNode::Element(inner));
        let outer = make_svg_el(vec![("width", "100%"), ("height", "100%")], vec![svg_inner]);
        let tree = parse_svg_from_element_with_ctx_and_viewport(
            &outer,
            SvgTextContext::default(),
            Some((400.0, 200.0)),
        )
        .unwrap();
        match &tree.children[0] {
            SvgNode::Group { transform, .. } => {
                assert!(matches!(
                    transform,
                    Some(SvgTransform::Matrix(10.0, 0.0, 0.0, 10.0, 0.0, 0.0))
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

    // ── SVG <image> element ────────────────────────────────────────────

    #[test]
    fn parse_image_no_href_returns_none() {
        // No href or xlink:href — parse_svg_image_href returns None, node skipped
        let el = make_el(
            "image",
            vec![("x", "0"), ("y", "0"), ("width", "10"), ("height", "10")],
        );
        assert!(parse_svg_node(&el).is_none());
    }

    #[test]
    fn parse_image_empty_href_returns_none() {
        let el = make_el(
            "image",
            vec![("href", "  "), ("width", "50"), ("height", "50")],
        );
        assert!(parse_svg_node(&el).is_none());
    }

    #[test]
    fn parse_image_data_uri_href() {
        let el = make_el(
            "image",
            vec![
                ("href", "data:image/png;base64,ABC=="),
                ("x", "5"),
                ("y", "5"),
                ("width", "80"),
                ("height", "40"),
            ],
        );
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Image {
                href,
                x,
                y,
                width,
                height,
                ..
            } => {
                assert_eq!(href, "data:image/png;base64,ABC==");
                assert_eq!((x, y, width, height), (5.0, 5.0, 80.0, 40.0));
            }
            _ => panic!("expected Image node"),
        }
    }

    #[test]
    fn parse_image_negative_dimensions_clamp_to_zero() {
        // width/height are clamped to 0 via .max(0.0)
        let el = make_el(
            "image",
            vec![("href", "test.png"), ("width", "-5"), ("height", "-10")],
        );
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Image { width, height, .. } => {
                assert_eq!(width, 0.0);
                assert_eq!(height, 0.0);
            }
            _ => panic!("expected Image node"),
        }
    }

    #[test]
    fn parse_image_preserve_aspect_ratio_xmidymid_meet() {
        let el = make_el(
            "image",
            vec![
                ("href", "test.png"),
                ("width", "100"),
                ("height", "50"),
                ("preserveAspectRatio", "xMidYMid meet"),
            ],
        );
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Image {
                preserve_aspect_ratio,
                ..
            } => {
                assert_eq!(
                    preserve_aspect_ratio,
                    SvgPreserveAspectRatio::Align {
                        align: SvgAlign::Center,
                        meet_or_slice: SvgMeetOrSlice::Meet,
                    }
                );
            }
            _ => panic!("expected Image node"),
        }
    }

    #[test]
    fn parse_image_preserve_aspect_ratio_slice() {
        let el = make_el(
            "image",
            vec![
                ("href", "test.png"),
                ("width", "100"),
                ("height", "50"),
                ("preserveAspectRatio", "xMinYMin slice"),
            ],
        );
        let node = parse_svg_node(&el).unwrap();
        match node {
            SvgNode::Image {
                preserve_aspect_ratio,
                ..
            } => {
                assert_eq!(
                    preserve_aspect_ratio,
                    SvgPreserveAspectRatio::Align {
                        align: SvgAlign::TopLeft,
                        meet_or_slice: SvgMeetOrSlice::Slice,
                    }
                );
            }
            _ => panic!("expected Image node"),
        }
    }

    // ── preserveAspectRatio all align variants ─────────────────────────

    #[test]
    fn parse_preserve_aspect_ratio_all_align_values() {
        let cases = [
            ("xMinYMin", SvgAlign::TopLeft),
            ("xMidYMin", SvgAlign::TopCenter),
            ("xMaxYMin", SvgAlign::TopRight),
            ("xMinYMid", SvgAlign::CenterLeft),
            ("xMidYMid", SvgAlign::Center),
            ("xMaxYMid", SvgAlign::CenterRight),
            ("xMinYMax", SvgAlign::BottomLeft),
            ("xMidYMax", SvgAlign::BottomCenter),
            ("xMaxYMax", SvgAlign::BottomRight),
        ];
        for (raw, expected_align) in cases {
            let result = parse_svg_preserve_aspect_ratio_value(raw).unwrap();
            match result {
                SvgPreserveAspectRatio::Align {
                    align,
                    meet_or_slice,
                } => {
                    assert_eq!(align, expected_align, "failed for {raw}");
                    assert_eq!(meet_or_slice, SvgMeetOrSlice::Meet);
                }
                SvgPreserveAspectRatio::None => panic!("expected Align for {raw}"),
            }
        }
    }

    #[test]
    fn parse_preserve_aspect_ratio_none_case_insensitive() {
        assert_eq!(
            parse_svg_preserve_aspect_ratio_value("none"),
            Some(SvgPreserveAspectRatio::None)
        );
        assert_eq!(
            parse_svg_preserve_aspect_ratio_value("NONE"),
            Some(SvgPreserveAspectRatio::None)
        );
        assert_eq!(
            parse_svg_preserve_aspect_ratio_value("None"),
            Some(SvgPreserveAspectRatio::None)
        );
    }

    #[test]
    fn parse_preserve_aspect_ratio_unknown_align_returns_none() {
        assert!(parse_svg_preserve_aspect_ratio_value("xMidYMed").is_none());
        assert!(parse_svg_preserve_aspect_ratio_value("center").is_none());
    }

    #[test]
    fn parse_preserve_aspect_ratio_unknown_meetorslice_returns_none() {
        assert!(parse_svg_preserve_aspect_ratio_value("xMidYMid zoom").is_none());
    }

    #[test]
    fn parse_preserve_aspect_ratio_extra_tokens_returns_none() {
        assert!(parse_svg_preserve_aspect_ratio_value("xMidYMid meet slice").is_none());
    }

    #[test]
    fn parse_preserve_aspect_ratio_missing_attr_returns_default() {
        let el = make_el("svg", vec![]);
        let par = parse_svg_preserve_aspect_ratio(&el);
        assert_eq!(
            par,
            SvgPreserveAspectRatio::Align {
                align: SvgAlign::Center,
                meet_or_slice: SvgMeetOrSlice::Meet,
            }
        );
    }

    #[test]
    fn parse_preserve_aspect_ratio_invalid_value_uses_default() {
        let el = make_el("svg", vec![("preserveAspectRatio", "bogus")]);
        let par = parse_svg_preserve_aspect_ratio(&el);
        // parse_svg_preserve_aspect_ratio_value returns None => unwrap_or_default
        assert_eq!(
            par,
            SvgPreserveAspectRatio::Align {
                align: SvgAlign::Center,
                meet_or_slice: SvgMeetOrSlice::Meet,
            }
        );
    }

    // ── <use> element with depth limiting ─────────────────────────────

    #[test]
    fn parse_use_with_xlink_href() {
        let rect = make_el("rect", vec![("id", "r"), ("width", "10"), ("height", "10")]);
        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(rect)];

        let use_el = make_el("use", vec![("xlink:href", "#r")]);
        let svg = make_svg_el(
            vec![("width", "100"), ("height", "100")],
            vec![defs, use_el],
        );
        let tree = parse_svg_from_element(&svg).unwrap();
        assert_eq!(tree.children.len(), 1);
        match &tree.children[0] {
            SvgNode::Group { children, .. } => {
                assert_eq!(children.len(), 1);
                assert!(matches!(children[0], SvgNode::Rect { .. }));
            }
            other => panic!("expected use group, got {other:?}"),
        }
    }

    #[test]
    fn parse_use_missing_href_skipped() {
        // <use> with no href/xlink:href → parse_svg_node returns None
        let el = make_el("use", vec![]);
        assert!(parse_svg_node(&el).is_none());
    }

    #[test]
    fn parse_use_href_not_in_defs_skipped() {
        // href references id that doesn't exist in defs
        let el = make_el("use", vec![("href", "#nonexistent")]);
        assert!(parse_svg_node(&el).is_none());
    }

    #[test]
    fn parse_use_depth_limit_exceeded() {
        // Build a <use> referencing itself via defs — the depth limit (>16) must fire
        // We test by directly exceeding ref_depth in the parse context.
        // Construct defs with a rect that has id "shape"
        let mut attrs_map = HashMap::new();
        attrs_map.insert("id".to_string(), "shape".to_string());
        attrs_map.insert("width".to_string(), "10".to_string());
        attrs_map.insert("height".to_string(), "10".to_string());
        let rect_el = ElementNode {
            tag: HtmlTag::Unknown,
            raw_tag_name: "rect".to_string(),
            attributes: attrs_map,
            children: vec![],
        };
        let mut defs_raw: HashMap<String, ElementNode> = HashMap::new();
        defs_raw.insert("shape".to_string(), rect_el);

        let mut ctx = SvgParseContext::new(&defs_raw);
        // Push ref depth beyond the limit (>16)
        for _ in 0..17 {
            ctx.push_ref();
        }
        assert!(ctx.ref_depth() > 16);
        let result = parse_svg_referenced_node("shape", None, &mut ctx);
        assert!(result.is_none(), "expected None when depth limit exceeded");
    }

    #[test]
    fn parse_use_no_translation_when_xy_zero() {
        let rect = make_el("rect", vec![("id", "r"), ("width", "10"), ("height", "10")]);
        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(rect)];

        // use with x=0, y=0 → no translation transform
        let use_el = make_el("use", vec![("href", "#r"), ("x", "0"), ("y", "0")]);
        let svg = make_svg_el(
            vec![("width", "100"), ("height", "100")],
            vec![defs, use_el],
        );
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Group { transform, .. } => {
                // No translate transform when x=0,y=0
                assert!(transform.is_none());
            }
            other => panic!("expected use group, got {other:?}"),
        }
    }

    // ── Arc path commands (A/a) edge cases ────────────────────────────

    #[test]
    fn parse_path_arc_zero_radii_becomes_lineto() {
        let cmds = parse_path_data("M 0 0 A 0 0 0 0 1 10 10");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0], PathCommand::MoveTo(0.0, 0.0));
        assert_eq!(cmds[1], PathCommand::LineTo(10.0, 10.0));
    }

    #[test]
    fn parse_path_arc_same_start_end_is_omitted() {
        let cmds = parse_path_data("M 10 10 A 5 5 0 0 1 10 10");
        assert_eq!(cmds, [PathCommand::MoveTo(10.0, 10.0)]);
    }

    #[test]
    fn parse_path_arc_tiny_positive_radii_are_not_zero() {
        let cmds = parse_path_data("M 0 0 A .00000001 .00000001 0 0 1 .00000001 0");
        assert!(
            cmds.iter()
                .any(|command| matches!(command, PathCommand::CubicTo(..))),
            "positive authored radii must not be replaced by a line"
        );
        assert!(
            !cmds
                .iter()
                .any(|command| matches!(command, PathCommand::LineTo(..)))
        );
    }

    #[test]
    fn parse_path_arc_one_ulp_endpoint_delta_is_not_omitted() {
        let end = f32::from_bits(1.0f32.to_bits() + 1);
        let cmds = parse_path_data(&format!("M 1 1 A 1 1 0 0 1 {end} 1"));
        assert!(
            cmds.iter()
                .any(|command| matches!(command, PathCommand::CubicTo(..)))
        );
    }

    #[test]
    fn parse_path_arc_large_arc_flag() {
        // large_arc=1 takes the larger arc path
        let cmds = parse_path_data("M 0 0 A 10 10 0 1 0 10 0");
        assert!(matches!(cmds.first(), Some(PathCommand::MoveTo(0.0, 0.0))));
        // Should produce multiple cubic segments for the large arc
        let cubics: Vec<_> = cmds
            .iter()
            .filter(|c| matches!(c, PathCommand::CubicTo(..)))
            .collect();
        assert!(!cubics.is_empty());
    }

    #[test]
    fn parse_path_relative_arc_a_lower() {
        // Relative arc command 'a'
        let cmds = parse_path_data("M 5 5 a 10 10 0 0 1 10 0");
        assert!(matches!(cmds.first(), Some(PathCommand::MoveTo(5.0, 5.0))));
        assert!(
            cmds.iter().any(|c| matches!(c, PathCommand::CubicTo(..))),
            "expected arc to produce cubic segments"
        );
    }

    #[test]
    fn parse_path_arc_with_rotation() {
        // x_axis_rotation != 0
        let cmds = parse_path_data("M 0 0 A 10 5 45 0 1 10 10");
        assert!(matches!(cmds.first(), Some(PathCommand::MoveTo(0.0, 0.0))));
        assert!(
            cmds.iter().any(|c| matches!(c, PathCommand::CubicTo(..))),
            "expected rotated arc to produce cubic segments"
        );
    }

    // ── linearGradient in defs ─────────────────────────────────────────

    #[test]
    fn parse_linear_gradient_object_bounding_box_units() {
        let stop0 = make_el("stop", vec![("offset", "0%"), ("stop-color", "red")]);
        let stop1 = make_el("stop", vec![("offset", "100%"), ("stop-color", "blue")]);
        let mut gradient = make_el(
            "linearGradient",
            vec![
                ("id", "g1"),
                ("x1", "0%"),
                ("y1", "0%"),
                ("x2", "100%"),
                ("y2", "0%"),
                ("gradientUnits", "objectBoundingBox"),
            ],
        );
        gradient.children = vec![DomNode::Element(stop0), DomNode::Element(stop1)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(gradient)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        let g = tree
            .defs
            .gradients
            .get("g1")
            .expect("gradient 'g1' not found");
        assert_eq!(g.gradient_units, SvgGradientUnits::ObjectBoundingBox);
        assert!((g.x1 - 0.0).abs() < 0.01);
        assert!((g.x2 - 1.0).abs() < 0.01);
        assert_eq!(g.stops.len(), 2);
        assert!((g.stops[0].offset - 0.0).abs() < 0.01);
        assert!((g.stops[1].offset - 1.0).abs() < 0.01);
    }

    #[test]
    fn parse_linear_gradient_with_transform() {
        let stop0 = make_el("stop", vec![("offset", "0"), ("stop-color", "black")]);
        let stop1 = make_el("stop", vec![("offset", "1"), ("stop-color", "white")]);
        let mut gradient = make_el(
            "linearGradient",
            vec![
                ("id", "g2"),
                ("x1", "0"),
                ("y1", "0"),
                ("x2", "10"),
                ("y2", "0"),
                ("gradientTransform", "translate(5, 5)"),
            ],
        );
        gradient.children = vec![DomNode::Element(stop0), DomNode::Element(stop1)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(gradient)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        let g = tree
            .defs
            .gradients
            .get("g2")
            .expect("gradient 'g2' not found");
        assert!(g.gradient_transform.is_some());
    }

    #[test]
    fn parse_linear_gradient_fewer_than_two_stops_skipped() {
        let stop0 = make_el("stop", vec![("offset", "0"), ("stop-color", "black")]);
        let mut gradient = make_el("linearGradient", vec![("id", "g3")]);
        gradient.children = vec![DomNode::Element(stop0)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(gradient)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        assert!(!tree.defs.gradients.contains_key("g3"));
    }

    #[test]
    fn parse_linear_gradient_stop_opacity_from_style() {
        let mut stop0 = make_el(
            "stop",
            vec![
                ("offset", "0"),
                ("style", "stop-color: red; stop-opacity: 0.5;"),
            ],
        );
        // Remove direct stop-color attribute so it falls back to the style
        stop0.attributes.remove("stop-color");
        let stop1 = make_el("stop", vec![("offset", "1"), ("stop-color", "blue")]);
        let mut gradient = make_el("linearGradient", vec![("id", "g4")]);
        gradient.children = vec![DomNode::Element(stop0), DomNode::Element(stop1)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(gradient)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        let g = tree
            .defs
            .gradients
            .get("g4")
            .expect("gradient 'g4' not found");
        assert!((g.stops[0].opacity - 0.5).abs() < 0.01);
    }

    #[test]
    fn parse_linear_gradient_stop_opacity_attribute() {
        let stop0 = make_el(
            "stop",
            vec![
                ("offset", "0"),
                ("stop-color", "red"),
                ("stop-opacity", "0.25"),
            ],
        );
        let stop1 = make_el("stop", vec![("offset", "1"), ("stop-color", "blue")]);
        let mut gradient = make_el("linearGradient", vec![("id", "g5")]);
        gradient.children = vec![DomNode::Element(stop0), DomNode::Element(stop1)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(gradient)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        let g = tree
            .defs
            .gradients
            .get("g5")
            .expect("gradient 'g5' not found");
        assert!((g.stops[0].opacity - 0.25).abs() < 0.01);
    }

    #[test]
    fn parse_linear_gradient_stop_missing_color_skipped() {
        // A stop with no stop-color is skipped; gradient needs 2 valid stops
        let stop_no_color = make_el("stop", vec![("offset", "0")]);
        let stop_valid = make_el("stop", vec![("offset", "1"), ("stop-color", "blue")]);
        let mut gradient = make_el("linearGradient", vec![("id", "g6")]);
        gradient.children = vec![
            DomNode::Element(stop_no_color),
            DomNode::Element(stop_valid),
        ];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(gradient)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        // Only 1 valid stop → gradient skipped
        assert!(!tree.defs.gradients.contains_key("g6"));
    }

    // ── clipPath in defs ───────────────────────────────────────────────

    #[test]
    fn parse_clip_path_object_bounding_box_units() {
        let rect = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut clip = make_el(
            "clipPath",
            vec![("id", "cp1"), ("clipPathUnits", "objectBoundingBox")],
        );
        clip.children = vec![DomNode::Element(rect)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(clip)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        let cp = tree
            .defs
            .clip_paths
            .get("cp1")
            .expect("clip path 'cp1' not found");
        assert_eq!(cp.clip_path_units, SvgClipPathUnits::ObjectBoundingBox);
    }

    #[test]
    fn parse_clip_path_with_transform() {
        let rect = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut clip = make_el(
            "clipPath",
            vec![("id", "cp2"), ("transform", "translate(5, 5)")],
        );
        clip.children = vec![DomNode::Element(rect)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(clip)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        let cp = tree
            .defs
            .clip_paths
            .get("cp2")
            .expect("clip path 'cp2' not found");
        assert!(cp.transform.is_some());
    }

    #[test]
    fn parse_clip_path_empty_children_skipped() {
        // A clipPath with no renderable children returns None → not inserted in defs
        let clip = make_el("clipPath", vec![("id", "cp3")]);
        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(clip)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        assert!(!tree.defs.clip_paths.contains_key("cp3"));
    }

    #[test]
    fn parse_clip_path_user_space_on_use_is_default() {
        let rect = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut clip = make_el("clipPath", vec![("id", "cp4")]);
        clip.children = vec![DomNode::Element(rect)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(clip)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        let cp = tree
            .defs
            .clip_paths
            .get("cp4")
            .expect("clip path 'cp4' not found");
        assert_eq!(cp.clip_path_units, SvgClipPathUnits::UserSpaceOnUse);
    }

    // ── parse_svg_from_string ──────────────────────────────────────────

    #[test]
    fn parse_svg_from_string_basic() {
        let svg_text = r#"<svg width="100" height="50"><rect width="10" height="10"/></svg>"#;
        let tree = parse_svg_from_string(svg_text).unwrap();
        assert_eq!(tree.width, 100.0);
        assert_eq!(tree.height, 50.0);
        assert!(tree.source_markup.is_some());
        assert_eq!(tree.source_markup.as_deref(), Some(svg_text));
    }

    #[test]
    fn parse_svg_from_string_no_svg_returns_none() {
        let result = parse_svg_from_string("<div><p>Not SVG</p></div>");
        assert!(result.is_none());
    }

    // ── parse_svg_gradient_coordinate ─────────────────────────────────

    #[test]
    fn parse_gradient_coordinate_percentage() {
        // "50%" should parse as 0.5
        let mut attrs = HashMap::new();
        attrs.insert("x1".to_string(), "50%".to_string());
        let el = ElementNode {
            tag: HtmlTag::Unknown,
            raw_tag_name: "linearGradient".to_string(),
            attributes: attrs,
            children: vec![],
        };
        let val = parse_svg_gradient_coordinate(el.attributes.get("x1"), 0.0);
        assert!((val - 0.5).abs() < 0.001);
    }

    #[test]
    fn parse_gradient_coordinate_absolute() {
        let mut attrs = HashMap::new();
        attrs.insert("x1".to_string(), "25".to_string());
        let el = ElementNode {
            tag: HtmlTag::Unknown,
            raw_tag_name: "linearGradient".to_string(),
            attributes: attrs,
            children: vec![],
        };
        let val = parse_svg_gradient_coordinate(el.attributes.get("x1"), 0.0);
        assert!((val - 25.0).abs() < 0.001);
    }

    #[test]
    fn parse_gradient_coordinate_missing_uses_fallback() {
        let val = parse_svg_gradient_coordinate(None, 42.0);
        assert_eq!(val, 42.0);
    }

    // ── parse_svg_gradient_offset ──────────────────────────────────────

    #[test]
    fn parse_gradient_offset_percentage() {
        assert!((parse_svg_gradient_offset("75%").unwrap() - 0.75).abs() < 0.001);
    }

    #[test]
    fn parse_gradient_offset_decimal() {
        assert!((parse_svg_gradient_offset("0.5").unwrap() - 0.5).abs() < 0.001);
    }

    #[test]
    fn parse_gradient_offset_integer() {
        assert!((parse_svg_gradient_offset("1").unwrap() - 1.0).abs() < 0.001);
    }

    #[test]
    fn parse_gradient_offset_invalid_returns_none() {
        assert!(parse_svg_gradient_offset("abc%").is_none());
    }

    // ── style_property_value with parenthesized url ────────────────────

    #[test]
    fn style_property_value_with_url_in_value() {
        // url(...) inside a style value should not be split on ';' inside the parens
        let el = make_el("rect", vec![("style", "fill: url(#grad); stroke: red;")]);
        let style = parse_svg_style(&el);
        assert!(matches!(style.fill, SvgPaint::Url(ref id) if id == "grad"));
        assert_eq!(style.stroke, SvgPaint::Color(Color::rgb(255, 0, 0)));
    }

    #[test]
    fn style_property_value_clip_path_from_style_attr() {
        // clip-path set via inline style
        let el = make_el("rect", vec![("style", "clip-path: url(#myClip);")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.clip_path.as_deref(), Some("myClip"));
    }

    // ── parse_svg_paint_server_reference edge cases ────────────────────

    #[test]
    fn parse_paint_server_reference_with_quotes() {
        // url("#id") with double quotes
        let result = parse_svg_paint_server_reference("url(\"#myGrad\")");
        assert_eq!(result, Some("myGrad".to_string()));
    }

    #[test]
    fn parse_paint_server_reference_with_single_quotes() {
        let result = parse_svg_paint_server_reference("url('#myGrad')");
        assert_eq!(result, Some("myGrad".to_string()));
    }

    #[test]
    fn parse_paint_server_reference_no_hash_returns_none() {
        let result = parse_svg_paint_server_reference("url(myGrad)");
        assert!(result.is_none());
    }

    #[test]
    fn parse_paint_server_reference_not_url_returns_none() {
        let result = parse_svg_paint_server_reference("red");
        assert!(result.is_none());
    }

    // ── parse_svg_reference_id ─────────────────────────────────────────

    #[test]
    fn parse_reference_id_hash_prefix() {
        assert_eq!(parse_svg_reference_id("#foo"), Some("foo".to_string()));
    }

    #[test]
    fn parse_reference_id_url_hash() {
        assert_eq!(parse_svg_reference_id("url(#bar)"), Some("bar".to_string()));
    }

    #[test]
    fn parse_reference_id_no_hash_no_url_returns_none() {
        assert!(parse_svg_reference_id("notanid").is_none());
    }

    // ── parse_absolute_length ──────────────────────────────────────────

    #[test]
    fn parse_absolute_length_percent_returns_none() {
        assert!(parse_absolute_length("50%").is_none());
    }

    #[test]
    fn parse_absolute_length_px_suffix() {
        assert_eq!(parse_absolute_length("72px"), Some(72.0));
    }

    #[test]
    fn parse_absolute_length_plain_number() {
        assert_eq!(parse_absolute_length("100"), Some(100.0));
    }

    // ── resolve_svg_viewport_length edge cases ─────────────────────────

    #[test]
    fn resolve_viewport_length_percentage_without_parent_uses_fallback() {
        // "50%" with no parent extent should use fallback
        let mut attrs = HashMap::new();
        attrs.insert("width".to_string(), "50%".to_string());
        let el = ElementNode {
            tag: HtmlTag::Unknown,
            raw_tag_name: "svg".to_string(),
            attributes: attrs,
            children: vec![],
        };
        let val = resolve_svg_viewport_length(el.attributes.get("width"), None, 99.0);
        assert_eq!(val, 99.0);
    }

    #[test]
    fn resolve_viewport_length_no_attr_uses_parent() {
        let val = resolve_svg_viewport_length(None, Some(200.0), 0.0);
        assert_eq!(val, 200.0);
    }

    #[test]
    fn resolve_viewport_length_no_attr_no_parent_uses_fallback() {
        let val = resolve_svg_viewport_length(None, None, 42.0);
        assert_eq!(val, 42.0);
    }

    // ── svg_style_is_default ───────────────────────────────────────────

    #[test]
    fn svg_style_is_default_true_for_empty() {
        let style = SvgStyle::default();
        assert!(svg_style_is_default(&style));
    }

    #[test]
    fn svg_style_is_default_false_when_fill_set() {
        let mut style = SvgStyle::default();
        style.fill = SvgPaint::Color(Color::rgb(255, 0, 0));
        assert!(!svg_style_is_default(&style));
    }

    #[test]
    fn svg_style_is_default_false_when_opacity_not_one() {
        let mut style = SvgStyle::default();
        style.opacity = 0.5;
        assert!(!svg_style_is_default(&style));
    }

    #[test]
    fn svg_style_is_default_false_when_color_set() {
        let mut style = SvgStyle::default();
        style.color = Some(Color::BLACK);
        assert!(!svg_style_is_default(&style));
    }

    #[test]
    fn svg_style_is_default_false_when_stroke_width_set() {
        let mut style = SvgStyle::default();
        style.stroke_width = Some(1.0);
        assert!(!svg_style_is_default(&style));
    }

    #[test]
    fn svg_style_is_default_false_when_clip_path_set() {
        let mut style = SvgStyle::default();
        style.clip_path = Some("clip".to_string());
        assert!(!svg_style_is_default(&style));
    }

    // ── compose_transform ──────────────────────────────────────────────

    #[test]
    fn compose_transform_both_none_is_none() {
        assert!(compose_transform(None, None).is_none());
    }

    #[test]
    fn compose_transform_outer_only() {
        let t = SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 5.0, 10.0);
        let result = compose_transform(Some(t), None);
        assert!(matches!(
            result,
            Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 5.0, 10.0))
        ));
    }

    #[test]
    fn compose_transform_inner_only() {
        let t = SvgTransform::Matrix(2.0, 0.0, 0.0, 2.0, 0.0, 0.0);
        let result = compose_transform(None, Some(t));
        assert!(matches!(
            result,
            Some(SvgTransform::Matrix(2.0, 0.0, 0.0, 2.0, 0.0, 0.0))
        ));
    }

    #[test]
    fn compose_transform_two_translates_adds_offsets() {
        let t1 = SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 3.0, 4.0);
        let t2 = SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 7.0, 6.0);
        let result = compose_transform(Some(t1), Some(t2)).unwrap();
        match result {
            SvgTransform::Matrix(a, b, c, d, e, f) => {
                assert!((a - 1.0).abs() < 0.001);
                assert!((b - 0.0).abs() < 0.001);
                assert!((c - 0.0).abs() < 0.001);
                assert!((d - 1.0).abs() < 0.001);
                assert!((e - 10.0).abs() < 0.001);
                assert!((f - 10.0).abs() < 0.001);
            }
        }
    }

    // ── collect_svg_defs_from_element ──────────────────────────────────

    #[test]
    fn collect_defs_from_element_registers_id_outside_defs() {
        // An element with an id directly inside <svg> (not in <defs>) gets registered
        let shape = make_el("circle", vec![("id", "c1"), ("r", "5")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![shape]);
        // parse_svg_from_element calls collect_svg_defs on svg.children
        // The circle with id "c1" should end up in defs_raw (though not as a gradient/clip)
        let tree = parse_svg_from_element(&svg).unwrap();
        // It appears as a normal child, not in processed defs
        assert_eq!(tree.children.len(), 1);
        assert!(matches!(tree.children[0], SvgNode::Circle { .. }));
    }

    #[test]
    fn collect_defs_nested_inside_defs_wrapper_are_collected() {
        // Elements nested in <defs> are collected by id
        let mut gradient = make_el(
            "linearGradient",
            vec![
                ("id", "nested_g"),
                ("x1", "0"),
                ("y1", "0"),
                ("x2", "1"),
                ("y2", "0"),
            ],
        );
        let s0 = make_el("stop", vec![("offset", "0"), ("stop-color", "black")]);
        let s1 = make_el("stop", vec![("offset", "1"), ("stop-color", "white")]);
        gradient.children = vec![DomNode::Element(s0), DomNode::Element(s1)];

        let mut defs = make_el("defs", vec![]);
        defs.children = vec![DomNode::Element(gradient)];

        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![defs]);
        let tree = parse_svg_from_element(&svg).unwrap();
        assert!(tree.defs.gradients.contains_key("nested_g"));
    }

    // ── stroke-width edge cases ────────────────────────────────────────

    #[test]
    fn parse_style_stroke_width_negative_rejected() {
        // Negative stroke-width should be rejected (filter)
        let el = make_el("rect", vec![("stroke-width", "-1")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.stroke_width, None);
    }

    #[test]
    fn parse_style_stroke_width_zero_accepted() {
        let el = make_el("rect", vec![("stroke-width", "0")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.stroke_width, Some(0.0));
    }

    #[test]
    fn parse_style_stroke_width_from_inline_style() {
        let el = make_el("rect", vec![("style", "stroke-width: 4.5;")]);
        let style = parse_svg_style(&el);
        assert_eq!(style.stroke_width, Some(4.5));
    }

    // ── font attribute parsing ─────────────────────────────────────────

    #[test]
    fn parse_svg_font_family_preserves_the_authored_name() {
        // Mapping to a base-14 face happens at render time; the parser keeps
        // what the author wrote so a registered custom face can still win.
        let el = make_el("text", vec![("font-family", "Times New Roman")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_family, .. } => {
                assert_eq!(font_family.as_deref(), Some("Times New Roman"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_font_family_preserves_a_quoted_list_with_commas() {
        // CSS Fonts allows a quoted family name to contain commas; splitting
        // or unquoting here would corrupt the list before stack resolution.
        let el = make_el("text", vec![("font-family", "'Acme, Sans', ParitySerif")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_family, .. } => {
                assert_eq!(font_family.as_deref(), Some("'Acme, Sans', ParitySerif"));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_font_family_inherit_returns_none() {
        let el = make_el("text", vec![("font-family", "inherit")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_family, .. } => {
                assert!(font_family.is_none());
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_font_weight_bold() {
        let el = make_el("text", vec![("font-weight", "bold")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_bold, .. } => {
                assert_eq!(*font_bold, Some(true));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_font_weight_numeric_700() {
        let el = make_el("text", vec![("font-weight", "700")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_bold, .. } => {
                assert_eq!(*font_bold, Some(true));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_font_weight_400_not_bold() {
        let el = make_el("text", vec![("font-weight", "400")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_bold, .. } => {
                assert_eq!(*font_bold, Some(false));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_font_style_italic() {
        let el = make_el("text", vec![("font-style", "italic")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_italic, .. } => {
                assert_eq!(*font_italic, Some(true));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_font_style_oblique() {
        let el = make_el("text", vec![("font-style", "oblique")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_italic, .. } => {
                assert_eq!(*font_italic, Some(true));
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn parse_svg_font_style_inherit_returns_none() {
        let el = make_el("text", vec![("font-style", "inherit")]);
        let svg = make_svg_el(vec![("width", "100"), ("height", "100")], vec![el]);
        let tree = parse_svg_from_element(&svg).unwrap();
        match &tree.children[0] {
            SvgNode::Text { font_italic, .. } => {
                assert!(font_italic.is_none());
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // ── tspan text content ─────────────────────────────────────────────

    #[test]
    fn collect_text_content_with_tspan_children() {
        let mut text_el = make_el("text", vec![]);
        let mut tspan = make_el("tspan", vec![]);
        tspan.children = vec![DomNode::Text("hello".to_string())];
        text_el.children = vec![
            DomNode::Text("prefix ".to_string()),
            DomNode::Element(tspan),
        ];
        let content = collect_text_content(&text_el);
        assert_eq!(content, "prefix hello");
    }

    #[test]
    fn collect_text_content_non_tspan_element_ignored() {
        let mut text_el = make_el("text", vec![]);
        let span = make_el("span", vec![]); // not a tspan
        text_el.children = vec![DomNode::Element(span), DomNode::Text(" world".to_string())];
        let content = collect_text_content(&text_el);
        assert_eq!(content, " world");
    }

    // ── parse_svg_from_element_with_viewport ──────────────────────────

    #[test]
    fn parse_svg_from_element_with_viewport_overrides_dimensions() {
        let svg = make_svg_el(vec![("width", "100"), ("height", "50")], vec![]);
        let tree = parse_svg_from_element_with_viewport(&svg, Some((800.0, 600.0))).unwrap();
        assert_eq!(tree.width, 800.0);
        assert_eq!(tree.height, 600.0);
    }

    #[test]
    fn parse_svg_from_element_with_viewport_none_uses_attrs() {
        let svg = make_svg_el(vec![("width", "200"), ("height", "100")], vec![]);
        let tree = parse_svg_from_element_with_viewport(&svg, None).unwrap();
        assert_eq!(tree.width, 200.0);
        assert_eq!(tree.height, 100.0);
    }

    // ── parse_svg_from_element_with_ctx ───────────────────────────────

    #[test]
    fn parse_svg_from_element_with_ctx_applies_text_context() {
        let svg = make_svg_el(vec![("width", "100"), ("height", "50")], vec![]);
        let ctx = SvgTextContext {
            font_family: "Courier".to_string(),
            font_size: 14.0,
            font_bold: true,
            font_italic: false,
            color: Some(Color::rgb(255, 0, 0)),
        };
        let tree = parse_svg_from_element_with_ctx(&svg, ctx.clone()).unwrap();
        assert_eq!(tree.text_ctx.font_family, "Courier");
        assert_eq!(tree.text_ctx.font_size, 14.0);
        assert!(tree.text_ctx.font_bold);
        assert_eq!(tree.text_ctx.color, Some(Color::rgb(255, 0, 0)));
    }

    // ── nested SVG without viewBox, x/y offset ────────────────────────

    #[test]
    fn nested_svg_no_viewbox_with_offset_applies_translate() {
        let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut svg_inner = make_el("svg", vec![("x", "15"), ("y", "25")]);
        svg_inner.children.push(DomNode::Element(inner));
        let node = parse_svg_node(&svg_inner).unwrap();
        match node {
            SvgNode::Group { transform, .. } => {
                assert!(matches!(
                    transform,
                    Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 15.0, 25.0))
                ));
            }
            other => panic!("expected nested svg group, got {other:?}"),
        }
    }

    #[test]
    fn nested_svg_no_viewbox_no_offset_has_no_transform() {
        let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut svg_inner = make_el("svg", vec![]);
        svg_inner.children.push(DomNode::Element(inner));
        let node = parse_svg_node(&svg_inner).unwrap();
        match node {
            SvgNode::Group { transform, .. } => {
                assert!(transform.is_none());
            }
            other => panic!("expected nested svg group, got {other:?}"),
        }
    }

    // ── nested SVG viewBox with zero dimensions ────────────────────────

    #[test]
    fn nested_svg_viewbox_zero_width_no_transform() {
        // viewBox with width=0 should NOT produce a scale transform
        let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
        let mut svg_inner = make_el(
            "svg",
            vec![
                ("width", "100"),
                ("height", "100"),
                ("viewBox", "0 0 0 100"),
            ],
        );
        svg_inner.children.push(DomNode::Element(inner));
        let node = parse_svg_node(&svg_inner).unwrap();
        match node {
            SvgNode::Group { transform, .. } => {
                // vb.width == 0 → condition vb.width > 0 && vb.height > 0 fails → no viewbox transform
                assert!(transform.is_none());
            }
            other => panic!("expected nested svg group, got {other:?}"),
        }
    }
}
