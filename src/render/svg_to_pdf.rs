//! SVG tree to PDF content stream renderer.

use crate::parser::svg::{
    PathCommand, SvgNode, SvgPaint, SvgStyle, SvgTextContext, SvgTransform, SvgTree,
};
use crate::render::pdf::encode_pdf_text;

/// Render an SVG tree to PDF content stream operators.
///
/// The caller must wrap this in a `q ... Q` block and set up the coordinate
/// transform (position on page + y-axis flip).
pub fn render_svg_tree(tree: &SvgTree, out: &mut String) {
    // SVG initial values: fill=black, stroke=none, stroke-width=1.
    let root_style = ResolvedStyle {
        fill: SvgPaint::Color((0.0, 0.0, 0.0)),
        stroke: SvgPaint::None,
        stroke_width: 1.0,
    };
    for node in &tree.children {
        render_node(node, root_style, &tree.text_ctx, out);
    }
}

#[derive(Debug, Clone, Copy)]
struct ResolvedStyle {
    fill: SvgPaint,
    stroke: SvgPaint,
    stroke_width: f32,
}

fn resolve_style(parent: ResolvedStyle, local: &SvgStyle) -> ResolvedStyle {
    let fill = match local.fill {
        SvgPaint::Unspecified => parent.fill,
        other => other,
    };
    let stroke = match local.stroke {
        SvgPaint::Unspecified => parent.stroke,
        other => other,
    };
    let stroke_width = local.stroke_width.unwrap_or(parent.stroke_width);
    ResolvedStyle {
        fill,
        stroke,
        stroke_width,
    }
}

fn paint_to_rgb(paint: SvgPaint, text_ctx: &SvgTextContext) -> Option<(f32, f32, f32)> {
    match paint {
        SvgPaint::None => None,
        SvgPaint::Color(c) => Some(c),
        SvgPaint::CurrentColor => Some(text_ctx.color.unwrap_or((0.0, 0.0, 0.0))),
        SvgPaint::Unspecified => None,
    }
}

fn render_node(
    node: &SvgNode,
    inherited: ResolvedStyle,
    text_ctx: &SvgTextContext,
    out: &mut String,
) {
    match node {
        SvgNode::Group {
            transform,
            children,
            style,
            ..
        } => {
            let inherited = resolve_style(inherited, style);
            out.push_str("q\n");
            if let Some(SvgTransform::Matrix(a, b, c, d, e, f)) = transform {
                out.push_str(&format!("{a} {b} {c} {d} {e} {f} cm\n"));
            }
            for child in children {
                render_node(child, inherited, text_ctx, out);
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
            let style = resolve_style(inherited, style);
            apply_style(style, text_ctx, out);
            out.push_str(&format!("{x} {y} {width} {height} re\n"));
            paint(style, text_ctx, out);
        }
        SvgNode::Circle { cx, cy, r, style } => {
            let style = resolve_style(inherited, style);
            apply_style(style, text_ctx, out);
            emit_circle(*cx, *cy, *r, out);
            paint(style, text_ctx, out);
        }
        SvgNode::Ellipse {
            cx,
            cy,
            rx,
            ry,
            style,
        } => {
            let style = resolve_style(inherited, style);
            apply_style(style, text_ctx, out);
            emit_ellipse(*cx, *cy, *rx, *ry, out);
            paint(style, text_ctx, out);
        }
        SvgNode::Line {
            x1,
            y1,
            x2,
            y2,
            style,
        } => {
            let style = resolve_style(inherited, style);
            apply_stroke_style(style, text_ctx, out);
            out.push_str(&format!("{x1} {y1} m {x2} {y2} l\n"));
            paint_stroke_only(style, text_ctx, out);
        }
        SvgNode::Polyline { points, style } => {
            let style = resolve_style(inherited, style);
            apply_stroke_style(style, text_ctx, out);
            emit_polyline(points, false, out);
            paint_stroke_only(style, text_ctx, out);
        }
        SvgNode::Polygon { points, style } => {
            let style = resolve_style(inherited, style);
            apply_style(style, text_ctx, out);
            emit_polyline(points, true, out);
            paint(style, text_ctx, out);
        }
        SvgNode::Path { commands, style } => {
            let style = resolve_style(inherited, style);
            apply_style(style, text_ctx, out);
            emit_path(commands, out);
            paint(style, text_ctx, out);
        }
        SvgNode::Text {
            x,
            y,
            font_size,
            font_size_attr,
            fill_specified: _fill_specified,
            fill_raw: _fill_raw,
            font_family,
            font_bold,
            font_italic,
            content,
            style,
        } => {
            let style = resolve_style(inherited, style);
            // Use SVG-explicit font_size if set, otherwise inherit from CSS context
            let size = font_size_attr
                .as_deref()
                .and_then(|raw| resolve_svg_font_size(raw, text_ctx.font_size))
                .or(*font_size)
                .unwrap_or(text_ctx.font_size);
            let fill = paint_to_rgb(style.fill, text_ctx);
            let stroke = paint_to_rgb(style.stroke, text_ctx).filter(|_| style.stroke_width > 0.0);
            // Use per-element font if specified, falling back to inherited CSS font
            let font =
                resolve_svg_text_font(font_family.as_deref(), *font_bold, *font_italic, text_ctx);
            let text_render_mode = match (fill.is_some(), stroke.is_some()) {
                (true, true) => 2,
                (true, false) => 0,
                (false, true) => 1,
                (false, false) => 3,
            };

            out.push_str("BT\n");
            out.push_str(&format!("/{font} {size} Tf\n"));
            out.push_str(&format!("{text_render_mode} Tr\n"));
            if let Some((r, g, b)) = fill {
                out.push_str(&format!("{r} {g} {b} rg\n"));
            }
            if let Some((r, g, b)) = stroke {
                out.push_str(&format!("{r} {g} {b} RG\n"));
                out.push_str(&format!("{} w\n", style.stroke_width));
            }
            // The parent SVG content stream has a y-flip (scale 1,-1) to convert
            // SVG coordinates to PDF coordinates. Text must counter this flip via
            // the text matrix, otherwise glyphs render upside-down.
            out.push_str(&format!("1 0 0 -1 {x} {y} Tm\n"));
            let encoded = encode_pdf_text(content);
            out.push_str(&format!("({encoded}) Tj\n"));
            out.push_str("ET\n");
        }
    }
}

fn resolve_svg_font_size(raw: &str, inherited_size: f32) -> Option<f32> {
    let raw = raw.trim();
    if let Some(pct) = raw.strip_suffix('%') {
        let pct = pct.trim().parse::<f32>().ok()?;
        return Some(inherited_size * pct / 100.0);
    }
    if let Some(em) = raw.strip_suffix("em") {
        let em = em.trim().parse::<f32>().ok()?;
        return Some(inherited_size * em);
    }
    if let Some(px) = raw.strip_suffix("px") {
        let px = px.trim().parse::<f32>().ok()?;
        return Some(px * 0.75);
    }
    if let Some(pt) = raw.strip_suffix("pt") {
        return pt.trim().parse::<f32>().ok();
    }
    raw.parse::<f32>().ok().map(|px| px * 0.75)
}

fn apply_style(style: ResolvedStyle, text_ctx: &SvgTextContext, out: &mut String) {
    // Fill color
    if let Some((r, g, b)) = paint_to_rgb(style.fill, text_ctx) {
        out.push_str(&format!("{r} {g} {b} rg\n"));
    }
    apply_stroke_style(style, text_ctx, out);
}

fn apply_stroke_style(style: ResolvedStyle, text_ctx: &SvgTextContext, out: &mut String) {
    if let Some((r, g, b)) = paint_to_rgb(style.stroke, text_ctx) {
        out.push_str(&format!("{r} {g} {b} RG\n"));
    }
    if style.stroke_width > 0.0 {
        out.push_str(&format!("{} w\n", style.stroke_width));
    }
}

fn paint(style: ResolvedStyle, text_ctx: &SvgTextContext, out: &mut String) {
    let has_fill = paint_to_rgb(style.fill, text_ctx).is_some();
    let has_stroke = paint_to_rgb(style.stroke, text_ctx).is_some() && style.stroke_width > 0.0;
    match (has_fill, has_stroke) {
        (true, true) => out.push_str("B\n"),   // fill + stroke
        (true, false) => out.push_str("f\n"),  // fill only
        (false, true) => out.push_str("S\n"),  // stroke only
        (false, false) => out.push_str("n\n"), // no paint
    }
}

fn paint_stroke_only(style: ResolvedStyle, text_ctx: &SvgTextContext, out: &mut String) {
    let has_stroke = paint_to_rgb(style.stroke, text_ctx).is_some() && style.stroke_width > 0.0;
    if has_stroke {
        out.push_str("S\n");
    } else {
        out.push_str("n\n");
    }
}

/// Resolve the PDF font name for an SVG `<text>` element.
///
/// If the element has a per-element `font_family` override, combine it with
/// bold/italic flags (falling back to the context flags when the element
/// doesn't specify them).  Otherwise use the context font as-is.
fn resolve_svg_text_font(
    font_family: Option<&str>,
    font_bold: Option<bool>,
    font_italic: Option<bool>,
    text_ctx: &SvgTextContext,
) -> String {
    if let Some(base) = font_family {
        let bold = font_bold.unwrap_or(text_ctx.font_bold);
        let italic = font_italic.unwrap_or(text_ctx.font_italic);
        pdf_font_name(base, bold, italic).to_string()
    } else if font_bold.is_some() || font_italic.is_some() {
        // No family override but bold/italic overrides -- derive from context family base name
        let base = base_family_from_pdf_name(&text_ctx.font_family);
        let bold = font_bold.unwrap_or(text_ctx.font_bold);
        let italic = font_italic.unwrap_or(text_ctx.font_italic);
        pdf_font_name(base, bold, italic).to_string()
    } else {
        text_ctx.font_family.clone()
    }
}

/// Extract the base family name from a fully-qualified PDF font name.
fn base_family_from_pdf_name(name: &str) -> &str {
    if name.starts_with("Times") {
        "Times-Roman"
    } else if name.starts_with("Courier") {
        "Courier"
    } else {
        "Helvetica"
    }
}

/// Map (base_family, bold, italic) to a concrete PDF built-in font name.
fn pdf_font_name(base: &str, bold: bool, italic: bool) -> &'static str {
    if base.starts_with("Times") {
        match (bold, italic) {
            (true, true) => "Times-BoldItalic",
            (true, false) => "Times-Bold",
            (false, true) => "Times-Italic",
            (false, false) => "Times-Roman",
        }
    } else if base.starts_with("Courier") {
        match (bold, italic) {
            (true, true) => "Courier-BoldOblique",
            (true, false) => "Courier-Bold",
            (false, true) => "Courier-Oblique",
            (false, false) => "Courier",
        }
    } else {
        match (bold, italic) {
            (true, true) => "Helvetica-BoldOblique",
            (true, false) => "Helvetica-Bold",
            (false, true) => "Helvetica-Oblique",
            (false, false) => "Helvetica",
        }
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
    use crate::parser::svg::{
        PathCommand, SvgNode, SvgPaint, SvgStyle, SvgTextContext, SvgTransform, SvgTree,
    };

    fn style_fill(r: f32, g: f32, b: f32) -> SvgStyle {
        SvgStyle {
            fill: SvgPaint::Color((r, g, b)),
            stroke: SvgPaint::Unspecified,
            stroke_width: None,
            opacity: 1.0,
        }
    }

    fn style_stroke(r: f32, g: f32, b: f32, w: f32) -> SvgStyle {
        SvgStyle {
            fill: SvgPaint::None,
            stroke: SvgPaint::Color((r, g, b)),
            stroke_width: Some(w),
            opacity: 1.0,
        }
    }

    fn style_fill_and_stroke() -> SvgStyle {
        SvgStyle {
            fill: SvgPaint::Color((1.0, 0.0, 0.0)),
            stroke: SvgPaint::Color((0.0, 0.0, 1.0)),
            stroke_width: Some(2.0),
            opacity: 1.0,
        }
    }

    fn style_none() -> SvgStyle {
        SvgStyle {
            fill: SvgPaint::None,
            stroke: SvgPaint::None,
            stroke_width: None,
            opacity: 1.0,
        }
    }

    fn tree_with(children: Vec<SvgNode>) -> SvgTree {
        SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            view_box: None,
            children,
            text_ctx: SvgTextContext::default(),
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
            out.contains("0 0 m 100 100 l\nS\n"),
            "should emit line with stroke"
        );
    }

    #[test]
    fn render_line_with_fill_style() {
        // Fill does not apply to <line>; without a stroke, the line is not painted.
        let tree = tree_with(vec![SvgNode::Line {
            x1: 5.0,
            y1: 10.0,
            x2: 50.0,
            y2: 60.0,
            style: style_fill(1.0, 1.0, 0.0),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            !out.contains(" rg\n"),
            "should not set fill color for <line>"
        );
        assert!(out.contains("5 10 m 50 60 l\n"), "should emit line path");
        assert!(
            out.contains("n\n"),
            "should not stroke without a stroke paint"
        );
    }

    #[test]
    fn render_line_without_stroke_is_not_painted() {
        let tree = tree_with(vec![SvgNode::Line {
            x1: 0.0,
            y1: 0.0,
            x2: 10.0,
            y2: 10.0,
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("n\n"));
        assert!(!out.contains("S\n"));
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

    #[test]
    fn render_polyline_without_stroke_is_not_painted() {
        let tree = tree_with(vec![SvgNode::Polyline {
            points: vec![(0.0, 0.0), (10.0, 10.0)],
            style: SvgStyle::default(),
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("n\n"));
        assert!(!out.contains("S\n"));
    }

    #[test]
    fn group_fill_is_inherited_by_children() {
        let tree = tree_with(vec![SvgNode::Group {
            transform: None,
            children: vec![SvgNode::Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
                style: SvgStyle::default(),
            }],
            style: SvgStyle {
                fill: SvgPaint::Color((1.0, 0.0, 0.0)),
                ..SvgStyle::default()
            },
        }]);
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("1 0 0 rg\n"),
            "child should inherit group fill"
        );
        assert!(out.contains("f\n"), "rect should be filled");
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
                fill: SvgPaint::Color((1.0, 0.0, 0.0)),
                stroke: SvgPaint::Color((0.0, 0.0, 0.0)),
                stroke_width: Some(0.0),
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

    #[test]
    fn text_fill_none_does_not_fallback_to_context_color() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            view_box: None,
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: true,
                fill_raw: Some("none".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    fill: SvgPaint::None,
                    stroke: SvgPaint::Unspecified,
                    stroke_width: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext {
                color: Some((1.0, 0.0, 0.0)),
                ..SvgTextContext::default()
            },
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("3 Tr\n"),
            "fill:none should disable text painting"
        );
        assert!(
            !out.contains(" rg\n"),
            "explicit fill:none should not fall back to inherited text color"
        );
        assert!(out.contains("(Hello) Tj\n"));
    }

    #[test]
    fn text_fill_none_with_stroke_renders_stroked_glyphs() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            view_box: None,
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: true,
                fill_raw: Some("none".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    fill: SvgPaint::None,
                    stroke: SvgPaint::Color((1.0, 0.0, 0.0)),
                    stroke_width: Some(1.5),
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext::default(),
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("1 Tr\n"),
            "stroke-only text should use stroke render mode"
        );
        assert!(
            out.contains("1 0 0 RG\n"),
            "stroke-only text should set the stroke color"
        );
        assert!(
            out.contains("1.5 w\n"),
            "stroke-only text should set the stroke width"
        );
        assert!(
            !out.contains("3 Tr\n"),
            "stroke-only text must not be invisible"
        );
    }

    #[test]
    fn text_fill_defaults_to_black_when_unspecified() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            view_box: None,
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: false,
                fill_raw: None,
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle::default(),
            }],
            text_ctx: SvgTextContext {
                color: Some((1.0, 0.0, 0.0)),
                ..SvgTextContext::default()
            },
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("0 0 0 rg\n"),
            "unspecified SVG text fill should default to black"
        );
    }

    #[test]
    fn text_font_size_percent_scales_from_context() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            view_box: None,
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: Some("150%".to_string()),
                fill_specified: true,
                fill_raw: Some("currentColor".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    fill: SvgPaint::Color((0.0, 0.0, 0.0)),
                    stroke: SvgPaint::Unspecified,
                    stroke_width: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext {
                font_size: 12.0,
                ..SvgTextContext::default()
            },
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("/Helvetica 18 Tf\n"),
            "150% font-size should resolve from the inherited SVG text size"
        );
    }

    #[test]
    fn text_font_size_unitless_number_treated_as_px() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            view_box: None,
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: Some("12".to_string()),
                fill_specified: true,
                fill_raw: Some("currentColor".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    fill: SvgPaint::Color((0.0, 0.0, 0.0)),
                    stroke: SvgPaint::Unspecified,
                    stroke_width: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext::default(),
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(
            out.contains("/Helvetica 9 Tf\n"),
            "unitless SVG font-size should resolve like px"
        );
    }

    #[test]
    fn text_fill_current_color_uses_context_color() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            view_box: None,
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: true,
                fill_raw: Some("currentColor".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle {
                    fill: SvgPaint::CurrentColor,
                    stroke: SvgPaint::Unspecified,
                    stroke_width: None,
                    opacity: 1.0,
                },
            }],
            text_ctx: SvgTextContext {
                color: Some((0.0, 0.5, 1.0)),
                ..SvgTextContext::default()
            },
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0.5 1 rg\n"));
    }

    #[test]
    fn text_invalid_fill_defaults_to_black() {
        let tree = SvgTree {
            width: 100.0,
            height: 100.0,
            width_attr: None,
            height_attr: None,
            view_box: None,
            children: vec![SvgNode::Text {
                x: 10.0,
                y: 20.0,
                font_size: None,
                font_size_attr: None,
                fill_specified: true,
                fill_raw: Some("bogus".to_string()),
                font_family: None,
                font_bold: None,
                font_italic: None,
                content: "Hello".to_string(),
                style: SvgStyle::default(),
            }],
            text_ctx: SvgTextContext {
                color: Some((1.0, 0.0, 0.0)),
                ..SvgTextContext::default()
            },
        };
        let mut out = String::new();
        render_svg_tree(&tree, &mut out);
        assert!(out.contains("0 0 0 rg\n"));
    }
}
