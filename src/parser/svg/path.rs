use super::PathCommand;

/// Parse SVG path `d` attribute data into PathCommands.
/// Supports: M/m, L/l, H/h, V/v, C/c, S/s, Q/q, T/t, Z/z.
pub fn parse_path_data(d: &str) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    let mut cur_x = 0.0;
    let mut cur_y = 0.0;
    let mut last_ctrl_x = 0.0;
    let mut last_ctrl_y = 0.0;
    let mut last_cmd = ' ';

    let tokens = tokenize_path(d);
    let mut index = 0;

    while index < tokens.len() {
        let token = &tokens[index];
        let command = match token.as_bytes() {
            [byte] if byte.is_ascii_alphabetic() => {
                index += 1;
                *byte as char
            }
            _ => match last_cmd {
                'M' => 'L',
                'm' => 'l',
                other => other,
            },
        };

        match command {
            'M' => {
                if let Some((x, y)) = read_pair(&tokens, &mut index) {
                    cur_x = x;
                    cur_y = y;
                    commands.push(PathCommand::MoveTo(cur_x, cur_y));
                    last_cmd = 'M';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'm' => {
                if let Some((dx, dy)) = read_pair(&tokens, &mut index) {
                    cur_x += dx;
                    cur_y += dy;
                    commands.push(PathCommand::MoveTo(cur_x, cur_y));
                    last_cmd = 'm';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'L' => {
                if let Some((x, y)) = read_pair(&tokens, &mut index) {
                    cur_x = x;
                    cur_y = y;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'L';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'l' => {
                if let Some((dx, dy)) = read_pair(&tokens, &mut index) {
                    cur_x += dx;
                    cur_y += dy;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'l';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'H' => {
                if let Some(x) = read_number(&tokens, &mut index) {
                    cur_x = x;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'H';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'h' => {
                if let Some(dx) = read_number(&tokens, &mut index) {
                    cur_x += dx;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'h';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'V' => {
                if let Some(y) = read_number(&tokens, &mut index) {
                    cur_y = y;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'V';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'v' => {
                if let Some(dy) = read_number(&tokens, &mut index) {
                    cur_y += dy;
                    commands.push(PathCommand::LineTo(cur_x, cur_y));
                    last_cmd = 'v';
                    last_ctrl_x = cur_x;
                    last_ctrl_y = cur_y;
                }
            }
            'C' => {
                if let Some((x1, y1, x2, y2, x, y)) = read_six(&tokens, &mut index) {
                    commands.push(PathCommand::CubicTo(x1, y1, x2, y2, x, y));
                    last_ctrl_x = x2;
                    last_ctrl_y = y2;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'C';
                }
            }
            'c' => {
                if let Some((dx1, dy1, dx2, dy2, dx, dy)) = read_six(&tokens, &mut index) {
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
                if let Some((x2, y2, x, y)) = read_four(&tokens, &mut index) {
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
                if let Some((dx2, dy2, dx, dy)) = read_four(&tokens, &mut index) {
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
                if let Some((x1, y1, x, y)) = read_four(&tokens, &mut index) {
                    commands.push(PathCommand::QuadTo(x1, y1, x, y));
                    last_ctrl_x = x1;
                    last_ctrl_y = y1;
                    cur_x = x;
                    cur_y = y;
                    last_cmd = 'Q';
                }
            }
            'q' => {
                if let Some((dx1, dy1, dx, dy)) = read_four(&tokens, &mut index) {
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
                if let Some((x, y)) = read_pair(&tokens, &mut index) {
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
                if let Some((dx, dy)) = read_pair(&tokens, &mut index) {
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
            _ => index += 1,
        }
    }

    commands
}

/// Parse polyline/polygon points attribute: "x1,y1 x2,y2 ..."
pub fn parse_points(val: &str) -> Vec<(f32, f32)> {
    let numbers: Vec<f32> = val
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect();

    numbers
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

/// Tokenize a path data string into numbers and command letters.
pub(crate) fn tokenize_path(d: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = d.chars().collect();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];

        if ch.is_ascii_alphabetic() {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            tokens.push(ch.to_string());
            index += 1;
            continue;
        }

        if ch == '-' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            current.push(ch);
            index += 1;
            continue;
        }

        if ch == '.' {
            if current.contains('.') {
                tokens.push(current.clone());
                current.clear();
            }
            current.push(ch);
            index += 1;
            continue;
        }

        if ch.is_ascii_digit() {
            current.push(ch);
            index += 1;
            continue;
        }

        if !current.is_empty() {
            tokens.push(current.clone());
            current.clear();
        }
        index += 1;
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

pub(crate) fn read_number(tokens: &[String], index: &mut usize) -> Option<f32> {
    let token = tokens.get(*index)?;
    let value = token.parse::<f32>().ok()?;
    *index += 1;
    Some(value)
}

pub(crate) fn read_pair(tokens: &[String], index: &mut usize) -> Option<(f32, f32)> {
    Some((read_number(tokens, index)?, read_number(tokens, index)?))
}

pub(crate) fn read_four(tokens: &[String], index: &mut usize) -> Option<(f32, f32, f32, f32)> {
    Some((
        read_number(tokens, index)?,
        read_number(tokens, index)?,
        read_number(tokens, index)?,
        read_number(tokens, index)?,
    ))
}

pub(crate) fn read_six(
    tokens: &[String],
    index: &mut usize,
) -> Option<(f32, f32, f32, f32, f32, f32)> {
    Some((
        read_number(tokens, index)?,
        read_number(tokens, index)?,
        read_number(tokens, index)?,
        read_number(tokens, index)?,
        read_number(tokens, index)?,
        read_number(tokens, index)?,
    ))
}
