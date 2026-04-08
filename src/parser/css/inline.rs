use super::{
    CssValue, StyleMap,
    lightning::parse_inline_style_with_lightning,
    parse_length,
    values::{border_spacing_value_count, parse_border_spacing_shorthand, parse_property_value},
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

    let prop = raw_prop.to_ascii_lowercase();
    if (prop == "margin" || prop == "padding") && !prop.contains('-') {
        expand_box_shorthand(map, &prop, val, is_important);
        return;
    }

    if (prop == "margin-left" || prop == "margin-right") && val == "auto" {
        map.set_with_importance(&prop, CssValue::Keyword("auto".to_string()), is_important);
        return;
    }

    if (prop == "background" || prop == "background-image")
        && val.trim_start().starts_with("linear-gradient(")
    {
        map.set_with_importance(
            "background-gradient",
            CssValue::Keyword(val.trim().to_string()),
            is_important,
        );
        return;
    }

    if (prop == "background" || prop == "background-image")
        && val.trim_start().starts_with("radial-gradient(")
    {
        map.set_with_importance(
            "background-radial-gradient",
            CssValue::Keyword(val.trim().to_string()),
            is_important,
        );
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

    if let Some(css_value) = parse_property_value(&prop, val) {
        map.set_with_importance(&prop, css_value, is_important);
    }
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
            | "border"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "outline"
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

    if let Some(CssValue::Length(value)) = parse_property_value(prop, val) {
        for side in ["top", "right", "bottom", "left"] {
            map.set_with_importance(
                &format!("{prop}-{side}"),
                CssValue::Length(value),
                is_important,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_inline_style;
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
        assert!(
            matches!(style.get("font-family"), Some(CssValue::Keyword(value)) if value == "'Times New Roman'")
        );
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
}
