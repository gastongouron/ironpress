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

    output
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
        assert!(!result.contains("/*"), "comments should be stripped: {result}");
    }

    #[test]
    fn strip_css_comments_preserves_strings() {
        // A `/*` inside a quoted value must survive.
        let out = super::strip_css_comments("a { content: \"/* not a comment */\" } /* real */ b{}");
        assert!(out.contains("\"/* not a comment */\""), "string mangled: {out}");
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
