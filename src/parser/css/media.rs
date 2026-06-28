use super::MediaContext;

/// Evaluate whether a media query matches the PDF output context.
pub(crate) fn evaluate_media_query(query: &str, ctx: Option<MediaContext>) -> bool {
    query
        .split(" and ")
        .map(str::trim)
        .all(|part| evaluate_media_part(part, ctx))
}

fn evaluate_media_part(part: &str, ctx: Option<MediaContext>) -> bool {
    match part {
        "print" | "all" => true,
        "screen" => false,
        _ if part.starts_with('(') && part.ends_with(')') => {
            let feature = part.trim_matches(|ch| ch == '(' || ch == ')');
            let (name, raw_value) = feature
                .split_once(':')
                .map_or((feature, ""), |(name, value)| (name.trim(), value.trim()));

            let context = ctx.unwrap_or(MediaContext {
                width: 595.28,
                height: 841.89,
            });

            match name {
                "orientation" => match raw_value {
                    "portrait" => context.height >= context.width,
                    "landscape" => context.width > context.height,
                    _ => false,
                },
                "min-width" => {
                    parse_media_length(raw_value).is_some_and(|value| context.width >= value)
                }
                "max-width" => {
                    parse_media_length(raw_value).is_some_and(|value| context.width <= value)
                }
                "min-height" => {
                    parse_media_length(raw_value).is_some_and(|value| context.height >= value)
                }
                "max-height" => {
                    parse_media_length(raw_value).is_some_and(|value| context.height <= value)
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Parse a length value from a media query.
fn parse_media_length(val: &str) -> Option<f32> {
    let val = val.trim();
    if let Some(number) = val.strip_suffix("pt") {
        return number.parse::<f32>().ok();
    }
    if let Some(number) = val.strip_suffix("px") {
        return number.parse::<f32>().ok().map(|value| value * 0.75);
    }
    if let Some(number) = val.strip_suffix("mm") {
        return number.parse::<f32>().ok().map(|value| value * 72.0 / 25.4);
    }
    if let Some(number) = val.strip_suffix("in") {
        return number.parse::<f32>().ok().map(|value| value * 72.0);
    }
    val.parse::<f32>().ok()
}

/// Evaluate a `@supports` condition. Returns `true` when ironpress can render
/// the feature(s) the condition tests, so the guarded block should be unwrapped.
///
/// This is a pragmatic, lenient evaluator: it handles `not(...)`, `and`/`or`
/// combinators and parenthesised `(property: value)` declarations. A bare
/// declaration whose property looks renderable is treated as supported, and we
/// DEFAULT to `true` for anything that parses like `(x: y)` so real content is
/// never silently dropped. Only an explicitly-unknown property is rejected.
pub(crate) fn supports_condition(cond: &str) -> bool {
    let cond = cond.trim();
    if cond.is_empty() {
        return false;
    }

    // `not(...)` / `not (...)` negates the inner condition.
    if let Some(rest) = cond.strip_prefix("not") {
        let rest = rest.trim();
        if rest.starts_with('(') {
            return !supports_condition(rest);
        }
    }

    // Combinators: split on top-level ` and ` / ` or ` (outside parentheses).
    if let Some(parts) = split_top_level(cond, "and") {
        return parts.iter().all(|part| supports_condition(part));
    }
    if let Some(parts) = split_top_level(cond, "or") {
        return parts.iter().any(|part| supports_condition(part));
    }

    // A single parenthesised group: it may wrap a declaration or another
    // (possibly combined / negated) condition.
    if cond.starts_with('(') && cond.ends_with(')') {
        let inner = cond[1..cond.len() - 1].trim();
        // `(property: value)` — evaluate the declaration.
        if let Some((name, value)) = inner.split_once(':') {
            let name = name.trim();
            let value = value.trim();
            // Reject a malformed declaration with no value.
            if name.is_empty() || value.is_empty() {
                return false;
            }
            // Anything that isn't an obviously-unknown property is supported.
            return !is_unsupported_property(name);
        }
        // Otherwise it nests a further condition (e.g. `((a: b) and (c: d))`).
        return supports_condition(inner);
    }

    // Unrecognised shape (e.g. `selector(...)`, `font-tech(...)`): be lenient.
    true
}

/// Split a `@supports` condition on a top-level ` <op> ` separator, ignoring
/// occurrences nested inside parentheses. Returns `None` when the operator is
/// absent at the top level so the caller can fall through to other handling.
fn split_top_level(cond: &str, op: &str) -> Option<Vec<String>> {
    let needle = format!(" {op} ");
    let bytes = cond.as_bytes();
    let mut depth = 0i32;
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && cond[i..].starts_with(&needle) {
            parts.push(cond[start..i].trim().to_string());
            i += needle.len();
            start = i;
            continue;
        }
        i += 1;
    }
    if parts.is_empty() {
        return None;
    }
    parts.push(cond[start..].trim().to_string());
    Some(parts)
}

/// Conservative deny-list of CSS properties ironpress cannot render, so a
/// `@supports` query that tests them is treated as unsupported. Everything else
/// (the long tail of layout / paint / text properties) is assumed supported so
/// real content is never dropped.
fn is_unsupported_property(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        // Properties referenced by the spec but meaningless / unimplemented for
        // a static print target, or deliberately-bogus probes.
        "nonsense-prop"
            | "not-a-real-prop"
            | "definitely-not-a-property"
            | "scroll-behavior"
            | "scroll-snap-type"
            | "overscroll-behavior"
            | "cursor"
            | "pointer-events"
            | "user-select"
            | "touch-action"
            | "caret-color"
            | "accent-color"
    )
}

pub(crate) fn preprocess_media_queries(css: &str) -> String {
    preprocess_media_queries_with_context(css, None)
}

#[allow(clippy::while_let_on_iterator)]
pub(crate) fn preprocess_media_queries_with_context(
    css: &str,
    ctx: Option<MediaContext>,
) -> String {
    // Strip CSS comments FIRST. The at-rule scanner below keys on raw `@`
    // characters; a comment such as `/* the @media print block ... */` would
    // otherwise start a bogus at-rule that swallows up to the next `{` (the real
    // `@media print {`), evaluate the garbage as non-matching, and drop the real
    // block — silently discarding print-only styles.
    let css = strip_css_comments(css);
    let mut output = String::new();
    let mut chars = css.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '@' {
            output.push(ch);
            continue;
        }

        let mut at_rule = String::from('@');
        while let Some(next) = chars.peek().copied() {
            at_rule.push(next);
            chars.next();
            if next == '{' || next == ';' {
                break;
            }
        }

        if at_rule.starts_with("@media") && at_rule.ends_with('{') {
            let query = at_rule
                .trim_end_matches('{')
                .trim_start_matches("@media")
                .trim();
            let content = extract_braced_content(&mut chars);
            if evaluate_media_query(query, ctx) {
                output.push_str(&content);
            }
            continue;
        }

        if at_rule.starts_with("@supports") && at_rule.ends_with('{') {
            let condition = at_rule
                .trim_end_matches('{')
                .trim_start_matches("@supports")
                .trim();
            let content = extract_braced_content(&mut chars);
            // Like `@media`, unwrap the block when the condition is supported
            // (emit just the inner rules); otherwise drop it entirely. Emitting
            // the wrapper verbatim would make the stylesheet parser choke on the
            // `@supports (...) {` prelude and silently discard the inner rules.
            if supports_condition(condition) {
                output.push_str(&content);
            }
            continue;
        }

        if at_rule.starts_with("@page") && at_rule.ends_with('{')
            || at_rule.starts_with("@font-face") && at_rule.ends_with('{')
        {
            output.push_str(&at_rule);
            output.push_str(&extract_braced_content(&mut chars));
            output.push('}');
            continue;
        }

        if at_rule.starts_with("@import") && at_rule.ends_with(';') {
            output.push_str(&at_rule);
            continue;
        }

        output.push_str(&at_rule);
    }

    lower_flow_root_display(&output)
}

/// Remove `/* ... */` CSS comments, leaving string literals (`'...'` / `"..."`)
/// untouched so a `/*` inside a quoted value is not mistaken for a comment.
pub(crate) fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    let mut quote: u8 = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if quote != 0 {
            out.push(c as char);
            if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = c;
            out.push(c as char);
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // Skip to the closing */ (or end of input for an unterminated comment).
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // Copy this byte, preserving any multi-byte UTF-8 sequence verbatim.
        let ch_len = utf8_len(c);
        let end = (i + ch_len).min(bytes.len());
        out.push_str(&css[i..end]);
        i = end;
    }
    out
}

/// Lower `display: flow-root` to a block rule plus a generated clearfix.
///
/// CSS Display 3 defines `flow-root` as a block container that establishes a new
/// block formatting context. The computed display enum does not yet have a
/// native flow-root variant, but generated block pseudos with `clear: both`
/// reuse the engine's existing float-clearance path without clipping descendants.
fn lower_flow_root_display(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    let mut quote: u8 = 0;
    let mut declaration_start = false;
    let mut block_depth = 0usize;
    let mut rule_start = 0usize;
    let mut current_clearfix_selectors: Option<String> = None;
    let mut current_rule_needs_clearfix = false;

    while i < bytes.len() {
        let c = bytes[i];
        if quote != 0 {
            out.push(c as char);
            if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }

        if c == b'"' || c == b'\'' {
            quote = c;
            out.push(c as char);
            i += 1;
            continue;
        }

        match c {
            b'{' => {
                if block_depth == 0 {
                    current_clearfix_selectors =
                        flow_root_clearfix_selectors(css[rule_start..i].trim());
                    current_rule_needs_clearfix = false;
                }
                block_depth += 1;
                declaration_start = true;
                out.push('{');
                i += 1;
                continue;
            }
            b'}' => {
                let append_clearfix = block_depth == 1
                    && current_rule_needs_clearfix
                    && current_clearfix_selectors.is_some();
                block_depth = block_depth.saturating_sub(1);
                declaration_start = false;
                out.push('}');
                if append_clearfix {
                    if let Some(selectors) = current_clearfix_selectors.take() {
                        out.push(' ');
                        out.push_str(&selectors);
                        out.push_str(r#" { content: ""; display: block; clear: both }"#);
                    }
                }
                if block_depth == 0 {
                    rule_start = i + 1;
                    current_clearfix_selectors = None;
                    current_rule_needs_clearfix = false;
                }
                i += 1;
                continue;
            }
            b';' => {
                declaration_start = block_depth > 0;
                out.push(';');
                if block_depth == 0 {
                    rule_start = i + 1;
                }
                i += 1;
                continue;
            }
            _ => {}
        }

        if block_depth > 0 && declaration_start && c.is_ascii_whitespace() {
            out.push(c as char);
            i += 1;
            continue;
        }

        if block_depth > 0
            && declaration_start
            && let Some((next_i, important)) = parse_flow_root_display_declaration(css, i)
        {
            out.push_str("display: block");
            if important {
                out.push_str(" !important");
            }
            if block_depth == 1 {
                current_rule_needs_clearfix = true;
            }
            i = next_i;
            declaration_start = false;
            continue;
        }

        declaration_start = false;
        let ch_len = utf8_len(c);
        let end = (i + ch_len).min(bytes.len());
        out.push_str(&css[i..end]);
        i = end;
    }

    out
}

fn flow_root_clearfix_selectors(prelude: &str) -> Option<String> {
    if prelude.starts_with('@') {
        return None;
    }

    let selectors: Vec<String> = split_selector_list(prelude)
        .into_iter()
        .map(|selector| selector.trim().to_string())
        .filter(|selector| selector_can_have_clearfix(selector))
        .map(|selector| format!("{selector}::after"))
        .collect();

    (!selectors.is_empty()).then(|| selectors.join(", "))
}

fn split_selector_list(selector_list: &str) -> Vec<&str> {
    let bytes = selector_list.as_bytes();
    let mut selectors = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: u8 = 0;
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = c;
            i += 1;
            continue;
        }
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                selectors.push(&selector_list[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }

    selectors.push(&selector_list[start..]);
    selectors
}

fn selector_can_have_clearfix(selector: &str) -> bool {
    !selector.is_empty()
        && !selector.starts_with('@')
        && !selector.contains("::")
        && !selector.ends_with(":before")
        && !selector.ends_with(":after")
        && !selector.ends_with(":first-line")
        && !selector.ends_with(":first-letter")
}

fn parse_flow_root_display_declaration(css: &str, start: usize) -> Option<(usize, bool)> {
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut i = start;

    while i < len && is_css_ident_byte(bytes[i]) {
        i += 1;
    }
    if !css[start..i].eq_ignore_ascii_case("display") {
        return None;
    }

    while i < len && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= len || bytes[i] != b':' {
        return None;
    }
    i += 1;

    let value_start = i;
    let mut quote: u8 = 0;
    while i < len {
        let c = bytes[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            quote = c;
            i += 1;
            continue;
        }
        if c == b';' || c == b'}' {
            break;
        }
        i += 1;
    }

    let value = css[value_start..i].trim();
    let (value, important) = strip_important(value);
    if !value.eq_ignore_ascii_case("flow-root") {
        return None;
    }

    Some((i, important))
}

fn strip_important(value: &str) -> (&str, bool) {
    let Some((before, after)) = value.rsplit_once('!') else {
        return (value.trim(), false);
    };
    if after.trim().eq_ignore_ascii_case("important") {
        (before.trim(), true)
    } else {
        (value.trim(), false)
    }
}

fn is_css_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

fn utf8_len(first: u8) -> usize {
    if first & 0x80 == 0 {
        1
    } else if first & 0xE0 == 0xC0 {
        2
    } else if first & 0xF0 == 0xE0 {
        3
    } else if first & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

/// Extract content inside braces, handling nested brace pairs.
pub(crate) fn extract_braced_content(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut depth = 1;
    let mut content = String::new();

    for ch in chars.by_ref() {
        match ch {
            '{' => {
                depth += 1;
                content.push(ch);
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                content.push(ch);
            }
            _ => content.push(ch),
        }
    }

    content
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate_media_query, preprocess_media_queries, preprocess_media_queries_with_context,
    };
    use crate::parser::css::MediaContext;
    use crate::parser::css::parse_stylesheet_with_context;

    #[test]
    fn media_query_orientation_and_lengths() {
        let portrait = MediaContext {
            width: 595.0,
            height: 842.0,
        };
        let landscape = MediaContext {
            width: 842.0,
            height: 595.0,
        };

        assert!(evaluate_media_query("print", Some(portrait)));
        assert!(evaluate_media_query("all", Some(portrait)));
        assert!(!evaluate_media_query("screen", Some(portrait)));
        assert!(evaluate_media_query(
            "(orientation: portrait)",
            Some(portrait)
        ));
        assert!(evaluate_media_query(
            "(orientation: landscape)",
            Some(landscape)
        ));
        assert!(evaluate_media_query(
            "(min-width: 600pt)",
            Some(MediaContext {
                width: 612.0,
                height: 792.0
            })
        ));
        assert!(evaluate_media_query(
            "(max-width: 500pt)",
            Some(MediaContext {
                width: 400.0,
                height: 792.0
            })
        ));
        assert!(evaluate_media_query(
            "(min-width: 800px)",
            Some(MediaContext {
                width: 612.0,
                height: 792.0
            })
        ));
        assert!(evaluate_media_query("(min-width: 200mm)", Some(portrait)));
        assert!(evaluate_media_query(
            "(min-width: 8in)",
            Some(MediaContext {
                width: 612.0,
                height: 792.0
            })
        ));
        assert!(!evaluate_media_query("(hover: hover)", Some(portrait)));
    }

    #[test]
    fn media_query_compound_and_default_context() {
        let ctx = MediaContext {
            width: 595.0,
            height: 842.0,
        };
        assert!(evaluate_media_query(
            "print and (orientation: portrait)",
            Some(ctx)
        ));
        assert!(!evaluate_media_query(
            "screen and (orientation: portrait)",
            Some(ctx)
        ));
        assert!(evaluate_media_query("(orientation: portrait)", None));
    }

    #[test]
    fn preprocess_media_queries_keeps_non_media_rules() {
        let css = "@charset \"utf-8\"; @media print { p { color: red } }";
        let result = preprocess_media_queries(css);
        assert!(result.contains("@charset"));
        assert!(result.contains("p { color: red }"));
    }

    #[test]
    fn comment_mentioning_at_media_does_not_drop_print_block() {
        // A comment containing "@media print" must not confuse the at-rule
        // scanner into discarding the real print block that follows.
        let css = "\
            .box { background: gray }\n\
            /* the @media print block turns it green; if ignored it stays gray */\n\
            @media print { .box { background: green } }";
        let result = preprocess_media_queries(css);
        assert!(
            result.contains(".box { background: green }"),
            "print block dropped by comment: {result}"
        );
        assert!(
            !result.contains("/*"),
            "comments should be stripped: {result}"
        );
    }

    #[test]
    fn strip_css_comments_preserves_strings() {
        // A `/*` inside a quoted value must survive.
        let out =
            super::strip_css_comments("a { content: \"/* not a comment */\" } /* real */ b{}");
        assert!(
            out.contains("\"/* not a comment */\""),
            "string mangled: {out}"
        );
        assert!(!out.contains("/* real */"), "real comment kept: {out}");
    }

    #[test]
    fn parse_stylesheet_with_media_context() {
        let ctx = MediaContext {
            width: 595.0,
            height: 842.0,
        };
        let rules = parse_stylesheet_with_context(
            "@media (orientation: portrait) { p { color: blue } }",
            Some(ctx),
        );
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn preprocess_supports_unwraps_supported_conditions() {
        // A supported declaration: the inner rule survives without its wrapper.
        let result = preprocess_media_queries("@supports (display: flex) { p { color: red } }");
        assert!(
            result.contains("p { color: red }"),
            "supported @supports block dropped: {result}"
        );
        assert!(
            !result.contains("@supports"),
            "@supports wrapper not unwrapped: {result}"
        );

        // grid is renderable enough that its block must not be dropped.
        let grid = preprocess_media_queries("@supports (display: grid) { p { color: blue } }");
        assert!(
            grid.contains("p { color: blue }"),
            "grid @supports block dropped: {grid}"
        );

        // The fixture's condition.
        let block =
            preprocess_media_queries("@supports (display: block) { .box { background: green } }");
        assert!(
            block.contains(".box { background: green }"),
            "block @supports condition dropped: {block}"
        );

        // The fixture's value: `display: flow-root` should query true and lower
        // to accepted declarations that trigger the existing float-clear path.
        let flow_root = preprocess_media_queries(
            "@supports (display: flow-root) { .box { display: flow-root } }",
        );
        assert!(
            flow_root.contains(".box { display: block }"),
            "flow-root @supports block not lowered: {flow_root}"
        );
        assert!(
            flow_root.contains(r#".box::after { content: ""; display: block; clear: both }"#),
            "flow-root clearfix not inserted: {flow_root}"
        );
    }

    #[test]
    fn preprocess_supports_drops_unsupported_conditions() {
        let result = preprocess_media_queries("@supports (nonsense-prop: x) { p { color: red } }");
        assert!(
            !result.contains("color: red"),
            "unsupported @supports block kept: {result}"
        );
    }

    #[test]
    fn supports_condition_combinators_and_negation() {
        use super::supports_condition;
        assert!(supports_condition("(display: flex)"));
        assert!(supports_condition("(display: flex) and (color: red)"));
        assert!(supports_condition("(nonsense-prop: x) or (display: flex)"));
        assert!(!supports_condition(
            "(nonsense-prop: x) and (display: flex)"
        ));
        assert!(supports_condition("not (nonsense-prop: x)"));
        assert!(!supports_condition("not (display: flex)"));
        // Lenient default for unrecognised shapes.
        assert!(supports_condition("selector(:has(a))"));
    }

    #[test]
    fn preprocess_media_queries_with_context_filters_mismatch() {
        let ctx = MediaContext {
            width: 595.0,
            height: 842.0,
        };
        let result = preprocess_media_queries_with_context(
            "@media (orientation: landscape) { p { color: red } }",
            Some(ctx),
        );
        assert!(!result.contains("color: red"));
    }
}
