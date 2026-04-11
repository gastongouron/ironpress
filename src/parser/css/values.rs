use crate::types::Color;

use super::{CalcOp, CalcToken, CssValue};

pub(crate) fn is_css_wide_keyword(value: &str) -> bool {
    matches!(
        value,
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    )
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
        match parse_length(&token)? {
            CssValue::Length(value) => tokens.push(CalcToken::Length(value)),
            CssValue::Percentage(value) => tokens.push(CalcToken::Percent(value)),
            CssValue::Number(value) => tokens.push(CalcToken::Em(value)),
            CssValue::Rem(value) => tokens.push(CalcToken::Rem(value)),
            CssValue::Vw(value) => tokens.push(CalcToken::Vw(value)),
            CssValue::Vh(value) => tokens.push(CalcToken::Vh(value)),
            _ => return None,
        }
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

    if let Some(inner) = lower
        .strip_prefix("rgba(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_rgba_function(inner);
    }

    lower
        .strip_prefix("rgb(")
        .and_then(|inner| inner.strip_suffix(')'))
        .and_then(parse_rgb_function)
}

pub(crate) fn parse_property_value(property: &str, val: &str) -> Option<CssValue> {
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

    if is_css_wide_keyword(&lower) {
        return Some(CssValue::Keyword(lower));
    }

    if property.contains("color") {
        return parse_color(val);
    }

    if matches!(property, "font-weight" | "font-style") {
        return Some(CssValue::Keyword(lower));
    }

    if property == "font-family" {
        return Some(CssValue::Keyword(val.trim().to_string()));
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
            | "filter"
            | "aspect-ratio"
            | "grid-template-columns"
            | "box-shadow"
            | "outline"
            | "box-sizing"
            | "text-overflow"
            | "border-collapse"
            | "table-layout"
            | "background-size"
            | "background-repeat"
            | "background-position"
            | "background-origin"
            | "background-image"
            | "white-space"
            | "overflow-wrap"
            | "word-wrap"
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

    // line-height: a bare number (e.g. `1.6`) is a unitless multiplier,
    // not a length.  Only values with explicit units should be Length.
    if property == "line-height" {
        if lower == "normal" {
            return Some(CssValue::Keyword("normal".into()));
        }
        // Try unit-based parsing first (px, pt, em, rem, %, etc.)
        let has_unit = val
            .trim()
            .ends_with(|c: char| c.is_ascii_alphabetic() || c == '%');
        if has_unit {
            return parse_length(val);
        }
        // Bare number → unitless line-height multiplier
        return val.trim().parse::<f32>().ok().map(CssValue::Number);
    }

    parse_length(val)
}

#[cfg(test)]
pub(crate) fn parse_border_spacing_component(val: &str, index: usize) -> Option<CssValue> {
    split_spacing_components(val)
        .and_then(|parts| parts.get(index).copied())
        .and_then(parse_length)
}

pub(crate) fn parse_border_spacing_shorthand(val: &str) -> Option<(CssValue, CssValue)> {
    match split_spacing_components(val)?.as_slice() {
        [single] => {
            let parsed = parse_property_value("border-spacing", single)?;
            Some((parsed.clone(), parsed))
        }
        [horizontal, vertical] => Some((parse_length(horizontal)?, parse_length(vertical)?)),
        _ => None,
    }
}

pub(crate) fn border_spacing_value_count(val: &str) -> Option<usize> {
    let count = split_spacing_components(val)?.len();
    matches!(count, 1 | 2).then_some(count)
}

fn split_spacing_components(val: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;

    for (index, ch) in val.char_indices() {
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            c if c.is_whitespace() && paren_depth == 0 => {
                if start < index {
                    parts.push(val[start..index].trim());
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    if start < val.len() {
        parts.push(val[start..].trim());
    }

    if matches!(parts.len(), 1 | 2) {
        Some(parts)
    } else {
        None
    }
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

/// Parse `rgba(r, g, b, a)` where alpha is 0.0–1.0.
///
/// The alpha channel is stored in the `Color` struct so the PDF renderer
/// can emit a proper ExtGState with `/ca` (fill opacity) instead of
/// pre-compositing against white.
fn parse_rgba_function(inner: &str) -> Option<CssValue> {
    let parts: Vec<&str> = inner.splitn(4, ',').collect();
    if parts.len() != 4 {
        return None;
    }
    let r = parts[0].trim().parse::<u8>().ok()?;
    let g = parts[1].trim().parse::<u8>().ok()?;
    let b = parts[2].trim().parse::<u8>().ok()?;
    let a: f32 = parts[3].trim().parse::<f32>().ok()?;
    let a = a.clamp(0.0, 1.0);

    Some(CssValue::Color(Color {
        r,
        g,
        b,
        a: (a * 255.0).round() as u8,
    }))
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
