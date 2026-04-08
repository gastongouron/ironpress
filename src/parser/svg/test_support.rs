use crate::parser::dom::{DomNode, ElementNode, HtmlTag};
use std::collections::HashMap;

pub(crate) fn make_el(raw_tag: &str, attrs: Vec<(&str, &str)>) -> ElementNode {
    let mut attributes = HashMap::new();
    for (key, value) in attrs {
        attributes.insert(key.to_string(), value.to_string());
    }

    ElementNode {
        tag: HtmlTag::Unknown,
        raw_tag_name: raw_tag.to_string(),
        attributes,
        children: Vec::new(),
    }
}

pub(crate) fn make_svg_el(attrs: Vec<(&str, &str)>, children: Vec<ElementNode>) -> ElementNode {
    let mut attributes = HashMap::new();
    for (key, value) in attrs {
        attributes.insert(key.to_string(), value.to_string());
    }

    ElementNode {
        tag: HtmlTag::Svg,
        raw_tag_name: "svg".to_string(),
        attributes,
        children: children.into_iter().map(DomNode::Element).collect(),
    }
}
