use super::{
    CssValue, StyleMap,
    imports::extract_svg_data_uri,
    is_css_wide_keyword,
    lightning::parse_inline_style_with_lightning,
    parse_length,
    values::{
        border_spacing_value_count, parse_border_spacing_shorthand, parse_property_value,
        parse_var_function,
    },
};

/// Parse an inline CSS style string (e.g. "color: red; font-size: 14px").
pub fn parse_inline_style(style: &str) -> StyleMap {
    let legacy = parse_inline_style_legacy(style);
    let Some(mut parsed) = parse_inline_style_with_lightning(style) else {
        return legacy;
    };

    reconcile_legacy_value_forms(&mut parsed, &legacy);
    parsed
}

pub(crate) fn parse_inline_style_legacy(style: &str) -> StyleMap {
    let mut map = StyleMap::new();

    for declaration in style
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Some((prop, val)) = declaration.split_once(':') else {
            continue;
        };

        let raw_prop = prop.trim();
        let val = val.trim();
        let (val, is_important) = if let Some(stripped) = val.strip_suffix("!important") {
            (stripped.trim_end(), true)
        } else {
            (val, false)
        };

        apply_declaration(&mut map, raw_prop, val, is_important);
    }

    map
}

pub(super) fn apply_declaration(map: &mut StyleMap, raw_prop: &str, val: &str, is_important: bool) {
    if raw_prop.starts_with("--") {
        map.set_with_importance(raw_prop, CssValue::Keyword(val.to_string()), is_important);
        return;
    }

    let mut prop = raw_prop.to_ascii_lowercase();
    // Vendor-prefixed CSS Masking aliases (`-webkit-mask*`) are treated as the
    // equivalent unprefixed properties (css-masking-1; widely used in the wild).
    if prop == "-webkit-background-clip" {
        prop = "background-clip".to_string();
    } else if prop == "-webkit-text-fill-color" {
        prop = "color".to_string();
    } else if let Some(unprefixed) = prop.strip_prefix("-webkit-mask") {
        prop = format!("mask{unprefixed}");
    }
    let prop = prop;
    if (prop == "margin" || prop == "padding") && !prop.contains('-') {
        expand_box_shorthand(map, &prop, val, is_important);
        return;
    }

    if (prop == "margin-left"
        || prop == "margin-right"
        || prop == "margin-top"
        || prop == "margin-bottom")
        && val == "auto"
    {
        map.set_with_importance(&prop, CssValue::Keyword("auto".to_string()), is_important);
        return;
    }

    if prop == "background" {
        let trimmed = val.trim();
        let lower = trimmed.to_ascii_lowercase();
        if is_css_wide_keyword(&lower) {
            clear_background_shorthand_keys(map);
            map.set_with_importance("background", CssValue::Keyword(lower), is_important);
            return;
        }

        // A bare `background: var(--x)` can't be classified at parse time
        // (custom properties resolve in the cascade). Defer it as a
        // background-color Var so computed-time var resolution handles it.
        if let Some(var_val) = parse_var_function(trimmed) {
            clear_background_shorthand_keys(map);
            map.set_with_importance("background-color", var_val, is_important);
            return;
        }

        let mut parsed = StyleMap::new();
        if parse_background_shorthand(trimmed, &mut parsed, is_important) {
            clear_background_shorthand_keys(map);
            map.merge(&parsed);
            return;
        }
    }

    if prop == "background-image" {
        clear_background_image_keys(map);
        if apply_background_image_value(map, val.trim(), is_important) {
            return;
        }
    }

    if prop == "background-position-x" || prop == "background-position-y" {
        apply_background_position_axis(map, &prop, val.trim(), is_important);
        return;
    }

    if prop == "border-spacing" {
        if let Some((horizontal, vertical)) = parse_border_spacing_shorthand(val) {
            if let Some(count) = border_spacing_value_count(val) {
                map.set_with_importance(
                    "border-spacing-value-count",
                    CssValue::Number(count as f32),
                    is_important,
                );
            }
            map.set_with_importance("border-spacing", horizontal.clone(), is_important);
            map.set_with_importance("border-spacing-horizontal", horizontal, is_important);
            map.set_with_importance("border-spacing-vertical", vertical, is_important);
            return;
        }
    }

    if prop == "border-image" {
        map.set_with_importance(
            "border-image",
            CssValue::Keyword(val.trim().to_string()),
            is_important,
        );
        return;
    }

    if let Some(css_value) = parse_property_value(&prop, val) {
        map.set_with_importance(&prop, css_value, is_important);
    }
}

fn clear_background_image_keys(map: &mut StyleMap) {
    for key in [
        "background-image",
        "background-svg",
        "background-gradient",
        "background-radial-gradient",
        "background-conic-gradient",
        "background-layer-slots",
    ] {
        map.remove(key);
    }
}

fn clear_background_shorthand_keys(map: &mut StyleMap) {
    clear_background_image_keys(map);
    for key in [
        "background",
        "background-color",
        "background-size",
        "background-repeat",
        "background-position",
        "background-origin",
        "background-clip",
        "background-attachment",
        "border-image",
    ] {
        map.remove(key);
    }
}

/// Split a comma-separated CSS value into its top-level parts, ignoring commas
/// that appear inside parentheses (e.g. `linear-gradient(a, b)`) or quotes
/// (e.g. a `url("data:...,...")` data URI). Used to separate comma-separated
/// `background-image` layers so each layer is parsed independently.
fn split_top_level_commas(val: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0u32;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    for ch in val.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if (in_single_quote || in_double_quote) && ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }

        match ch {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
                current.push(ch);
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                current.push(ch);
            }
            '(' if !in_single_quote && !in_double_quote => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' if !in_single_quote && !in_double_quote && paren_depth > 0 => {
                paren_depth -= 1;
                current.push(ch);
            }
            ',' if paren_depth == 0 && !in_single_quote && !in_double_quote => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    parts.push(current);
    parts
}

/// The paint slot a single `background-image` layer maps to. The data model
/// carries one raster/SVG slot and one gradient slot, so each comma-separated
/// layer is classified into one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackgroundLayerSlot {
    /// A raster (`url(...)`) or SVG (`data:image/svg+xml`) image layer.
    Raster,
    /// A `linear-gradient(...)` or `radial-gradient(...)` layer.
    Gradient,
    /// A `none` layer (occupies a list position but paints nothing).
    None,
}

impl BackgroundLayerSlot {
    fn as_str(self) -> &'static str {
        match self {
            BackgroundLayerSlot::Raster => "raster",
            BackgroundLayerSlot::Gradient => "gradient",
            BackgroundLayerSlot::None => "none",
        }
    }
}

/// Apply a `background-image` value, supporting multiple comma-separated layers.
///
/// Each layer is parsed independently via [`apply_single_background_image_value`].
/// The data model carries a single raster/SVG layer (`background-image` /
/// `background-svg`) and a single gradient layer (`background-gradient` /
/// `background-radial-gradient`) separately, so a `url(...), linear-gradient(...)`
/// list can populate both keys. Per CSS the first listed layer paints on top,
/// which matches the renderer's gradient-then-raster paint order.
///
/// When more than one layer is present, the slot of each list position is
/// recorded in the `background-layer-slots` key (a comma-joined keyword list,
/// e.g. `raster,gradient`). The style cascade uses that mapping to assign the
/// matching comma-separated `background-size` / `-position` / `-repeat` entry to
/// each slot.
///
/// Returns `true` if at least one layer was recognised and applied.
fn apply_background_image_value(map: &mut StyleMap, value: &str, is_important: bool) -> bool {
    let layers = split_top_level_commas(value);
    if layers.len() <= 1 {
        return apply_single_background_image_value(map, value, is_important).is_some();
    }

    let mut applied = false;
    let mut saw_raster = false;
    let mut saw_gradient = false;
    let mut first_none: Option<StyleMap> = None;
    let mut slots: Vec<&'static str> = Vec::with_capacity(layers.len());
    for layer in &layers {
        let mut layer_map = StyleMap::new();
        match apply_single_background_image_value(&mut layer_map, layer, is_important) {
            Some(slot) => {
                slots.push(slot.as_str());
                applied = true;
                match slot {
                    BackgroundLayerSlot::Raster if !saw_raster => {
                        map.merge(&layer_map);
                        saw_raster = true;
                    }
                    BackgroundLayerSlot::Gradient if !saw_gradient => {
                        map.merge(&layer_map);
                        saw_gradient = true;
                    }
                    BackgroundLayerSlot::None if first_none.is_none() => {
                        first_none = Some(layer_map);
                    }
                    _ => {}
                }
            }
            None => slots.push(BackgroundLayerSlot::None.as_str()),
        }
    }
    if applied {
        if !saw_raster
            && !saw_gradient
            && let Some(none_map) = first_none
        {
            map.merge(&none_map);
        }
        map.set_with_importance(
            "background-layer-slots",
            CssValue::Keyword(slots.join(",")),
            is_important,
        );
    }
    applied
}

/// Apply a single (non-comma-separated) `background-image` layer. Returns the
/// paint slot it occupies, or `None` if the layer was not recognised.
fn apply_single_background_image_value(
    map: &mut StyleMap,
    value: &str,
    is_important: bool,
) -> Option<BackgroundLayerSlot> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("linear-gradient(") || lower.starts_with("repeating-linear-gradient(") {
        map.set_with_importance(
            "background-gradient",
            CssValue::Keyword(trimmed.to_string()),
            is_important,
        );
        return Some(BackgroundLayerSlot::Gradient);
    }

    if lower.starts_with("radial-gradient(") || lower.starts_with("repeating-radial-gradient(") {
        map.set_with_importance(
            "background-radial-gradient",
            CssValue::Keyword(trimmed.to_string()),
            is_important,
        );
        return Some(BackgroundLayerSlot::Gradient);
    }

    if lower.starts_with("conic-gradient(") || lower.starts_with("repeating-conic-gradient(") {
        map.set_with_importance(
            "background-conic-gradient",
            CssValue::Keyword(trimmed.to_string()),
            is_important,
        );
        return Some(BackgroundLayerSlot::Gradient);
    }

    if lower == "none" {
        map.set_with_importance(
            "background-image",
            CssValue::Keyword("none".to_string()),
            is_important,
        );
        return Some(BackgroundLayerSlot::None);
    }

    if let Some(svg_text) = extract_svg_data_uri(trimmed) {
        map.set_with_importance("background-svg", CssValue::Keyword(svg_text), is_important);
        return Some(BackgroundLayerSlot::Raster);
    }

    // A non-SVG `url(...)` is a raster image layer. Preserve the full `url(...)`
    // token (rather than just the path) so the raster builder can resolve it.
    if let Some(url) = extract_image_set_url(trimmed) {
        map.set_with_importance("background-image", CssValue::Keyword(url), is_important);
        return Some(BackgroundLayerSlot::Raster);
    }

    if lower.starts_with("url(") {
        map.set_with_importance(
            "background-image",
            CssValue::Keyword(trimmed.to_string()),
            is_important,
        );
        return Some(BackgroundLayerSlot::Raster);
    }

    None
}

fn extract_image_set_url(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    let inner = lower
        .strip_prefix("image-set(")
        .or_else(|| lower.strip_prefix("-webkit-image-set("))?;
    if !inner.ends_with(')') {
        return None;
    }
    let raw_inner = &value.trim()[value.trim().find('(')? + 1..value.trim().len() - 1];
    split_top_level_commas(raw_inner)
        .into_iter()
        .find_map(|candidate| {
            let token = candidate.trim();
            token.find("url(").and_then(|start| {
                let tail = &token[start..];
                let end = tail.find(')')?;
                Some(tail[..=end].to_string())
            })
        })
}

fn apply_background_shorthand_defaults(map: &mut StyleMap, is_important: bool) {
    map.set_with_importance(
        "background-color",
        CssValue::Keyword("initial".to_string()),
        is_important,
    );
    map.set_with_importance(
        "background-image",
        CssValue::Keyword("none".to_string()),
        is_important,
    );
    map.set_with_importance(
        "background-size",
        CssValue::Keyword("auto".to_string()),
        is_important,
    );
    map.set_with_importance(
        "background-repeat",
        CssValue::Keyword("repeat".to_string()),
        is_important,
    );
    map.set_with_importance(
        "background-position",
        CssValue::Keyword("0% 0%".to_string()),
        is_important,
    );
    map.set_with_importance(
        "background-origin",
        CssValue::Keyword("padding-box".to_string()),
        is_important,
    );
    map.set_with_importance(
        "background-clip",
        CssValue::Keyword("border-box".to_string()),
        is_important,
    );
    map.set_with_importance(
        "background-attachment",
        CssValue::Keyword("scroll".to_string()),
        is_important,
    );
}

fn ensure_background_shorthand_defaults(
    map: &mut StyleMap,
    defaults_applied: &mut bool,
    is_important: bool,
) {
    if !*defaults_applied {
        apply_background_shorthand_defaults(map, is_important);
        *defaults_applied = true;
    }
}

#[derive(Default)]
struct BackgroundLayerParts {
    image: Option<String>,
    size: Option<String>,
    repeat: Option<String>,
    position: Option<String>,
    origin: Option<String>,
    clip: Option<String>,
    attachment: Option<String>,
    color: Option<CssValue>,
    recognized: bool,
}

impl BackgroundLayerParts {
    fn has_any(&self) -> bool {
        self.recognized || self.color.is_some()
    }
}

fn parse_background_layer(val: &str, allow_color: bool) -> BackgroundLayerParts {
    const ORIGIN_KEYWORDS: [&str; 3] = ["padding-box", "border-box", "content-box"];
    const REPEAT_KEYWORDS: [&str; 6] = [
        "no-repeat",
        "repeat",
        "repeat-x",
        "repeat-y",
        "space",
        "round",
    ];
    const ATTACHMENT_KEYWORDS: [&str; 3] = ["scroll", "fixed", "local"];
    const POSITION_KEYWORDS: [&str; 5] = ["center", "top", "bottom", "left", "right"];

    let mut layer = BackgroundLayerParts::default();
    let mut found_image = false;
    let mut found_repeat = false;
    let mut found_origin = false;
    let mut found_clip = false;
    let mut found_size = false;
    let mut found_color = false;
    let mut position_parts = Vec::new();
    let tokens = tokenize_background_value(val);
    let mut index = 0usize;

    while let Some(token) = tokens.get(index) {
        let lower = token.trim().to_ascii_lowercase();

        if !found_image
            && (lower.starts_with("linear-gradient(")
                || lower.starts_with("repeating-linear-gradient(")
                || lower.starts_with("radial-gradient(")
                || lower.starts_with("repeating-radial-gradient(")
                || lower.starts_with("conic-gradient(")
                || lower.starts_with("repeating-conic-gradient(")
                || lower.starts_with("url(")
                || lower.starts_with("image-set(")
                || lower.starts_with("-webkit-image-set(")
                || lower == "none")
        {
            layer.image = Some(token.trim().to_string());
            layer.recognized = true;
            found_image = true;
            index += 1;
            continue;
        }

        // In the `background` shorthand the box value sets `background-origin`
        // then `background-clip` (css-backgrounds-3 §3.10). The first box token
        // is the origin AND the clip; a second box token overrides the clip.
        if ORIGIN_KEYWORDS.contains(&lower.as_str()) && (!found_origin || !found_clip) {
            if !found_origin {
                layer.origin = Some(lower.clone());
                found_origin = true;
                // A lone box value also sets the clip; `found_clip` stays false
                // so a later box token can still override it below.
                layer.clip = Some(lower);
            } else {
                layer.clip = Some(lower);
                found_clip = true;
            }
            layer.recognized = true;
            index += 1;
            continue;
        }

        if !found_repeat && REPEAT_KEYWORDS.contains(&lower.as_str()) {
            let mut repeat = lower;
            if matches!(repeat.as_str(), "repeat" | "space" | "round" | "no-repeat")
                && let Some(next_token) = tokens.get(index + 1)
            {
                let next = next_token.trim().to_ascii_lowercase();
                if matches!(next.as_str(), "repeat" | "space" | "round" | "no-repeat") {
                    repeat.push(' ');
                    repeat.push_str(&next);
                    index += 1;
                }
            }
            layer.repeat = Some(repeat);
            layer.recognized = true;
            found_repeat = true;
            index += 1;
            continue;
        }

        if ATTACHMENT_KEYWORDS.contains(&lower.as_str()) {
            layer.attachment = Some(lower);
            layer.recognized = true;
            index += 1;
            continue;
        }

        if lower == "/" {
            index += 1;
            if !found_size {
                if let Some(size_token) = tokens.get(index) {
                    let mut size = size_token.trim().to_string();
                    if let Some(next_token) = tokens.get(index + 1) {
                        let next = next_token.trim().to_ascii_lowercase();
                        if is_background_size_continuation(
                            &next,
                            &ORIGIN_KEYWORDS,
                            &REPEAT_KEYWORDS,
                            &POSITION_KEYWORDS,
                        ) {
                            size.push(' ');
                            size.push_str(next_token.trim());
                            index += 1;
                        }
                    }
                    layer.size = Some(size);
                    layer.recognized = true;
                    found_size = true;
                }
            }
            index += 1;
            continue;
        }

        if POSITION_KEYWORDS.contains(&lower.as_str()) || is_background_position_length(token) {
            position_parts.push(token.trim().to_string());
            index += 1;
            continue;
        }

        if allow_color && !found_color {
            if let Some(color_value) = super::values::parse_color(token) {
                layer.color = Some(color_value);
                found_color = true;
                index += 1;
                continue;
            }
        }

        index += 1;
    }

    if !position_parts.is_empty() {
        layer.position = Some(position_parts.join(" "));
        layer.recognized = true;
    }

    layer
}

fn parse_background_shorthand(val: &str, map: &mut StyleMap, is_important: bool) -> bool {
    let layer_values = split_top_level_commas(val);
    let mut layers = Vec::with_capacity(layer_values.len());
    for (index, layer_value) in layer_values.iter().enumerate() {
        layers.push(parse_background_layer(
            layer_value,
            index + 1 == layer_values.len(),
        ));
    }

    if layers.iter().all(|layer| !layer.has_any()) {
        return false;
    }

    let mut defaults_applied = false;
    ensure_background_shorthand_defaults(map, &mut defaults_applied, is_important);

    let image_list = layers
        .iter()
        .map(|layer| layer.image.as_deref().unwrap_or("none"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = apply_background_image_value(map, &image_list, is_important);

    let size_list = layers
        .iter()
        .map(|layer| layer.size.as_deref().unwrap_or("auto"))
        .collect::<Vec<_>>()
        .join(", ");
    map.set_with_importance(
        "background-size",
        CssValue::Keyword(size_list),
        is_important,
    );

    let repeat_list = layers
        .iter()
        .map(|layer| layer.repeat.as_deref().unwrap_or("repeat"))
        .collect::<Vec<_>>()
        .join(", ");
    map.set_with_importance(
        "background-repeat",
        CssValue::Keyword(repeat_list),
        is_important,
    );

    let position_list = layers
        .iter()
        .map(|layer| layer.position.as_deref().unwrap_or("0% 0%"))
        .collect::<Vec<_>>()
        .join(", ");
    map.set_with_importance(
        "background-position",
        CssValue::Keyword(position_list),
        is_important,
    );

    let origin_list = layers
        .iter()
        .map(|layer| layer.origin.as_deref().unwrap_or("padding-box"))
        .collect::<Vec<_>>()
        .join(", ");
    map.set_with_importance(
        "background-origin",
        CssValue::Keyword(origin_list),
        is_important,
    );

    let clip_list = layers
        .iter()
        .map(|layer| layer.clip.as_deref().unwrap_or("border-box"))
        .collect::<Vec<_>>()
        .join(", ");
    map.set_with_importance(
        "background-clip",
        CssValue::Keyword(clip_list),
        is_important,
    );

    let attachment_list = layers
        .iter()
        .map(|layer| layer.attachment.as_deref().unwrap_or("scroll"))
        .collect::<Vec<_>>()
        .join(", ");
    map.set_with_importance(
        "background-attachment",
        CssValue::Keyword(attachment_list),
        is_important,
    );

    if let Some(color_value) = layers.last().and_then(|layer| layer.color.clone()) {
        map.set_with_importance("background-color", color_value, is_important);
    }

    true
}

fn is_background_size_continuation(
    token: &str,
    origin_keywords: &[&str],
    repeat_keywords: &[&str],
    position_keywords: &[&str],
) -> bool {
    !origin_keywords.contains(&token)
        && !repeat_keywords.contains(&token)
        && !position_keywords.contains(&token)
        && token != "/"
        && !token.starts_with("url(")
        && !token.starts_with('#')
        && super::values::parse_color(token).is_none()
}

fn is_background_position_length(token: &str) -> bool {
    matches!(
        parse_length(token),
        Some(CssValue::Length(_) | CssValue::Percentage(_) | CssValue::Calc(_))
    )
}

fn apply_background_position_axis(map: &mut StyleMap, prop: &str, value: &str, is_important: bool) {
    let axis_values: Vec<String> = split_top_level_commas(value)
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect();
    if axis_values.is_empty() {
        return;
    }

    let existing_positions = map
        .get("background-position")
        .and_then(|value| match value {
            CssValue::Keyword(position) => Some(
                split_top_level_commas(position)
                    .into_iter()
                    .map(|part| part.trim().to_string())
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .filter(|positions| !positions.is_empty())
        .unwrap_or_else(|| vec!["0% 0%".to_string()]);

    let layer_count = axis_values.len().max(existing_positions.len());
    let mut positions = Vec::with_capacity(layer_count);
    for index in 0..layer_count {
        let (mut x, mut y) =
            split_background_position_axes(&existing_positions[index % existing_positions.len()]);
        let axis = axis_values[index % axis_values.len()].clone();
        if prop == "background-position-x" {
            x = axis;
        } else {
            y = axis;
        }
        positions.push(format!("{x} {y}"));
    }

    map.set_with_importance(
        "background-position",
        CssValue::Keyword(positions.join(", ")),
        is_important,
    );
}

fn split_background_position_axes(position: &str) -> (String, String) {
    let tokens = tokenize_background_value(position);
    match tokens.as_slice() {
        [] => ("0%".to_string(), "0%".to_string()),
        [token] => {
            let lower = token.to_ascii_lowercase();
            if matches!(lower.as_str(), "top" | "bottom") {
                ("center".to_string(), token.trim().to_string())
            } else if lower == "center" {
                ("center".to_string(), "center".to_string())
            } else {
                (token.trim().to_string(), "center".to_string())
            }
        }
        [first, second] => {
            let first_lower = first.to_ascii_lowercase();
            let second_lower = second.to_ascii_lowercase();
            if matches!(first_lower.as_str(), "top" | "bottom")
                || matches!(second_lower.as_str(), "left" | "right")
            {
                (second.trim().to_string(), first.trim().to_string())
            } else {
                (first.trim().to_string(), second.trim().to_string())
            }
        }
        _ => {
            let split_at = tokens.len() / 2;
            (tokens[..split_at].join(" "), tokens[split_at..].join(" "))
        }
    }
}

fn tokenize_background_value(val: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut paren_depth = 0u32;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for ch in val.chars() {
        match ch {
            '\'' if !in_double_quote && paren_depth > 0 => {
                in_single_quote = !in_single_quote;
                current.push(ch);
            }
            '"' if !in_single_quote && paren_depth > 0 => {
                in_double_quote = !in_double_quote;
                current.push(ch);
            }
            '(' if !in_single_quote && !in_double_quote => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' if !in_single_quote && !in_double_quote && paren_depth > 0 => {
                paren_depth -= 1;
                current.push(ch);
            }
            ' ' | '\t' if paren_depth == 0 && !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            '/' if paren_depth == 0 && !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push("/".to_string());
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn reconcile_legacy_value_forms(parsed: &mut StyleMap, legacy: &StyleMap) {
    for (key, value) in &legacy.properties {
        let prefer_legacy = parsed
            .properties
            .get(key)
            .is_some_and(|parsed_value| prefer_legacy_value_form(key, parsed_value, value));
        if !parsed.properties.contains_key(key) || prefer_legacy {
            parsed.set_with_importance(key, value.clone(), legacy.is_important(key));
        }
    }
}

fn prefer_legacy_value_form(key: &str, parsed: &CssValue, legacy: &CssValue) -> bool {
    matches!(
        key,
        "font-family"
            | "filter"
            | "border"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "outline"
            | "background-image"
            | "background-size"
            | "background-position"
    ) || prefers_legacy_relative_length(key, parsed, legacy)
}

fn prefers_legacy_relative_length(key: &str, parsed: &CssValue, legacy: &CssValue) -> bool {
    matches!((parsed, legacy), (CssValue::Length(_), CssValue::Number(_)))
        && matches!(
            key,
            "width"
                | "height"
                | "max-width"
                | "min-width"
                | "max-height"
                | "min-height"
                | "margin-top"
                | "margin-right"
                | "margin-bottom"
                | "margin-left"
                | "padding-top"
                | "padding-right"
                | "padding-bottom"
                | "padding-left"
                | "top"
                | "left"
                | "gap"
                | "grid-gap"
                | "column-gap"
                | "border-width"
                | "border-radius"
                | "text-indent"
                | "letter-spacing"
                | "word-spacing"
                | "border-spacing"
                | "border-spacing-horizontal"
                | "border-spacing-vertical"
        )
}

fn expand_box_shorthand(map: &mut StyleMap, prop: &str, val: &str, is_important: bool) {
    let parts: Vec<&str> = val.split_whitespace().collect();
    if parts.len() > 1 {
        let (top, right, bottom, left) = match parts.as_slice() {
            [top, right] => (*top, *right, *top, *right),
            [top, right, bottom] => (*top, *right, *bottom, *right),
            [top, right, bottom, left] => (*top, *right, *bottom, *left),
            _ => return,
        };
        for (side, token) in [
            ("top", top),
            ("right", right),
            ("bottom", bottom),
            ("left", left),
        ] {
            let key = format!("{prop}-{side}");
            if token == "auto" {
                map.set_with_importance(&key, CssValue::Keyword("auto".to_string()), is_important);
            } else if let Some(length) = parse_length(token) {
                map.set_with_importance(&key, length, is_important);
            }
        }
        return;
    }

    if val.trim() == "auto" {
        for side in ["top", "right", "bottom", "left"] {
            map.set_with_importance(
                &format!("{prop}-{side}"),
                CssValue::Keyword("auto".to_string()),
                is_important,
            );
        }
        return;
    }

    // Single-value shorthand: applies to all four sides. Use `parse_length`,
    // which preserves percentages (`padding: 10%`), calc(), var(), and relative
    // units — `parse_property_value` only surfaced absolute lengths, silently
    // dropping percentage padding/margin (CSS 2.1 § 8.4: % resolves against the
    // containing block WIDTH on every side, including vertical).
    if let Some(value) = parse_length(val) {
        for side in ["top", "right", "bottom", "left"] {
            map.set_with_importance(&format!("{prop}-{side}"), value.clone(), is_important);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_declaration, parse_inline_style, split_top_level_commas};
    use crate::parser::css::{CssValue, StyleMap};

    #[test]
    fn inline_relative_length_preserves_em_units() {
        assert!(matches!(
            parse_inline_style("width: 10em").get("width"),
            Some(CssValue::Number(value)) if (*value - 10.0).abs() < 0.01
        ));
    }

    #[test]
    fn parse_basic_inline_styles() {
        let style = parse_inline_style("font-size: 16px; color: red; text-align: center");
        assert!(
            matches!(style.get("font-size"), Some(CssValue::Length(v)) if (*v - 12.0).abs() < 0.1)
        );
        assert!(matches!(style.get("color"), Some(CssValue::Color(c)) if c.r == 255));
        assert!(
            matches!(style.get("text-align"), Some(CssValue::Keyword(value)) if value == "center")
        );
    }

    #[test]
    fn parse_margin_and_padding_shorthand() {
        let margin = parse_inline_style("margin: 10px");
        assert!(margin.get("margin-top").is_some());
        assert!(margin.get("margin-right").is_some());
        assert!(margin.get("margin-bottom").is_some());
        assert!(margin.get("margin-left").is_some());

        let padding = parse_inline_style("padding: 8px");
        assert!(padding.get("padding-top").is_some());
        assert!(padding.get("padding-right").is_some());
        assert!(padding.get("padding-bottom").is_some());
        assert!(padding.get("padding-left").is_some());
    }

    #[test]
    fn parse_font_keywords() {
        let style = parse_inline_style(
            "font-weight: bold; font-style: italic; font-family: 'Times New Roman', serif",
        );
        assert!(
            matches!(style.get("font-weight"), Some(CssValue::Keyword(value)) if value == "bold")
        );
        assert!(
            matches!(style.get("font-style"), Some(CssValue::Keyword(value)) if value == "italic")
        );
        assert!(matches!(
            style.get("font-family"),
            Some(CssValue::Keyword(value)) if value == "'Times New Roman', serif"
        ));
    }

    #[test]
    fn parse_border_and_outline_properties() {
        let style = parse_inline_style(
            "border: 1px solid black; border-top: 1pt solid red; border-width: 2pt; outline-color: blue",
        );
        assert!(
            matches!(style.get("border"), Some(CssValue::Keyword(value)) if value == "1px solid black")
        );
        assert!(
            matches!(style.get("border-top"), Some(CssValue::Keyword(value)) if value == "1pt solid red")
        );
        assert!(
            matches!(style.get("border-width"), Some(CssValue::Length(v)) if (*v - 2.0).abs() < 0.1)
        );
        assert!(matches!(style.get("outline-color"), Some(CssValue::Color(c)) if c.b == 255));
    }

    #[test]
    fn parse_layout_keywords_and_lengths() {
        let style = parse_inline_style(
            "display: none; position: absolute; width: auto; height: 50vh; gap: 10px; border-spacing: 12pt 24pt",
        );
        assert!(matches!(style.get("display"), Some(CssValue::Keyword(value)) if value == "none"));
        assert!(
            matches!(style.get("position"), Some(CssValue::Keyword(value)) if value == "absolute")
        );
        assert!(matches!(style.get("width"), Some(CssValue::Keyword(value)) if value == "auto"));
        assert!(matches!(style.get("height"), Some(CssValue::Vh(v)) if (*v - 50.0).abs() < 0.01));
        assert!(matches!(style.get("gap"), Some(CssValue::Length(v)) if (*v - 7.5).abs() < 0.01));
        assert!(
            matches!(style.get("border-spacing"), Some(CssValue::Length(v)) if (*v - 12.0).abs() < 0.01)
        );
        assert!(
            matches!(style.get("border-spacing-horizontal"), Some(CssValue::Length(v)) if (*v - 12.0).abs() < 0.01)
        );
        assert!(
            matches!(style.get("border-spacing-vertical"), Some(CssValue::Length(v)) if (*v - 24.0).abs() < 0.01)
        );
    }

    #[test]
    fn parse_border_spacing_rejects_invalid_second_component() {
        let style = parse_inline_style("border-spacing: 10pt foo");
        assert!(style.get("border-spacing").is_none());
        assert!(style.get("border-spacing-horizontal").is_none());
        assert!(style.get("border-spacing-vertical").is_none());
    }

    #[test]
    fn parse_background_gradients() {
        let linear = parse_inline_style("background-image: linear-gradient(red, blue)");
        let radial = parse_inline_style("background: radial-gradient(circle, white, black)");
        assert!(linear.get("background-gradient").is_some());
        assert!(radial.get("background-radial-gradient").is_some());
    }

    #[test]
    fn parse_calc_and_var_values() {
        let style = parse_inline_style("width: calc(100% - 20pt); color: var(--text-color, red)");
        assert!(matches!(style.get("width"), Some(CssValue::Calc(tokens)) if tokens.len() == 3));
        assert!(matches!(
            style.get("color"),
            Some(CssValue::Var(name, Some(fallback))) if name == "--text-color" && fallback == "red"
        ));
    }

    #[test]
    fn parse_important_keeps_stronger_value() {
        let style = parse_inline_style("width: 40% !important; width: 10%");
        assert!(
            matches!(style.get("width"), Some(CssValue::Percentage(v)) if (*v - 40.0).abs() < 0.01)
        );
    }

    #[test]
    fn parse_custom_properties_and_content_keywords() {
        let style =
            parse_inline_style("--accent: blue; content: \"hello\"; counter-reset: section 0");
        assert!(matches!(style.get("--accent"), Some(CssValue::Keyword(value)) if value == "blue"));
        assert!(
            matches!(style.get("content"), Some(CssValue::Keyword(value)) if value == "\"hello\"")
        );
        assert!(
            matches!(style.get("counter-reset"), Some(CssValue::Keyword(value)) if value == "section 0")
        );
    }

    #[test]
    fn parse_list_and_text_properties() {
        let style = parse_inline_style(
            "list-style: circle inside; list-style-type: square; list-style-position: outside; text-transform: uppercase; white-space: pre-wrap",
        );
        assert!(style.get("list-style").is_some());
        assert!(style.get("list-style-type").is_some());
        assert!(style.get("list-style-position").is_some());
        assert!(
            matches!(style.get("text-transform"), Some(CssValue::Keyword(value)) if value == "uppercase")
        );
        assert!(
            matches!(style.get("white-space"), Some(CssValue::Keyword(value)) if value == "pre-wrap")
        );
    }

    #[test]
    fn parse_content_string_with_semicolon() {
        let style = parse_inline_style("content: \"a; b\"; color: red");
        assert!(
            matches!(style.get("content"), Some(CssValue::Keyword(value)) if value == "\"a; b\"")
        );
        assert!(matches!(style.get("color"), Some(CssValue::Color(color)) if color.r == 255));
    }

    #[test]
    fn parse_empty_style_is_empty() {
        let style = parse_inline_style("");
        assert!(style.properties.is_empty());
    }

    #[test]
    fn style_map_merge_preserves_importance() {
        let mut base = StyleMap::new();
        base.set("font-size", CssValue::Length(12.0));

        let mut overlay = StyleMap::new();
        overlay.set_with_importance("font-size", CssValue::Length(16.0), true);
        overlay.set("color", CssValue::Keyword("red".into()));

        base.merge(&overlay);
        assert!(
            matches!(base.get("font-size"), Some(CssValue::Length(v)) if (*v - 16.0).abs() < 0.01)
        );
        assert!(base.get("color").is_some());
    }

    #[test]
    fn inline_custom_property() {
        let map = parse_inline_style("--my-color: red");
        assert!(matches!(
            map.get("--my-color"),
            Some(CssValue::Keyword(v)) if v == "red"
        ));
    }

    #[test]
    fn inline_margin_auto() {
        let map = parse_inline_style("margin: auto");
        assert!(matches!(
            map.get("margin-left"),
            Some(CssValue::Keyword(v)) if v == "auto"
        ));
        assert!(matches!(
            map.get("margin-right"),
            Some(CssValue::Keyword(v)) if v == "auto"
        ));
    }

    #[test]
    fn inline_margin_individual_auto() {
        let map = parse_inline_style("margin-left: auto; margin-right: auto");
        assert!(matches!(
            map.get("margin-left"),
            Some(CssValue::Keyword(v)) if v == "auto"
        ));
    }

    #[test]
    fn inline_border_spacing() {
        let map = parse_inline_style("border-spacing: 5pt 10pt");
        assert!(map.get("border-spacing-horizontal").is_some());
        assert!(map.get("border-spacing-vertical").is_some());
    }

    #[test]
    fn inline_box_shorthand_3_values() {
        // 3-value margin: top right bottom (left = right)
        let map = parse_inline_style("margin: 10pt 20pt 30pt");
        assert!(map.get("margin-top").is_some());
        assert!(map.get("margin-right").is_some());
        assert!(map.get("margin-bottom").is_some());
        assert!(map.get("margin-left").is_some());
    }

    #[test]
    fn inline_important_flag() {
        let map = parse_inline_style("color: red !important");
        assert!(map.get("color").is_some());
    }

    #[test]
    fn inline_empty_string() {
        let map = parse_inline_style("");
        assert!(map.properties.is_empty());
    }

    #[test]
    fn inline_malformed_no_colon() {
        let map = parse_inline_style("not-a-declaration");
        assert!(map.properties.is_empty());
    }

    #[test]
    fn inline_background_image_svg_data_uri_plain() {
        // SVG data URI via background-image property — exercises apply_background_image_value
        // percent-encoded path
        let svg = "%3Csvg xmlns='http://www.w3.org/2000/svg'%3E%3C/svg%3E";
        let style = parse_inline_style(&format!(
            "background-image: url(\"data:image/svg+xml,{svg}\")"
        ));
        assert!(
            style.get("background-svg").is_some(),
            "expected background-svg to be set from SVG data URI"
        );
    }

    #[test]
    fn inline_background_shorthand_svg_data_uri() {
        // SVG data URI via background shorthand — exercises apply_background_image_value inside
        // parse_background_shorthand
        let svg_b64 = base64_svg();
        let style = parse_inline_style(&format!(
            "background: url(\"data:image/svg+xml;base64,{svg_b64}\")"
        ));
        assert!(
            style.get("background-svg").is_some(),
            "expected background-svg from SVG data URI in background shorthand"
        );
    }

    #[test]
    fn split_top_level_commas_respects_parens_and_quotes() {
        let parts = split_top_level_commas(
            "url(\"data:image/png;base64,AAA\"), linear-gradient(to bottom, #fff, #000)",
        );
        assert_eq!(parts.len(), 2, "got: {parts:?}");
        assert!(parts[0].contains("url("));
        assert!(parts[1].trim().starts_with("linear-gradient("));
    }

    #[test]
    fn inline_background_image_layers_url_and_gradient() {
        // A comma-separated `background-image` with a raster url() layer and a
        // gradient layer should populate BOTH the raster and gradient keys
        // (one raster + one gradient layer can coexist in the data model).
        // Use apply_declaration directly so the data-URI `;` is not split by
        // the legacy declaration tokenizer.
        let mut style = StyleMap::new();
        apply_declaration(
            &mut style,
            "background-image",
            "url(\"data:image/png;base64,iVBORw0KGgo=\"), \
             linear-gradient(to bottom, #ffd600, #00bcd4)",
            false,
        );
        assert!(
            matches!(style.get("background-image"), Some(CssValue::Keyword(v)) if v.contains("url(")),
            "expected raster background-image layer to be captured: {:?}",
            style.get("background-image")
        );
        assert!(
            matches!(style.get("background-gradient"), Some(CssValue::Keyword(v)) if v.starts_with("linear-gradient(")),
            "expected gradient background-image layer to be captured: {:?}",
            style.get("background-gradient")
        );
    }

    #[test]
    fn inline_background_image_same_slot_keeps_top_layer() {
        let mut style = StyleMap::new();
        apply_declaration(
            &mut style,
            "background-image",
            "url(top.png), url(bottom.png)",
            false,
        );
        assert!(
            matches!(style.get("background-image"), Some(CssValue::Keyword(v)) if v == "url(top.png)"),
            "single raster slot should retain the topmost CSS layer: {:?}",
            style.get("background-image")
        );
        assert!(
            matches!(style.get("background-layer-slots"), Some(CssValue::Keyword(v)) if v == "raster,raster"),
            "slot list should still preserve both source layers"
        );
    }

    #[test]
    fn inline_background_image_single_layer_unchanged() {
        // A single gradient layer must still parse exactly as before (no
        // spurious background-image key).
        let mut style = StyleMap::new();
        apply_declaration(
            &mut style,
            "background-image",
            "linear-gradient(to right, red, blue)",
            false,
        );
        assert!(
            matches!(style.get("background-gradient"), Some(CssValue::Keyword(v)) if v.starts_with("linear-gradient(")),
            "single gradient layer should set background-gradient"
        );
        assert!(
            !matches!(style.get("background-image"), Some(CssValue::Keyword(v)) if v.contains("url(")),
            "single gradient must not set a raster background-image"
        );
    }

    #[test]
    fn inline_background_shorthand_expands_layer_lists_and_final_color() {
        let mut style = StyleMap::new();
        apply_declaration(
            &mut style,
            "background",
            "url(top.png) left top / 10px 20px no-repeat content-box, \
             linear-gradient(red, blue) right bottom / 30px 40px repeat padding-box border-box #fdd835",
            false,
        );
        assert!(
            matches!(style.get("background-layer-slots"), Some(CssValue::Keyword(v)) if v == "raster,gradient"),
            "slot list should preserve layer order"
        );
        assert!(
            matches!(style.get("background-position"), Some(CssValue::Keyword(v)) if v == "left top, right bottom"),
            "background-position list should match layers: {:?}",
            style.get("background-position")
        );
        assert!(
            matches!(style.get("background-size"), Some(CssValue::Keyword(v)) if v == "10px 20px, 30px 40px"),
            "background-size list should match layers: {:?}",
            style.get("background-size")
        );
        assert!(
            matches!(style.get("background-origin"), Some(CssValue::Keyword(v)) if v == "content-box, padding-box"),
            "background-origin list should match layers: {:?}",
            style.get("background-origin")
        );
        assert!(
            matches!(style.get("background-clip"), Some(CssValue::Keyword(v)) if v == "content-box, border-box"),
            "background-clip list should match layers: {:?}",
            style.get("background-clip")
        );
        assert!(
            matches!(style.get("background-color"), Some(CssValue::Color(color)) if color.r == 0xfd && color.g == 0xd8 && color.b == 0x35),
            "final-layer background-color should survive"
        );
    }

    #[test]
    fn inline_background_position_xy_longhands_compose_position() {
        let mut style = StyleMap::new();
        apply_declaration(&mut style, "background-position-x", "80px", false);
        apply_declaration(&mut style, "background-position-y", "30px", false);
        assert!(
            matches!(style.get("background-position"), Some(CssValue::Keyword(v)) if v == "80px 30px"),
            "x/y longhands should compose background-position: {:?}",
            style.get("background-position")
        );
    }

    #[test]
    fn inline_filter_blur_is_keyword() {
        let style = parse_inline_style("filter: blur(4px)");
        assert!(
            matches!(style.get("filter"), Some(CssValue::Keyword(v)) if v == "blur(4px)"),
            "filter value should be stored as keyword"
        );
    }

    #[test]
    fn inline_overflow_wrap_property() {
        let style = parse_inline_style("overflow-wrap: break-word");
        assert!(
            matches!(style.get("overflow-wrap"), Some(CssValue::Keyword(v)) if v == "break-word"),
            "overflow-wrap should be stored as keyword"
        );
    }

    #[test]
    fn inline_table_layout_property() {
        let style = parse_inline_style("table-layout: fixed");
        assert!(
            matches!(style.get("table-layout"), Some(CssValue::Keyword(v)) if v == "fixed"),
            "table-layout should be stored as keyword"
        );
    }

    #[test]
    fn inline_background_shorthand_size_two_tokens() {
        // background with position/size using two-token size "100% auto" — exercises
        // is_background_size_continuation picking up the second size token
        let style = parse_inline_style("background: center / 100% auto no-repeat");
        assert!(
            matches!(style.get("background-size"), Some(CssValue::Keyword(v)) if v.contains("100%")),
            "two-token background-size should be captured: {:?}",
            style.get("background-size")
        );
    }

    #[test]
    fn inline_box_shorthand_auto_single_value() {
        // "margin: auto" single-value auto path in expand_box_shorthand
        let map = parse_inline_style("margin: auto");
        for side in ["top", "right", "bottom", "left"] {
            assert!(
                matches!(map.get(&format!("margin-{side}")), Some(CssValue::Keyword(v)) if v == "auto"),
                "margin-{side} should be auto"
            );
        }
    }

    #[test]
    fn inline_box_shorthand_4_values_with_auto() {
        // 4-value padding where one token is "auto" — exercises the auto branch inside the
        // multi-value loop in expand_box_shorthand
        let map = parse_inline_style("padding: 10pt auto 5pt 0pt");
        assert!(
            matches!(map.get("padding-right"), Some(CssValue::Keyword(v)) if v == "auto"),
            "padding-right should be auto"
        );
        assert!(map.get("padding-top").is_some());
        assert!(map.get("padding-bottom").is_some());
        assert!(map.get("padding-left").is_some());
    }

    #[test]
    fn inline_background_shorthand_css_wide_keyword() {
        // background: inherit — exercises the css-wide-keyword branch
        let style = parse_inline_style("background: inherit");
        assert!(
            matches!(style.get("background"), Some(CssValue::Keyword(v)) if v == "inherit"),
            "background should be 'inherit'"
        );
    }

    #[test]
    fn inline_background_image_none() {
        // background-image: none — exercises the "none" branch in apply_background_image_value
        let style = parse_inline_style("background-image: none");
        assert!(
            matches!(style.get("background-image"), Some(CssValue::Keyword(v)) if v == "none"),
            "background-image: none should be stored"
        );
    }

    #[test]
    fn inline_background_image_url() {
        // background-image: url(...) — exercises the url( fallback in parse_background_shorthand
        let style = parse_inline_style("background: url(hero.png) no-repeat center");
        assert!(
            matches!(style.get("background-image"), Some(CssValue::Keyword(v)) if v.starts_with("url(")),
            "url() background image should be stored"
        );
        assert!(
            matches!(style.get("background-repeat"), Some(CssValue::Keyword(v)) if v == "no-repeat"),
        );
    }

    /// Minimal base64-encoded SVG used in tests.
    fn base64_svg() -> String {
        use std::fmt::Write;
        let svg = b"<svg xmlns='http://www.w3.org/2000/svg'></svg>";
        // simple base64 encoding without external crate dependency
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in svg.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            let _ = write!(out, "{}", TABLE[((n >> 18) & 63) as usize] as char);
            let _ = write!(out, "{}", TABLE[((n >> 12) & 63) as usize] as char);
            if chunk.len() > 1 {
                let _ = write!(out, "{}", TABLE[((n >> 6) & 63) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                let _ = write!(out, "{}", TABLE[(n & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }
}
