use std::borrow::Cow;

use lightningcss::properties::{
    Property,
    border::LineStyle,
    custom::{TokenList, TokenOrValue},
    effects::{Filter, FilterList},
};
use lightningcss::rules::CssRule as LightningRule;
use lightningcss::stylesheet::{
    ParserFlags, ParserOptions, PrinterOptions, StyleAttribute, StyleSheet,
};
use lightningcss::traits::{ToCss, TrySign};

use super::{
    CssRule, CssValue, SpecifiedColor, StyleMap,
    inline::{apply_declaration, split_border_image_shorthand},
    is_css_wide_keyword,
    rules::extract_pseudo_element,
    values::{
        border_radius_value_is_valid, column_rule_value_is_valid, parse_color, parse_property_value,
    },
};

pub(super) fn parse_inline_style_with_lightning(style: &str) -> Option<StyleMap> {
    let authored = parse_authored_declarations(style);
    let sanitized = sanitize_declaration_list(style);
    let compatible = gradients_for_lightning(&sanitized);
    let precise = preserve_authored_color_semantics(&compatible);
    let attribute = StyleAttribute::parse(&precise, parser_options(&precise)).ok()?;
    Some(declaration_block_to_style_map(
        &attribute.declarations,
        Some(&authored),
    ))
}

pub(super) fn parse_stylesheet_rules_with_lightning(css: &str) -> Option<Vec<CssRule>> {
    let authored_blocks = parse_authored_style_blocks(css);
    let sanitized = sanitize_stylesheet(css);
    let compatible = gradients_for_lightning(&sanitized);
    let precise = preserve_authored_color_semantics(&compatible);
    let stylesheet = StyleSheet::parse(&precise, parser_options(&precise)).ok()?;
    let mut rules = Vec::new();
    let mut authored_blocks = authored_blocks.iter();

    for rule in &stylesheet.rules.0 {
        if let LightningRule::Style(style_rule) = rule {
            let declarations = declaration_block_to_style_map(
                &style_rule.declarations,
                authored_blocks.next().map(Vec::as_slice),
            );
            if declarations.properties.is_empty() {
                continue;
            }

            for selector in &style_rule.selectors.0 {
                let selector = selector.to_css_string(PrinterOptions::default()).ok()?;
                let (selector, pseudo_element) = extract_pseudo_element(&selector);
                rules.push(CssRule::new(selector, declarations.clone(), pseudo_element));
            }
        }
    }

    Some(rules)
}

#[derive(Debug)]
struct AuthoredDeclaration {
    property: String,
    value: String,
    important: bool,
}

fn parse_authored_declarations(input: &str) -> Vec<AuthoredDeclaration> {
    let mut declarations = Vec::new();
    let mut start = 0usize;
    for end in top_level_delimiters(input, b';')
        .into_iter()
        .chain(std::iter::once(input.len()))
    {
        let declaration = input[start..end].trim();
        start = end.saturating_add(1);
        let Some(colon) = first_top_level_delimiter(declaration, b':') else {
            continue;
        };
        let property = declaration[..colon].trim().to_ascii_lowercase();
        let raw_value = declaration[colon + 1..].trim();
        let value = strip_important(raw_value);
        declarations.push(AuthoredDeclaration {
            property,
            value: value.to_string(),
            important: value.len() != raw_value.trim_end().len(),
        });
    }
    declarations
}

fn parse_authored_style_blocks(input: &str) -> Vec<Vec<AuthoredDeclaration>> {
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    while let Some(open_rel) = first_top_level_delimiter(&input[cursor..], b'{') {
        let open = cursor + open_rel;
        let Some(close) = matching_brace(input, open) else {
            break;
        };
        let prelude = input[cursor..open].trim();
        let body = &input[open + 1..close];
        if !prelude.starts_with('@') && first_top_level_delimiter(body, b'{').is_none() {
            blocks.push(parse_authored_declarations(body));
        }
        cursor = close + 1;
    }
    blocks
}

/// Lightning alpha.71 predates gradient color-interpolation methods. Remove a
/// supported method only from the temporary Lightning input, then restore the
/// owned authored declaration after Lightning has validated the declaration.
/// No marker enters CSS source or shared state.
fn gradients_for_lightning(input: &str) -> Cow<'_, str> {
    let bytes = input.as_bytes();
    let mut output: Option<String> = None;
    let mut copied_until = 0usize;
    let mut cursor = 0usize;
    let mut state = LexState::default();

    while cursor < bytes.len() {
        if state.advance(bytes, &mut cursor) {
            continue;
        }
        if bytes[cursor].is_ascii_alphabetic() {
            let name_start = cursor;
            cursor += 1;
            while bytes
                .get(cursor)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
            {
                cursor += 1;
            }
            let name = &input[name_start..cursor];
            if bytes.get(cursor) == Some(&b'(')
                && is_gradient_function(name)
                && let Some(close) = matching_paren(input, cursor)
                && let Some(replacement) = gradient_for_lightning(name, &input[cursor + 1..close])
            {
                let output = output.get_or_insert_with(|| String::with_capacity(input.len()));
                output.push_str(&input[copied_until..name_start]);
                output.push_str(&replacement);
                copied_until = close + 1;
                cursor = close + 1;
                continue;
            }
            continue;
        }
        state.update_depth(bytes[cursor]);
        cursor += 1;
    }

    let Some(mut output) = output else {
        return Cow::Borrowed(input);
    };
    output.push_str(&input[copied_until..]);
    Cow::Owned(output)
}

fn gradient_for_lightning(name: &str, arguments: &str) -> Option<String> {
    let without_interpolation = gradient_arguments_without_interpolation_method(arguments);
    let compatible_arguments = without_interpolation.as_deref().unwrap_or(arguments);
    let conic_percentages =
        matches_ignore_ascii_case(name, &["conic-gradient", "repeating-conic-gradient"])
            .then(|| conic_percent_stops_for_lightning(compatible_arguments))
            .flatten();
    if without_interpolation.is_none() && conic_percentages.is_none() {
        return None;
    }

    Some(format!(
        "{name}({})",
        conic_percentages.as_deref().unwrap_or(compatible_arguments)
    ))
}

fn gradient_arguments_without_interpolation_method(arguments: &str) -> Option<String> {
    let comma = first_top_level_delimiter(arguments, b',')?;
    let mut prelude = split_components(&arguments[..comma]);
    let indices = prelude
        .iter()
        .enumerate()
        .filter_map(|(index, token)| token.eq_ignore_ascii_case("in").then_some(index))
        .collect::<Vec<_>>();
    let [index] = indices.as_slice() else {
        return None;
    };
    let space = prelude.get(index + 1)?.to_ascii_lowercase();
    if !matches!(space.as_str(), "srgb" | "oklab") {
        return None;
    }
    prelude.drain(*index..=*index + 1);

    let mut compatible = String::with_capacity(arguments.len());
    if prelude.is_empty() {
        compatible.push_str(arguments[comma + 1..].trim_start());
    } else {
        compatible.push_str(&prelude.join(" "));
        compatible.push_str(&arguments[comma..]);
    }
    Some(compatible)
}

/// Lightning alpha.71 rejects percentage positions in conic color stops even
/// though CSS Images defines them as fractions of one turn. Convert only the
/// temporary validation copy to equivalent `turn` positions; the authored
/// declaration is restored after Lightning has established cascade order.
/// Percentages inside colors/functions and the `at <position>` prelude remain
/// untouched because components are split only at the gradient's top level.
fn conic_percent_stops_for_lightning(arguments: &str) -> Option<String> {
    let mut changed = false;
    let parts = split_top_level(arguments, b',');
    let mut compatible_parts = Vec::with_capacity(parts.len());

    for (index, part) in parts.into_iter().enumerate() {
        let components = split_components(part);
        let is_prelude = index == 0
            && components.iter().any(|component| {
                component.eq_ignore_ascii_case("from") || component.eq_ignore_ascii_case("at")
            });
        if is_prelude {
            compatible_parts.push(part.to_string());
            continue;
        }

        let mut compatible = Vec::with_capacity(components.len());
        for component in components {
            let replacement = if component == "0" {
                Some("0deg".to_string())
            } else {
                component.strip_suffix('%').and_then(|number| {
                    number
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .map(|value| format!("{}deg", value * 3.6))
                })
            };
            if let Some(replacement) = replacement {
                compatible.push(replacement);
                changed = true;
            } else {
                compatible.push(component);
            }
        }
        compatible_parts.push(compatible.join(" "));
    }

    changed.then(|| compatible_parts.join(", "))
}

/// Preserve authored color-function semantics across the Lightning AST.
///
/// Lightning alpha.71 stores ordinary `rgb()`, `hsl()`, and `hwb()` values in a
/// byte-backed `CssColor::RGBA`, erasing the authored color model before we see
/// the declaration. Blink's PDF path does not erase them identically: RGB alpha
/// is byte-quantized, while percentage RGB channels and HSL/HWB conversion and
/// alpha remain continuous. Convert constant source functions to the equivalent
/// floating `color(srgb ...)` representation before Lightning owns the tokens,
/// using our source-aware color parser to encode those semantics exactly once.
///
/// This is a lexical transformation rather than a textual search: comments,
/// quoted strings, escapes, and `url(...)` payloads are protected. Constant
/// fallbacks nested inside `var()` are visited, while a color function containing
/// deferred components remains untouched and is validated by Lightning.
fn preserve_authored_color_semantics(input: &str) -> Cow<'_, str> {
    let bytes = input.as_bytes();
    let mut output: Option<String> = None;
    let mut copied_until = 0usize;
    let mut cursor = 0usize;
    let mut state = LexState::default();

    while cursor < bytes.len() {
        if state.advance(bytes, &mut cursor) {
            continue;
        }

        if bytes[cursor].is_ascii_alphabetic() {
            let name_start = cursor;
            cursor += 1;
            while bytes
                .get(cursor)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_'))
            {
                cursor += 1;
            }
            let name = &input[name_start..cursor];
            if bytes.get(cursor) == Some(&b'(') {
                let Some(close) = matching_paren(input, cursor) else {
                    break;
                };

                // URL payloads are opaque source. Gradients must also retain
                // their authored color functions: CSS uses the legacy-vs-modern
                // distinction to select Auto's interpolation space, so the
                // general `rgb()` -> `color(srgb)` precision rewrite would erase
                // a semantic bit before the computed gradient parser sees it.
                if name.eq_ignore_ascii_case("url") || is_gradient_function(name) {
                    cursor = close + 1;
                    continue;
                }

                if matches_ignore_ascii_case(name, &["rgb", "rgba", "hsl", "hsla", "hwb"])
                    && let Some(canonical) =
                        canonical_source_color_function(name, &input[cursor + 1..close])
                {
                    let output = output.get_or_insert_with(|| String::with_capacity(input.len()));
                    output.push_str(&input[copied_until..name_start]);
                    output.push_str(&canonical);
                    copied_until = close + 1;
                    cursor = close + 1;
                    continue;
                }
            }
            continue;
        }

        state.update_depth(bytes[cursor]);
        cursor += 1;
    }

    let Some(mut output) = output else {
        return Cow::Borrowed(input);
    };
    output.push_str(&input[copied_until..]);
    Cow::Owned(output)
}

fn is_gradient_function(name: &str) -> bool {
    matches_ignore_ascii_case(
        name,
        &[
            "linear-gradient",
            "repeating-linear-gradient",
            "radial-gradient",
            "repeating-radial-gradient",
            "conic-gradient",
            "repeating-conic-gradient",
        ],
    )
}

fn matches_ignore_ascii_case(value: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

fn canonical_source_color_function(name: &str, arguments: &str) -> Option<String> {
    let arguments = css_comments_as_spaces(arguments)?;
    if arguments.contains('(')
        || arguments.contains(')')
        || arguments.contains('"')
        || arguments.contains('\'')
        || arguments.contains('\\')
    {
        return None;
    }
    // Missing components carry interpolation semantics that cannot be collapsed
    // to one absolute sRGB value. Let Lightning retain those tokens.
    if arguments
        .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, ',' | '/'))
        .any(|token| token.eq_ignore_ascii_case("none"))
    {
        return None;
    }
    let source = format!("{name}({arguments})");
    let CssValue::Color(SpecifiedColor::Absolute(color)) = parse_color(&source)? else {
        return None;
    };

    let (r, g, b, mut alpha) = color.to_f32_rgba();
    // Blink's PDF path resolves rgb()/rgba() alpha through an 8-bit color,
    // unlike HSL/HWB alpha. Keep that observed backend behavior at this
    // explicit Lightning boundary rather than contaminating the CSS parser.
    if matches_ignore_ascii_case(name, &["rgb", "rgba"]) {
        alpha = (alpha * 255.0).round() / 255.0;
    }
    let mut canonical = format!("color(srgb {r} {g} {b}");
    if alpha < 1.0 {
        canonical.push_str(&format!(" / {alpha}"));
    }
    canonical.push(')');
    Some(canonical)
}

/// Replace comments with CSS whitespace while rejecting unterminated comments.
/// The caller only uses this normalized copy to parse numeric components; the
/// original source remains untouched when no safe constant rewrite applies.
fn css_comments_as_spaces(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if bytes[cursor] == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            let end = input[cursor + 2..].find("*/")? + cursor + 2;
            output.push(' ');
            cursor = end + 2;
        } else {
            let ch = input[cursor..].chars().next()?;
            output.push(ch);
            cursor += ch.len_utf8();
        }
    }
    Some(output)
}

/// Remove declarations that Lightning would otherwise normalize into a valid
/// value. lightningcss alpha.71 accepts unitless numbers as pixel lengths in a
/// few grammars and may retain the valid prefix of an invalid comma list. Once
/// normalized, `40` is indistinguishable from authored `40px`, so validation
/// must happen against the original token spelling.
fn sanitize_declaration_list(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut start = 0usize;

    for end in top_level_delimiters(input, b';') {
        push_sanitized_declaration(&mut output, &input[start..end]);
        output.push(';');
        start = end + 1;
    }
    push_sanitized_declaration(&mut output, &input[start..]);
    output
}

fn push_sanitized_declaration(output: &mut String, declaration: &str) {
    let Some(colon) = first_top_level_delimiter(declaration, b':') else {
        output.push_str(declaration);
        return;
    };
    let property = declaration[..colon].trim().to_ascii_lowercase();
    let raw_value = declaration[colon + 1..].trim();
    let value = strip_important(raw_value);
    if property == "transform-origin"
        && let Some(compatible) = transform_origin_for_lightning(value)
    {
        output.push_str(&declaration[..=colon]);
        output.push(' ');
        output.push_str(&compatible);
        if value.len() != raw_value.trim_end().len() {
            output.push_str(" !important");
        }
        return;
    }
    if property == "flex"
        && let Some(compatible) = flex_intrinsic_basis_for_lightning(value)
    {
        output.push_str(&declaration[..=colon]);
        output.push(' ');
        output.push_str(&compatible);
        if value.len() != raw_value.trim_end().len() {
            output.push_str(" !important");
        }
        return;
    }
    if property == "flex-basis" && is_intrinsic_flex_basis(value) {
        output.push_str(&declaration[..=colon]);
        output.push_str(" auto");
        if value.len() != raw_value.trim_end().len() {
            output.push_str(" !important");
        }
        return;
    }
    // Lightning alpha.71 implements the former calc multiplication rule and
    // rejects valid CSS Values 4 expressions such as `40px * 3px / 1px`.
    // Feed it a grammar-compatible length only to retain this declaration's
    // cascade slot; the typed Ironpress AST is restored after Lightning has
    // ordered the block. This is property-generic: the stand-in itself must be
    // accepted by Lightning, so number-, color-, and transform-valued
    // properties cannot accidentally enter the length path.
    if typed_length_math_requires_compatibility(&property, value) {
        output.push_str(&declaration[..=colon]);
        output.push_str(" 0px");
        if value.len() != raw_value.trim_end().len() {
            output.push_str(" !important");
        }
        return;
    }
    // This LightningCSS release rejects a valid second `border-image`
    // slash (the outset component). Its source/slice/width grammar is still
    // enough to retain a validated cascade slot; the full authored shorthand
    // is restored below and parsed by Ironpress' border-image model.
    if property == "border-image"
        && top_level_delimiters(value, b'/').len() >= 2
        && let Some((source, slices, _, _, _)) = split_border_image_shorthand(value)
    {
        output.push_str(&declaration[..=colon]);
        output.push(' ');
        output.push_str(&source);
        output.push(' ');
        output.push_str(&slices);
        if value.len() != raw_value.trim_end().len() {
            output.push_str(" !important");
        }
        return;
    }
    // This LightningCSS release predates Grid Level 2's `subgrid` track-list
    // grammar. A definite track is used only while Lightning validates source
    // order and importance; the validated authored list is restored below.
    if let Some(compatible) = subgrid_tracks_for_lightning(&property, value) {
        output.push_str(&declaration[..=colon]);
        output.push(' ');
        output.push_str(compatible);
        if value.len() != raw_value.trim_end().len() {
            output.push_str(" !important");
        }
        return;
    }
    // This LightningCSS release predates CSS Text's `<length-percentage>`
    // grammar for `word-spacing`, and drops an explicit `normal` even though it
    // resets inherited text spacing. Keep those valid authored values in their
    // cascade slot with a length stand-in, then restore the typed source once
    // Lightning has done its normal validation and ordering.
    if text_spacing_requires_compatibility(&property, value) {
        output.push_str(&declaration[..=colon]);
        output.push_str(" 0px");
        if value.len() != raw_value.trim_end().len() {
            output.push_str(" !important");
        }
        return;
    }
    if raw_declaration_is_valid(&property, value) {
        output.push_str(declaration);
    }
}

fn typed_length_math_requires_compatibility(property: &str, value: &str) -> bool {
    if property.starts_with("--")
        || !matches!(
            parse_property_value(property, value),
            Some(CssValue::Math(_))
        )
    {
        return false;
    }

    let stand_in = format!("{property}: 0px");
    StyleAttribute::parse(&stand_in, parser_options(&stand_in))
        .ok()
        .is_some_and(|attribute| {
            attribute.declarations.iter().any(|(candidate, _)| {
                candidate
                    .property_id()
                    .name()
                    .eq_ignore_ascii_case(property)
            })
        })
}

/// Lightning alpha.71 does not accept the three-value `transform-origin`
/// grammar. Validate the XY position and Z length independently, then feed it
/// an XY-only stand-in so the property retains its cascade slot. The authored
/// value is restored after Lightning has validated the declaration block.
fn transform_origin_for_lightning(value: &str) -> Option<String> {
    let components = split_components(value);
    let [x, y, z] = components.as_slice() else {
        return None;
    };
    let xy_source = format!("transform-origin: {x} {y}");
    let xy = StyleAttribute::parse(&xy_source, parser_options(&xy_source)).ok()?;
    if !xy.declarations.iter().any(|(property, _)| {
        property
            .property_id()
            .name()
            .eq_ignore_ascii_case("transform-origin")
    }) {
        return None;
    }

    if is_unitless_nonzero(z) || z.trim_end().ends_with('%') {
        return None;
    }
    let z_source = format!("transform: translateZ({z})");
    let z = StyleAttribute::parse(&z_source, parser_options(&z_source)).ok()?;
    if !z.declarations.iter().any(|(property, _)| {
        property
            .property_id()
            .name()
            .eq_ignore_ascii_case("transform")
    }) {
        return None;
    }

    Some(format!("{x} {y}"))
}

/// LightningCSS alpha.71 rejects an intrinsic keyword when it appears beside
/// explicit flex factors (`flex: 0 0 content`). Preserve the authored
/// declaration after validating its supported grammar through an `auto` basis
/// stand-in, which has the same shorthand structure for cascading purposes.
fn flex_intrinsic_basis_for_lightning(value: &str) -> Option<String> {
    let components = split_components(value);
    let factor = |value: &str| {
        value
            .parse::<f32>()
            .ok()
            .is_some_and(|number| number.is_finite() && number >= 0.0)
    };

    match components.as_slice() {
        [basis_value] if is_intrinsic_flex_basis(basis_value) => Some("auto".to_string()),
        [grow, basis_value] if factor(grow) && is_intrinsic_flex_basis(basis_value) => {
            Some(format!("{grow} auto"))
        }
        [grow, shrink, basis_value]
            if factor(grow) && factor(shrink) && is_intrinsic_flex_basis(basis_value) =>
        {
            Some(format!("{grow} {shrink} auto"))
        }
        _ => None,
    }
}

/// Return a Grid Level 1-compatible stand-in for a valid Grid Level 2
/// `subgrid` track list. Subgrids can only carry optional bracketed line-name
/// groups after the `subgrid` keyword; actual track sizes make the declaration
/// invalid.
fn subgrid_tracks_for_lightning(property: &str, value: &str) -> Option<&'static str> {
    if !matches!(property, "grid-template-columns" | "grid-template-rows") {
        return None;
    }

    let value = value.trim();
    let keyword = value.get(.."subgrid".len())?;
    if !keyword.eq_ignore_ascii_case("subgrid") {
        return None;
    }
    let mut remaining = value.get("subgrid".len()..)?;
    if !remaining.is_empty() && !remaining.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }

    while !remaining.trim().is_empty() {
        remaining = remaining.trim_start();
        let names = remaining.strip_prefix('[')?;
        let close = names.find(']')?;
        remaining = &names[close + 1..];
    }

    Some("1px")
}

fn text_spacing_requires_compatibility(property: &str, value: &str) -> bool {
    if !matches!(property, "letter-spacing" | "word-spacing") {
        return false;
    }
    parse_property_value(property, value).is_some_and(|value| match value {
        CssValue::Keyword(ref keyword) => keyword.eq_ignore_ascii_case("normal"),
        value if property == "word-spacing" => css_value_contains_percentage(&value),
        _ => false,
    })
}

fn css_value_contains_percentage(value: &CssValue) -> bool {
    match value {
        CssValue::Percentage(_) => true,
        CssValue::Math(expression) => expression.contains_percentage(),
        _ => false,
    }
}

fn is_intrinsic_flex_basis(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "content" | "min-content" | "max-content" | "fit-content"
    )
}

fn sanitize_stylesheet(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(open_rel) = first_top_level_delimiter(&input[cursor..], b'{') {
        let open = cursor + open_rel;
        output.push_str(&input[cursor..=open]);
        let Some(close) = matching_brace(input, open) else {
            output.push_str(&input[open + 1..]);
            return output;
        };
        let body = &input[open + 1..close];
        if first_top_level_delimiter(body, b'{').is_some() {
            output.push_str(&sanitize_stylesheet(body));
        } else {
            output.push_str(&sanitize_declaration_list(body));
        }
        output.push('}');
        cursor = close + 1;
    }

    output.push_str(&input[cursor..]);
    output
}

fn matching_brace(input: &str, open: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut state = LexState::default();
    let mut depth = 1usize;
    let mut index = open + 1;
    while index < bytes.len() {
        if state.advance(bytes, &mut index) {
            continue;
        }
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn top_level_delimiters(input: &str, delimiter: u8) -> Vec<usize> {
    let bytes = input.as_bytes();
    let mut state = LexState::default();
    let mut indices = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if state.advance(bytes, &mut index) {
            continue;
        }
        if bytes[index] == delimiter && state.at_top_level() {
            indices.push(index);
        }
        state.update_depth(bytes[index]);
        index += 1;
    }
    indices
}

fn first_top_level_delimiter(input: &str, delimiter: u8) -> Option<usize> {
    top_level_delimiters(input, delimiter).into_iter().next()
}

#[derive(Default)]
struct LexState {
    quote: Option<u8>,
    escaped: bool,
    comment: bool,
    paren_depth: usize,
    bracket_depth: usize,
}

impl LexState {
    /// Consume comments, quoted strings, and escapes. Returns true when the
    /// caller should continue without examining the current byte.
    fn advance(&mut self, bytes: &[u8], index: &mut usize) -> bool {
        let byte = bytes[*index];
        if self.comment {
            if byte == b'*' && bytes.get(*index + 1) == Some(&b'/') {
                self.comment = false;
                *index += 2;
            } else {
                *index += 1;
            }
            return true;
        }
        if let Some(quote) = self.quote {
            if self.escaped {
                self.escaped = false;
            } else if byte == b'\\' {
                self.escaped = true;
            } else if byte == quote {
                self.quote = None;
            }
            *index += 1;
            return true;
        }
        if byte == b'/' && bytes.get(*index + 1) == Some(&b'*') {
            self.comment = true;
            *index += 2;
            return true;
        }
        if matches!(byte, b'\'' | b'"') {
            self.quote = Some(byte);
            *index += 1;
            return true;
        }
        if byte == b'\\' {
            *index = (*index + 2).min(bytes.len());
            return true;
        }
        false
    }

    fn update_depth(&mut self, byte: u8) {
        match byte {
            b'(' => self.paren_depth += 1,
            b')' => self.paren_depth = self.paren_depth.saturating_sub(1),
            b'[' => self.bracket_depth += 1,
            b']' => self.bracket_depth = self.bracket_depth.saturating_sub(1),
            _ => {}
        }
    }

    fn at_top_level(&self) -> bool {
        self.paren_depth == 0 && self.bracket_depth == 0
    }
}

fn strip_important(value: &str) -> &str {
    let trimmed = value.trim_end();
    let lower = trimmed.to_ascii_lowercase();
    lower
        .strip_suffix("!important")
        .map(|prefix| &trimmed[..prefix.len()])
        .unwrap_or(trimmed)
        .trim_end()
}

fn raw_declaration_is_valid(property: &str, value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    if is_css_wide_keyword(&lower) || lower.contains("var(") || lower.contains("env(") {
        return true;
    }

    match property {
        "flex-grow" | "flex-shrink" => lower
            .parse::<f32>()
            .is_ok_and(|number| number.is_finite() && number >= 0.0),
        "column-count" => lower == "auto" || lower.parse::<u32>().is_ok_and(|count| count >= 1),
        "column-rule" | "column-rule-width" | "column-rule-style" | "column-rule-color" => {
            column_rule_value_is_valid(property, value)
        }
        "transform" => transform_source_is_valid(&lower),
        "box-shadow" | "text-shadow" => shadow_source_has_no_unitless_nonzero(&lower),
        "filter" => filter_source_is_valid(&lower),
        "background-size" | "background-position" => {
            source_has_no_unitless_nonzero_component(&lower)
        }
        "border-radius" => border_radius_value_is_valid(&lower, true),
        "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius"
        | "border-start-start-radius"
        | "border-start-end-radius"
        | "border-end-start-radius"
        | "border-end-end-radius" => border_radius_value_is_valid(&lower, false),
        _ => true,
    }
}

fn transform_source_is_valid(value: &str) -> bool {
    if value == "none" {
        return true;
    }
    let Some(functions) = css_functions(value) else {
        return true; // Lightning's typed grammar handles non-numeric syntax.
    };
    functions.into_iter().all(|(name, arguments)| {
        let components = split_components(arguments);
        match name.as_str() {
            "rotate" | "rotatex" | "rotatey" | "rotatez" | "skewx" | "skewy" => {
                components.iter().all(|value| !is_unitless_nonzero(value))
            }
            "skew" | "translate" | "translatex" | "translatey" | "translatez" | "translate3d"
            | "perspective" => components.iter().all(|value| !is_unitless_nonzero(value)),
            "rotate3d" => components
                .last()
                .is_none_or(|angle| !is_unitless_nonzero(angle)),
            _ => true,
        }
    })
}

fn shadow_source_has_no_unitless_nonzero(value: &str) -> bool {
    value == "none"
        || split_top_level(value, b',').into_iter().all(|shadow| {
            split_components(shadow)
                .iter()
                .all(|component| !is_unitless_nonzero(component))
        })
}

fn filter_source_is_valid(value: &str) -> bool {
    if value == "none" {
        return true;
    }
    let Some(functions) = css_functions(value) else {
        return true; // Unknown/malformed functions remain Lightning's concern.
    };
    functions
        .into_iter()
        .all(|(name, argument)| match name.as_str() {
            "blur" => {
                let argument = argument.trim();
                argument.is_empty()
                    || (!is_unitless_nonzero(argument) && !has_negative_dimension(argument))
            }
            "brightness" | "contrast" | "grayscale" | "invert" | "opacity" | "saturate"
            | "sepia" => {
                let argument = argument.trim();
                argument.is_empty()
                    || is_css_math_function(argument)
                    || parse_number_or_percentage(argument)
                        .is_some_and(|amount| amount.is_finite() && amount >= 0.0)
            }
            "hue-rotate" => {
                let argument = argument.trim();
                argument.is_empty() || !is_unitless_nonzero(argument)
            }
            "drop-shadow" => shadow_source_has_no_unitless_nonzero(argument),
            "url" => !argument.trim().is_empty(),
            _ => true,
        })
}

fn is_css_math_function(value: &str) -> bool {
    ["calc(", "min(", "max(", "clamp("]
        .iter()
        .any(|prefix| value.starts_with(prefix) && value.ends_with(')'))
}

fn source_has_no_unitless_nonzero_component(value: &str) -> bool {
    split_top_level(value, b',').into_iter().all(|layer| {
        split_components(layer)
            .iter()
            .all(|component| !is_unitless_nonzero(component))
    })
}

fn is_unitless_nonzero(value: &str) -> bool {
    value
        .trim()
        .parse::<f32>()
        .is_ok_and(|number| number != 0.0)
}

fn has_negative_dimension(value: &str) -> bool {
    let value = value.trim_start();
    if !value.starts_with('-') {
        return false;
    }
    value[1..]
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
}

fn parse_number_or_percentage(value: &str) -> Option<f32> {
    value
        .strip_suffix('%')
        .unwrap_or(value)
        .trim()
        .parse::<f32>()
        .ok()
}

fn css_functions(value: &str) -> Option<Vec<(String, &str)>> {
    let mut functions = Vec::new();
    let mut cursor = 0usize;
    let bytes = value.as_bytes();
    while cursor < value.len() {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if cursor == value.len() {
            break;
        }
        let name_start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        {
            cursor += 1;
        }
        if name_start == cursor || bytes.get(cursor) != Some(&b'(') {
            return None;
        }
        let open = cursor;
        let close = matching_paren(value, open)?;
        functions.push((
            value[name_start..cursor].to_ascii_lowercase(),
            &value[open + 1..close],
        ));
        cursor = close + 1;
    }
    (!functions.is_empty()).then_some(functions)
}

fn matching_paren(value: &str, open: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut depth = 1usize;
    let mut state = LexState::default();
    let mut cursor = open + 1;
    while cursor < bytes.len() {
        if state.advance(bytes, &mut cursor) {
            continue;
        }
        match bytes[cursor] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cursor);
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn split_top_level(value: &str, delimiter: u8) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    for end in top_level_delimiters(value, delimiter) {
        parts.push(value[start..end].trim());
        start = end + 1;
    }
    parts.push(value[start..].trim());
    parts
}

fn split_components(value: &str) -> Vec<String> {
    let bytes = value.as_bytes();
    let mut state = LexState::default();
    let mut components = Vec::new();
    let mut start: Option<usize> = None;
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if state.advance(bytes, &mut cursor) {
            continue;
        }
        let separator =
            state.at_top_level() && (bytes[cursor] == b',' || bytes[cursor].is_ascii_whitespace());
        if separator {
            if let Some(component_start) = start.take() {
                components.push(value[component_start..cursor].trim().to_string());
            }
        } else if start.is_none() {
            start = Some(cursor);
        }
        state.update_depth(bytes[cursor]);
        cursor += 1;
    }
    if let Some(component_start) = start {
        components.push(value[component_start..].trim().to_string());
    }
    components
}

fn declaration_block_to_style_map(
    declarations: &lightningcss::declaration::DeclarationBlock<'_>,
    authored: Option<&[AuthoredDeclaration]>,
) -> StyleMap {
    let mut map = lightning_declarations_to_style_map(declarations);

    if let Some(authored) = authored {
        restore_authored_sources(&mut map, authored);
    }

    map
}

fn lightning_declarations_to_style_map(
    declarations: &lightningcss::declaration::DeclarationBlock<'_>,
) -> StyleMap {
    let mut map = StyleMap::new();

    for (property, is_important) in declarations.iter() {
        let property_id = property.property_id();
        let property_name = property_id.name().to_string();
        let mut value = match property.value_to_css_string(PrinterOptions::default()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        preserve_omitted_border_style(property, &mut value);
        if !is_authoritative_declaration(property, &property_name, &value) {
            continue;
        }
        apply_declaration(&mut map, &property_name, &value, is_important);
    }

    map
}

fn restore_authored_sources(map: &mut StyleMap, authored: &[AuthoredDeclaration]) {
    let restore_background = authored.iter().any(|declaration| {
        (declaration.property == "background" || declaration.property == "background-image")
            && contains_gradient_function(&declaration.value)
    });
    let restore_mask = authored.iter().any(|declaration| {
        (declaration.property == "mask"
            || declaration.property.starts_with("mask-")
            || declaration.property.starts_with("-webkit-mask"))
            && contains_gradient_function(&declaration.value)
    });
    let restore_border_image = authored.iter().any(|declaration| {
        declaration.property.starts_with("border-image")
            && contains_gradient_function(&declaration.value)
    });
    // Preserve authored logical border declarations after Lightning validates
    // them. This Lightning release may serialize a logical shorthand into
    // physical-looking components or omit its initial `none` style; either
    // rewrite would destroy the declaration-order pairing required by CSS
    // Logical §4.
    let restore_logical_borders = authored.iter().any(|declaration| {
        declaration.property.starts_with("border-block")
            || declaration.property.starts_with("border-inline")
            || matches!(
                declaration.property.as_str(),
                "border-start-start-radius"
                    | "border-start-end-radius"
                    | "border-end-start-radius"
                    | "border-end-end-radius"
            )
    });
    // LightningCSS does not yet accept the three-value grammar. The sanitized
    // input carries a validated XY stand-in; restore the authored value so its
    // Z component is not flattened away after the cascade pass.
    let restore_transform_origin = authored
        .iter()
        .any(|declaration| declaration.property == "transform-origin");
    let restore_transforms = authored.iter().any(|declaration| {
        matches!(
            declaration.property.as_str(),
            "transform" | "translate" | "rotate" | "scale"
        )
    });
    // LightningCSS currently serializes `flex: 0 0 content` as
    // `flex: content`, changing the authored zero grow/shrink factors. Keep
    // the validated source shorthand so the computed-style parser receives the
    // complete grammar.
    let restore_flex = authored
        .iter()
        .any(|declaration| declaration.property == "flex");
    let restore_flex_basis = authored.iter().any(|declaration| {
        declaration.property == "flex-basis" && is_intrinsic_flex_basis(&declaration.value)
    });
    let restore_letter_spacing = authored.iter().any(|declaration| {
        declaration.property == "letter-spacing"
            && text_spacing_requires_compatibility(&declaration.property, &declaration.value)
    });
    let restore_word_spacing = authored.iter().any(|declaration| {
        declaration.property == "word-spacing"
            && text_spacing_requires_compatibility(&declaration.property, &declaration.value)
    });
    let restore_subgrid_tracks = authored.iter().any(|declaration| {
        subgrid_tracks_for_lightning(&declaration.property, &declaration.value).is_some()
    });
    let restore_typed_math = authored
        .iter()
        .filter(|declaration| {
            typed_length_math_requires_compatibility(&declaration.property, &declaration.value)
        })
        .map(|declaration| declaration.property.as_str())
        .collect::<Vec<_>>();

    let mut source = StyleMap::new();
    for declaration in authored {
        let custom = declaration.property.starts_with("--");
        let background = restore_background
            && (declaration.property == "background"
                || declaration.property.starts_with("background-"));
        let mask = restore_mask
            && (declaration.property == "mask"
                || declaration.property.starts_with("mask-")
                || declaration.property.starts_with("-webkit-mask"));
        let border_image = restore_border_image
            && (declaration.property.starts_with("border-image")
                || declaration.property == "border");
        let logical_border = restore_logical_borders
            && (declaration.property.starts_with("border-block")
                || declaration.property.starts_with("border-inline")
                || matches!(
                    declaration.property.as_str(),
                    "border-start-start-radius"
                        | "border-start-end-radius"
                        | "border-end-start-radius"
                        | "border-end-end-radius"
                ));
        let transform_origin =
            restore_transform_origin && declaration.property == "transform-origin";
        let transform = restore_transforms
            && matches!(
                declaration.property.as_str(),
                "transform" | "translate" | "rotate" | "scale"
            );
        let flex = restore_flex && declaration.property == "flex";
        let flex_basis = restore_flex_basis
            && declaration.property == "flex-basis"
            && is_intrinsic_flex_basis(&declaration.value);
        // Replay every declaration for a spacing property once it uses a
        // compatibility value. Replaying only that value would let an earlier
        // percentage overwrite a later length (or an `!important` winner).
        let text_spacing = (restore_letter_spacing && declaration.property == "letter-spacing")
            || (restore_word_spacing && declaration.property == "word-spacing");
        // Replay every declaration for the axis, not only the `subgrid` one:
        // a later ordinary declaration must still supersede an earlier subgrid
        // declaration, and `!important` remains part of the same cascade.
        let subgrid_tracks = restore_subgrid_tracks
            && matches!(
                declaration.property.as_str(),
                "grid-template-columns" | "grid-template-rows"
            );
        // Replay the complete cascade for every affected longhand. Restoring
        // only the compound expression would incorrectly resurrect it over a
        // later ordinary value or lose an earlier !important winner.
        let typed_math = restore_typed_math
            .iter()
            .any(|property| *property == declaration.property);
        if (custom
            || background
            || mask
            || border_image
            || logical_border
            || transform_origin
            || transform
            || flex
            || flex_basis
            || text_spacing
            || subgrid_tracks
            || typed_math)
            && authored_declaration_is_accepted(declaration)
        {
            apply_declaration(
                &mut source,
                &declaration.property,
                &declaration.value,
                declaration.important,
            );
        }
    }

    // Lightning has already validated and ordered these properties. Replace
    // their typed values in place, preserving the declaration-order slots used
    // by `all` and shorthand/longhand cascade processing.
    for key in &source.declaration_order {
        let border_image_component = key.starts_with("border-image-");
        let logical_border_component = key.starts_with("border-block")
            || key.starts_with("border-inline")
            || matches!(
                key.as_str(),
                "border-start-start-radius"
                    | "border-start-end-radius"
                    | "border-end-start-radius"
                    | "border-end-end-radius"
            );
        if !map.properties.contains_key(key) && !border_image_component && !logical_border_component
        {
            continue;
        }
        let Some(value) = source.properties.get(key) else {
            continue;
        };
        if border_image_component {
            map.set_with_importance(key, value.clone(), source.is_important(key));
        } else if logical_border_component && map.properties.contains_key(key) {
            map.properties.insert(key.clone(), value.clone());
            map.important.insert(key.clone(), source.is_important(key));
        } else if logical_border_component {
            map.set_with_importance(key, value.clone(), source.is_important(key));
        } else {
            map.properties.insert(key.clone(), value.clone());
            map.important.insert(key.clone(), source.is_important(key));
        }
    }
}

/// Validate one authored declaration through the same temporary Lightning
/// representation used by the owning block. This prevents source restoration
/// from resurrecting a declaration Lightning rejected while still allowing the
/// interpolation methods unsupported by this Lightning release.
fn authored_declaration_is_accepted(declaration: &AuthoredDeclaration) -> bool {
    let source =
        sanitize_declaration_list(&format!("{}: {}", declaration.property, declaration.value));
    let compatible = gradients_for_lightning(&source);
    let precise = preserve_authored_color_semantics(&compatible);
    let Ok(attribute) = StyleAttribute::parse(&precise, parser_options(&precise)) else {
        return false;
    };
    !lightning_declarations_to_style_map(&attribute.declarations)
        .properties
        .is_empty()
}

fn contains_gradient_function(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "linear-gradient(",
        "repeating-linear-gradient(",
        "radial-gradient(",
        "repeating-radial-gradient(",
        "conic-gradient(",
        "repeating-conic-gradient(",
    ]
    .iter()
    .any(|function| lower.contains(function))
}

fn preserve_omitted_border_style(property: &Property<'_>, serialized_value: &mut String) {
    let style = match property {
        Property::Border(border) => Some(border.style),
        Property::BorderTop(border) => Some(border.style),
        Property::BorderRight(border) => Some(border.style),
        Property::BorderBottom(border) => Some(border.style),
        Property::BorderLeft(border) => Some(border.style),
        _ => None,
    };
    if style == Some(LineStyle::None)
        && !serialized_value
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("none"))
    {
        serialized_value.push_str(" none");
    }
}

/// Lightning preserves a known property whose grammar did not parse as an
/// `Unparsed` token list. That representation is needed for deferred `var()` /
/// `env()` substitution, but a token list without either function is an invalid
/// declaration and must not replace an earlier valid declaration in the same
/// block.
fn is_authoritative_declaration(
    property: &Property<'_>,
    property_name: &str,
    serialized_value: &str,
) -> bool {
    match property {
        Property::Unparsed(unparsed) => {
            is_css_wide_keyword(&serialized_value.to_ascii_lowercase())
                || token_list_has_deferred_value(&unparsed.value)
                || strict_engine_extension_is_valid(property_name, serialized_value)
        }
        // The lightningcss representation is an unconstrained CSS number, but
        // the flex grammar requires a non-negative number.
        Property::FlexGrow(value, _) | Property::FlexShrink(value, _) => {
            value.is_finite() && *value >= 0.0
        }
        Property::Filter(filters, _) => filter_list_has_valid_ranges(filters),
        _ => true,
    }
}

/// Known Lightning properties normally use Lightning's typed grammar. Keep a
/// deliberately small escape hatch for properties whose syntax ironpress owns
/// but Lightning currently leaves unparsed. Every entry here must use a strict
/// decoder: a broad raw-keyword fallback would re-admit the invalid declaration
/// that `Unparsed` is meant to quarantine.
fn strict_engine_extension_is_valid(property_name: &str, value: &str) -> bool {
    match property_name {
        "column-count" => parse_property_value(property_name, value).is_some(),
        "image-rendering" => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "auto"
                | "smooth"
                | "high-quality"
                | "pixelated"
                | "crisp-edges"
                | "optimizespeed"
                | "optimizequality"
        ),
        "clip-path" => crate::style::computed::is_supported_clip_path(value),
        "position" => {
            let value = value.trim();
            value
                .strip_prefix("running(")
                .and_then(|value| value.strip_suffix(')'))
                .is_some_and(|name| {
                    let name = name.trim();
                    !name.is_empty()
                        && name
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
                })
        }
        _ => false,
    }
}

fn token_list_has_deferred_value(tokens: &TokenList<'_>) -> bool {
    tokens.0.iter().any(|token| match token {
        TokenOrValue::Var(_) | TokenOrValue::Env(_) | TokenOrValue::UnresolvedColor(_) => true,
        TokenOrValue::Function(function) => token_list_has_deferred_value(&function.arguments),
        _ => false,
    })
}

fn filter_list_has_valid_ranges(filters: &FilterList<'_>) -> bool {
    let FilterList::Filters(filters) = filters else {
        return true;
    };

    filters.iter().all(|filter| match filter {
        Filter::Blur(length) => !length.is_sign_negative(),
        Filter::Brightness(value)
        | Filter::Contrast(value)
        | Filter::Grayscale(value)
        | Filter::Invert(value)
        | Filter::Opacity(value)
        | Filter::Saturate(value)
        | Filter::Sepia(value) => {
            let value: f32 = value.into();
            value.is_finite() && value >= 0.0
        }
        Filter::DropShadow(shadow) => !shadow.blur.is_sign_negative(),
        Filter::HueRotate(_) | Filter::Url(_) => true,
    })
}

fn parser_options<'i>(input: &'i str) -> ParserOptions<'i> {
    let _ = input;
    ParserOptions {
        filename: String::new(),
        css_modules: None,
        source_index: 0,
        error_recovery: true,
        warnings: None,
        flags: ParserFlags::empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CssValue, SpecifiedColor, gradients_for_lightning, parse_color,
        parse_inline_style_with_lightning, parse_stylesheet_rules_with_lightning,
        preserve_authored_color_semantics,
    };
    use crate::parser::css::BackgroundLayerSource;

    fn assert_color_value(value: &CssValue, expected: (f32, f32, f32, f32)) {
        let CssValue::Color(SpecifiedColor::Absolute(color)) = value else {
            panic!("expected an absolute color, got {value:?}");
        };
        let actual = color.to_f32_rgba();
        for (actual, expected) in [actual.0, actual.1, actual.2, actual.3]
            .into_iter()
            .zip([expected.0, expected.1, expected.2, expected.3])
        {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
        }
    }

    #[test]
    fn constant_rgb_percentages_become_precise_srgb() {
        assert_eq!(
            preserve_authored_color_semantics("color: rgb(80% 20% 10%)"),
            "color: color(srgb 0.8 0.2 0.1)"
        );
    }

    #[test]
    fn numeric_rgb_channels_use_the_255_basis() {
        assert_eq!(
            preserve_authored_color_semantics("color: rgb(1, 2, 255)"),
            "color: color(srgb 0.003921569 0.007843138 1)"
        );
    }

    #[test]
    fn preserves_image_rendering_values_from_inline_and_stylesheet_css() {
        let inline = parse_inline_style_with_lightning("image-rendering: pixelated")
            .expect("pixelated declaration should parse");
        assert!(matches!(
            inline.get("image-rendering"),
            Some(CssValue::Keyword(value)) if value == "pixelated"
        ));

        let rules =
            parse_stylesheet_rules_with_lightning(".sample { image-rendering: crisp-edges }")
                .expect("crisp-edges stylesheet declaration should parse");
        assert!(matches!(
            rules[0].declarations.get("image-rendering"),
            Some(CssValue::Keyword(value)) if value == "crisp-edges"
        ));
    }

    #[test]
    fn rgb_alpha_uses_blinks_byte_backed_path() {
        assert_eq!(
            preserve_authored_color_semantics("color: rgba(10% 20% 30% / 12.5%)"),
            "color: color(srgb 0.1 0.2 0.3 / 0.1254902)"
        );
        assert_eq!(
            preserve_authored_color_semantics("color: rgba(255, 0, 0, 0.05)"),
            "color: color(srgb 1 0 0 / 0.050980393)"
        );
    }

    #[test]
    fn hsl_and_hwb_channels_and_alpha_remain_continuous() {
        for (source, expected) in [
            ("hsl(280 60% 45% / 0.5)", (0.54, 0.18, 0.72, 0.5)),
            ("hwb(30 0% 0% / 50%)", (1.0, 0.5, 0.0, 0.5)),
        ] {
            let declaration = format!("color: {source}");
            let rewritten = preserve_authored_color_semantics(&declaration);
            let value = rewritten
                .strip_prefix("color: ")
                .and_then(parse_color)
                .expect("rewritten color should parse");
            assert_color_value(&value, expected);
        }
    }

    #[test]
    fn scanner_does_not_rewrite_comments_strings_or_data_urls() {
        let source = concat!(
            "/* rgb(1,2,3) */ .a { content: \"rgb(4,5,6)\"; ",
            "background: url(\"data:image/svg+xml,<svg fill='rgb(7,8,9)'/>\"); ",
            "color: rgb(10 20 30); }"
        );
        let rewritten = preserve_authored_color_semantics(source);
        assert!(rewritten.contains("/* rgb(1,2,3) */"));
        assert!(rewritten.contains("content: \"rgb(4,5,6)\""));
        assert!(rewritten.contains("fill='rgb(7,8,9)'"));
        assert!(rewritten.contains("color: color(srgb 0.039215688 0.078431375 0.11764706)"));
        assert_eq!(rewritten.matches("rgb(").count(), 3);
    }

    #[test]
    fn preserves_valid_unparsed_clip_path_path_data() {
        let rules = parse_stylesheet_rules_with_lightning(
            r#".box { clip-path: path(\"M 20 120 L 100 12 L 180 120 Z\") }"#,
        )
        .expect("stylesheet should parse");

        assert!(matches!(
            rules[0].declarations.get("clip-path"),
            Some(CssValue::Keyword(value)) if value == r#"path(\"M 20 120 L 100 12 L 180 120 Z\")"#
        ));
    }

    #[test]
    fn gradient_color_sources_are_not_canonicalized_before_lightning() {
        let source = concat!(
            "background-image: linear-gradient(",
            "rgb(255 0 0), color(srgb 0 0 1)); ",
            "color: rgb(255 0 0)"
        );
        let rewritten = preserve_authored_color_semantics(source);
        assert!(rewritten.contains("linear-gradient(rgb(255 0 0), color(srgb 0 0 1))"));
        assert!(rewritten.ends_with("color: color(srgb 1 0 0)"));
    }

    #[test]
    fn lightning_gradient_compatibility_copy_removes_method_in_any_prelude_order() {
        for (source, expected) in [
            (
                "background: linear-gradient(in oklab to right, red, blue)",
                "background: linear-gradient(to right, red, blue)",
            ),
            (
                "background: radial-gradient(circle in srgb, red, blue)",
                "background: radial-gradient(circle, red, blue)",
            ),
            (
                "background: conic-gradient(in oklab from 30deg, red, blue)",
                "background: conic-gradient(from 30deg, red, blue)",
            ),
        ] {
            assert_eq!(gradients_for_lightning(source), expected);
        }
    }

    #[test]
    fn gradient_round_trip_restores_owned_authored_source() {
        let source = "linear-gradient(in oklab, rgb(10% 20% 30%), color(srgb 0 0 1))";
        let inline = parse_inline_style_with_lightning(&format!("background-image: {source}"))
            .expect("inline gradient should parse");
        let CssValue::BackgroundLayers(layers) = inline
            .get("background-image")
            .expect("inline background image")
        else {
            panic!("expected typed image layers");
        };
        assert!(matches!(layers.as_slice(), [BackgroundLayerSource::Linear(raw)] if raw == source));

        let stylesheet = parse_stylesheet_rules_with_lightning(&format!(
            ".sample {{ background-image: {source} }}"
        ))
        .expect("stylesheet gradient should parse");
        let CssValue::BackgroundLayers(layers) = stylesheet[0]
            .declarations
            .get("background-image")
            .expect("stylesheet background image")
        else {
            panic!("expected typed image layers");
        };
        assert!(matches!(layers.as_slice(), [BackgroundLayerSource::Linear(raw)] if raw == source));
    }

    #[test]
    fn conic_percentage_range_stops_survive_stylesheet_validation() {
        let source = "conic-gradient(from 23deg at 64% 34%, #ff00c8 0 25%, #00ff67 25% 55%, #ff6a00 55% 83%, #402080 83% 100%)";
        assert_eq!(
            gradients_for_lightning(source),
            "conic-gradient(from 23deg at 64% 34%, #ff00c8 0deg 90deg, #00ff67 90deg 198deg, #ff6a00 198deg 298.8deg, #402080 298.8deg 360deg)"
        );
        let stylesheet = parse_stylesheet_rules_with_lightning(&format!(
            ".sample {{ background-image: {source} }}"
        ))
        .expect("stylesheet conic gradient should parse");
        let CssValue::BackgroundLayers(layers) = stylesheet[0]
            .declarations
            .get("background-image")
            .expect("stylesheet background image")
        else {
            panic!("expected typed image layers");
        };
        assert!(matches!(layers.as_slice(), [BackgroundLayerSource::Conic(raw)] if raw == source));
    }

    #[test]
    fn rejected_authored_gradient_does_not_replace_an_earlier_valid_image() {
        let style = parse_inline_style_with_lightning(concat!(
            "background-image: linear-gradient(red, blue); ",
            "background-image: linear-gradient(in display-p3, black, white)"
        ))
        .expect("declaration list should parse");
        let CssValue::BackgroundLayers(layers) = style
            .get("background-image")
            .expect("valid earlier background image")
        else {
            panic!("expected typed image layers");
        };
        assert!(
            matches!(layers.as_slice(), [BackgroundLayerSource::Linear(raw)] if raw == "linear-gradient(red, blue)")
        );
    }

    #[test]
    fn typed_length_arithmetic_survives_lightning_and_keeps_cascade_order() {
        let source = "calc(40px * 3px / 1px)";
        let inline = parse_inline_style_with_lightning(&format!("width: 8px; width: {source}"))
            .expect("typed inline width should parse");
        assert!(matches!(
            inline.get("width"),
            Some(CssValue::Math(expression))
                if expression.to_css_string().as_deref() == Some(source)
        ));

        let rules = parse_stylesheet_rules_with_lightning(&format!(
            ".sample {{ width: {source}; width: 24px }}"
        ))
        .expect("typed stylesheet width should preserve its cascade slot");
        assert!(matches!(
            rules[0].declarations.get("width"),
            Some(CssValue::Length(value)) if (*value - 18.0).abs() < f32::EPSILON
        ));

        let important =
            parse_inline_style_with_lightning(&format!("width: {source} !important; width: 24px"))
                .expect("important typed width should parse");
        assert!(matches!(important.get("width"), Some(CssValue::Math(_))));
    }

    #[test]
    fn unmatched_compound_dimension_does_not_replace_a_valid_width() {
        let inline = parse_inline_style_with_lightning("width: 8px; width: calc(10px * 2px)")
            .expect("declaration list should parse");
        assert!(matches!(
            inline.get("width"),
            Some(CssValue::Length(value)) if (*value - 6.0).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn word_spacing_percentage_survives_lightning_compatibility() {
        let inline = parse_inline_style_with_lightning("word-spacing: 200%")
            .expect("percentage word spacing should parse");
        assert!(matches!(
            inline.get("word-spacing"),
            Some(CssValue::Percentage(value)) if (*value - 200.0).abs() < f32::EPSILON
        ));

        let rules =
            parse_stylesheet_rules_with_lightning(".sample { word-spacing: calc(50% + 3pt) }")
                .expect("percentage word spacing rule should parse");
        assert!(matches!(
            rules[0].declarations.get("word-spacing"),
            Some(CssValue::Math(expression)) if expression.contains_percentage()
        ));

        let later_length =
            parse_inline_style_with_lightning("word-spacing: 20%; word-spacing: 4pt")
                .expect("later ordinary length should parse");
        assert!(matches!(
            later_length.get("word-spacing"),
            Some(CssValue::Length(value)) if (*value - 4.0).abs() < f32::EPSILON
        ));

        let important_percentage =
            parse_inline_style_with_lightning("word-spacing: 20% !important; word-spacing: 4pt")
                .expect("important percentage should win");
        assert!(matches!(
            important_percentage.get("word-spacing"),
            Some(CssValue::Percentage(value)) if (*value - 20.0).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn grid_level_two_subgrid_tracks_survive_lightning_compatibility() {
        let inline = parse_inline_style_with_lightning(
            "grid-template-columns: subgrid [inner-start] [inner-end]",
        )
        .expect("subgrid declaration should parse");
        assert!(matches!(
            inline.get("grid-template-columns"),
            Some(CssValue::Keyword(value)) if value == "subgrid [inner-start] [inner-end]"
        ));

        let stylesheet = parse_stylesheet_rules_with_lightning(
            ".sub { grid-template-rows: subgrid [first] [last] }",
        )
        .expect("subgrid stylesheet should parse");
        assert!(matches!(
            stylesheet[0].declarations.get("grid-template-rows"),
            Some(CssValue::Keyword(value)) if value == "subgrid [first] [last]"
        ));

        let later_track = parse_inline_style_with_lightning(
            "grid-template-columns: subgrid; grid-template-columns: 40px",
        )
        .expect("later ordinary track should parse");
        assert!(matches!(
            later_track.get("grid-template-columns"),
            Some(CssValue::Keyword(value)) if value == "40px"
        ));
    }

    #[test]
    fn deferred_components_are_untouched_but_constant_var_fallbacks_are_preserved() {
        let source = concat!(
            "color: rgb(var(--red) 0 0); ",
            "background-color: rgb(calc(10% + 1%) 20% 30%); ",
            "--fallback: var(--missing, rgb(1 2 3))"
        );
        assert_eq!(
            preserve_authored_color_semantics(source),
            concat!(
                "color: rgb(var(--red) 0 0); ",
                "background-color: rgb(calc(10% + 1%) 20% 30%); ",
                "--fallback: var(--missing, color(srgb 0.003921569 0.007843138 0.011764706))"
            )
        );
    }

    #[test]
    fn comments_between_constant_components_are_css_whitespace() {
        assert_eq!(
            preserve_authored_color_semantics("color: rgb(10%/* red */ 20% 30%)"),
            "color: color(srgb 0.1 0.2 0.3)"
        );
    }

    #[test]
    fn invalid_mixed_legacy_rgb_is_not_rewritten_into_valid_css() {
        let source = "color: rgb(10%, 20, 30%)";
        assert_eq!(preserve_authored_color_semantics(source), source);
    }

    #[test]
    fn lightning_round_trip_keeps_source_model_precision() {
        let inline = parse_inline_style_with_lightning(
            "color: rgba(255, 87, 34, .5); background-color: hsl(280 60% 45% / .5)",
        )
        .expect("inline declaration should parse");
        assert_color_value(
            inline.get("color").expect("inline foreground color"),
            (1.0, 87.0 / 255.0, 34.0 / 255.0, 128.0 / 255.0),
        );
        assert_color_value(
            inline
                .get("background-color")
                .expect("inline background color"),
            (0.54, 0.18, 0.72, 0.5),
        );

        let stylesheet = parse_stylesheet_rules_with_lightning(
            ".sample { background-color: hwb(30 0% 0% / 50%) }",
        )
        .expect("stylesheet should parse");
        assert_color_value(
            stylesheet[0]
                .declarations
                .get("background-color")
                .expect("stylesheet background color"),
            (1.0, 0.5, 0.0, 0.5),
        );
    }
}
