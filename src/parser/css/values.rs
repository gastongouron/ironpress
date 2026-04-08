use crate::types::Color;

use super::{CalcOp, CalcToken, CssValue};

#[derive(Debug, Clone)]
pub(super) enum BorderSpacingValue {
    Keyword(String),
    Lengths {
        horizontal: CssValue,
        vertical: CssValue,
    },
}

pub(crate) fn parse_length(val: &str) -> Option<CssValue> {
    let val = val.trim();

    if let Some(var_value) = parse_var_function(val) {
        return Some(var_value);
    }

    if let Some(calc_value) = parse_calc_expression(val) {
        return Some(calc_value);
    }

    if let Some(number) = val.strip_suffix("px") {
        return number
            .parse::<f32>()
            .ok()
            .map(|value| CssValue::Length(value * 0.75));
    }

    if let Some(number) = val.strip_suffix("pt") {
        return number.parse::<f32>().ok().map(CssValue::Length);
    }

    if let Some(number) = val.strip_suffix("rem") {
        return number.parse::<f32>().ok().map(CssValue::Rem);
    }

    if let Some(number) = val.strip_suffix("vw") {
        return number.parse::<f32>().ok().map(CssValue::Vw);
    }

    if let Some(number) = val.strip_suffix("vh") {
        return number.parse::<f32>().ok().map(CssValue::Vh);
    }

    if let Some(number) = val.strip_suffix('%') {
        return number.parse::<f32>().ok().map(CssValue::Percentage);
    }

    if let Some(number) = val.strip_suffix("em") {
        return number.parse::<f32>().ok().map(CssValue::Number);
    }

    val.parse::<f32>().ok().map(CssValue::Length)
}

pub(crate) fn parse_var_function(val: &str) -> Option<CssValue> {
    let inner = val.strip_prefix("var(")?.strip_suffix(')')?.trim();
    let (name, fallback) = match inner.split_once(',') {
        Some((name, fallback)) => (name.trim(), Some(fallback.trim().to_string())),
        None => (inner, None),
    };

    if !name.starts_with("--") {
        return None;
    }

    Some(CssValue::Var(name.to_string(), fallback))
}

pub(crate) fn parse_calc_expression(val: &str) -> Option<CssValue> {
    let inner = val.strip_prefix("calc(")?.strip_suffix(')')?.trim();
    if inner.is_empty() {
        return None;
    }

    tokenize_calc(inner).map(CssValue::Calc)
}

fn parse_calc_token(token: &str) -> Option<CalcToken> {
    if let Some(number) = token.strip_suffix("em") {
        return number.parse::<f32>().ok().map(CalcToken::Em);
    }

    match parse_length(token)? {
        CssValue::Length(value) => Some(CalcToken::Length(value)),
        CssValue::Percentage(value) => Some(CalcToken::Percent(value)),
        CssValue::Rem(value) => Some(CalcToken::Rem(value)),
        CssValue::Vw(value) => Some(CalcToken::Vw(value)),
        CssValue::Vh(value) => Some(CalcToken::Vh(value)),
        _ => None,
    }
}

pub(crate) fn tokenize_calc(expr: &str) -> Option<Vec<CalcToken>> {
    let chars: Vec<char> = expr.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut expects_value = true;

    while index < chars.len() {
        while chars.get(index).is_some_and(|ch| ch.is_whitespace()) {
            index += 1;
        }

        let Some(ch) = chars.get(index).copied() else {
            break;
        };

        if matches!(ch, '*' | '/') || ((ch == '+' || ch == '-') && !expects_value) {
            if expects_value {
                return None;
            }
            let operator = match ch {
                '+' => CalcOp::Add,
                '-' => CalcOp::Sub,
                '*' => CalcOp::Mul,
                '/' => CalcOp::Div,
                _ => unreachable!(),
            };
            tokens.push(CalcToken::Op(operator));
            index += 1;
            expects_value = true;
            continue;
        }

        let start = index;
        if matches!(chars.get(index), Some('+') | Some('-')) {
            index += 1;
        }

        while chars
            .get(index)
            .is_some_and(|next| next.is_ascii_digit() || *next == '.')
        {
            index += 1;
        }

        if start == index {
            return None;
        }

        while chars
            .get(index)
            .is_some_and(|next| next.is_ascii_alphabetic() || *next == '%')
        {
            index += 1;
        }

        let token = chars[start..index].iter().collect::<String>();
        tokens.push(parse_calc_token(&token)?);
        expects_value = false;
    }

    if tokens.is_empty() || expects_value {
        None
    } else {
        Some(tokens)
    }
}

pub(crate) fn parse_color(val: &str) -> Option<CssValue> {
    let val = val.trim();
    let lower = val.to_ascii_lowercase();

    if let Some(color) = named_color(&lower) {
        return Some(CssValue::Color(color));
    }

    if let Some(hex) = val.strip_prefix('#') {
        return parse_hex_color(hex);
    }

    lower
        .strip_prefix("rgb(")
        .and_then(|inner| inner.strip_suffix(')'))
        .and_then(parse_rgb_function)
}

pub(super) fn parse_property_value(property: &str, val: &str) -> Option<CssValue> {
    let val = val
        .trim()
        .strip_suffix("!important")
        .map(str::trim_end)
        .unwrap_or(val.trim());
    let lower = val.to_ascii_lowercase();

    if let Some(var_value) = parse_var_function(val) {
        return Some(var_value);
    }

    if let Some(calc_value) = parse_calc_expression(val) {
        return Some(calc_value);
    }

    if matches!(lower.as_str(), "inherit" | "initial" | "unset") {
        return Some(CssValue::Keyword(lower));
    }

    if property.contains("color") {
        return parse_color(val);
    }

    if matches!(property, "font-weight" | "font-style") {
        return Some(CssValue::Keyword(lower));
    }

    if property.starts_with("border-spacing")
        && matches!(
            lower.as_str(),
            "inherit" | "initial" | "unset" | "revert" | "revert-layer"
        )
    {
        return Some(CssValue::Keyword(lower));
    }

    if property == "font-family" {
        let first_font = val.split(',').next().unwrap_or(val).trim();
        return Some(CssValue::Keyword(first_font.to_string()));
    }

    if matches!(property, "text-align" | "text-decoration" | "display") {
        return Some(CssValue::Keyword(lower));
    }

    if property.starts_with("page-break") {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(
        property,
        "border" | "border-style" | "border-top" | "border-right" | "border-bottom" | "border-left"
    ) {
        return Some(CssValue::Keyword(val.to_string()));
    }

    if property == "border-width" {
        return parse_length(val);
    }

    if property == "border-color" {
        return parse_color(val);
    }

    if property == "z-index" {
        if lower == "auto" {
            return Some(CssValue::Keyword("auto".to_string()));
        }
        return val
            .parse::<i32>()
            .ok()
            .map(|number| CssValue::Number(number as f32));
    }

    if matches!(property, "float" | "clear" | "position") {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(
        property,
        "flex-direction" | "justify-content" | "align-items" | "flex-wrap"
    ) {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(
        property,
        "flex-grow" | "flex-shrink" | "gap" | "grid-gap" | "column-gap"
    ) {
        return parse_length(val);
    }

    if property == "flex-basis" {
        if matches!(lower.as_str(), "auto" | "content") {
            return Some(CssValue::Keyword(lower));
        }
        return parse_length(val);
    }

    if matches!(
        property,
        "flex"
            | "content"
            | "counter-reset"
            | "counter-increment"
            | "list-style-type"
            | "list-style-position"
            | "list-style"
            | "overflow"
            | "visibility"
            | "transform"
            | "grid-template-columns"
            | "box-shadow"
            | "outline"
            | "box-sizing"
            | "text-overflow"
            | "border-collapse"
            | "background-size"
            | "background-repeat"
            | "background-position"
            | "white-space"
            | "text-transform"
    ) {
        return Some(CssValue::Keyword(val.to_string()));
    }

    if matches!(property, "column-count" | "columns") {
        return parse_length(val).or_else(|| Some(CssValue::Keyword(val.to_string())));
    }

    if matches!(property, "border-radius" | "outline-width") {
        return parse_length(val);
    }

    if property == "outline-color" {
        return parse_color(val);
    }

    if matches!(property, "width" | "height") && lower == "auto" {
        return Some(CssValue::Keyword("auto".to_string()));
    }

    parse_length(val)
}

pub(super) fn parse_border_spacing_component(val: &str, index: usize) -> Option<CssValue> {
    split_css_components(val)
        .get(index)
        .and_then(|component| parse_length(component))
}

pub(super) fn parse_border_spacing_value(val: &str) -> Option<BorderSpacingValue> {
    let trimmed = val.trim();
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    ) {
        return Some(BorderSpacingValue::Keyword(lower));
    }

    let parts = split_css_components(trimmed);
    match parts.as_slice() {
        [horizontal] => Some(BorderSpacingValue::Lengths {
            horizontal: parse_length(horizontal)?,
            vertical: parse_length(horizontal)?,
        }),
        [horizontal, vertical] => Some(BorderSpacingValue::Lengths {
            horizontal: parse_length(horizontal)?,
            vertical: parse_length(vertical)?,
        }),
        _ => None,
    }
}

fn split_css_components(val: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let bytes = val.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] as char {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ch if ch.is_whitespace() && paren_depth == 0 => {
                if start < index {
                    parts.push(val[start..index].trim());
                }
                while index < bytes.len() && (bytes[index] as char).is_whitespace() {
                    index += 1;
                }
                start = index;
                continue;
            }
            _ => {}
        }
        index += 1;
    }

    if start < val.len() {
        let tail = val[start..].trim();
        if !tail.is_empty() {
            parts.push(tail);
        }
    }

    parts
}

fn named_color(name: &str) -> Option<Color> {
    match name {
        "black" => Some(Color::rgb(0, 0, 0)),
        "white" => Some(Color::rgb(255, 255, 255)),
        "red" => Some(Color::rgb(255, 0, 0)),
        "green" => Some(Color::rgb(0, 128, 0)),
        "blue" => Some(Color::rgb(0, 0, 255)),
        "yellow" => Some(Color::rgb(255, 255, 0)),
        "orange" => Some(Color::rgb(255, 165, 0)),
        "purple" => Some(Color::rgb(128, 0, 128)),
        "gray" | "grey" => Some(Color::rgb(128, 128, 128)),
        "silver" => Some(Color::rgb(192, 192, 192)),
        "maroon" => Some(Color::rgb(128, 0, 0)),
        "navy" => Some(Color::rgb(0, 0, 128)),
        "teal" => Some(Color::rgb(0, 128, 128)),
        "aqua" | "cyan" => Some(Color::rgb(0, 255, 255)),
        "fuchsia" | "magenta" => Some(Color::rgb(255, 0, 255)),
        "lime" => Some(Color::rgb(0, 255, 0)),
        "transparent" => Some(Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }),
        _ => None,
    }
}

fn parse_hex_color(hex: &str) -> Option<CssValue> {
    let bytes = hex.as_bytes();
    match bytes {
        [r, g, b] => Some(CssValue::Color(Color::rgb(
            hex_digit(*r)? * 17,
            hex_digit(*g)? * 17,
            hex_digit(*b)? * 17,
        ))),
        [r1, r2, g1, g2, b1, b2] => Some(CssValue::Color(Color::rgb(
            hex_pair(*r1, *r2)?,
            hex_pair(*g1, *g2)?,
            hex_pair(*b1, *b2)?,
        ))),
        _ => None,
    }
}

fn parse_rgb_function(inner: &str) -> Option<CssValue> {
    let parts: Vec<u8> = inner
        .split(',')
        .map(str::trim)
        .map(str::parse::<u8>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;

    match parts.as_slice() {
        [r, g, b] => Some(CssValue::Color(Color::rgb(*r, *g, *b))),
        _ => None,
    }
}

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
