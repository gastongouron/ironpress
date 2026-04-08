use super::length::{parse_absolute_length, parse_length, parse_viewbox};
use super::path::{parse_path_data, parse_points};
use super::style::{parse_svg_style, SvgPaint, SvgStyle};
use super::transform::{compose_transform, parse_transform};
use super::{SvgNode, SvgTransform, SvgTree};
use crate::parser::dom::{DomNode, ElementNode};

/// Entry point: parse an `<svg>` ElementNode into an SvgTree.
pub fn parse_svg_from_element(el: &ElementNode) -> Option<SvgTree> {
    parse_svg_from_element_with_viewport(el, None)
}

pub(crate) fn parse_svg_from_element_with_viewport(
    el: &ElementNode,
    root_viewport: Option<(f32, f32)>,
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
    let view_box = el
        .attributes
        .get("viewBox")
        .and_then(|value| parse_viewbox(value));
    let mut children = parse_svg_children(el, root_viewport.or(Some((width, height))));

    let root_style = parse_svg_style(el);
    let root_transform = el
        .attributes
        .get("transform")
        .and_then(|value| parse_transform(value));
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
    })
}

pub(crate) fn parse_svg_node(el: &ElementNode) -> Option<SvgNode> {
    parse_svg_node_with_viewport(el, None)
}

fn parse_svg_children(el: &ElementNode, parent_viewport: Option<(f32, f32)>) -> Vec<SvgNode> {
    el.children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(child_el) => parse_svg_node_with_viewport(child_el, parent_viewport),
            _ => None,
        })
        .collect()
}

fn parse_svg_node_with_viewport(
    el: &ElementNode,
    parent_viewport: Option<(f32, f32)>,
) -> Option<SvgNode> {
    match el.raw_tag_name.as_str() {
        "g" => Some(SvgNode::Group {
            transform: el
                .attributes
                .get("transform")
                .and_then(|value| parse_transform(value)),
            children: parse_svg_children(el, parent_viewport),
            style: parse_svg_style(el),
        }),
        "svg" => {
            let child_viewport = resolve_nested_svg_viewport(el, parent_viewport);
            let transform = compose_transform(
                el.attributes
                    .get("transform")
                    .and_then(|value| parse_transform(value)),
                nested_svg_viewport_transform(el, parent_viewport),
            );
            Some(SvgNode::Group {
                transform,
                children: parse_svg_children(el, child_viewport),
                style: parse_svg_style(el),
            })
        }
        "rect" => Some(SvgNode::Rect {
            x: attr_f32(el, "x"),
            y: attr_f32(el, "y"),
            width: attr_f32(el, "width"),
            height: attr_f32(el, "height"),
            rx: attr_f32(el, "rx"),
            ry: attr_f32(el, "ry"),
            style: parse_svg_style(el),
        }),
        "circle" => Some(SvgNode::Circle {
            cx: attr_f32(el, "cx"),
            cy: attr_f32(el, "cy"),
            r: attr_f32(el, "r"),
            style: parse_svg_style(el),
        }),
        "ellipse" => Some(SvgNode::Ellipse {
            cx: attr_f32(el, "cx"),
            cy: attr_f32(el, "cy"),
            rx: attr_f32(el, "rx"),
            ry: attr_f32(el, "ry"),
            style: parse_svg_style(el),
        }),
        "line" => Some(SvgNode::Line {
            x1: attr_f32(el, "x1"),
            y1: attr_f32(el, "y1"),
            x2: attr_f32(el, "x2"),
            y2: attr_f32(el, "y2"),
            style: parse_svg_style(el),
        }),
        "polyline" => Some(SvgNode::Polyline {
            points: el
                .attributes
                .get("points")
                .map(|value| parse_points(value))
                .unwrap_or_default(),
            style: parse_svg_style(el),
        }),
        "polygon" => Some(SvgNode::Polygon {
            points: el
                .attributes
                .get("points")
                .map(|value| parse_points(value))
                .unwrap_or_default(),
            style: parse_svg_style(el),
        }),
        "path" => Some(SvgNode::Path {
            commands: el
                .attributes
                .get("d")
                .map(|value| parse_path_data(value))
                .unwrap_or_default(),
            style: parse_svg_style(el),
        }),
        _ => None,
    }
}

fn svg_style_is_default(style: &SvgStyle) -> bool {
    style.color.is_none()
        && matches!(style.fill, SvgPaint::Unspecified)
        && matches!(style.stroke, SvgPaint::Unspecified)
        && style.stroke_width.is_none()
        && (style.opacity - 1.0).abs() < f32::EPSILON
}

fn resolve_nested_svg_viewport(
    el: &ElementNode,
    parent_viewport: Option<(f32, f32)>,
) -> Option<(f32, f32)> {
    parent_viewport.map(|viewport| resolve_svg_viewport_dimensions(el, Some(viewport)))
}

fn resolve_svg_viewport_dimensions(
    el: &ElementNode,
    parent_viewport: Option<(f32, f32)>,
) -> (f32, f32) {
    match parent_viewport {
        Some((parent_width, parent_height)) => (
            resolve_svg_viewport_length(el.attributes.get("width"), Some(parent_width), 300.0),
            resolve_svg_viewport_length(el.attributes.get("height"), Some(parent_height), 150.0),
        ),
        None => (
            resolve_svg_viewport_length(el.attributes.get("width"), None, 300.0),
            resolve_svg_viewport_length(el.attributes.get("height"), None, 150.0),
        ),
    }
}

fn resolve_svg_viewport_length(
    attr: Option<&String>,
    parent_extent: Option<f32>,
    fallback: f32,
) -> f32 {
    match attr.map(String::as_str) {
        Some(value) => {
            let trimmed = value.trim();
            if let Some(percent) = trimmed.strip_suffix('%') {
                percent
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .and_then(|value| parent_extent.map(|extent| extent * value / 100.0))
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
    let view_box = el
        .attributes
        .get("viewBox")
        .and_then(|value| parse_viewbox(value));

    if let Some(view_box) = view_box {
        let (width, height) = resolve_svg_viewport_dimensions(el, parent_viewport);
        if view_box.width > 0.0 && view_box.height > 0.0 {
            let scale_x = width / view_box.width;
            let scale_y = height / view_box.height;
            return Some(SvgTransform::Matrix(
                scale_x,
                0.0,
                0.0,
                scale_y,
                x - view_box.min_x * scale_x,
                y - view_box.min_y * scale_y,
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
pub(crate) fn attr_f32(el: &ElementNode, name: &str) -> f32 {
    el.attributes
        .get(name)
        .and_then(|value| parse_length(value))
        .unwrap_or(0.0)
}
