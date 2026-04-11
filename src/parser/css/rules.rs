use super::{
    CssRule, MediaContext, PseudoElement, lightning::parse_stylesheet_rules_with_lightning,
    parse_inline_style, preprocess_media_queries_with_context,
};

/// Parse a CSS stylesheet string into a list of rules.
#[allow(dead_code)]
pub fn parse_stylesheet(css: &str) -> Vec<CssRule> {
    parse_stylesheet_with_context(css, None)
}

/// Parse a CSS stylesheet with page-aware media query evaluation.
pub fn parse_stylesheet_with_context(css: &str, ctx: Option<MediaContext>) -> Vec<CssRule> {
    let preprocessed = preprocess_media_queries_with_context(css, ctx);
    parse_stylesheet_rules_with_lightning(&preprocessed).unwrap_or_else(|| {
        let mut rules = Vec::new();
        parse_rules_from(&preprocessed, &mut rules);
        rules
    })
}

fn parse_rules_from(css: &str, rules: &mut Vec<CssRule>) {
    for chunk in css.split('}') {
        let Some((selector_text, decls)) = chunk.split_once('{') else {
            continue;
        };

        let selector_text = selector_text.trim();
        if selector_text.is_empty() {
            continue;
        }

        let declarations = parse_inline_style(decls);
        if declarations.properties.is_empty() {
            continue;
        }

        for selector in selector_text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let (selector, pseudo_element) = extract_pseudo_element(selector);
            rules.push(CssRule {
                selector,
                declarations: declarations.clone(),
                pseudo_element,
            });
        }
    }
}

/// Extract `::before` or `::after` from a selector.
pub(crate) fn extract_pseudo_element(selector: &str) -> (String, Option<PseudoElement>) {
    for (suffix, pseudo_element) in [
        ("::before", PseudoElement::Before),
        ("::after", PseudoElement::After),
        (":before", PseudoElement::Before),
        (":after", PseudoElement::After),
    ] {
        if let Some(base) = selector.strip_suffix(suffix) {
            let base = base.trim();
            let base = if base.is_empty() {
                "*".to_string()
            } else {
                base.to_string()
            };
            return (base, Some(pseudo_element));
        }
    }

    (selector.trim().to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::{extract_pseudo_element, parse_stylesheet};
    use crate::parser::css::PseudoElement;

    #[test]
    fn parse_stylesheet_basic_rules() {
        let rules = parse_stylesheet("p { color: red; font-size: 14pt } h1 { font-weight: bold }");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, "p");
        assert_eq!(rules[1].selector, "h1");
        assert!(rules[0].declarations.get("color").is_some());
    }

    #[test]
    fn parse_stylesheet_class_and_id_rules() {
        let rules = parse_stylesheet(".highlight { font-weight: bold } #main { color: blue }");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, ".highlight");
        assert_eq!(rules[1].selector, "#main");
    }

    #[test]
    fn parse_stylesheet_skips_empty_rules() {
        assert!(parse_stylesheet("{ color: red }").is_empty());
        assert!(parse_stylesheet("p { }").is_empty());
    }

    #[test]
    fn parse_stylesheet_media_rules_follow_preprocessing() {
        assert_eq!(
            parse_stylesheet("@media print { p { color: red } }").len(),
            1
        );
        assert!(parse_stylesheet("@media screen { p { color: red } }").is_empty());
        assert_eq!(
            parse_stylesheet(
                "h1 { font-size: 24pt } @media print { p { color: blue } } h2 { font-size: 18pt }"
            )
            .len(),
            3
        );
    }

    #[test]
    fn parse_pseudo_elements() {
        let before = parse_stylesheet(r#"p::before { content: "Hello" }"#);
        let after = parse_stylesheet(r#"p:after { content: "!" }"#);
        assert_eq!(before[0].pseudo_element, Some(PseudoElement::Before));
        assert_eq!(before[0].selector, "p");
        assert_eq!(after[0].pseudo_element, Some(PseudoElement::After));
        assert_eq!(after[0].selector, "p");
    }

    #[test]
    fn extract_pseudo_element_variants() {
        assert_eq!(
            extract_pseudo_element("p::after"),
            ("p".to_string(), Some(PseudoElement::After))
        );
        assert_eq!(
            extract_pseudo_element("div:before"),
            ("div".to_string(), Some(PseudoElement::Before))
        );
        assert_eq!(extract_pseudo_element("p"), ("p".to_string(), None));
    }

    #[test]
    fn extract_pseudo_element_bare_pseudo() {
        let (base, pe) = extract_pseudo_element("::before");
        assert_eq!(base, "*");
        assert_eq!(pe, Some(PseudoElement::Before));
    }

    #[test]
    fn parse_stylesheet_comma_selectors() {
        let rules = parse_stylesheet("h1, h2, h3 { font-weight: bold }");
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].selector, "h1");
        assert_eq!(rules[1].selector, "h2");
        assert_eq!(rules[2].selector, "h3");
    }

    #[test]
    fn parse_stylesheet_with_context_none() {
        use super::parse_stylesheet_with_context;
        let rules = parse_stylesheet_with_context("p { color: red }", None);
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn parse_stylesheet_with_context_some() {
        use super::super::MediaContext;
        use super::parse_stylesheet_with_context;
        let ctx = MediaContext {
            width: 595.0,
            height: 842.0,
        };
        let rules = parse_stylesheet_with_context("p { color: red }", Some(ctx));
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn parse_stylesheet_malformed_css() {
        // Missing closing brace should not panic
        let rules = parse_stylesheet("p { color: red");
        // May or may not produce a rule, but should not panic
        let _ = rules;
    }

    #[test]
    fn parse_stylesheet_keeps_pseudo_rules_with_embedded_braces_and_comments() {
        let rules = parse_stylesheet(
            r#"
            /* legacy brace splitting would break this */
            p::before,
            div:after {
                content: "}";
                color: red;
            }
        "#,
        );

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, "p");
        assert_eq!(rules[0].pseudo_element, Some(PseudoElement::Before));
        assert_eq!(rules[1].selector, "div");
        assert_eq!(rules[1].pseudo_element, Some(PseudoElement::After));
        assert!(
            rules
                .iter()
                .all(|rule| rule.declarations.get("color").is_some())
        );
        assert!(rules.iter().all(|rule| matches!(
            rule.declarations.get("content"),
            Some(crate::parser::css::CssValue::Keyword(value)) if value == "\"}\""
        )));
    }
}
