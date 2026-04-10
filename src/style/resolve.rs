//! CSS value resolution for calc(), var(), and new unit types (%, em, rem, vw, vh).

use std::collections::HashMap;

use crate::parser::css::{CalcOp, CalcToken, CssValue};

const DEFAULT_FONT_SIZE: f32 = 12.0;
const DEFAULT_PAGE_WIDTH: f32 = 595.28;
const DEFAULT_PAGE_HEIGHT: f32 = 841.89;

#[derive(Debug, Clone, Copy)]
pub struct LengthResolutionContext {
    pub parent_width: f32,
    pub font_size: f32,
    pub root_font_size: f32,
    pub page_width: f32,
    pub page_height: f32,
}

impl LengthResolutionContext {
    pub const fn new(
        parent_width: f32,
        font_size: f32,
        root_font_size: f32,
        page_width: f32,
        page_height: f32,
    ) -> Self {
        Self {
            parent_width,
            font_size,
            root_font_size,
            page_width,
            page_height,
        }
    }

    pub const fn pdf_defaults(parent_width: f32) -> Self {
        Self::new(
            parent_width,
            DEFAULT_FONT_SIZE,
            DEFAULT_FONT_SIZE,
            DEFAULT_PAGE_WIDTH,
            DEFAULT_PAGE_HEIGHT,
        )
    }

    pub const fn pdf_with_font_sizes(
        parent_width: f32,
        font_size: f32,
        root_font_size: f32,
    ) -> Self {
        Self::new(
            parent_width,
            font_size,
            root_font_size,
            DEFAULT_PAGE_WIDTH,
            DEFAULT_PAGE_HEIGHT,
        )
    }
}

/// Resolve a calc() expression given resolution context.
pub fn resolve_calc(
    tokens: &[CalcToken],
    parent_width: f32,
    font_size: f32,
    root_font_size: f32,
    page_width: f32,
    page_height: f32,
) -> f32 {
    let mut values: Vec<f32> = Vec::new();
    let mut ops: Vec<CalcOp> = Vec::new();
    for token in tokens {
        match token {
            CalcToken::Length(v) => values.push(*v),
            CalcToken::Em(v) => values.push(*v * font_size),
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
        let Some(next_value) = values.get(i + 1).copied() else {
            break;
        };
        match op {
            CalcOp::Mul => {
                if let Some(last) = rv.last_mut() {
                    *last *= next_value;
                }
            }
            CalcOp::Div => {
                if next_value != 0.0 {
                    if let Some(last) = rv.last_mut() {
                        *last /= next_value;
                    }
                }
            }
            _ => {
                rv.push(next_value);
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

/// Resolve a CssValue to absolute length in points using a caller-provided
/// `font_size` basis for em units.
pub fn resolve_length_value_in_context(
    val: &CssValue,
    ctx: LengthResolutionContext,
    custom_properties: &HashMap<String, String>,
) -> Option<f32> {
    match val {
        CssValue::Length(v) => Some(*v),
        CssValue::Number(v) => Some(*v),
        CssValue::Percentage(v) => Some(ctx.parent_width * v / 100.0),
        CssValue::Rem(v) => Some(*v * ctx.root_font_size),
        CssValue::Vw(v) => Some(ctx.page_width * v / 100.0),
        CssValue::Vh(v) => Some(ctx.page_height * v / 100.0),
        CssValue::Calc(tokens) => Some(resolve_calc(
            tokens,
            ctx.parent_width,
            ctx.font_size,
            ctx.root_font_size,
            ctx.page_width,
            ctx.page_height,
        )),
        CssValue::Var(name, fallback) => {
            let raw = custom_properties
                .get(name.as_str())
                .cloned()
                .or_else(|| fallback.clone())?;
            let parsed = crate::parser::css::parse_inline_style(&format!("_x: {raw}"));
            if let Some(inner) = parsed.get("_x") {
                resolve_length_value_in_context(inner, ctx, custom_properties)
            } else {
                None
            }
        }
        _ => None,
    }
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
    resolve_length_value_in_context(
        val,
        LengthResolutionContext::new(
            parent_width,
            DEFAULT_FONT_SIZE,
            root_font_size,
            page_width,
            page_height,
        ),
        custom_properties,
    )
}

/// Try to resolve a CssValue to an absolute length using defaults.
pub fn try_resolve_to_length(
    val: &CssValue,
    custom_properties: &HashMap<String, String>,
    parent_width_hint: f32,
) -> Option<f32> {
    resolve_length_value_in_context(
        val,
        LengthResolutionContext::pdf_defaults(parent_width_hint),
        custom_properties,
    )
}

/// Try to resolve a CssValue to an absolute length using a caller-provided
/// `font_size` basis for em units.
pub fn try_resolve_to_length_in_context(
    val: &CssValue,
    custom_properties: &HashMap<String, String>,
    ctx: LengthResolutionContext,
) -> Option<f32> {
    resolve_length_value_in_context(val, ctx, custom_properties)
}

/// Try to resolve a CssValue to an absolute length using a caller-provided
/// `font_size` basis for em units.
pub fn try_resolve_to_length_with_font_size(
    val: &CssValue,
    custom_properties: &HashMap<String, String>,
    parent_width_hint: f32,
    font_size: f32,
    root_font_size: f32,
) -> Option<f32> {
    try_resolve_to_length_in_context(
        val,
        custom_properties,
        LengthResolutionContext::pdf_with_font_sizes(parent_width_hint, font_size, root_font_size),
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
#[path = "resolve_tests.rs"]
mod tests;
