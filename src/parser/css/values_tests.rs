use super::values::{
    parse_border_spacing_component, parse_calc_expression, parse_color, parse_length,
    parse_property_value, parse_var_function, tokenize_calc,
};
use crate::parser::css::{CalcOp, CalcToken, CssValue};

#[test]
fn parse_length_units() {
    assert!(matches!(parse_length("10px"), Some(CssValue::Length(v)) if (v - 7.5).abs() < 0.01));
    assert!(matches!(parse_length("14pt"), Some(CssValue::Length(v)) if (v - 14.0).abs() < 0.01));
    assert!(
        matches!(parse_length("50%"), Some(CssValue::Percentage(v)) if (v - 50.0).abs() < 0.01)
    );
    assert!(matches!(parse_length("2rem"), Some(CssValue::Rem(v)) if (v - 2.0).abs() < 0.01));
    assert!(matches!(parse_length("100vw"), Some(CssValue::Vw(v)) if (v - 100.0).abs() < 0.01));
    assert!(matches!(parse_length("50vh"), Some(CssValue::Vh(v)) if (v - 50.0).abs() < 0.01));
    assert!(matches!(parse_length("1.5em"), Some(CssValue::Number(v)) if (v - 1.5).abs() < 0.01));
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
fn parse_calc_expression_accepts_em_units() {
    let Some(CssValue::Calc(tokens)) = parse_calc_expression("calc(1em + 2pt)") else {
        panic!("expected calc tokens");
    };
    assert_eq!(tokens.len(), 3);
    assert!(matches!(&tokens[0], CalcToken::Em(v) if (*v - 1.0).abs() < 0.01));
    assert!(matches!(&tokens[1], CalcToken::Op(CalcOp::Add)));
    assert!(matches!(&tokens[2], CalcToken::Length(v) if (*v - 2.0).abs() < 0.01));
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
    assert!(matches!(
        parse_color("Aqua"),
        Some(CssValue::Color(c)) if c.g == 255 && c.b == 255
    ));
    assert!(matches!(
        parse_color("fuchsia"),
        Some(CssValue::Color(c)) if c.r == 255 && c.b == 255
    ));
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

#[test]
fn parse_border_spacing_component_preserves_calc_and_var_tokens() {
    assert!(matches!(
        parse_border_spacing_component("calc(10px + 5px) var(--gap, 8px)", 0),
        Some(CssValue::Calc(tokens)) if tokens.len() == 3
    ));
    assert!(matches!(
        parse_border_spacing_component("calc(10px + 5px) var(--gap, 8px)", 1),
        Some(CssValue::Var(name, Some(fallback))) if name == "--gap" && fallback == "8px"
    ));
}
