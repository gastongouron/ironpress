use std::collections::HashMap;

use super::SelectorContext;
use super::selectors::{
    ancestor_info, rfind_descendant_space, selector_matches, selector_matches_with_context,
    specificity,
};
use crate::parser::dom::{ElementNode, HtmlTag};

/// Pack an (a, b, c) specificity triple the same way `specificity` does.
fn spec(a: u32, b: u32, c: u32) -> u32 {
    (a << 20) | (b << 10) | c
}

fn make_element(tag: &str) -> ElementNode {
    let mut element = ElementNode::new(HtmlTag::from_tag_name(tag));
    element.raw_tag_name = tag.to_string();
    element
}

#[test]
fn selector_matches_basic_tag_class_id_and_comma() {
    assert!(selector_matches("p", "p", &[], None));
    assert!(selector_matches(".foo", "p", &["foo", "bar"], None));
    assert!(selector_matches("div#main", "div", &[], Some("main")));
    assert!(selector_matches("h1, h2, h3", "h2", &[], None));
    assert!(!selector_matches("", "p", &[], None));
}

#[test]
fn selector_matches_descendant_and_child_combinators() {
    let parent = make_element("div");
    let child_ctx = SelectorContext {
        ancestors: vec![ancestor_info(&parent)],
        child_index: 0,
        sibling_count: 1,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    assert!(selector_matches_with_context(
        "div > p",
        "p",
        &[],
        None,
        &HashMap::new(),
        &child_ctx,
    ));
    assert!(selector_matches_with_context(
        "div p",
        "p",
        &[],
        None,
        &HashMap::new(),
        &child_ctx,
    ));
}

#[test]
fn selector_matches_chained_child_and_descendant_combinators() {
    let grandparent = make_element("div");
    let parent = make_element("section");
    let child_ctx = SelectorContext {
        ancestors: vec![
            ancestor_info(&grandparent),
            super::AncestorInfo {
                element: &parent,
                child_index: 0,
                sibling_count: 1,
                preceding_siblings: Vec::new(),
                following_siblings: Vec::new(),
                is_empty: false,
            },
        ],
        child_index: 0,
        sibling_count: 1,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };

    assert!(selector_matches_with_context(
        "div > section > p",
        "p",
        &[],
        None,
        &HashMap::new(),
        &child_ctx,
    ));
    assert!(selector_matches_with_context(
        "div section p",
        "p",
        &[],
        None,
        &HashMap::new(),
        &child_ctx,
    ));
}

#[test]
fn selector_matches_ancestor_side_sibling_combinators() {
    let article = make_element("article");
    let article_ctx = SelectorContext {
        ancestors: Vec::new(),
        child_index: 1,
        sibling_count: 2,
        preceding_siblings: vec![("section".to_string(), vec![])],
        following_siblings: Vec::new(),
        is_empty: false,
    };
    assert!(selector_matches_with_context(
        "section + article",
        "article",
        &[],
        None,
        &HashMap::new(),
        &article_ctx,
    ));
    let child_ctx = SelectorContext {
        ancestors: vec![super::AncestorInfo {
            element: &article,
            child_index: 1,
            sibling_count: 2,
            preceding_siblings: vec![("section".to_string(), vec![])],
            following_siblings: Vec::new(),
            is_empty: false,
        }],
        child_index: 0,
        sibling_count: 1,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };

    assert!(selector_matches_with_context(
        "section + article p",
        "p",
        &[],
        None,
        &HashMap::new(),
        &child_ctx,
    ));
}

#[test]
fn selector_matches_sibling_combinators() {
    let ctx = SelectorContext {
        ancestors: Vec::new(),
        child_index: 1,
        sibling_count: 2,
        preceding_siblings: vec![("h1".to_string(), vec![])],
        following_siblings: Vec::new(),
        is_empty: false,
    };
    assert!(selector_matches_with_context(
        "h1 + p",
        "p",
        &[],
        None,
        &HashMap::new(),
        &ctx,
    ));
    assert!(selector_matches_with_context(
        "h1 ~ p",
        "p",
        &[],
        None,
        &HashMap::new(),
        &ctx,
    ));
}

#[test]
fn selector_matches_chained_sibling_combinators() {
    let ctx = SelectorContext {
        ancestors: Vec::new(),
        child_index: 2,
        sibling_count: 3,
        preceding_siblings: vec![("h1".to_string(), vec![]), ("p".to_string(), vec![])],
        following_siblings: Vec::new(),
        is_empty: false,
    };

    assert!(selector_matches_with_context(
        "h1 + p + span",
        "span",
        &[],
        None,
        &HashMap::new(),
        &ctx,
    ));
    assert!(selector_matches_with_context(
        "h1 ~ p ~ span",
        "span",
        &[],
        None,
        &HashMap::new(),
        &ctx,
    ));
}

#[test]
fn selector_matches_class_sibling_combinators_like_fixtures() {
    // Mirrors selectors-cascade-adjacent-sibling / general-sibling fixtures:
    // <div class="box marker"></div><div class="box"></div><div class="box"></div>
    // The first .box.marker is index 0; the second is index 1 (preceded by the marker);
    // the third is index 2 (preceded by marker + plain box).
    let second_box = SelectorContext {
        ancestors: Vec::new(),
        child_index: 1,
        sibling_count: 3,
        preceding_siblings: vec![(
            "div".to_string(),
            vec!["box".to_string(), "marker".to_string()],
        )],
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let third_box = SelectorContext {
        ancestors: Vec::new(),
        child_index: 2,
        sibling_count: 3,
        preceding_siblings: vec![
            (
                "div".to_string(),
                vec!["box".to_string(), "marker".to_string()],
            ),
            ("div".to_string(), vec!["box".to_string()]),
        ],
        following_siblings: Vec::new(),
        is_empty: false,
    };

    // Adjacent sibling: only the box immediately after the marker matches.
    assert!(
        selector_matches_with_context(
            ".marker + .box",
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &second_box,
        ),
        "adjacent sibling should match the box immediately after the marker"
    );
    assert!(
        !selector_matches_with_context(
            ".marker + .box",
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &third_box,
        ),
        "adjacent sibling should NOT match the third box (not immediately after marker)"
    );

    // General sibling: every box following the marker matches.
    assert!(
        selector_matches_with_context(
            ".marker ~ .box",
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &second_box,
        ),
        "general sibling should match the second box"
    );
    assert!(
        selector_matches_with_context(
            ".marker ~ .box",
            "div",
            &["box"],
            None,
            &HashMap::new(),
            &third_box,
        ),
        "general sibling should match the third box"
    );
}

#[test]
fn selector_matches_attribute_variants() {
    let attrs = HashMap::from([
        ("href".to_string(), "https://example.com".to_string()),
        ("type".to_string(), "text".to_string()),
    ]);
    assert!(selector_matches_with_context(
        "a[href]",
        "a",
        &[],
        None,
        &attrs,
        &SelectorContext::default(),
    ));
    assert!(selector_matches_with_context(
        "input[type=\"text\"]",
        "input",
        &[],
        None,
        &attrs,
        &SelectorContext::default(),
    ));
    assert!(!selector_matches_with_context(
        "input[type=\"password\"]",
        "input",
        &[],
        None,
        &attrs,
        &SelectorContext::default(),
    ));
}

#[test]
fn selector_matches_pseudo_classes() {
    let first_child = SelectorContext {
        ancestors: Vec::new(),
        child_index: 0,
        sibling_count: 3,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let third_child = SelectorContext {
        ancestors: Vec::new(),
        child_index: 2,
        sibling_count: 3,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    assert!(selector_matches_with_context(
        "p:first-child",
        "p",
        &[],
        None,
        &HashMap::new(),
        &first_child,
    ));
    assert!(selector_matches_with_context(
        "p:last-child",
        "p",
        &[],
        None,
        &HashMap::new(),
        &third_child,
    ));
    assert!(selector_matches_with_context(
        "p:nth-child(2n+1)",
        "p",
        &[],
        None,
        &HashMap::new(),
        &third_child,
    ));
    assert!(selector_matches(":not(.active)", "p", &[], None));
    assert!(!selector_matches(":hover", "p", &[], None));
}

#[test]
fn selector_matches_nth_child_keywords_and_spaced_formulas() {
    let first_child = SelectorContext {
        ancestors: Vec::new(),
        child_index: 0,
        sibling_count: 4,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let second_child = SelectorContext {
        ancestors: Vec::new(),
        child_index: 1,
        sibling_count: 4,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };
    let third_child = SelectorContext {
        ancestors: Vec::new(),
        child_index: 2,
        sibling_count: 4,
        preceding_siblings: Vec::new(),
        following_siblings: Vec::new(),
        is_empty: false,
    };

    assert!(selector_matches_with_context(
        "p:nth-child(odd)",
        "p",
        &[],
        None,
        &HashMap::new(),
        &first_child,
    ));
    assert!(selector_matches_with_context(
        "p:nth-child(even)",
        "p",
        &[],
        None,
        &HashMap::new(),
        &second_child,
    ));
    assert!(selector_matches_with_context(
        "p:nth-child(2n + 1)",
        "p",
        &[],
        None,
        &HashMap::new(),
        &third_child,
    ));
}

#[test]
fn selector_space_finder_ignores_attribute_and_paren_content() {
    assert_eq!(rfind_descendant_space("div p"), Some(3));
    assert_eq!(rfind_descendant_space("section + article"), None);
    assert_eq!(rfind_descendant_space("div > p"), None);
    assert_eq!(rfind_descendant_space("p[data-x=\"a b\"]"), None);
    assert_eq!(rfind_descendant_space("p:not(.a .b)"), None);
}

fn attrs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// --- Selectors-4 §6: attribute selector operators + case flags ---

#[test]
fn attribute_operators_match_per_spec() {
    let a = attrs(&[
        ("class", "btn-lg primary"),
        ("lang", "en-US"),
        ("href", "a.png"),
    ]);
    let ctx = SelectorContext::default();
    let m = |sel: &str| selector_matches_with_context(sel, "div", &[], None, &a, &ctx);

    // Presence
    assert!(m("[class]"));
    assert!(!m("[data-missing]"));
    // Exact equals
    assert!(m("[lang=en-US]"));
    assert!(!m("[lang=en]"));
    // Whitespace-list ~=
    assert!(m("[class~=primary]"));
    assert!(m("[class~=btn-lg]"));
    assert!(!m("[class~=btn]"));
    // Hyphen |=
    assert!(m("[lang|=en]"));
    assert!(!m("[lang|=e]"));
    assert!(m("[lang|=en-US]"));
    // Prefix ^=
    assert!(m("[href^=a.]"));
    assert!(!m("[href^=b]"));
    // Suffix $=
    assert!(m("[href$=.png]"));
    assert!(!m("[href$=.jpg]"));
    // Substring *=
    assert!(m("[href*=.pn]"));
    assert!(!m("[href*=zzz]"));
}

#[test]
fn attribute_case_flags() {
    let a = attrs(&[("data-x", "Foo")]);
    let ctx = SelectorContext::default();
    let m = |sel: &str| selector_matches_with_context(sel, "div", &[], None, &a, &ctx);
    // Default is case-sensitive.
    assert!(!m("[data-x=foo]"));
    assert!(m("[data-x=Foo]"));
    // `i` flag = ASCII case-insensitive.
    assert!(m("[data-x=foo i]"));
    assert!(m("[data-x=FOO i]"));
    // `s` flag = case-sensitive.
    assert!(!m("[data-x=foo s]"));
}

#[test]
fn multiple_attribute_selectors_chained() {
    let a = attrs(&[("data-a", "1"), ("data-b", "2")]);
    let ctx = SelectorContext::default();
    assert!(selector_matches_with_context(
        "[data-a=1][data-b=2]",
        "div",
        &[],
        None,
        &a,
        &ctx
    ));
    assert!(!selector_matches_with_context(
        "[data-a=1][data-b=9]",
        "div",
        &[],
        None,
        &a,
        &ctx
    ));
}

// --- Selectors-4 §9: structural pseudo-classes ---

#[test]
fn structural_pseudo_classes() {
    let only = SelectorContext {
        sibling_count: 1,
        child_index: 0,
        is_empty: true,
        ..Default::default()
    };
    assert!(selector_matches_with_context(
        ":only-child",
        "p",
        &[],
        None,
        &HashMap::new(),
        &only
    ));
    assert!(selector_matches_with_context(
        ":empty",
        "p",
        &[],
        None,
        &HashMap::new(),
        &only
    ));

    let not_empty = SelectorContext {
        sibling_count: 2,
        child_index: 0,
        is_empty: false,
        ..Default::default()
    };
    assert!(!selector_matches_with_context(
        ":only-child",
        "p",
        &[],
        None,
        &HashMap::new(),
        &not_empty
    ));
    assert!(!selector_matches_with_context(
        ":empty",
        "p",
        &[],
        None,
        &HashMap::new(),
        &not_empty
    ));
}

#[test]
fn root_pseudo_class() {
    let root = SelectorContext::default();
    assert!(selector_matches_with_context(
        ":root",
        "html",
        &[],
        None,
        &HashMap::new(),
        &root
    ));
    // A non-root element never matches :root.
    let body = make_element("body");
    let nested = SelectorContext {
        ancestors: vec![ancestor_info(&body)],
        ..Default::default()
    };
    assert!(!selector_matches_with_context(
        ":root",
        "div",
        &[],
        None,
        &HashMap::new(),
        &nested
    ));
}

#[test]
fn of_type_pseudo_classes() {
    // <p/><span/><p/><p/>; the third element (second <p>) is at child_index 2,
    // with two preceding siblings (p, span) — one of which is a <p>.
    let third = SelectorContext {
        child_index: 2,
        sibling_count: 4,
        preceding_siblings: vec![("p".to_string(), vec![]), ("span".to_string(), vec![])],
        following_siblings: vec![("p".to_string(), vec![])],
        ..Default::default()
    };
    // It's the 2nd <p> from the start.
    assert!(selector_matches_with_context(
        "p:nth-of-type(2)",
        "p",
        &[],
        None,
        &HashMap::new(),
        &third
    ));
    assert!(!selector_matches_with_context(
        "p:first-of-type",
        "p",
        &[],
        None,
        &HashMap::new(),
        &third
    ));
    // There is one following <p>, so it is NOT the last of type.
    assert!(!selector_matches_with_context(
        "p:last-of-type",
        "p",
        &[],
        None,
        &HashMap::new(),
        &third
    ));

    // First <p> (no preceding <p>, two following <p>): first-of-type but not last.
    let first = SelectorContext {
        child_index: 0,
        sibling_count: 4,
        preceding_siblings: vec![],
        following_siblings: vec![
            ("span".to_string(), vec![]),
            ("p".to_string(), vec![]),
            ("p".to_string(), vec![]),
        ],
        ..Default::default()
    };
    assert!(selector_matches_with_context(
        "p:first-of-type",
        "p",
        &[],
        None,
        &HashMap::new(),
        &first
    ));
}

#[test]
fn nth_last_child_counts_from_end() {
    // 4 children; the 3rd (index 2) is the 2nd from the end.
    let third_of_four = SelectorContext {
        child_index: 2,
        sibling_count: 4,
        ..Default::default()
    };
    assert!(selector_matches_with_context(
        "p:nth-last-child(2)",
        "p",
        &[],
        None,
        &HashMap::new(),
        &third_of_four
    ));
    assert!(!selector_matches_with_context(
        "p:nth-last-child(1)",
        "p",
        &[],
        None,
        &HashMap::new(),
        &third_of_four
    ));
}

// --- Selectors-4 §4: logical pseudo-classes ---

#[test]
fn logical_pseudo_classes() {
    let ctx = SelectorContext::default();
    let m = |sel: &str, classes: &[&str]| {
        selector_matches_with_context(sel, "div", classes, None, &HashMap::new(), &ctx)
    };
    // :is() — any of the list matches.
    assert!(m(":is(.a, .b)", &["b"]));
    assert!(!m(":is(.a, .b)", &["c"]));
    // :where() — same matching as :is.
    assert!(m(":where(.a, .b)", &["a"]));
    // :not() with a list — none may match.
    assert!(m(":not(.a, .b)", &["c"]));
    assert!(!m(":not(.a, .b)", &["a"]));
    // Compound + logical pseudo.
    assert!(m(".x:not(.skip)", &["x"]));
    assert!(!m(".x:not(.skip)", &["x", "skip"]));
}

#[test]
fn has_relational_forward_sibling() {
    // div:has(+ .next) where the element has a following sibling with .next.
    let ctx = SelectorContext {
        following_siblings: vec![("span".to_string(), vec!["next".to_string()])],
        ..Default::default()
    };
    assert!(selector_matches_with_context(
        "div:has(+ .next)",
        "div",
        &[],
        None,
        &HashMap::new(),
        &ctx
    ));
    assert!(selector_matches_with_context(
        "div:has(~ .next)",
        "div",
        &[],
        None,
        &HashMap::new(),
        &ctx
    ));
    // No following sibling with .other.
    assert!(!selector_matches_with_context(
        "div:has(+ .other)",
        "div",
        &[],
        None,
        &HashMap::new(),
        &ctx
    ));
}

// --- css-cascade-4 §6.3 / Selectors-4 §17: specificity ---

#[test]
fn specificity_basic_ordering() {
    // id > class > type
    assert!(specificity("#id") > specificity(".cls"));
    assert!(specificity(".cls") > specificity("div"));
    assert!(specificity("div") > specificity("*"));
    assert_eq!(specificity("*"), 0);
}

#[test]
fn specificity_component_counts() {
    assert_eq!(specificity("div"), spec(0, 0, 1));
    assert_eq!(specificity(".cls"), spec(0, 1, 0));
    assert_eq!(specificity("#id"), spec(1, 0, 0));
    assert_eq!(specificity("div.cls#id"), spec(1, 1, 1));
    assert_eq!(specificity("[data-x]"), spec(0, 1, 0));
    assert_eq!(specificity("a[href][lang]"), spec(0, 2, 1));
    assert_eq!(specificity("li:first-child"), spec(0, 1, 1));
    // Three types break a tie against one class (0,0,3) < (0,1,0).
    assert!(specificity("a b c") < specificity(".one"));
    // Descendant combinator sums the compounds.
    assert_eq!(specificity("ul li a"), spec(0, 0, 3));
    assert_eq!(specificity("#nav .item a"), spec(1, 1, 1));
}

#[test]
fn specificity_logical_pseudo_contributions() {
    // :where() contributes zero.
    assert_eq!(specificity(":where(#id)"), spec(0, 0, 0));
    assert_eq!(specificity("a:where(#id)"), spec(0, 0, 1));
    // :is()/:not()/:has() take the most specific argument.
    assert_eq!(specificity(":is(#id, .cls)"), spec(1, 0, 0));
    assert_eq!(specificity(":not(.a)"), spec(0, 1, 0));
    assert_eq!(specificity(":not(#a, .b)"), spec(1, 0, 0));
}

#[test]
fn specificity_selector_list_takes_max() {
    // A selector list takes the highest-specificity alternative.
    assert_eq!(specificity("#id, div"), spec(1, 0, 0));
    assert_eq!(specificity("div, #id"), spec(1, 0, 0));
}
