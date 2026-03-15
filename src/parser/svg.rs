//! SVG parser — converts DOM SVG elements into an SvgTree for PDF rendering.

use crate::parser::dom::ElementNode;

/// A parsed SVG tree ready for rendering.
#[derive(Debug, Clone)]
pub struct SvgTree {
    pub width: f32,
    pub height: f32,
    pub view_box: Option<ViewBox>,
    pub children: Vec<SvgNode>,
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
}

#[derive(Debug, Clone, Default)]
pub struct SvgStyle {
    pub fill: Option<(f32, f32, f32)>,   // RGB 0.0-1.0, None = no fill
    pub stroke: Option<(f32, f32, f32)>, // RGB 0.0-1.0, None = no stroke
    pub stroke_width: f32,
    pub opacity: f32,
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
    let width = el
        .attributes
        .get("width")
        .and_then(|v| parse_length(v))
        .unwrap_or(300.0);
    let height = el
        .attributes
        .get("height")
        .and_then(|v| parse_length(v))
        .unwrap_or(150.0);
    let view_box = el.attributes.get("viewBox").and_then(|v| parse_viewbox(v));

    let mut children = Vec::new();
    for child in &el.children {
        if let crate::parser::dom::DomNode::Element(child_el) = child {
            if let Some(node) = parse_svg_node(child_el) {
                children.push(node);
            }
        }
    }

    Some(SvgTree {
        width,
        height,
        view_box,
        children,
    })
}

/// Parse a single SVG element node into an SvgNode.
fn parse_svg_node(el: &ElementNode) -> Option<SvgNode> {
    let tag = el.raw_tag_name.as_str();
    match tag {
        "g" | "svg" => {
            let transform = el
                .attributes
                .get("transform")
                .and_then(|v| parse_transform(v));
            let style = parse_svg_style(el);
            let mut children = Vec::new();
            for child in &el.children {
                if let crate::parser::dom::DomNode::Element(child_el) = child {
                    if let Some(node) = parse_svg_node(child_el) {
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
        _ => None,
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
fn parse_length(val: &str) -> Option<f32> {
    let trimmed = val.trim();
    let num_str = trimmed.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
    num_str.trim().parse::<f32>().ok()
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
    let fill = el.attributes.get("fill").and_then(|v| parse_svg_color(v));
    let stroke = el.attributes.get("stroke").and_then(|v| parse_svg_color(v));
    let stroke_width = el
        .attributes
        .get("stroke-width")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    let opacity = el
        .attributes
        .get("opacity")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);

    SvgStyle {
        fill,
        stroke,
        stroke_width,
        opacity,
    }
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
}
