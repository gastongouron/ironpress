use super::node::{attr_f32, parse_svg_node};
use super::test_support::{make_el, make_svg_el};
use super::{
    parse_svg_from_element, parse_svg_from_element_with_viewport, SvgNode, SvgPaint, SvgTransform,
};
use crate::parser::dom::DomNode;

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
        } => assert_eq!(
            (x, y, width, height, rx, ry),
            (10.0, 20.0, 100.0, 50.0, 5.0, 3.0)
        ),
        _ => panic!("Expected Rect"),
    }
}

#[test]
fn parse_node_circle() {
    let el = make_el("circle", vec![("cx", "50"), ("cy", "50"), ("r", "25")]);
    let node = parse_svg_node(&el).unwrap();
    match node {
        SvgNode::Circle { cx, cy, r, .. } => assert_eq!((cx, cy, r), (50.0, 50.0, 25.0)),
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
            assert_eq!((cx, cy, rx, ry), (50.0, 50.0, 30.0, 20.0))
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
            assert_eq!((x1, y1, x2, y2), (0.0, 0.0, 100.0, 100.0))
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
    let node = parse_svg_node(&make_el("polyline", vec![])).unwrap();
    match node {
        SvgNode::Polyline { points, .. } => assert!(points.is_empty()),
        _ => panic!("Expected Polyline"),
    }
}

#[test]
fn parse_node_polygon() {
    let node =
        parse_svg_node(&make_el("polygon", vec![("points", "0,0 50,0 50,50 0,50")])).unwrap();
    match node {
        SvgNode::Polygon { points, .. } => assert_eq!(points.len(), 4),
        _ => panic!("Expected Polygon"),
    }
}

#[test]
fn parse_node_polygon_no_points() {
    let node = parse_svg_node(&make_el("polygon", vec![])).unwrap();
    match node {
        SvgNode::Polygon { points, .. } => assert!(points.is_empty()),
        _ => panic!("Expected Polygon"),
    }
}

#[test]
fn parse_node_path() {
    let node = parse_svg_node(&make_el("path", vec![("d", "M 0 0 L 10 10 Z")])).unwrap();
    match node {
        SvgNode::Path { commands, .. } => assert_eq!(commands.len(), 3),
        _ => panic!("Expected Path"),
    }
}

#[test]
fn parse_node_path_no_d_attr() {
    let node = parse_svg_node(&make_el("path", vec![])).unwrap();
    match node {
        SvgNode::Path { commands, .. } => assert!(commands.is_empty()),
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
        SvgNode::Group { children, .. } => assert!(children.is_empty()),
        _ => panic!("Expected Group"),
    }
}

#[test]
fn parse_node_group_no_transform() {
    let node = parse_svg_node(&make_el("g", vec![])).unwrap();
    match node {
        SvgNode::Group { transform, .. } => assert!(transform.is_none()),
        _ => panic!("Expected Group"),
    }
}

#[test]
fn parse_node_unknown_tag_returns_none() {
    assert!(parse_svg_node(&make_el("text", vec![])).is_none());
}

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
    assert_eq!((tree.width, tree.height), (200.0, 100.0));
    assert!(tree.view_box.is_some());
    assert_eq!(tree.children.len(), 1);
}

#[test]
fn parse_svg_from_element_defaults() {
    let tree = parse_svg_from_element(&make_svg_el(vec![], vec![])).unwrap();
    assert_eq!((tree.width, tree.height), (300.0, 150.0));
    assert!(tree.view_box.is_none());
    assert!(tree.children.is_empty());
}

#[test]
fn parse_svg_from_element_percent_dimensions_preserve_raw_attrs() {
    let tree = parse_svg_from_element(&make_svg_el(
        vec![("width", "100%"), ("height", "50%")],
        vec![],
    ))
    .unwrap();
    assert_eq!((tree.width, tree.height), (300.0, 150.0));
    assert_eq!(tree.width_attr.as_deref(), Some("100%"));
    assert_eq!(tree.height_attr.as_deref(), Some("50%"));
}

#[test]
fn parse_svg_from_element_wraps_root_style_and_transform() {
    let rect = make_el("rect", vec![("width", "10"), ("height", "10")]);
    let svg = make_svg_el(
        vec![("fill", "red"), ("transform", "translate(5, 6)")],
        vec![rect],
    );
    let tree = parse_svg_from_element(&svg).unwrap();
    match tree.children.first().unwrap() {
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
fn parse_svg_from_element_preserves_root_viewbox() {
    let rect = make_el("rect", vec![("width", "10"), ("height", "10")]);
    let svg = make_svg_el(
        vec![
            ("width", "200"),
            ("height", "100"),
            ("viewBox", "0 0 20 10"),
        ],
        vec![rect],
    );
    let tree = parse_svg_from_element(&svg).unwrap();
    assert!(tree.view_box.is_some());
    assert_eq!(tree.children.len(), 1);
}

#[test]
fn parse_svg_from_element_wraps_root_color_only() {
    let rect = make_el("rect", vec![("width", "10"), ("height", "10")]);
    let svg = make_svg_el(vec![("color", "#336699")], vec![rect]);
    let tree = parse_svg_from_element(&svg).unwrap();
    match tree.children.first().unwrap() {
        SvgNode::Group {
            style, children, ..
        } => {
            assert_eq!(style.color, Some((0.2, 0.4, 0.6)));
            assert_eq!(children.len(), 1);
        }
        other => panic!("expected wrapped root group, got {other:?}"),
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
    let tree = parse_svg_from_element(&make_svg_el(
        vec![("width", "100"), ("height", "100")],
        vec![make_el("text", vec![])],
    ))
    .unwrap();
    assert!(tree.children.is_empty());
}

#[test]
fn attr_f32_present() {
    let el = make_el("rect", vec![("x", "42px")]);
    assert_eq!(attr_f32(&el, "x"), 42.0);
}

#[test]
fn attr_f32_missing() {
    assert_eq!(attr_f32(&make_el("rect", vec![]), "x"), 0.0);
}

#[test]
fn parse_node_nested_svg_acts_as_group() {
    let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
    let mut inner_svg = make_el("svg", vec![]);
    inner_svg.children.push(DomNode::Element(inner));
    let node = parse_svg_node(&inner_svg).unwrap();
    match node {
        SvgNode::Group { children, .. } => assert_eq!(children.len(), 1),
        _ => panic!("Expected Group for inner svg"),
    }
}

#[test]
fn parse_node_nested_svg_applies_viewport_transform() {
    let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
    let mut inner_svg = make_el(
        "svg",
        vec![
            ("x", "10"),
            ("y", "20"),
            ("width", "100"),
            ("height", "50"),
            ("viewBox", "0 0 10 5"),
        ],
    );
    inner_svg.children.push(DomNode::Element(inner));
    let node = parse_svg_node(&inner_svg).unwrap();
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
    let mut inner_svg = make_el(
        "svg",
        vec![
            ("width", "100%"),
            ("height", "50%"),
            ("viewBox", "0 0 20 10"),
        ],
    );
    inner_svg.children.push(DomNode::Element(inner));
    let outer = make_svg_el(vec![("width", "200"), ("height", "100")], vec![inner_svg]);
    let tree = parse_svg_from_element(&outer).unwrap();
    match tree.children.first().unwrap() {
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
fn parse_node_nested_svg_percent_viewport_uses_explicit_root_size() {
    let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
    let mut inner_svg = make_el(
        "svg",
        vec![
            ("width", "50%"),
            ("height", "50%"),
            ("viewBox", "0 0 10 10"),
        ],
    );
    inner_svg.children.push(DomNode::Element(inner));
    let mut outer = make_el(
        "svg",
        vec![
            ("width", "100%"),
            ("height", "50%"),
            ("viewBox", "0 0 20 10"),
        ],
    );
    outer.children.push(DomNode::Element(inner_svg));
    let tree = parse_svg_from_element_with_viewport(&outer, Some((400.0, 100.0))).unwrap();
    match tree.children.first().unwrap() {
        SvgNode::Group { transform, .. } => {
            assert!(matches!(
                transform,
                Some(SvgTransform::Matrix(20.0, 0.0, 0.0, 5.0, 0.0, 0.0))
            ));
        }
        other => panic!("expected nested svg group, got {other:?}"),
    }
}

#[test]
fn parse_node_nested_svg_composes_transform_with_viewport() {
    let inner = make_el("rect", vec![("width", "10"), ("height", "10")]);
    let mut inner_svg = make_el(
        "svg",
        vec![
            ("x", "10"),
            ("y", "20"),
            ("width", "100"),
            ("height", "50"),
            ("viewBox", "0 0 10 5"),
            ("transform", "translate(3, 4)"),
        ],
    );
    inner_svg.children.push(DomNode::Element(inner));
    let node = parse_svg_node(&inner_svg).unwrap();
    match node {
        SvgNode::Group { transform, .. } => {
            assert!(matches!(
                transform,
                Some(SvgTransform::Matrix(10.0, 0.0, 0.0, 10.0, 13.0, 24.0))
            ));
        }
        other => panic!("expected nested svg group, got {other:?}"),
    }
}

#[test]
fn parse_node_rect_defaults() {
    let node = parse_svg_node(&make_el("rect", vec![])).unwrap();
    match node {
        SvgNode::Rect {
            x,
            y,
            width,
            height,
            rx,
            ry,
            ..
        } => assert_eq!(
            (x, y, width, height, rx, ry),
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        ),
        _ => panic!("Expected Rect"),
    }
}
