use crate::types::Color;
use cssparser_color::{Color as ParsedColor, hsl_to_rgb, hwb_to_rgb, parse_color_keyword};

use super::{CssMathExpression, CssValue, PendingMathExpression, SpecifiedColor};

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

    if let Some(math_value) = parse_math_expression(val) {
        return Some(math_value);
    }

    if let Some(pending) = PendingMathExpression::parse(val) {
        return Some(CssValue::PendingMath(pending));
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
        return number.parse::<f32>().ok().map(CssValue::Em);
    }

    val.parse::<f32>().ok().map(|value| {
        // CSS Values 4 permits a unitless zero wherever a <length> is
        // expected. Preserve non-zero bare values as numbers so typed
        // arithmetic and number-valued properties cannot confuse them with
        // lengths, while letting every length consumer handle the universal
        // zero without property-by-property exceptions.
        if value == 0.0 {
            CssValue::Length(0.0)
        } else {
            CssValue::Number(value)
        }
    })
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

pub(crate) fn parse_math_expression(value: &str) -> Option<CssValue> {
    const FUNCTIONS: &[&str] = &[
        "calc(", "min(", "max(", "clamp(", "round(", "rem(", "mod(", "sin(", "cos(", "tan(",
        "asin(", "acos(", "atan(", "atan2(", "pow(", "sqrt(", "hypot(", "log(", "exp(", "abs(",
        "sign(",
    ];
    let value = value.trim();
    FUNCTIONS
        .iter()
        .any(|function| {
            value
                .get(..function.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(function))
        })
        .then(|| CssMathExpression::parse(value).map(CssValue::Math))?
}

pub(crate) fn parse_color(val: &str) -> Option<CssValue> {
    let val = val.trim();
    let lower = val.to_ascii_lowercase();

    if lower == "currentcolor" {
        return Some(CssValue::Color(SpecifiedColor::CurrentColor));
    }

    if let Some(color) = named_color(&lower) {
        return Some(css_color(color));
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
        .strip_prefix("hsla(")
        .and_then(|s| s.strip_suffix(')'))
    {
        return parse_hsl_function(inner);
    }

    if let Some(inner) = lower.strip_prefix("hsl(").and_then(|s| s.strip_suffix(')')) {
        return parse_hsl_function(inner);
    }

    if let Some(inner) = lower.strip_prefix("hwb(").and_then(|s| s.strip_suffix(')')) {
        return parse_hwb_function(inner);
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

/// Validate the CSS Multi-column line grammar before an unsupported property
/// enters Lightning's generic custom-property representation. This keeps a
/// malformed later declaration from replacing an earlier valid rule while the
/// computed layer remains the single owner of used widths and colors.
pub(super) fn column_rule_value_is_valid(property: &str, value: &str) -> bool {
    match property {
        "column-rule-style" => value.parse::<crate::style::computed::BorderStyle>().is_ok(),
        "column-rule-width" => line_width_token_is_valid(value),
        "column-rule-color" => parse_color(value).is_some(),
        "column-rule" => {
            let Some(tokens) = radius_components(value) else {
                return false;
            };
            if tokens.is_empty() {
                return false;
            }

            let mut width = false;
            let mut style = false;
            let mut color = false;
            for token in tokens {
                if token.parse::<crate::style::computed::BorderStyle>().is_ok() {
                    if std::mem::replace(&mut style, true) {
                        return false;
                    }
                } else if line_width_token_is_valid(token) {
                    if std::mem::replace(&mut width, true) {
                        return false;
                    }
                } else if parse_color(token).is_some() {
                    if std::mem::replace(&mut color, true) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            true
        }
        _ => true,
    }
}

fn line_width_token_is_valid(value: &str) -> bool {
    let value = value.trim();
    if matches!(
        value.to_ascii_lowercase().as_str(),
        "thin" | "medium" | "thick"
    ) {
        return true;
    }
    if let Ok(number) = value.parse::<f32>() {
        return number.is_finite() && number == 0.0;
    }
    match parse_length(value) {
        Some(
            CssValue::Length(value)
            | CssValue::Em(value)
            | CssValue::Ex(value)
            | CssValue::Ch(value)
            | CssValue::Rem(value)
            | CssValue::Vw(value)
            | CssValue::Vh(value)
            | CssValue::Vmin(value)
            | CssValue::Vmax(value),
        ) => value.is_finite() && value >= 0.0,
        Some(CssValue::Number(value)) => value.is_finite() && value == 0.0,
        // Math and deferred substitutions are checked at computed-value time.
        Some(CssValue::Math(_) | CssValue::PendingMath(_) | CssValue::Var(_, _)) => true,
        Some(CssValue::Keyword(value)) => value
            .strip_suffix("cap")
            .or_else(|| value.strip_suffix("lh"))
            .and_then(|number| number.parse::<f32>().ok())
            .is_some_and(|number| number.is_finite() && number >= 0.0),
        _ => false,
    }
}

fn css_color(color: Color) -> CssValue {
    CssValue::Color(SpecifiedColor::Absolute(color))
}

fn radius_groups(value: &str) -> Option<(&str, Option<&str>)> {
    let mut depth = 0usize;
    let mut slash = None;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            '/' if depth == 0 && slash.replace(index).is_some() => {
                return None;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    let (horizontal, vertical) = slash.map_or((value, None), |index| {
        (&value[..index], Some(&value[index + 1..]))
    });
    let horizontal = horizontal.trim();
    let vertical = vertical.map(str::trim);
    (!horizontal.is_empty() && !vertical.is_some_and(str::is_empty))
        .then_some((horizontal, vertical))
}

fn radius_components(value: &str) -> Option<Vec<&str>> {
    let mut components = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' => depth = depth.checked_sub(1)?,
            ch if ch.is_whitespace() && depth == 0 => {
                if let Some(start) = start.take() {
                    components.push(value[start..index].trim());
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if depth != 0 {
        return None;
    }
    if let Some(start) = start {
        components.push(value[start..].trim());
    }
    Some(components)
}

pub(crate) struct RadiusComponents<'a> {
    pub horizontal: Vec<&'a str>,
    pub vertical: Option<Vec<&'a str>>,
}

pub(crate) fn split_radius_components(
    value: &str,
    shorthand: bool,
) -> Option<RadiusComponents<'_>> {
    let (horizontal, vertical) = radius_groups(value)?;
    if !shorthand && vertical.is_some() {
        return None;
    }
    let horizontal = radius_components(horizontal)?;
    let vertical = match vertical {
        Some(value) => Some(radius_components(value)?),
        None => None,
    };
    if !(1..=if shorthand { 4 } else { 2 }).contains(&horizontal.len())
        || vertical
            .as_ref()
            .is_some_and(|components| !(1..=4).contains(&components.len()))
    {
        return None;
    }
    Some(RadiusComponents {
        horizontal,
        vertical,
    })
}

fn radius_component_is_valid(value: &str) -> bool {
    if value
        .parse::<f32>()
        .is_ok_and(|number| number.is_finite() && number == 0.0)
    {
        return true;
    }
    if value.parse::<f32>().is_ok() {
        return false;
    }
    if (value.starts_with("var(") || value.starts_with("env(")) && value.ends_with(')') {
        return true;
    }
    match parse_length(value) {
        Some(
            CssValue::Length(value)
            | CssValue::Em(value)
            | CssValue::Percentage(value)
            | CssValue::Ex(value)
            | CssValue::Ch(value)
            | CssValue::Rem(value)
            | CssValue::Vw(value)
            | CssValue::Vh(value)
            | CssValue::Vmin(value)
            | CssValue::Vmax(value),
        ) => value.is_finite() && value >= 0.0,
        Some(CssValue::Number(value)) => value.is_finite() && value == 0.0,
        Some(CssValue::Math(_) | CssValue::PendingMath(_) | CssValue::Var(_, _)) => true,
        Some(CssValue::Keyword(value)) => value
            .strip_suffix("cap")
            .or_else(|| value.strip_suffix("lh"))
            .and_then(|number| number.parse::<f32>().ok())
            .is_some_and(|number| number.is_finite() && number >= 0.0),
        _ => false,
    }
}

pub(super) fn border_radius_value_is_valid(value: &str, shorthand: bool) -> bool {
    let Some(components) = split_radius_components(value, shorthand) else {
        return false;
    };
    components
        .horizontal
        .into_iter()
        .chain(components.vertical.into_iter().flatten())
        .all(radius_component_is_valid)
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

    if let Some(math_value) = parse_math_expression(val) {
        return Some(math_value);
    }

    if is_css_wide_keyword(&lower) {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(
        property,
        "border-color" | "border-block-color" | "border-inline-color"
    ) {
        return parse_color(val).or_else(|| Some(CssValue::Keyword(val.to_string())));
    }

    if property.contains("color") {
        return parse_color(val);
    }

    if property == "font-size-adjust" {
        if lower == "none" {
            return Some(CssValue::Keyword(lower));
        }
        if let Ok(value) = val.parse::<f32>() {
            return Some(CssValue::Number(value));
        }
        return Some(CssValue::Keyword(val.to_string()));
    }

    if matches!(
        property,
        "font-weight" | "font-style" | "font-stretch" | "font-width"
    ) {
        return Some(CssValue::Keyword(lower));
    }

    if matches!(property, "font-family" | "font") {
        return Some(CssValue::Keyword(val.trim().to_string()));
    }

    if matches!(
        property,
        "text-align"
            | "text-align-last"
            | "text-decoration"
            | "text-decoration-line"
            | "text-decoration-style"
            | "text-decoration-skip-ink"
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
        return Some(CssValue::Keyword(val.trim().to_string()));
    }

    if matches!(
        property,
        "border"
            | "border-style"
            | "border-block"
            | "border-inline"
            | "border-block-start"
            | "border-block-end"
            | "border-inline-start"
            | "border-inline-end"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-block-style"
            | "border-inline-style"
            | "border-block-start-style"
            | "border-block-end-style"
            | "border-inline-start-style"
            | "border-inline-end-style"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
    ) {
        return Some(CssValue::Keyword(val.to_string()));
    }

    if matches!(
        property,
        "border-width" | "border-block-width" | "border-inline-width"
    ) {
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
        "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-block-start-width"
            | "border-block-end-width"
            | "border-inline-start-width"
            | "border-inline-end-width"
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

    if property == "footnote-display" {
        return matches!(lower.as_str(), "block" | "inline" | "compact")
            .then_some(CssValue::Keyword(lower));
    }

    if property == "footnote-policy" {
        return matches!(lower.as_str(), "auto" | "line" | "block")
            .then_some(CssValue::Keyword(lower));
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
        return val
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|number| number.is_finite() && *number >= 0.0)
            .map(CssValue::Number);
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
            | "system"
            | "symbols"
            | "prefix"
            | "suffix"
            | "pad"
            | "negative"
            | "fallback"
            | "range"
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
            | "scrollbar-width"
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
            | "-webkit-background-clip"
            | "background-attachment"
            | "border-image"
            | "border-image-source"
            | "border-image-slice"
            | "border-image-width"
            | "border-image-outset"
            | "border-image-repeat"
            | "background-image"
            | "white-space"
            | "overflow-wrap"
            | "word-wrap"
            | "word-break"
            | "line-break"
            | "text-transform"
            | "font-variant"
            | "font-variant-caps"
            | "font-variant-position"
            | "font-variant-ligatures"
            | "font-kerning"
            | "font-size-adjust"
            | "font-synthesis"
            | "initial-letter"
            | "text-emphasis"
            | "text-emphasis-style"
            | "text-emphasis-position"
            | "-webkit-text-emphasis"
            | "-webkit-text-emphasis-style"
            | "-webkit-text-emphasis-position"
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
            | "image-rendering"
            | "vertical-align"
            | "inset"
            | "line-clamp"
            | "-webkit-line-clamp"
    ) {
        return Some(CssValue::Keyword(val.to_string()));
    }

    if property == "column-count" {
        if lower == "auto" {
            return Some(CssValue::Keyword(lower));
        }
        return val
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|count| *count >= 1)
            .map(|count| CssValue::Number(count as f32));
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

    // Radius declarations are validated as a whole before entering StyleMap so
    // a malformed later declaration cannot replace an earlier cascaded winner.
    if matches!(
        property,
        "border-radius"
            | "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-right-radius"
            | "border-bottom-left-radius"
            | "border-start-start-radius"
            | "border-start-end-radius"
            | "border-end-start-radius"
            | "border-end-end-radius"
    ) {
        if !border_radius_value_is_valid(val, property == "border-radius") {
            return None;
        }
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

    // CSS Text defines `normal` as a real computed value for both text
    // spacing properties. It must survive parsing so it can reset an inherited
    // non-normal value.
    if matches!(property, "letter-spacing" | "word-spacing") && lower == "normal" {
        return Some(CssValue::Keyword(lower));
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
    match parse_color_keyword::<ParsedColor>(name).ok()? {
        ParsedColor::Rgba(color) => Some(Color::from_css_rgb(
            f32::from(color.red),
            f32::from(color.green),
            f32::from(color.blue),
            color.alpha * 255.0,
        )),
        ParsedColor::CurrentColor => None,
        _ => None,
    }
}

fn parse_hex_color(hex: &str) -> Option<CssValue> {
    let bytes = hex.as_bytes();
    match bytes {
        // #rgb
        [r, g, b] => Some(css_color(Color::rgb(
            hex_digit(*r)? * 17,
            hex_digit(*g)? * 17,
            hex_digit(*b)? * 17,
        ))),
        // #rgba
        [r, g, b, a] => Some(css_color(Color::rgba8(
            hex_digit(*r)? * 17,
            hex_digit(*g)? * 17,
            hex_digit(*b)? * 17,
            hex_digit(*a)? * 17,
        ))),
        // #rrggbb
        [r1, r2, g1, g2, b1, b2] => Some(css_color(Color::rgb(
            hex_pair(*r1, *r2)?,
            hex_pair(*g1, *g2)?,
            hex_pair(*b1, *b2)?,
        ))),
        // #rrggbbaa
        [r1, r2, g1, g2, b1, b2, a1, a2] => Some(css_color(Color::rgba8(
            hex_pair(*r1, *r2)?,
            hex_pair(*g1, *g2)?,
            hex_pair(*b1, *b2)?,
            hex_pair(*a1, *a2)?,
        ))),
        _ => None,
    }
}

fn parse_rgb_function(inner: &str) -> Option<CssValue> {
    if inner.contains(',') {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        let (r, g, b, alpha) = match parts.as_slice() {
            [r, g, b] => (*r, *g, *b, None),
            [r, g, b, alpha] => (*r, *g, *b, Some(*alpha)),
            _ => return None,
        };

        // The comma form is the legacy grammar: all three channels must use
        // the same number/percentage type. Keep parsed values continuous;
        // backend-specific representation belongs at that backend seam.
        let percentage = rgb_component_is_percentage(r);
        if rgb_component_is_percentage(g) != percentage
            || rgb_component_is_percentage(b) != percentage
        {
            return None;
        }
        Some(css_color(Color::from_css_rgb(
            parse_rgb_255_component(r)?,
            parse_rgb_255_component(g)?,
            parse_rgb_255_component(b)?,
            parse_alpha_component(alpha)?,
        )))
    } else {
        let (components, alpha) = split_color_alpha(inner);
        let parts: Vec<&str> = components.split_whitespace().collect();
        match parts.as_slice() {
            [r, g, b] => Some(css_color(Color::from_css_rgb(
                parse_rgb_255_component(r)?,
                parse_rgb_255_component(g)?,
                parse_rgb_255_component(b)?,
                parse_alpha_component(alpha)?,
            ))),
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
    parse_rgb_function(inner)
}

/// Parse hsl()/hsla() without routing through Lightning's byte-backed RGBA
/// representation. Blink's PDF output retains the continuous HSL conversion
/// and authored alpha, so both are kept as floats until a real raster boundary.
fn parse_hsl_function(inner: &str) -> Option<CssValue> {
    let (hue, saturation, lightness, alpha) = if inner.contains(',') {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        match parts.as_slice() {
            [h, s, l] => (*h, *s, *l, None),
            [h, s, l, a] => (*h, *s, *l, Some(*a)),
            _ => return None,
        }
    } else {
        let (components, alpha) = split_color_alpha(inner);
        let parts: Vec<&str> = components.split_whitespace().collect();
        let [h, s, l] = parts.as_slice() else {
            return None;
        };
        (*h, *s, *l, alpha)
    };

    let h = parse_hue_degrees(hue)? / 360.0;
    let s = parse_hsl_percentage(saturation)?;
    let l = parse_hsl_percentage(lightness)?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Some(css_color(Color::from_srgb(
        r,
        g,
        b,
        parse_alpha_component(alpha)? / 255.0,
    )))
}

/// Parse hwb() in continuous sRGB. When whiteness plus blackness is at least
/// one, CSS Color 4 normalizes the two components to an achromatic result.
fn parse_hwb_function(inner: &str) -> Option<CssValue> {
    if inner.contains(',') {
        return None;
    }
    let (components, alpha) = split_color_alpha(inner);
    let parts: Vec<&str> = components.split_whitespace().collect();
    let [h, w, b] = parts.as_slice() else {
        return None;
    };
    let h = parse_hue_degrees(h)? / 360.0;
    let w = parse_hsl_percentage(w)?;
    let black = parse_hsl_percentage(b)?;
    let (r, g, blue) = hwb_to_rgb(h, w, black);
    Some(css_color(Color::from_srgb(
        r,
        g,
        blue,
        parse_alpha_component(alpha)? / 255.0,
    )))
}

fn parse_hue_degrees(raw: &str) -> Option<f32> {
    let raw = raw.trim();
    if raw == "none" {
        return Some(0.0);
    }
    let degrees = if let Some(value) = raw.strip_suffix("deg") {
        value.trim().parse::<f32>().ok()?
    } else if let Some(value) = raw.strip_suffix("grad") {
        value.trim().parse::<f32>().ok()? * 0.9
    } else if let Some(value) = raw.strip_suffix("rad") {
        value.trim().parse::<f32>().ok()?.to_degrees()
    } else if let Some(value) = raw.strip_suffix("turn") {
        value.trim().parse::<f32>().ok()? * 360.0
    } else {
        raw.parse::<f32>().ok()?
    };
    degrees.is_finite().then_some(degrees.rem_euclid(360.0))
}

fn parse_hsl_percentage(raw: &str) -> Option<f32> {
    let raw = raw.trim();
    if raw == "none" {
        return Some(0.0);
    }
    let value = raw.strip_suffix('%')?.trim().parse::<f32>().ok()?;
    value.is_finite().then_some((value / 100.0).clamp(0.0, 1.0))
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
            return Some(css_color(Color::from_css_rgb(
                unit_to_css_channel(coords[0]),
                unit_to_css_channel(coords[1]),
                unit_to_css_channel(coords[2]),
                parse_alpha_component(alpha)?,
            )));
        }
        "srgb-linear" => linear_srgb_to_srgb(coords[0], coords[1], coords[2]),
        "display-p3" => display_p3_to_srgb(coords[0], coords[1], coords[2]),
        "xyz" | "xyz-d65" => xyz_d65_to_srgb(coords[0], coords[1], coords[2]),
        _ => return None,
    };
    Some(css_color(rgb_color(rgb, parse_alpha_component(alpha)?)))
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
    Some(css_color(rgb_color(
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
    Some(css_color(rgb_color(
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
    Some(css_color(rgb_color(
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
    Some(css_color(rgb_color(
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

fn parse_rgb_255_component(raw: &str) -> Option<f32> {
    let raw = raw.trim();
    if let Some(percent) = raw.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .map(|v| (v.clamp(0.0, 100.0) / 100.0) * 255.0);
    }
    raw.parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|v| v.clamp(0.0, 255.0))
}

fn rgb_component_is_percentage(raw: &str) -> bool {
    raw.trim().ends_with('%')
}

fn parse_alpha_component(alpha: Option<&str>) -> Option<f32> {
    let Some(raw) = alpha else {
        return Some(255.0);
    };
    if let Some(percent) = raw.strip_suffix('%') {
        return percent
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .map(|v| (v.clamp(0.0, 100.0) / 100.0) * 255.0);
    }
    raw.trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|v| v.clamp(0.0, 1.0) * 255.0)
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

fn unit_to_css_channel(value: f32) -> f32 {
    value.clamp(0.0, 1.0) * 255.0
}

fn rgb_color(rgb: (f32, f32, f32), alpha: f32) -> Color {
    Color::from_css_rgb(
        unit_to_css_channel(rgb.0),
        unit_to_css_channel(rgb.1),
        unit_to_css_channel(rgb.2),
        alpha,
    )
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
