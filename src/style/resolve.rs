//! CSS value resolution for calc(), var(), and new unit types (%, rem, vw, vh).

use std::collections::HashMap;

use crate::parser::css::{CalcOp, CalcToken, CssValue};

/// Resolve a calc() expression given resolution context.
pub fn resolve_calc(
    tokens: &[CalcToken],
    parent_width: f32,
    root_font_size: f32,
    page_width: f32,
    page_height: f32,
) -> f32 {
    let mut values: Vec<f32> = Vec::new();
    let mut ops: Vec<CalcOp> = Vec::new();
    for token in tokens {
        match token {
            CalcToken::Length(v) => values.push(*v),
            CalcToken::Percent(v) => values.push(parent_width * v / 100.0),
            CalcToken::Rem(v) => values.push(*v * root_font_size),
            CalcToken::Vw(v) => values.push(page_width * v / 100.0),
            CalcToken::Vh(v) => values.push(page_height * v / 100.0),
            CalcToken::Op(op) => ops.push(*op),
        }
    }
    if values.is_empty() {
        return 0.0;
    }
    // First pass: * and /
    let mut rv: Vec<f32> = vec![values[0]];
    let mut ro: Vec<CalcOp> = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        if i + 1 >= values.len() {
            break;
        }
        match op {
            CalcOp::Mul => *rv.last_mut().unwrap() *= values[i + 1],
            CalcOp::Div => {
                if values[i + 1] != 0.0 {
                    *rv.last_mut().unwrap() /= values[i + 1];
                }
            }
            _ => {
                rv.push(values[i + 1]);
                ro.push(*op);
            }
        }
    }
    // Second pass: + and -
    let mut result = rv[0];
    for (i, op) in ro.iter().enumerate() {
        if i + 1 >= rv.len() {
            break;
        }
        match op {
            CalcOp::Add => result += rv[i + 1],
            CalcOp::Sub => result -= rv[i + 1],
            _ => {}
        }
    }
    result
}

/// Resolve a CssValue to absolute length in points.
pub fn resolve_length_value(
    val: &CssValue,
    parent_width: f32,
    root_font_size: f32,
    page_width: f32,
    page_height: f32,
    custom_properties: &HashMap<String, String>,
) -> Option<f32> {
    match val {
        CssValue::Length(v) => Some(*v),
        CssValue::Number(v) => Some(*v),
        CssValue::Percentage(v) => Some(parent_width * v / 100.0),
        CssValue::Rem(v) => Some(*v * root_font_size),
        CssValue::Vw(v) => Some(page_width * v / 100.0),
        CssValue::Vh(v) => Some(page_height * v / 100.0),
        CssValue::Calc(tokens) => Some(resolve_calc(
            tokens,
            parent_width,
            root_font_size,
            page_width,
            page_height,
        )),
        CssValue::Var(name, fallback) => {
            let raw = custom_properties
                .get(name.as_str())
                .cloned()
                .or_else(|| fallback.clone())?;
            let parsed = crate::parser::css::parse_inline_style(&format!("_x: {raw}"));
            if let Some(inner) = parsed.get("_x") {
                resolve_length_value(
                    inner,
                    parent_width,
                    root_font_size,
                    page_width,
                    page_height,
                    custom_properties,
                )
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Try to resolve a CssValue to an absolute length using defaults.
pub fn try_resolve_to_length(
    val: &CssValue,
    custom_properties: &HashMap<String, String>,
    parent_width_hint: f32,
) -> Option<f32> {
    resolve_length_value(
        val,
        parent_width_hint,
        12.0,
        595.28,
        841.89,
        custom_properties,
    )
}

/// Resolve a var() name to its value string.
pub fn resolve_var_to_string(
    name: &str,
    fallback: Option<&str>,
    custom_properties: &HashMap<String, String>,
) -> Option<String> {
    custom_properties
        .get(name)
        .cloned()
        .or_else(|| fallback.map(|s| s.to_string()))
}

/// Try to resolve a CssValue::Var to a color.
pub fn try_resolve_var_to_color(
    val: &CssValue,
    custom_properties: &HashMap<String, String>,
) -> Option<crate::types::Color> {
    if let CssValue::Var(name, fallback) = val {
        let raw = resolve_var_to_string(name, fallback.as_deref(), custom_properties)?;
        let parsed = crate::parser::css::parse_inline_style(&format!("color: {raw}"));
        if let Some(CssValue::Color(c)) = parsed.get("color") {
            Some(*c)
        } else {
            None
        }
    } else {
        None
    }
}

/// Try to resolve a CssValue::Var to a keyword string.
pub fn try_resolve_var_to_keyword(
    val: &CssValue,
    custom_properties: &HashMap<String, String>,
) -> Option<String> {
    if let CssValue::Var(name, fallback) = val {
        resolve_var_to_string(name, fallback.as_deref(), custom_properties)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_calc_basic_subtraction() {
        let tokens = vec![
            CalcToken::Percent(100.0),
            CalcToken::Op(CalcOp::Sub),
            CalcToken::Length(20.0),
        ];
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 380.0).abs() < 0.01);
    }

    #[test]
    fn resolve_calc_addition() {
        let tokens = vec![
            CalcToken::Percent(50.0),
            CalcToken::Op(CalcOp::Add),
            CalcToken::Length(7.5),
        ];
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 207.5).abs() < 0.01);
    }

    #[test]
    fn resolve_calc_mul() {
        let tokens = vec![
            CalcToken::Length(10.0),
            CalcToken::Op(CalcOp::Mul),
            CalcToken::Length(3.0),
        ];
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 30.0).abs() < 0.01);
    }

    #[test]
    fn resolve_calc_div() {
        let tokens = vec![
            CalcToken::Length(100.0),
            CalcToken::Op(CalcOp::Div),
            CalcToken::Length(2.0),
        ];
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 50.0).abs() < 0.01);
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
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 25.0).abs() < 0.01);
    }

    #[test]
    fn resolve_calc_with_rem() {
        let tokens = vec![
            CalcToken::Rem(2.0),
            CalcToken::Op(CalcOp::Add),
            CalcToken::Length(5.0),
        ];
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 29.0).abs() < 0.01);
    }

    #[test]
    fn resolve_calc_with_vw() {
        let tokens = vec![
            CalcToken::Vw(100.0),
            CalcToken::Op(CalcOp::Sub),
            CalcToken::Length(20.0),
        ];
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 575.28).abs() < 0.01);
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
        assert_eq!(resolve_calc(&[], 400.0, 12.0, 595.28, 841.89), 0.0);
    }

    #[test]
    fn resolve_calc_div_by_zero() {
        let tokens = vec![
            CalcToken::Length(100.0),
            CalcToken::Op(CalcOp::Div),
            CalcToken::Length(0.0),
        ];
        assert_eq!(resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89), 100.0);
    }

    #[test]
    fn resolve_calc_from_parsed() {
        let map = crate::parser::css::parse_inline_style("width: calc(100% - 20pt)");
        if let Some(CssValue::Calc(tokens)) = map.get("width") {
            assert!((resolve_calc(tokens, 400.0, 12.0, 595.28, 841.89) - 380.0).abs() < 0.01);
        } else {
            panic!("Expected Calc");
        }
    }

    // --- Coverage: CalcToken::Vh in resolve_calc (line 23) ---

    #[test]
    fn resolve_calc_with_vh() {
        let tokens = vec![
            CalcToken::Vh(50.0),
            CalcToken::Op(CalcOp::Add),
            CalcToken::Length(10.0),
        ];
        let result = resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89);
        // 50% of 841.89 = 420.945, plus 10 = 430.945
        assert!((result - 430.945).abs() < 0.01);
    }

    // --- Coverage: break when i+1 >= values.len() in first pass (line 35) ---

    #[test]
    fn resolve_calc_trailing_op() {
        // More ops than value pairs: the trailing op is skipped
        let tokens = vec![CalcToken::Length(10.0), CalcToken::Op(CalcOp::Add)];
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 10.0).abs() < 0.01);
    }

    // --- Coverage: break in second pass (line 54) ---

    #[test]
    fn resolve_calc_trailing_add_op() {
        // After first pass, there's an add op but no second value
        let tokens = vec![
            CalcToken::Length(5.0),
            CalcToken::Op(CalcOp::Mul),
            CalcToken::Length(2.0),
            CalcToken::Op(CalcOp::Add),
        ];
        // 5 * 2 = 10, then trailing add is ignored
        assert!((resolve_calc(&tokens, 400.0, 12.0, 595.28, 841.89) - 10.0).abs() < 0.01);
    }

    // --- Coverage: CssValue::Number in resolve_length_value (line 76) ---

    #[test]
    fn resolve_length_value_number() {
        let val = CssValue::Number(42.0);
        assert_eq!(
            resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()),
            Some(42.0)
        );
    }

    // --- Coverage: _ => None in resolve_length_value (line 107) ---

    #[test]
    fn resolve_length_value_keyword_returns_none() {
        let val = CssValue::Keyword("auto".to_string());
        assert_eq!(
            resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &HashMap::new()),
            None
        );
    }

    // --- Coverage: Var resolves to unparseable value -> None (line 104) ---

    #[test]
    fn resolve_var_to_unparseable_length() {
        let mut props = HashMap::new();
        props.insert("--x".to_string(), "auto".to_string());
        let val = CssValue::Var("--x".to_string(), None);
        // "auto" parses as a Keyword, which then hits _ => None
        assert_eq!(
            resolve_length_value(&val, 400.0, 12.0, 595.28, 841.89, &props),
            None
        );
    }

    // --- Coverage: try_resolve_var_to_color when val is not Var (line 153) ---

    #[test]
    fn try_resolve_var_to_color_non_var_returns_none() {
        let val = CssValue::Keyword("red".to_string());
        assert!(try_resolve_var_to_color(&val, &HashMap::new()).is_none());
    }

    // --- Coverage: try_resolve_var_to_color when value doesn't parse as color (line 150) ---

    #[test]
    fn try_resolve_var_to_color_non_color_value() {
        let mut props = HashMap::new();
        props.insert("--x".to_string(), "10pt".to_string());
        let val = CssValue::Var("--x".to_string(), None);
        assert!(try_resolve_var_to_color(&val, &props).is_none());
    }

    // --- Coverage: try_resolve_var_to_keyword (lines 158, 162-163, 165) ---

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
}
