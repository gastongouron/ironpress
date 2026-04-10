use super::*;

#[test]
fn resolve_calc_basic_subtraction() {
    let tokens = vec![
        CalcToken::Percent(100.0),
        CalcToken::Op(CalcOp::Sub),
        CalcToken::Length(20.0),
    ];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 380.0).abs() < 0.01);
}

#[test]
fn resolve_calc_addition() {
    let tokens = vec![
        CalcToken::Percent(50.0),
        CalcToken::Op(CalcOp::Add),
        CalcToken::Length(7.5),
    ];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 207.5).abs() < 0.01);
}

#[test]
fn resolve_calc_mul() {
    let tokens = vec![
        CalcToken::Length(10.0),
        CalcToken::Op(CalcOp::Mul),
        CalcToken::Length(3.0),
    ];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 30.0).abs() < 0.01);
}

#[test]
fn resolve_calc_div() {
    let tokens = vec![
        CalcToken::Length(100.0),
        CalcToken::Op(CalcOp::Div),
        CalcToken::Length(2.0),
    ];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 50.0).abs() < 0.01);
}

#[test]
fn resolve_calc_mul_before_add() {
    let tokens = vec![
        CalcToken::Length(10.0),
        CalcToken::Op(CalcOp::Add),
        CalcToken::Length(5.0),
        CalcToken::Op(CalcOp::Mul),
        CalcToken::Length(3.0),
    ];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 25.0).abs() < 0.01);
}

#[test]
fn resolve_calc_with_rem() {
    let tokens = vec![
        CalcToken::Rem(2.0),
        CalcToken::Op(CalcOp::Add),
        CalcToken::Length(5.0),
    ];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 29.0).abs() < 0.01);
}

#[test]
fn resolve_calc_with_vw() {
    let tokens = vec![
        CalcToken::Vw(100.0),
        CalcToken::Op(CalcOp::Sub),
        CalcToken::Length(20.0),
    ];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 575.28).abs() < 0.01);
}

#[test]
fn resolve_calc_with_em_uses_font_size() {
    let tokens = vec![
        CalcToken::Em(2.0),
        CalcToken::Op(CalcOp::Add),
        CalcToken::Length(5.0),
    ];
    assert!((resolve_calc(&tokens, 400.0, 20.0, 12.0, 595.28, 841.89) - 45.0).abs() < 0.01);
}

#[test]
fn resolve_percentage_val() {
    let val = CssValue::Percentage(50.0);
    assert_eq!(
        resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()),
        Some(200.0)
    );
}

#[test]
fn resolve_rem_val() {
    let val = CssValue::Rem(2.0);
    assert_eq!(
        resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()),
        Some(24.0)
    );
}

#[test]
fn resolve_rem_val_with_custom_root_size() {
    let val = CssValue::Rem(2.0);
    let ctx = LengthResolutionContext::new(400.0, 12.0, 10.0, 595.28, 841.89);
    assert_eq!(
        resolve_length_value_in_context(&val, ctx, &HashMap::new()),
        Some(20.0)
    );
}

#[test]
fn resolve_vw_val() {
    let val = CssValue::Vw(100.0);
    let r = resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()).unwrap();
    assert!((r - 595.28).abs() < 0.01);
}

#[test]
fn resolve_vh_val() {
    let val = CssValue::Vh(100.0);
    let r = resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()).unwrap();
    assert!((r - 841.89).abs() < 0.01);
}

#[test]
fn resolve_var_defined() {
    let mut props = HashMap::new();
    props.insert("--spacing".to_string(), "10pt".to_string());
    let val = CssValue::Var("--spacing".to_string(), None);
    assert_eq!(
        resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &props),
        Some(10.0)
    );
}

#[test]
fn resolve_var_fallback() {
    let val = CssValue::Var("--spacing".to_string(), Some("20pt".to_string()));
    assert_eq!(
        resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()),
        Some(20.0)
    );
}

#[test]
fn resolve_var_undefined_no_fallback() {
    let val = CssValue::Var("--spacing".to_string(), None);
    assert_eq!(
        resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()),
        None
    );
}

#[test]
fn resolve_var_color_test() {
    let mut props = HashMap::new();
    props.insert("--text-color".to_string(), "red".to_string());
    let val = CssValue::Var("--text-color".to_string(), None);
    let c = try_resolve_var_to_color(&val, &props).unwrap();
    assert_eq!(c.r, 255);
    assert_eq!(c.g, 0);
}

#[test]
fn resolve_calc_empty() {
    assert_eq!(resolve_calc(&[], 400.0, 12.0, 12.0, 595.28, 841.89), 0.0);
}

#[test]
fn resolve_calc_div_by_zero() {
    let tokens = vec![
        CalcToken::Length(100.0),
        CalcToken::Op(CalcOp::Div),
        CalcToken::Length(0.0),
    ];
    assert_eq!(
        resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89),
        100.0
    );
}

#[test]
fn resolve_calc_from_parsed() {
    let map = crate::parser::css::parse_inline_style("width: calc(100% - 20pt)");
    let Some(CssValue::Calc(tokens)) = map.get("width") else {
        panic!("Expected Calc");
    };
    assert!((resolve_calc(tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 380.0).abs() < 0.01);
}

#[test]
fn resolve_calc_with_vh() {
    let tokens = vec![
        CalcToken::Vh(50.0),
        CalcToken::Op(CalcOp::Add),
        CalcToken::Length(10.0),
    ];
    let result = resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89);
    assert!((result - 430.945).abs() < 0.01);
}

#[test]
fn resolve_calc_trailing_op() {
    let tokens = vec![CalcToken::Length(10.0), CalcToken::Op(CalcOp::Add)];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 10.0).abs() < 0.01);
}

#[test]
fn resolve_calc_trailing_add_op() {
    let tokens = vec![
        CalcToken::Length(5.0),
        CalcToken::Op(CalcOp::Mul),
        CalcToken::Length(2.0),
        CalcToken::Op(CalcOp::Add),
    ];
    assert!((resolve_calc(&tokens, 400.0, 12.0, 12.0, 595.28, 841.89) - 10.0).abs() < 0.01);
}

#[test]
fn resolve_length_value_number() {
    let val = CssValue::Number(42.0);
    assert_eq!(
        resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()),
        Some(42.0)
    );
}

#[test]
fn resolve_length_value_keyword_returns_none() {
    let val = CssValue::Keyword("auto".to_string());
    assert_eq!(
        resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()),
        None
    );
}

#[test]
fn resolve_var_to_unparseable_length() {
    let mut props = HashMap::new();
    props.insert("--x".to_string(), "auto".to_string());
    let val = CssValue::Var("--x".to_string(), None);
    assert_eq!(
        resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &props),
        None
    );
}

#[test]
fn try_resolve_var_to_color_non_var_returns_none() {
    let val = CssValue::Keyword("red".to_string());
    assert!(try_resolve_var_to_color(&val, &HashMap::new()).is_none());
}

#[test]
fn try_resolve_var_to_color_non_color_value() {
    let mut props = HashMap::new();
    props.insert("--x".to_string(), "10pt".to_string());
    let val = CssValue::Var("--x".to_string(), None);
    assert!(try_resolve_var_to_color(&val, &props).is_none());
}

#[test]
fn try_resolve_var_to_keyword_defined() {
    let mut props = HashMap::new();
    props.insert("--display".to_string(), "flex".to_string());
    let val = CssValue::Var("--display".to_string(), None);
    assert_eq!(
        try_resolve_var_to_keyword(&val, &props),
        Some("flex".to_string())
    );
}

#[test]
fn try_resolve_var_to_keyword_with_fallback() {
    let val = CssValue::Var("--missing".to_string(), Some("block".to_string()));
    assert_eq!(
        try_resolve_var_to_keyword(&val, &HashMap::new()),
        Some("block".to_string())
    );
}

#[test]
fn try_resolve_var_to_keyword_non_var_returns_none() {
    let val = CssValue::Keyword("block".to_string());
    assert!(try_resolve_var_to_keyword(&val, &HashMap::new()).is_none());
}

#[test]
fn try_resolve_var_to_keyword_undefined_no_fallback() {
    let val = CssValue::Var("--missing".to_string(), None);
    assert!(try_resolve_var_to_keyword(&val, &HashMap::new()).is_none());
}
