use super::{CalcOp, CalcToken, CssValue, parse_length};

pub(crate) fn parse_border_spacing_component(val: &str, index: usize) -> Option<CssValue> {
    split_border_spacing_tokens(val)?
        .get(index)
        .and_then(|token| parse_length(token))
        .filter(border_spacing_value_is_valid)
}

pub(crate) fn parse_border_spacing_values(val: &str) -> Option<(CssValue, CssValue)> {
    let tokens = split_border_spacing_tokens(val)?;
    let horizontal = tokens.first().and_then(|token| parse_length(token))?;
    if !border_spacing_value_is_valid(&horizontal) {
        return None;
    }

    let vertical = if let Some(token) = tokens.get(1) {
        let value = parse_length(token)?;
        if !border_spacing_value_is_valid(&value) {
            return None;
        }
        value
    } else {
        horizontal.clone()
    };

    Some((horizontal, vertical))
}

fn border_spacing_value_is_valid(value: &CssValue) -> bool {
    match value {
        CssValue::Percentage(_) => false,
        CssValue::Length(v)
        | CssValue::Number(v)
        | CssValue::Rem(v)
        | CssValue::Vw(v)
        | CssValue::Vh(v) => *v >= 0.0,
        CssValue::Calc(tokens) => border_spacing_calc_is_valid(tokens),
        CssValue::Var(_, Some(fallback)) => parse_border_spacing_values(fallback).is_some(),
        _ => true,
    }
}

fn border_spacing_calc_is_valid(tokens: &[CalcToken]) -> bool {
    if tokens
        .iter()
        .any(|token| matches!(token, CalcToken::Percent(_)))
    {
        return false;
    }

    let mut values = Vec::new();
    let mut ops = Vec::new();
    for token in tokens {
        match token {
            CalcToken::Length(v)
            | CalcToken::Number(v)
            | CalcToken::Rem(v)
            | CalcToken::Vw(v)
            | CalcToken::Vh(v) => {
                if *v < 0.0 {
                    return false;
                }
                values.push(*v);
            }
            CalcToken::Op(op) => ops.push(*op),
            CalcToken::Percent(_) => unreachable!(),
        }
    }

    if values.is_empty() {
        return false;
    }

    let mut reduced = vec![values[0]];
    let mut remaining_ops = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        let Some(rhs) = values.get(index + 1).copied() else {
            break;
        };
        match op {
            CalcOp::Mul => {
                if let Some(last) = reduced.last_mut() {
                    *last *= rhs;
                }
            }
            CalcOp::Div => {
                if rhs == 0.0 {
                    return false;
                }
                if let Some(last) = reduced.last_mut() {
                    *last /= rhs;
                }
            }
            _ => {
                reduced.push(rhs);
                remaining_ops.push(*op);
            }
        }
    }

    let mut result = reduced[0];
    for (index, op) in remaining_ops.iter().enumerate() {
        let Some(rhs) = reduced.get(index + 1).copied() else {
            break;
        };
        match op {
            CalcOp::Add => result += rhs,
            CalcOp::Sub => result -= rhs,
            _ => {}
        }
    }

    result.is_finite() && result >= 0.0
}

fn split_border_spacing_tokens(val: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;

    for ch in val.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    if matches!(tokens.len(), 1 | 2) {
        Some(tokens)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_border_spacing_component, parse_border_spacing_values};
    use crate::parser::css::CssValue;

    #[test]
    fn border_spacing_components_keep_calc_and_var_tokens() {
        assert!(matches!(
            parse_border_spacing_component("calc(10pt + 2pt) var(--gap, 4pt)", 0),
            Some(CssValue::Calc(_))
        ));
        assert!(matches!(
            parse_border_spacing_component("calc(10pt + 2pt) var(--gap, 4pt)", 1),
            Some(CssValue::Var(name, Some(fallback))) if name == "--gap" && fallback == "4pt"
        ));
    }

    #[test]
    fn border_spacing_rejects_more_than_two_components() {
        assert!(parse_border_spacing_component("5pt 10pt 15pt", 0).is_none());
        assert!(parse_border_spacing_component("5pt 10pt 15pt", 1).is_none());
    }

    #[test]
    fn border_spacing_rejects_percentage_and_negative_values() {
        assert!(parse_border_spacing_component("10%", 0).is_none());
        assert!(parse_border_spacing_component("-1pt", 0).is_none());
        assert!(parse_border_spacing_component("calc(1pt - 2pt)", 0).is_none());
        assert!(parse_border_spacing_values("4pt calc(1pt - 2pt)").is_none());
    }
}
