use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use std::collections::HashMap;

use super::dom::{DomNode, ElementNode, HtmlTag};
use crate::error::IronpressError;

/// Result of parsing HTML — nodes plus any embedded stylesheets.
pub struct ParseResult {
    pub nodes: Vec<DomNode>,
    pub stylesheets: Vec<String>,
}

/// Parse an HTML string into an internal DOM tree.
pub fn parse_html(html: &str) -> Result<Vec<DomNode>, IronpressError> {
    let result = parse_html_with_styles(html)?;
    Ok(result.nodes)
}

/// Parse an HTML string, returning both DOM nodes and embedded stylesheets.
pub fn parse_html_with_styles(html: &str) -> Result<ParseResult, IronpressError> {
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .map_err(|e| IronpressError::ParseError(e.to_string()))?;

    let mut stylesheets = Vec::new();
    let nodes = convert_handle(&dom.document, &mut stylesheets);
    Ok(ParseResult { nodes, stylesheets })
}

fn convert_handle(handle: &Handle, stylesheets: &mut Vec<String>) -> Vec<DomNode> {
    let node = handle;
    let data = &node.data;

    match data {
        NodeData::Document => {
            let mut result = Vec::new();
            for child in node.children.borrow().iter() {
                result.extend(convert_handle(child, stylesheets));
            }
            result
        }
        NodeData::Text { contents } => {
            let text = contents.borrow().to_string();
            if text.trim().is_empty() {
                vec![]
            } else {
                vec![DomNode::Text(text)]
            }
        }
        NodeData::Element { name, attrs, .. } => {
            let tag_name = name.local.as_ref();
            let tag = HtmlTag::from_tag_name(tag_name);

            // Extract <style> content
            if tag_name == "style" {
                let mut css = String::new();
                for child in node.children.borrow().iter() {
                    if let NodeData::Text { contents } = &child.data {
                        css.push_str(&contents.borrow());
                    }
                }
                if !css.trim().is_empty() {
                    stylesheets.push(css);
                }
                return vec![];
            }

            // Skip <head> but extract styles from it first
            if tag == HtmlTag::Head {
                for child in node.children.borrow().iter() {
                    convert_handle(child, stylesheets);
                }
                return vec![];
            }

            let mut attributes = HashMap::new();
            for attr in attrs.borrow().iter() {
                attributes.insert(attr.name.local.as_ref().to_string(), attr.value.to_string());
            }

            let mut children = Vec::new();
            for child in node.children.borrow().iter() {
                children.extend(convert_handle(child, stylesheets));
            }

            let elem = ElementNode {
                tag,
                attributes,
                children,
            };

            // Unwrap structural tags (html, body) — just return their children
            if tag == HtmlTag::Html || tag == HtmlTag::Body {
                return elem.children;
            }

            vec![DomNode::Element(elem)]
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_paragraph() {
        let nodes = parse_html("<p>Hello World</p>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            DomNode::Element(el) => {
                assert_eq!(el.tag, HtmlTag::P);
                assert_eq!(el.children.len(), 1);
                match &el.children[0] {
                    DomNode::Text(t) => assert_eq!(t, "Hello World"),
                    _ => panic!("Expected text node"),
                }
            }
            _ => panic!("Expected element"),
        }
    }

    #[test]
    fn parse_heading_with_style() {
        let nodes = parse_html(r#"<h1 style="color: red">Title</h1>"#).unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            DomNode::Element(el) => {
                assert_eq!(el.tag, HtmlTag::H1);
                assert_eq!(el.style_attr(), Some("color: red"));
            }
            _ => panic!("Expected element"),
        }
    }

    #[test]
    fn parse_nested_inline() {
        let nodes = parse_html("<p>Hello <strong>World</strong></p>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            DomNode::Element(el) => {
                assert_eq!(el.tag, HtmlTag::P);
                assert_eq!(el.children.len(), 2);
            }
            _ => panic!("Expected element"),
        }
    }

    #[test]
    fn skip_head_section() {
        let nodes =
            parse_html("<html><head><title>Test</title></head><body><p>Hi</p></body></html>")
                .unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            DomNode::Element(el) => assert_eq!(el.tag, HtmlTag::P),
            _ => panic!("Expected paragraph"),
        }
    }

    #[test]
    fn unwrap_html_body() {
        let nodes = parse_html("<html><body><h1>Test</h1><p>Text</p></body></html>").unwrap();
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn parse_empty() {
        let nodes = parse_html("").unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn html_comment_ignored() {
        // Comments should hit the _ => vec![] branch
        let nodes = parse_html("<!-- comment --><p>Hi</p>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            DomNode::Element(el) => assert_eq!(el.tag, HtmlTag::P),
            _ => panic!("Expected element"),
        }
    }
}
