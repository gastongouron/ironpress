use crate::types::Color;

use super::{CalcOp, CalcToken, CssValue};

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
            CssValue::Number(value) => tokens.push(CalcToken::Number(value)),
            CssValue::Percentage(value) => tokens.push(CalcToken::Percent(value)),
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

    if matches!(lower.as_str(), "inherit" | "initial" | "unset" | "revert") {
        return Some(CssValue::Keyword(lower));
    }

    if property.contains("color") {
        return parse_color(val);
    }

    if matches!(property, "font-weight" | "font-style") {
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

#[cfg(test)]
mod tests {
    use super::{
        parse_calc_expression, parse_color, parse_length, parse_property_value, parse_var_function,
        tokenize_calc,
    };
    use crate::parser::css::{CalcOp, CalcToken, CssValue};

    #[test]
    fn parse_length_units() {
        assert!(
            matches!(parse_length("10px"), Some(CssValue::Length(v)) if (v - 7.5).abs() < 0.01)
        );
        assert!(
            matches!(parse_length("14pt"), Some(CssValue::Length(v)) if (v - 14.0).abs() < 0.01)
        );
        assert!(
            matches!(parse_length("50%"), Some(CssValue::Percentage(v)) if (v - 50.0).abs() < 0.01)
        );
        assert!(matches!(parse_length("2rem"), Some(CssValue::Rem(v)) if (v - 2.0).abs() < 0.01));
        assert!(matches!(parse_length("100vw"), Some(CssValue::Vw(v)) if (v - 100.0).abs() < 0.01));
        assert!(matches!(parse_length("50vh"), Some(CssValue::Vh(v)) if (v - 50.0).abs() < 0.01));
        assert!(
            matches!(parse_length("1.5em"), Some(CssValue::Number(v)) if (v - 1.5).abs() < 0.01)
        );
    }

    #[test]
    fn parse_var_function_basic() {
        assert!(matches!(
            parse_var_function("var(--my-width)"),
            Some(CssValue::Var(name, None)) if name == "--my-width"
        ));
        assert!(matches!(
            parse_var_function("var(--text-color, red)"),
            Some(CssValue::Var(name, Some(fallback))) if name == "--text-color" && fallback == "red"
        ));
    }

    #[test]
    fn parse_var_function_invalid_name() {
        assert!(parse_var_function("var(invalid)").is_none());
        assert!(parse_var_function("var(invalid, fallback)").is_none());
    }

    #[test]
    fn parse_calc_expression_basic() {
        let Some(CssValue::Calc(tokens)) = parse_calc_expression("calc(100% - 20pt)") else {
            panic!("expected calc tokens");
        };
        assert_eq!(tokens.len(), 3);
        assert!(matches!(&tokens[0], CalcToken::Percent(v) if (*v - 100.0).abs() < 0.01));
        assert!(matches!(&tokens[1], CalcToken::Op(CalcOp::Sub)));
        assert!(matches!(&tokens[2], CalcToken::Length(v) if (*v - 20.0).abs() < 0.01));
    }

    #[test]
    fn parse_calc_expression_accepts_em_operands() {
        let Some(CssValue::Calc(tokens)) = parse_calc_expression("calc(100% - 2em)") else {
            panic!("expected calc tokens");
        };
        assert!(matches!(&tokens[2], CalcToken::Number(v) if (*v - 2.0).abs() < 0.01));
    }

    #[test]
    fn parse_calc_expression_empty_is_none() {
        assert!(parse_calc_expression("calc()").is_none());
    }

    #[test]
    fn tokenize_calc_variants() {
        assert_eq!(tokenize_calc("10px   ").unwrap().len(), 1);
        assert!(tokenize_calc("-5px + 10px").is_some());
        assert!(tokenize_calc("+").is_none());
        assert!(tokenize_calc("10xyz").is_none());
    }

    #[test]
    fn parse_keyword_values_case_insensitively() {
        assert!(matches!(
            parse_property_value("width", "AUTO"),
            Some(CssValue::Keyword(value)) if value == "auto"
        ));
        assert!(matches!(
            parse_property_value("height", "Auto"),
            Some(CssValue::Keyword(value)) if value == "auto"
        ));
        assert!(matches!(
            parse_property_value("display", "BLOCK"),
            Some(CssValue::Keyword(value)) if value == "block"
        ));
        assert!(matches!(
            parse_property_value("width", "UNSET"),
            Some(CssValue::Keyword(value)) if value == "unset"
        ));
        assert!(matches!(
            parse_property_value("width", "revert"),
            Some(CssValue::Keyword(value)) if value == "revert"
        ));
    }

    #[test]
    fn parse_color_variants() {
        assert!(matches!(parse_color("red"), Some(CssValue::Color(c)) if c.r == 255 && c.g == 0));
        assert!(matches!(parse_color("#ff0000"), Some(CssValue::Color(c)) if c.r == 255));
        assert!(matches!(parse_color("#f00"), Some(CssValue::Color(c)) if c.r == 255));
        assert!(
            matches!(parse_color("rgb(10, 20, 30)"), Some(CssValue::Color(c)) if c.r == 10 && c.g == 20 && c.b == 30)
        );
    }

    #[test]
    fn parse_color_named_keywords_are_case_insensitive() {
        assert!(matches!(parse_color("Blue"), Some(CssValue::Color(c)) if c.b == 255));
        assert!(matches!(parse_color("NAVY"), Some(CssValue::Color(c)) if c.b == 128));
        assert!(
            matches!(parse_color("Aqua"), Some(CssValue::Color(c)) if c.g == 255 && c.b == 255)
        );
        assert!(
            matches!(parse_color("fuchsia"), Some(CssValue::Color(c)) if c.r == 255 && c.b == 255)
        );
        assert!(matches!(parse_color("Lime"), Some(CssValue::Color(c)) if c.g == 255));
    }

    #[test]
    fn parse_color_transparent_preserves_alpha() {
        assert!(matches!(parse_color("transparent"), Some(CssValue::Color(c)) if c.a == 0));
    }

    #[test]
    fn parse_color_invalid_inputs() {
        assert!(parse_color("nonexistentcolor").is_none());
        assert!(parse_color("#12345").is_none());
        assert!(parse_color("rgb(1,2)").is_none());
    }
}
