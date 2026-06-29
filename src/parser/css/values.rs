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

    if let Some(clamp_value) = parse_clamp_expression(val) {
        return Some(clamp_value);
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

    // Small/large/dynamic viewport units collapse to the static page viewport
    // in this paged renderer.
    if let Some(number) = val
        .strip_suffix("svw")
        .or_else(|| val.strip_suffix("lvw"))
        .or_else(|| val.strip_suffix("dvw"))
    {
        return number.parse::<f32>().ok().map(CssValue::Vw);
    }

    if let Some(number) = val
        .strip_suffix("svh")
        .or_else(|| val.strip_suffix("lvh"))
        .or_else(|| val.strip_suffix("dvh"))
    {
        return number.parse::<f32>().ok().map(CssValue::Vh);
    }

    if let Some(number) = val.strip_suffix("vw") {
        return number.parse::<f32>().ok().map(CssValue::Vw);
    }

    if let Some(number) = val.strip_suffix("vh") {
        return number.parse::<f32>().ok().map(CssValue::Vh);
    }

    // vmin/vmax (css-values-4 §6.1.2.2): checked before the bare `vh`/`vw`
    // suffixes can't match these (they end in "vmin"/"vmax").
    if let Some(number) = val.strip_suffix("vmin") {
        return number.parse::<f32>().ok().map(CssValue::Vmin);
    }

    if let Some(number) = val.strip_suffix("vmax") {
        return number.parse::<f32>().ok().map(CssValue::Vmax);
    }

    if let Some(number) = val.strip_suffix('%') {
        return number.parse::<f32>().ok().map(CssValue::Percentage);
    }

    // Font-relative ex/ch (css-values-4 §6.1.1): `ex` is the resolved font's
    // x-height, `ch` the advance of its `'0'` glyph. The raw coefficient is
    // preserved so the metric can be applied against the actual font downstream
    // (falling back to 0.5em only when no font metric is available). Checked
    // before the `em` branch — they don't end in "em" so they don't collide.
    if let Some(number) = val.strip_suffix("ex") {
        return number.parse::<f32>().ok().map(CssValue::Ex);
    }
    if let Some(number) = val.strip_suffix("ch") {
        return number.parse::<f32>().ok().map(CssValue::Ch);
    }

    // `cap` and `lh` need the element's resolved font metrics / line-height,
    // which are only known in the computed-style layer. Preserve the token.
    if val.strip_suffix("cap").is_some() || val.strip_suffix("lh").is_some() {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Absolute length units → points (1pt = 1/72in). CssValue::Length is in pt.
    if let Some(number) = val.strip_suffix("cm") {
        return number
            .parse::<f32>()
            .ok()
            .map(|v| CssValue::Length(v * 72.0 / 2.54));
    }
    if let Some(number) = val.strip_suffix("mm") {
        return number
            .parse::<f32>()
            .ok()
            .map(|v| CssValue::Length(v * 72.0 / 25.4));
    }
    if let Some(number) = val.strip_suffix("q") {
        return number
            .parse::<f32>()
            .ok()
            .map(|v| CssValue::Length(v * 72.0 / 25.4 / 4.0));
    }
    if let Some(number) = val.strip_suffix("in") {
        return number
            .parse::<f32>()
            .ok()
            .map(|v| CssValue::Length(v * 72.0));
    }
    if let Some(number) = val.strip_suffix("pc") {
        return number
            .parse::<f32>()
            .ok()
            .map(|v| CssValue::Length(v * 12.0));
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

/// Parse a `clamp(min, preferred, max)` expression. Each of the three operands
/// is a length-like value (length, percentage, calc, var, …) parsed via
/// [`parse_length`] and stored lazily so the percentage basis can be applied at
/// resolution time. Resolves to `max(min, min(preferred, max))`.
pub(crate) fn parse_clamp_expression(val: &str) -> Option<CssValue> {
    let inner = val.strip_prefix("clamp(")?.strip_suffix(')')?.trim();
    let parts = split_top_level_args(inner);
    if parts.len() != 3 {
        return None;
    }
    let min = parse_length(parts[0].trim())?;
    let preferred = parse_length(parts[1].trim())?;
    let max = parse_length(parts[2].trim())?;
    Some(CssValue::Clamp(
        Box::new(min),
        Box::new(preferred),
        Box::new(max),
    ))
}

/// Split a comma-separated argument list at top level, ignoring commas nested
/// inside parentheses (e.g. `calc(50% - 1px), 2px`).
fn split_top_level_args(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    for ch in inner.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    if !current.is_empty() || !parts.is_empty() {
        parts.push(current);
    }
    parts
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
            CssValue::Vmin(value) => tokens.push(CalcToken::Vmin(value)),
            CssValue::Vmax(value) => tokens.push(CalcToken::Vmax(value)),
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

    if lower == "currentcolor" {
        return Some(CssValue::Color(Color {
            r: 1,
            g: 2,
            b: 3,
            a: 254,
        }));
    }

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

    if let Some(inner) = lower
        .strip_prefix("color(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_color_function(inner);
    }

    if let Some(inner) = lower.strip_prefix("lab(").and_then(|s| s.strip_suffix(')')) {
        return parse_lab_function(inner);
    }

    if let Some(inner) = lower.strip_prefix("lch(").and_then(|s| s.strip_suffix(')')) {
        return parse_lch_function(inner);
    }

    if let Some(inner) = lower
        .strip_prefix("oklab(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_oklab_function(inner);
    }

    if let Some(inner) = lower
        .strip_prefix("oklch(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_oklch_function(inner);
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

    if let Some(clamp_value) = parse_clamp_expression(val) {
        return Some(clamp_value);
    }

    if is_css_wide_keyword(&lower) {
        return Some(CssValue::Keyword(lower));
    }

    if property == "border-color" {
        return parse_color(val).or_else(|| Some(CssValue::Keyword(val.to_string())));
    }

    if property.contains("color") {
        return parse_color(val);
    }

    if matches!(property, "font-weight" | "font-style") {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(property, "font-family" | "font") {
        return Some(CssValue::Keyword(val.trim().to_string()));
    }

    if matches!(
        property,
        "text-align"
            | "text-decoration"
            | "text-decoration-line"
            | "text-decoration-style"
            | "display"
    ) {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(
        property,
        "text-decoration-thickness" | "text-underline-offset"
    ) {
        return parse_length(val).or(Some(CssValue::Keyword(lower)));
    }

    if property == "vertical-align" {
        return parse_length(val).or(Some(CssValue::Keyword(lower)));
    }

    if property.starts_with("page-break")
        || matches!(property, "break-before" | "break-after" | "break-inside")
    {
        // Legacy `page-break-*` and modern CSS Fragmentation 3 `break-*`
        // keywords (`auto`/`avoid`/`page`/`left`/`right`/`recto`/`verso`) are
        // preserved verbatim so the style resolver can map them.
        return Some(CssValue::Keyword(lower));
    }

    // CSS Paged Media 3 §3.4 `page: <name>` — the value is a page name
    // identifier (or `auto`). Preserved as a keyword so `compute_style` can
    // record the named page; otherwise it would fall through to `parse_length`
    // and be dropped.
    if property == "page" {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(
        property,
        "border"
            | "border-style"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
    ) {
        return Some(CssValue::Keyword(val.to_string()));
    }

    if property == "border-width" {
        // CSS keyword widths map to the usual 1px/3px/5px (-> pt) values.
        match lower.as_str() {
            "thin" => return Some(CssValue::Length(1.0 * 0.75)),
            "medium" => return Some(CssValue::Length(3.0 * 0.75)),
            "thick" => return Some(CssValue::Length(5.0 * 0.75)),
            _ => {}
        }
        return parse_length(val).or_else(|| Some(CssValue::Keyword(val.to_string())));
    }

    if matches!(
        property,
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width"
    ) {
        // CSS keyword widths map to the usual 1px/3px/5px (→ pt) values.
        match lower.as_str() {
            "thin" => return Some(CssValue::Length(1.0 * 0.75)),
            "medium" => return Some(CssValue::Length(3.0 * 0.75)),
            "thick" => return Some(CssValue::Length(5.0 * 0.75)),
            _ => {}
        }
        return parse_length(val);
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

    if matches!(
        property,
        "float" | "clear" | "position" | "box-decoration-break"
    ) {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(
        property,
        "mix-blend-mode" | "background-blend-mode" | "isolation"
    ) {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(
        property,
        "flex-direction"
            | "flex-flow"
            | "justify-content"
            | "align-items"
            | "align-content"
            | "align-self"
            | "place-content"
            | "flex-wrap"
    ) {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(property, "flex-grow" | "flex-shrink") {
        return parse_length(val);
    }

    if property == "order" {
        return val
            .trim()
            .parse::<i32>()
            .ok()
            .map(|number| CssValue::Number(number as f32));
    }

    // Gap properties accept a single length or — for `gap` / `grid-gap` — a
    // two-value `<row> <column>` form. A single value parses as a length; the
    // two-value form is kept as a Keyword for the computed-style layer to split.
    if matches!(
        property,
        "gap" | "grid-gap" | "grid-column-gap" | "grid-row-gap" | "column-gap" | "row-gap"
    ) {
        return parse_length(val).or_else(|| Some(CssValue::Keyword(lower.clone())));
    }

    if property == "flex-basis" {
        if matches!(
            lower.as_str(),
            "auto" | "content" | "min-content" | "max-content" | "fit-content"
        ) {
            return Some(CssValue::Keyword(lower));
        }
        return parse_length(val);
    }

    if matches!(
        property,
        "flex"
            | "content"
            | "quotes"
            | "counter-reset"
            | "counter-increment"
            | "counter-set"
            | "string-set"
            | "list-style-type"
            | "list-style-position"
            | "list-style-image"
            | "list-style"
            | "marker-side"
            | "overflow"
            | "overflow-x"
            | "overflow-y"
            | "overflow-inline"
            | "overflow-block"
            | "scrollbar-gutter"
            | "visibility"
            | "transform"
            | "transform-origin"
            | "transform-box"
            | "translate"
            | "rotate"
            | "scale"
            | "perspective"
            | "perspective-origin"
            | "filter"
            | "clip"
            | "aspect-ratio"
            | "grid-template-columns"
            | "grid-template-rows"
            | "grid-auto-rows"
            | "grid-auto-flow"
            | "grid-auto-columns"
            | "justify-items"
            | "place-items"
            | "grid-column"
            | "grid-row"
            | "grid-column-start"
            | "grid-column-end"
            | "grid-row-start"
            | "grid-row-end"
            | "grid-template-areas"
            | "grid-area"
            | "grid-template"
            | "grid"
            | "justify-self"
            | "place-self"
            | "clip-path"
            | "mask"
            | "mask-image"
            | "mask-mode"
            | "mask-repeat"
            | "mask-position"
            | "mask-size"
            | "mask-origin"
            | "mask-clip"
            | "mask-composite"
            | "mask-type"
            | "mask-border-source"
            | "mask-border-slice"
            | "mask-border-width"
            | "mask-border-repeat"
            | "-webkit-mask"
            | "-webkit-mask-image"
            | "-webkit-mask-mode"
            | "-webkit-mask-repeat"
            | "-webkit-mask-position"
            | "-webkit-mask-size"
            | "-webkit-mask-origin"
            | "-webkit-mask-clip"
            | "-webkit-mask-composite"
            | "box-shadow"
            | "text-shadow"
            | "unicode-bidi"
            | "outline"
            | "box-sizing"
            | "text-overflow"
            | "border-collapse"
            | "table-layout"
            | "empty-cells"
            | "caption-side"
            | "background-size"
            | "background-repeat"
            | "background-position"
            | "background-origin"
            | "background-clip"
            | "background-attachment"
            | "border-image"
            | "background-image"
            | "white-space"
            | "overflow-wrap"
            | "word-wrap"
            | "word-break"
            | "text-transform"
            | "font-variant"
            | "font-variant-caps"
            | "font-variant-ligatures"
            | "font-kerning"
            | "font-size-adjust"
            | "font-synthesis"
            | "text-emphasis"
            | "text-emphasis-style"
            | "text-emphasis-position"
            | "hyphens"
            | "font-feature-settings"
            | "direction"
            | "writing-mode"
            | "text-orientation"
            | "text-combine-upright"
            | "white-space-collapse"
            | "text-wrap-mode"
            | "object-fit"
            | "object-position"
            | "vertical-align"
            | "inset"
            | "line-clamp"
            | "-webkit-line-clamp"
    ) {
        return Some(CssValue::Keyword(val.to_string()));
    }

    if property == "column-count" {
        return parse_length(val).or_else(|| Some(CssValue::Keyword(val.to_string())));
    }

    // The `columns` shorthand (`<column-width> || <column-count>`) is ambiguous
    // once units are stripped: `columns: 4` (count) and `columns: 140px` (width)
    // would both collapse to a bare `Length`. Preserve the raw string so the
    // shorthand decoder in `compute_style` can keep px-vs-unitless apart.
    if property == "columns" {
        return Some(CssValue::Keyword(val.to_string()));
    }

    // Multi-column shorthands/longhands whose values are best preserved verbatim
    // and decoded later in `compute_style` (e.g. `column-rule: 6px solid #d6005a`,
    // `column-width: 140px`, `column-span: all`, `column-fill: auto`).
    if matches!(
        property,
        "column-width"
            | "column-rule"
            | "column-rule-width"
            | "column-rule-style"
            | "column-rule-color"
            | "column-span"
            | "column-fill"
    ) {
        return parse_length(val).or_else(|| Some(CssValue::Keyword(val.to_string())));
    }

    if property == "outline-width" {
        return parse_length(val);
    }

    // `border-radius` (and the per-corner longhands) accept 1-4 space-separated
    // values plus an optional `/` for elliptical radii. A bare single length is
    // kept as `Length` for the fast uniform path; anything else is preserved
    // verbatim and expanded into per-corner radii in `compute_style`.
    if matches!(
        property,
        "border-radius"
            | "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-right-radius"
            | "border-bottom-left-radius"
    ) {
        return parse_length(val).or_else(|| Some(CssValue::Keyword(val.trim().to_string())));
    }

    if property == "outline-color" {
        return parse_color(val);
    }

    // `outline-offset` is a single length that may be negative (inward outline).
    if property == "outline-offset" {
        return parse_length(val);
    }

    if matches!(property, "width" | "height") && lower == "auto" {
        return Some(CssValue::Keyword("auto".to_string()));
    }

    // css-sizing-3 § 5.1 intrinsic-sizing keywords on `width` (`min-content`,
    // `max-content`, `fit-content`). Preserve them as keywords so the computed
    // style layer can record `width_keyword`; otherwise they would fall through
    // to `parse_length` and be dropped (treated as `auto`).
    if matches!(property, "width" | "min-width" | "max-width")
        && matches!(
            lower.as_str(),
            "min-content" | "max-content" | "fit-content"
        )
    {
        return Some(CssValue::Keyword(lower));
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
            let trimmed = val.trim();
            if trimmed.ends_with("em") || trimmed.ends_with("lh") || trimmed.ends_with("cap") {
                return Some(CssValue::Keyword(trimmed.to_string()));
            }
            return parse_length(val);
        }
        // Bare number → unitless line-height multiplier
        return val.trim().parse::<f32>().ok().map(CssValue::Number);
    }

    // orphans / widows (css-break-3 §3.4): a bare positive `<integer>` count of
    // line boxes, kept as Number so `compute_style` reads it directly.
    if property == "orphans" || property == "widows" {
        return val
            .trim()
            .parse::<i32>()
            .ok()
            .map(|n| CssValue::Number(n as f32));
    }

    // tab-size (css-text-3 §6.3): a bare `<number>` is a count of space
    // advances (kept as Number); a value with a unit is a `<length>`.
    if property == "tab-size" || property == "-moz-tab-size" {
        let has_unit = val
            .trim()
            .ends_with(|c: char| c.is_ascii_alphabetic() || c == '%');
        if has_unit {
            return parse_length(val);
        }
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
        // #rgb
        [r, g, b] => Some(CssValue::Color(Color::rgb(
            hex_digit(*r)? * 17,
            hex_digit(*g)? * 17,
            hex_digit(*b)? * 17,
        ))),
        // #rgba
        [r, g, b, a] => Some(CssValue::Color(Color {
            r: hex_digit(*r)? * 17,
            g: hex_digit(*g)? * 17,
            b: hex_digit(*b)? * 17,
            a: hex_digit(*a)? * 17,
        })),
        // #rrggbb
        [r1, r2, g1, g2, b1, b2] => Some(CssValue::Color(Color::rgb(
            hex_pair(*r1, *r2)?,
            hex_pair(*g1, *g2)?,
            hex_pair(*b1, *b2)?,
        ))),
        // #rrggbbaa
        [r1, r2, g1, g2, b1, b2, a1, a2] => Some(CssValue::Color(Color {
            r: hex_pair(*r1, *r2)?,
            g: hex_pair(*g1, *g2)?,
            b: hex_pair(*b1, *b2)?,
            a: hex_pair(*a1, *a2)?,
        })),
        _ => None,
    }
}

fn parse_rgb_function(inner: &str) -> Option<CssValue> {
    if inner.contains(',') {
        let parts: Vec<u8> = inner
            .split(',')
            .map(str::trim)
            .map(parse_rgb_255_component)
            .collect::<Option<Vec<_>>>()?;

        match parts.as_slice() {
            [r, g, b] => Some(CssValue::Color(Color::rgb(*r, *g, *b))),
            _ => None,
        }
    } else {
        let (components, alpha) = split_color_alpha(inner);
        let parts: Vec<&str> = components.split_whitespace().collect();
        match parts.as_slice() {
            [r, g, b] => Some(CssValue::Color(Color {
                r: parse_rgb_255_component(r)?,
                g: parse_rgb_255_component(g)?,
                b: parse_rgb_255_component(b)?,
                a: parse_alpha_component(alpha)?,
            })),
            _ => None,
        }
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

fn parse_color_function(inner: &str) -> Option<CssValue> {
    let mut parts = inner.splitn(2, char::is_whitespace);
    let space = parts.next()?.trim();
    let rest = parts.next()?.trim();
    let (components, alpha) = split_color_alpha(rest);
    let coords: Vec<f32> = components
        .split_whitespace()
        .map(parse_unit_component)
        .collect::<Option<Vec<_>>>()?;
    if coords.len() != 3 {
        return None;
    }

    let rgb = match space {
        "srgb" => {
            return Some(CssValue::Color(Color {
                r: unit_to_byte_floor(coords[0]),
                g: unit_to_byte_floor(coords[1]),
                b: unit_to_byte_floor(coords[2]),
                a: parse_alpha_component(alpha)?,
            }));
        }
        "srgb-linear" => linear_srgb_to_srgb(coords[0], coords[1], coords[2]),
        "display-p3" => display_p3_to_srgb(coords[0], coords[1], coords[2]),
        "xyz" | "xyz-d65" => xyz_d65_to_srgb(coords[0], coords[1], coords[2]),
        _ => return None,
    };
    Some(CssValue::Color(rgb_color(
        rgb,
        parse_alpha_component(alpha)?,
    )))
}

fn parse_lab_function(inner: &str) -> Option<CssValue> {
    let (components, alpha) = split_color_alpha(inner);
    let parts: Vec<&str> = components.split_whitespace().collect();
    let [l, a, b] = parts.as_slice() else {
        return None;
    };
    let l = parse_lightness_percent(l)?;
    let a = parse_number_component(a)?;
    let b = parse_number_component(b)?;
    Some(CssValue::Color(rgb_color(
        lab_to_srgb(l, a, b),
        parse_alpha_component(alpha)?,
    )))
}

fn parse_lch_function(inner: &str) -> Option<CssValue> {
    let (components, alpha) = split_color_alpha(inner);
    let parts: Vec<&str> = components.split_whitespace().collect();
    let [l, c, h] = parts.as_slice() else {
        return None;
    };
    let l = parse_lightness_percent(l)?;
    let c = parse_number_component(c)?;
    let h = parse_number_component(h)?.to_radians();
    Some(CssValue::Color(rgb_color(
        lab_to_srgb(l, c * h.cos(), c * h.sin()),
        parse_alpha_component(alpha)?,
    )))
}

fn parse_oklab_function(inner: &str) -> Option<CssValue> {
    let (components, alpha) = split_color_alpha(inner);
    let parts: Vec<&str> = components.split_whitespace().collect();
    let [l, a, b] = parts.as_slice() else {
        return None;
    };
    let l = parse_unit_lightness(l)?;
    let a = parse_number_component(a)?;
    let b = parse_number_component(b)?;
    Some(CssValue::Color(rgb_color(
        oklab_to_srgb(l, a, b),
        parse_alpha_component(alpha)?,
    )))
}

fn parse_oklch_function(inner: &str) -> Option<CssValue> {
    let (components, alpha) = split_color_alpha(inner);
    let parts: Vec<&str> = components.split_whitespace().collect();
    let [l, c, h] = parts.as_slice() else {
        return None;
    };
    let l = parse_unit_lightness(l)?;
    let c = parse_number_component(c)?;
    let h = parse_number_component(h)?.to_radians();
    Some(CssValue::Color(rgb_color(
        oklab_to_srgb(l, c * h.cos(), c * h.sin()),
        parse_alpha_component(alpha)?,
    )))
}

fn split_color_alpha(inner: &str) -> (&str, Option<&str>) {
    match inner.split_once('/') {
        Some((components, alpha)) => (components.trim(), Some(alpha.trim())),
        None => (inner.trim(), None),
    }
}

fn parse_rgb_255_component(raw: &str) -> Option<u8> {
    let raw = raw.trim();
    if let Some(percent) = raw.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| unit_to_byte_round(v / 100.0));
    }
    raw.parse::<f32>().ok().map(|v| {
        if v <= 1.0 {
            unit_to_byte_round(v)
        } else {
            v.clamp(0.0, 255.0).round() as u8
        }
    })
}

fn parse_alpha_component(alpha: Option<&str>) -> Option<u8> {
    let Some(raw) = alpha else {
        return Some(255);
    };
    if let Some(percent) = raw.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| unit_to_byte_round(v / 100.0));
    }
    raw.trim().parse::<f32>().ok().map(unit_to_byte_round)
}

fn parse_unit_component(raw: &str) -> Option<f32> {
    if raw == "none" {
        return Some(0.0);
    }
    if let Some(percent) = raw.strip_suffix('%') {
        return percent.trim().parse::<f32>().ok().map(|v| v / 100.0);
    }
    raw.trim().parse::<f32>().ok()
}

fn parse_number_component(raw: &str) -> Option<f32> {
    if raw == "none" {
        return Some(0.0);
    }
    raw.trim().parse::<f32>().ok()
}

fn parse_lightness_percent(raw: &str) -> Option<f32> {
    if let Some(percent) = raw.trim().strip_suffix('%') {
        return percent.trim().parse::<f32>().ok();
    }
    raw.trim().parse::<f32>().ok()
}

fn parse_unit_lightness(raw: &str) -> Option<f32> {
    if let Some(percent) = raw.trim().strip_suffix('%') {
        return percent.trim().parse::<f32>().ok().map(|v| v / 100.0);
    }
    raw.trim().parse::<f32>().ok()
}

fn unit_to_byte_round(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn unit_to_byte_floor(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).floor() as u8
}

fn rgb_color(rgb: (f32, f32, f32), alpha: u8) -> Color {
    Color {
        r: unit_to_byte_round(rgb.0),
        g: unit_to_byte_round(rgb.1),
        b: unit_to_byte_round(rgb.2),
        a: alpha,
    }
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn linear_srgb_to_srgb(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    (linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b))
}

fn display_p3_to_srgb(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let r = srgb_to_linear(r);
    let g = srgb_to_linear(g);
    let b = srgb_to_linear(b);
    let x = 0.486_570_95 * r + 0.265_667_7 * g + 0.198_217_29 * b;
    let y = 0.228_974_57 * r + 0.691_738_55 * g + 0.079_286_92 * b;
    let z = 0.045_113_38 * g + 1.043_944_4 * b;
    xyz_d65_to_srgb(x, y, z)
}

fn xyz_d65_to_srgb(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let r = 3.240_97 * x - 1.537_383_2 * y - 0.498_610_76 * z;
    let g = -0.969_243_65 * x + 1.875_967_5 * y + 0.041_555_06 * z;
    let b = 0.055_630_08 * x - 0.203_976_96 * y + 1.056_971_5 * z;
    linear_srgb_to_srgb(r, g, b)
}

fn lab_to_srgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let x = 0.964_22 * lab_inv_f(fx);
    let y = lab_inv_f(fy);
    let z = 0.825_21 * lab_inv_f(fz);
    let x_d65 = 0.955_576_6 * x - 0.023_039_3 * y + 0.063_163_6 * z;
    let y_d65 = -0.028_289_5 * x + 1.009_941_6 * y + 0.021_007_7 * z;
    let z_d65 = 0.012_298_2 * x - 0.020_483 * y + 1.329_909_8 * z;
    xyz_d65_to_srgb(x_d65, y_d65, z_d65)
}

fn lab_inv_f(v: f32) -> f32 {
    const EPSILON: f32 = 216.0 / 24_389.0;
    const KAPPA: f32 = 24_389.0 / 27.0;
    let cube = v * v * v;
    if cube > EPSILON {
        cube
    } else {
        (116.0 * v - 16.0) / KAPPA
    }
}

fn oklab_to_srgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    linear_srgb_to_srgb(
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    )
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
