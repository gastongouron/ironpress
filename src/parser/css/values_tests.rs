use super::{
    CalcOp, CalcToken, CssValue, parse_border_spacing_component, parse_calc_expression,
    parse_color, parse_length, parse_property_value, parse_var_function, tokenize_calc,
};

#[test]
fn border_spacing_component_preserves_calc_and_var_tokens() {
    let spacing = "calc(1rem + 2pt) var(--gap, 3pt)";
    assert!(matches!(
        parse_border_spacing_component(spacing, 0),
        Some(CssValue::Calc(_))
    ));
    assert!(matches!(
        parse_border_spacing_component(spacing, 1),
        Some(CssValue::Var(_, _))
    ));
}

#[test]
fn border_spacing_component_rejects_more_than_two_values() {
    assert!(parse_border_spacing_component("5pt 10pt 15pt", 0).is_none());
    assert!(parse_border_spacing_component("5pt 10pt 15pt", 1).is_none());
}

#[test]
fn parse_length_units() {
    assert!(matches!(
        parse_length("10px"),
        Some(CssValue::Length(v)) if (v - 7.5).abs() < 0.01
    ));
    assert!(matches!(
        parse_length("14pt"),
        Some(CssValue::Length(v)) if (v - 14.0).abs() < 0.01
    ));
    assert!(matches!(
        parse_length("50%"),
        Some(CssValue::Percentage(v)) if (v - 50.0).abs() < 0.01
    ));
    assert!(matches!(
        parse_length("2rem"),
        Some(CssValue::Rem(v)) if (v - 2.0).abs() < 0.01
    ));
    assert!(matches!(
        parse_length("100vw"),
        Some(CssValue::Vw(v)) if (v - 100.0).abs() < 0.01
    ));
    assert!(matches!(
        parse_length("50vh"),
        Some(CssValue::Vh(v)) if (v - 50.0).abs() < 0.01
    ));
    assert!(matches!(
        parse_length("1.5em"),
        Some(CssValue::Number(v)) if (v - 1.5).abs() < 0.01
    ));
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
fn parse_calc_expression_empty_is_none() {
    assert!(parse_calc_expression("calc()").is_none());
}

#[test]
fn tokenize_calc_variants() {
    assert_eq!(tokenize_calc("10px   ").unwrap().len(), 1);
    assert!(tokenize_calc("-5px + 10px").is_some());
    assert!(matches!(
        tokenize_calc("1em").as_deref(),
        Some([CalcToken::Em(value)]) if (*value - 1.0).abs() < 0.01
    ));
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
    assert!(matches!(
        parse_property_value("width", "revert-layer"),
        Some(CssValue::Keyword(value)) if value == "revert-layer"
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

/// BUG P2-1: rgba() background-color must be parsed and pre-composited.
/// Previously `parse_color` did not handle `rgba(...)` at all, so any
/// `background-color: rgba(...)` preserves the original RGB values and alpha.
#[test]
fn parse_color_rgba_preserves_rgb_and_alpha() {
    // rgba(239, 68, 68, 0.05) should store the raw RGB values and alpha.
    let color = parse_color("rgba(239, 68, 68, 0.05)");
    assert!(
        color.is_some(),
        "rgba() should be parsed as a Color, not None"
    );
    if let Some(CssValue::Color(c)) = color {
        assert_eq!(c.r, 239, "r should be preserved as-is");
        assert_eq!(c.g, 68, "g should be preserved as-is");
        assert_eq!(c.b, 68, "b should be preserved as-is");
        // alpha 0.05 * 255 = 12.75, rounds to 13
        assert_eq!(c.a, 13, "alpha 0.05 should be stored as 13/255");
    }
}

#[test]
fn parse_color_rgba_fully_opaque() {
    // rgba(0, 128, 255, 1.0) should yield the same colour as rgb(0, 128, 255).
    let c_rgba = parse_color("rgba(0, 128, 255, 1.0)");
    let c_rgb = parse_color("rgb(0, 128, 255)");
    match (c_rgba, c_rgb) {
        (Some(CssValue::Color(a)), Some(CssValue::Color(b))) => {
            assert_eq!(a.r, b.r);
            assert_eq!(a.g, b.g);
            assert_eq!(a.b, b.b);
        }
        _ => panic!("both rgba(,,,1.0) and rgb() should parse successfully"),
    }
}

#[test]
fn parse_color_rgba_fully_transparent() {
    // rgba(0, 0, 0, 0.0) should store RGB as-is with alpha = 0.
    let color = parse_color("rgba(0, 0, 0, 0.0)");
    if let Some(CssValue::Color(c)) = color {
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
        assert_eq!(c.a, 0, "alpha 0.0 should be stored as 0");
    } else {
        panic!("rgba(0,0,0,0) should parse to a Color");
    }
}

#[test]
fn line_height_bare_number_is_not_length() {
    // A bare number like `1.6` for line-height must be parsed as Number
    // (unitless multiplier), not Length. Previously this was parsed as
    // CssValue::Length(1.6) which caused line-height to be divided by
    // font-size, producing tiny line heights and text overlap.
    let val = parse_property_value("line-height", "1.6");
    assert!(
        matches!(val, Some(CssValue::Number(v)) if (v - 1.6).abs() < 0.001),
        "line-height: 1.6 should be Number(1.6), got {:?}",
        val
    );

    let val = parse_property_value("line-height", "1.8");
    assert!(matches!(val, Some(CssValue::Number(v)) if (v - 1.8).abs() < 0.001));

    let val = parse_property_value("line-height", "2");
    assert!(matches!(val, Some(CssValue::Number(v)) if (v - 2.0).abs() < 0.001));

    // Values with units should still be parsed as Length
    let val = parse_property_value("line-height", "18pt");
    assert!(matches!(val, Some(CssValue::Length(v)) if (v - 18.0).abs() < 0.001));

    let val = parse_property_value("line-height", "24px");
    assert!(matches!(val, Some(CssValue::Length(v)) if (v - 18.0).abs() < 0.001)); // 24 * 0.75

    // em values should be Number (the em-to-number conversion)
    let val = parse_property_value("line-height", "1.5em");
    assert!(matches!(val, Some(CssValue::Number(v)) if (v - 1.5).abs() < 0.001));

    // "normal" should be Keyword
    let val = parse_property_value("line-height", "normal");
    assert!(matches!(val, Some(CssValue::Keyword(ref k)) if k == "normal"));
}
