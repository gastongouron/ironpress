//! SVG tree to PDF content stream renderer.

use crate::parser::svg::{PathCommand, SvgNode, SvgStyle, SvgTransform, SvgTree};

/// Render an SVG tree to PDF content stream operators.
///
/// The caller must wrap this in a `q ... Q` block and set up the coordinate
/// transform (position on page + y-axis flip).
pub fn render_svg_tree(tree: &SvgTree, out: &mut String) {
    for node in &tree.children {
        render_node(node, out);
    }
}

fn render_node(node: &SvgNode, out: &mut String) {
    match node {
        SvgNode::Group {
            transform,
            children,
            ..
        } => {
            out.push_str("q\n");
            if let Some(SvgTransform::Matrix(a, b, c, d, e, f)) = transform {
                out.push_str(&format!("{a} {b} {c} {d} {e} {f} cm\n"));
            }
            for child in children {
                render_node(child, out);
            }
            out.push_str("Q\n");
        }
        SvgNode::Rect {
            x,
            y,
            width,
            height,
            style,
            ..
        } => {
            apply_style(style, out);
            out.push_str(&format!("{x} {y} {width} {height} re\n"));
            paint(style, out);
        }
        SvgNode::Circle { cx, cy, r, style } => {
            apply_style(style, out);
            // Approximate circle with 4 cubic bezier curves
            emit_circle(*cx, *cy, *r, out);
            paint(style, out);
        }
        SvgNode::Ellipse {
            cx,
            cy,
            rx,
            ry,
            style,
        } => {
            apply_style(style, out);
            emit_ellipse(*cx, *cy, *rx, *ry, out);
            paint(style, out);
        }
        SvgNode::Line {
            x1,
            y1,
            x2,
            y2,
            style,
        } => {
            apply_style(style, out);
            out.push_str(&format!("{x1} {y1} m {x2} {y2} l S\n"));
        }
        SvgNode::Polyline { points, style } => {
            apply_style(style, out);
            emit_polyline(points, false, out);
            out.push_str("S\n"); // stroke only for polyline
        }
        SvgNode::Polygon { points, style } => {
            apply_style(style, out);
            emit_polyline(points, true, out);
            paint(style, out);
        }
        SvgNode::Path { commands, style } => {
            apply_style(style, out);
            emit_path(commands, out);
            paint(style, out);
        }
    }
}

fn apply_style(style: &SvgStyle, out: &mut String) {
    // Fill color
    if let Some((r, g, b)) = style.fill {
        out.push_str(&format!("{r} {g} {b} rg\n"));
    }
    // Stroke color
    if let Some((r, g, b)) = style.stroke {
        out.push_str(&format!("{r} {g} {b} RG\n"));
    }
    // Stroke width
    if style.stroke_width > 0.0 {
        out.push_str(&format!("{} w\n", style.stroke_width));
    }
}

fn paint(style: &SvgStyle, out: &mut String) {
    let has_fill = style.fill.is_some();
    let has_stroke = style.stroke.is_some() && style.stroke_width > 0.0;
    match (has_fill, has_stroke) {
        (true, true) => out.push_str("B\n"),   // fill + stroke
        (true, false) => out.push_str("f\n"),  // fill only
        (false, true) => out.push_str("S\n"),  // stroke only
        (false, false) => out.push_str("n\n"), // no paint
    }
}

// Emit a circle approximation using 4 cubic bezier curves
fn emit_circle(cx: f32, cy: f32, r: f32, out: &mut String) {
    emit_ellipse(cx, cy, r, r, out);
}

fn emit_ellipse(cx: f32, cy: f32, rx: f32, ry: f32, out: &mut String) {
    let k = 0.552_284_8_f32;
    let kx = rx * k;
    let ky = ry * k;
    // Start at (cx+rx, cy)
    out.push_str(&format!("{} {} m\n", cx + rx, cy));
    // Top-right quadrant
    out.push_str(&format!(
        "{} {} {} {} {} {} c\n",
        cx + rx,
        cy + ky,
        cx + kx,
        cy + ry,
        cx,
        cy + ry
    ));
    // Top-left quadrant
    out.push_str(&format!(
        "{} {} {} {} {} {} c\n",
        cx - kx,
        cy + ry,
        cx - rx,
        cy + ky,
        cx - rx,
        cy
    ));
    // Bottom-left quadrant
    out.push_str(&format!(
        "{} {} {} {} {} {} c\n",
        cx - rx,
        cy - ky,
        cx - kx,
        cy - ry,
        cx,
        cy - ry
    ));
    // Bottom-right quadrant
    out.push_str(&format!(
        "{} {} {} {} {} {} c\n",
        cx + kx,
        cy - ry,
        cx + rx,
        cy - ky,
        cx + rx,
        cy
    ));
    out.push_str("h\n"); // close path
}

fn emit_polyline(points: &[(f32, f32)], close: bool, out: &mut String) {
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            out.push_str(&format!("{x} {y} m\n"));
        } else {
            out.push_str(&format!("{x} {y} l\n"));
        }
    }
    if close {
        out.push_str("h\n");
    }
}

fn emit_path(commands: &[PathCommand], out: &mut String) {
    for cmd in commands {
        match cmd {
            PathCommand::MoveTo(x, y) => out.push_str(&format!("{x} {y} m\n")),
            PathCommand::LineTo(x, y) => out.push_str(&format!("{x} {y} l\n")),
            PathCommand::CubicTo(x1, y1, x2, y2, x, y) => {
                out.push_str(&format!("{x1} {y1} {x2} {y2} {x} {y} c\n"));
            }
            PathCommand::QuadTo(_cx, _cy, x, y) => {
                // Convert quadratic to cubic bezier (would need current point tracking)
                // For simplicity, approximate as line to endpoint
                out.push_str(&format!("{x} {y} l\n"));
            }
            PathCommand::ClosePath => out.push_str("h\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::svg::{PathCommand, SvgNode, SvgStyle, SvgTransform, SvgTree};

    fn style_fill(r: f32, g: f32, b: f32) -> SvgStyle {
        SvgStyle {
            fill: Some((r, g, b)),
            stroke: None,
            stroke_width: 0.0,
            opacity: 1.0,
        }
    }

    fn style_stroke(r: f32, g: f32, b: f32, w: f32) -> SvgStyle {
        SvgStyle {
            fill: None,
            stroke: Some((r, g, b)),
            stroke_width: w,
            opacity: 1.0,
        }
    }

    fn style_fill_and_stroke() -> SvgStyle {
        SvgStyle {
            fill: Some((1.0, 0.0, 0.0)),
            stroke: Some((0.0, 0.0, 1.0)),
            stroke_width: 2.0,
            opacity: 1.0,
        }
    }

    fn style_none() -> SvgStyle {
        SvgStyle {
            fill: None,
            stroke: None,
            stroke_width: 0.0,
            opacity: 1.0,
        }
    }

    fn tree_with(children: Vec<SvgNode>) -> SvgTree {
        SvgTree {
            width: 100.0,
            height: 100.0,
            view_box: None,
            children,
        }
    }

    // ---- Rect tests ----

    #[test]
    fn render_rect_with_fill() {
        let tree = tree_with(vec![SvgNode::Rect {
            x: 10.0,
            y: 20.0,
            width: 80.0,
            height: 60.0,
            rx: 0.0,
            ry: 0.0,
            style: style_fill(1.0, 0.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 0 0 rg\n"), "should set red fill");
        assert!(out.contains("10 20 80 60 re\n"), "should emit rect");
        assert!(out.contains("f\n"), "should paint fill only");
    }

    #[test]
    fn render_rect_with_stroke_only() {
        let tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
            rx: 0.0,
            ry: 0.0,
            style: style_stroke(0.0, 1.0, 0.0, 3.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 1 0 RG\n"), "should set green stroke");
        assert!(out.contains("3 w\n"), "should set stroke width");
        assert!(out.contains("0 0 50 50 re\n"), "should emit rect");
        assert!(out.contains("S\n"), "should paint stroke only");
    }

    #[test]
    fn render_rect_fill_and_stroke() {
        let tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
            rx: 0.0,
            ry: 0.0,
            style: style_fill_and_stroke(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 0 0 rg\n"), "should set fill color");
        assert!(out.contains("0 0 1 RG\n"), "should set stroke color");
        assert!(out.contains("2 w\n"), "should set stroke width");
        assert!(out.contains("B\n"), "should paint fill+stroke");
    }

    #[test]
    fn render_rect_no_paint() {
        let tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rx: 0.0,
            ry: 0.0,
            style: style_none(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("n\n"), "should emit no-paint operator");
    }

    // ---- Circle tests ----

    #[test]
    fn render_circle_with_fill() {
        let tree = tree_with(vec![SvgNode::Circle {
            cx: 50.0,
            cy: 50.0,
            r: 25.0,
            style: style_fill(0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 1 rg\n"), "should set blue fill");
        // Circle start point: cx+r = 75, cy = 50
        assert!(out.contains("75 50 m\n"), "should move to circle start");
        // Should have 4 cubic bezier curves
        assert_eq!(out.matches(" c\n").count(), 4, "should have 4 cubic curves");
        assert!(out.contains("h\n"), "should close path");
        assert!(out.contains("f\n"), "should paint fill");
    }

    #[test]
    fn render_circle_with_stroke() {
        let tree = tree_with(vec![SvgNode::Circle {
            cx: 50.0,
            cy: 50.0,
            r: 10.0,
            style: style_stroke(1.0, 0.0, 0.0, 1.5),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 0 0 RG\n"), "should set stroke color");
        assert!(out.contains("1.5 w\n"), "should set stroke width");
        assert!(out.contains("S\n"), "should stroke only");
    }

    // ---- Ellipse tests ----

    #[test]
    fn render_ellipse_with_fill() {
        let tree = tree_with(vec![SvgNode::Ellipse {
            cx: 50.0,
            cy: 50.0,
            rx: 30.0,
            ry: 20.0,
            style: style_fill(0.0, 1.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 1 0 rg\n"), "should set green fill");
        // Ellipse start point: cx+rx = 80, cy = 50
        assert!(out.contains("80 50 m\n"), "should move to ellipse start");
        assert_eq!(out.matches(" c\n").count(), 4, "should have 4 cubic curves");
        assert!(out.contains("h\n"), "should close path");
        assert!(out.contains("f\n"), "should paint fill");
    }

    #[test]
    fn render_ellipse_fill_and_stroke() {
        let tree = tree_with(vec![SvgNode::Ellipse {
            cx: 0.0,
            cy: 0.0,
            rx: 10.0,
            ry: 5.0,
            style: style_fill_and_stroke(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("B\n"), "should paint fill+stroke");
    }

    // ---- Line tests ----

    #[test]
    fn render_line() {
        let tree = tree_with(vec![SvgNode::Line {
            x1: 0.0,
            y1: 0.0,
            x2: 100.0,
            y2: 100.0,
            style: style_stroke(0.0, 0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("0 0 m 100 100 l S\n"),
            "should emit line with stroke"
        );
    }

    #[test]
    fn render_line_with_fill_style() {
        // Lines use apply_style but always stroke via the inline S operator
        let tree = tree_with(vec![SvgNode::Line {
            x1: 5.0,
            y1: 10.0,
            x2: 50.0,
            y2: 60.0,
            style: style_fill(1.0, 1.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 1 0 rg\n"), "should apply fill style");
        assert!(out.contains("5 10 m 50 60 l S\n"), "should emit line");
    }

    // ---- Polyline tests ----

    #[test]
    fn render_polyline() {
        let tree = tree_with(vec![SvgNode::Polyline {
            points: vec![(0.0, 0.0), (10.0, 20.0), (30.0, 40.0)],
            style: style_stroke(1.0, 0.0, 0.0, 2.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 m\n"), "first point should be moveto");
        assert!(out.contains("10 20 l\n"), "second point should be lineto");
        assert!(out.contains("30 40 l\n"), "third point should be lineto");
        assert!(!out.contains("h\n"), "polyline should not close path");
        assert!(out.contains("S\n"), "polyline should stroke");
    }

    #[test]
    fn render_polyline_empty() {
        let tree = tree_with(vec![SvgNode::Polyline {
            points: vec![],
            style: style_stroke(0.0, 0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        // Should still emit S even with no points
        assert!(out.contains("S\n"));
    }

    // ---- Polygon tests ----

    #[test]
    fn render_polygon_with_fill() {
        let tree = tree_with(vec![SvgNode::Polygon {
            points: vec![(0.0, 0.0), (50.0, 0.0), (25.0, 50.0)],
            style: style_fill(0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 m\n"), "first point should be moveto");
        assert!(out.contains("50 0 l\n"), "second point should be lineto");
        assert!(out.contains("25 50 l\n"), "third point should be lineto");
        assert!(out.contains("h\n"), "polygon should close path");
        assert!(out.contains("f\n"), "polygon should paint fill");
    }

    #[test]
    fn render_polygon_fill_and_stroke() {
        let tree = tree_with(vec![SvgNode::Polygon {
            points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
            style: style_fill_and_stroke(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("h\n"), "polygon should close path");
        assert!(out.contains("B\n"), "should paint fill+stroke");
    }

    // ---- Path tests ----

    #[test]
    fn render_path_moveto_lineto() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 10.0),
            ],
            style: style_fill(1.0, 0.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 m\n"), "should emit moveto");
        assert!(out.contains("10 10 l\n"), "should emit lineto");
        assert!(out.contains("f\n"), "should paint fill");
    }

    #[test]
    fn render_path_cubic_to() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::CubicTo(1.0, 2.0, 3.0, 4.0, 5.0, 6.0),
            ],
            style: style_stroke(0.0, 0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("1 2 3 4 5 6 c\n"), "should emit cubic bezier");
        assert!(out.contains("S\n"), "should stroke");
    }

    #[test]
    fn render_path_quad_to() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::QuadTo(5.0, 5.0, 10.0, 10.0),
            ],
            style: style_fill(0.0, 1.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        // QuadTo is approximated as lineto to endpoint
        assert!(
            out.contains("10 10 l\n"),
            "QuadTo should approximate as lineto"
        );
    }

    #[test]
    fn render_path_close() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 0.0),
                PathCommand::LineTo(10.0, 10.0),
                PathCommand::ClosePath,
            ],
            style: style_fill_and_stroke(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("h\n"), "should emit close path");
        assert!(out.contains("B\n"), "should paint fill+stroke");
    }

    #[test]
    fn render_path_all_commands() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 0.0),
                PathCommand::CubicTo(20.0, 0.0, 20.0, 10.0, 10.0, 10.0),
                PathCommand::QuadTo(5.0, 15.0, 0.0, 10.0),
                PathCommand::ClosePath,
            ],
            style: style_fill(0.5, 0.5, 0.5),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 m\n"));
        assert!(out.contains("10 0 l\n"));
        assert!(out.contains("20 0 20 10 10 10 c\n"));
        assert!(out.contains("0 10 l\n")); // QuadTo approximated
        assert!(out.contains("h\n"));
        assert!(out.contains("f\n"));
    }

    // ---- Group tests ----

    #[test]
    fn render_group_without_transform() {
        let tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(1.0, 0.0, 0.0),
            }],
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.starts_with("q\n"), "should save graphics state");
        assert!(out.contains("0 0 10 10 re\n"), "should render child rect");
        assert!(out.ends_with("Q\n"), "should restore graphics state");
        // No cm operator since no transform
        assert!(
            !out.contains(" cm\n"),
            "should not have cm without transform"
        );
    }

    #[test]
    fn render_group_with_transform() {
        let tree = tree_with(vec![SvgNode::Group {
            transform: Some(SvgTransform::Matrix(1.0, 0.0, 0.0, 1.0, 10.0, 20.0)),
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 5.0,
                height: 5.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(0.0, 0.0, 0.0),
            }],
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("q\n"), "should save state");
        assert!(out.contains("1 0 0 1 10 20 cm\n"), "should apply transform");
        assert!(out.contains("0 0 5 5 re\n"), "should render child");
        assert!(out.contains("Q\n"), "should restore state");
    }

    #[test]
    fn render_nested_groups() {
        let tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Group {
                transform: Some(SvgTransform::Matrix(2.0, 0.0, 0.0, 2.0, 0.0, 0.0)),
                children: vec![SvgNode::Circle {
                    cx: 10.0,
                    cy: 10.0,
                    r: 5.0,
                    style: style_fill(1.0, 1.0, 0.0),
                }],
                style: SvgStyle::default(),
            }],
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        // Should have two q/Q pairs (nested groups)
        assert_eq!(out.matches("q\n").count(), 2, "two nested save states");
        assert_eq!(out.matches("Q\n").count(), 2, "two nested restore states");
        assert!(out.contains("2 0 0 2 0 0 cm\n"), "inner transform");
    }

    // ---- Empty tree ----

    #[test]
    fn render_empty_tree() {
        let tree = tree_with(vec![]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.is_empty(), "empty tree should produce no output");
    }

    // ---- Multiple children ----

    #[test]
    fn render_multiple_children() {
        let tree = tree_with(vec![
            SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: style_fill(1.0, 0.0, 0.0),
            },
            SvgNode::Circle {
                cx: 50.0,
                cy: 50.0,
                r: 10.0,
                style: style_fill(0.0, 1.0, 0.0),
            },
        ]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 10 10 re\n"), "should render rect");
        assert!(out.contains("60 50 m\n"), "should render circle start");
    }

    // ---- apply_style edge cases ----

    #[test]
    fn apply_style_stroke_with_zero_width_not_emitted_in_paint() {
        // stroke is Some but stroke_width is 0 => paint treats as no stroke
        let tree = tree_with(vec![SvgNode::Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rx: 0.0,
            ry: 0.0,
            style: SvgStyle {
                fill: Some((1.0, 0.0, 0.0)),
                stroke: Some((0.0, 0.0, 0.0)),
                stroke_width: 0.0,
                opacity: 1.0,
            },
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        // stroke color is emitted by apply_style (it doesn't check width)
        assert!(out.contains("0 0 0 RG\n"), "stroke color still applied");
        // but paint should be fill-only because stroke_width is 0
        assert!(out.contains("f\n"), "paint should be fill only");
        assert!(!out.contains("B\n"), "should not be fill+stroke");
    }

    // ---- paint edge: stroke present but no fill ----

    #[test]
    fn paint_stroke_only_no_fill() {
        let tree = tree_with(vec![SvgNode::Path {
            commands: vec![
                PathCommand::MoveTo(0.0, 0.0),
                PathCommand::LineTo(10.0, 10.0),
            ],
            style: style_stroke(0.0, 0.0, 0.0, 1.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("S\n"), "should stroke only");
        assert!(!out.contains("f\n"), "should not fill");
        assert!(!out.contains("B\n"), "should not fill+stroke");
    }

    // ---- paint edge: neither fill nor stroke ----

    #[test]
    fn paint_no_fill_no_stroke() {
        let tree = tree_with(vec![SvgNode::Ellipse {
            cx: 0.0,
            cy: 0.0,
            rx: 10.0,
            ry: 10.0,
            style: style_none(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("n\n"), "should emit no-paint");
    }
}
