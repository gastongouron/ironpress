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
