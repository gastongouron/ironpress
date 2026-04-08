use std::collections::HashMap;

use super::SelectorContext;
use super::selectors::{
    ancestor_info, rfind_descendant_space, selector_matches, selector_matches_with_context,
};
use crate::parser::dom::{ElementNode, HtmlTag};

fn make_element(tag: &str) -> ElementNode {
    let mut element = ElementNode::new(HtmlTag::from_tag_name(tag));
    element.raw_tag_name = tag.to_string();
    element
}

#[test]
fn selector_matches_basic_tag_class_id_and_comma() {
    assert!(selector_matches("p", "p", &[], None));
    assert!(selector_matches("*", "p", &[], None));
    assert!(selector_matches(".foo", "p", &["foo", "bar"], None));
    assert!(selector_matches(".foo.bar", "p", &["foo", "bar"], None));
    assert!(selector_matches("*.foo", "p", &["foo", "bar"], None));
    assert!(selector_matches("div#main", "div", &[], Some("main")));
    assert!(selector_matches("#main.foo", "div", &["foo"], Some("main")));
    assert!(selector_matches("*#main", "div", &[], Some("main")));
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
            },
        ],
        child_index: 0,
        sibling_count: 1,
        preceding_siblings: Vec::new(),
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
        }],
        child_index: 0,
        sibling_count: 1,
        preceding_siblings: Vec::new(),
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
fn selector_matches_attribute_variants() {
    let attrs = HashMap::from([
        ("href".to_string(), "https://example.com".to_string()),
        ("id".to_string(), "link".to_string()),
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
    assert!(selector_matches_with_context(
        "a[href].foo",
        "a",
        &["foo"],
        None,
        &attrs,
        &SelectorContext::default(),
    ));
    assert!(selector_matches_with_context(
        "a[href]#link",
        "a",
        &[],
        Some("link"),
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
    };
    let third_child = SelectorContext {
        ancestors: Vec::new(),
        child_index: 2,
        sibling_count: 3,
        preceding_siblings: Vec::new(),
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
    };
    let second_child = SelectorContext {
        ancestors: Vec::new(),
        child_index: 1,
        sibling_count: 4,
        preceding_siblings: Vec::new(),
    };
    let third_child = SelectorContext {
        ancestors: Vec::new(),
        child_index: 2,
        sibling_count: 4,
        preceding_siblings: Vec::new(),
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
