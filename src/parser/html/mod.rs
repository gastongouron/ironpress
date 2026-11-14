use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use std::collections::HashMap;

use self::sink::{Document, NodeData, NodeId, Sink};
use super::dom::{DomNode, ElementNode, HtmlTag};
use crate::error::IronpressError;

pub(super) mod sink;

/// Result of parsing HTML — nodes plus any embedded stylesheets.
pub struct ParseResult {
    pub nodes: Vec<DomNode>,
    pub stylesheets: Vec<String>,
    /// What `<head>`, `<html>` and `<body>` declared before they were unwrapped.
    pub(crate) head: super::document_head::DocumentHead,
}

/// Parse an HTML string into an internal DOM tree.
#[allow(dead_code)]
pub fn parse_html(html: &str) -> Result<Vec<DomNode>, IronpressError> {
    let result = parse_html_with_styles(html)?;
    Ok(result.nodes)
}

/// Parse an HTML string, returning both DOM nodes and embedded stylesheets.
pub fn parse_html_with_styles(html: &str) -> Result<ParseResult, IronpressError> {
    let document = parse_document(Sink::new(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .map_err(|e| IronpressError::ParseError(e.to_string()))?;

    let mut stylesheets = Vec::new();
    let head = super::document_head::DocumentHead::scan(&document);
    let nodes = convert_node(&document, Document::ROOT, &mut stylesheets);
    Ok(ParseResult {
        nodes,
        stylesheets,
        head,
    })
}

fn convert_node(document: &Document, id: NodeId, stylesheets: &mut Vec<String>) -> Vec<DomNode> {
    let node = document.node(id);

    match &node.data {
        NodeData::Document => {
            let mut result = Vec::new();
            for &child in &node.children {
                result.extend(convert_node(document, child, stylesheets));
            }
            result
        }
        NodeData::Text(contents) => {
            let text = contents.to_string();
            // A non-breaking space (U+00A0) is *content*, not collapsible
            // whitespace, so a node made only of NBSP must survive (e.g. an
            // `&nbsp;` table cell). Only ASCII whitespace is collapsible here.
            if text.chars().all(|c| c.is_ascii_whitespace()) {
                // Preserve whitespace-only text nodes that contain at least
                // one space (as opposed to consisting solely of newlines and
                // tabs).  These often represent inter-element spacing, e.g.
                // `<span>A</span> <span>B</span>`.
                if text.contains(' ') {
                    vec![DomNode::Text(" ".to_string())]
                } else {
                    vec![]
                }
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
                for &child in &node.children {
                    if let NodeData::Text(contents) = &document.node(child).data {
                        css.push_str(contents);
                    }
                }
                if !css.trim().is_empty() {
                    stylesheets.push(css);
                }
                return vec![];
            }

            // Skip <head> but extract styles from it first
            if tag == HtmlTag::Head {
                for &child in &node.children {
                    convert_node(document, child, stylesheets);
                }
                return vec![];
            }

            let mut attributes = HashMap::new();
            for attr in attrs {
                attributes.insert(attr.name.local.to_string(), attr.value.to_string());
            }

            let mut children = Vec::new();
            for &child in &node.children {
                children.extend(convert_node(document, child, stylesheets));
            }

            let elem = ElementNode {
                tag,
                ordinal: crate::accessibility::ElementOrdinal::ANONYMOUS,
                raw_tag_name: tag_name.to_ascii_lowercase(),
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
    fn document_language_survives_unwrapped_html_and_body_elements() {
        let japanese =
            parse_html_with_styles("<html lang='ja'><body><p>第</p></body></html>").unwrap();
        let korean =
            parse_html_with_styles("<html lang='ja'><body lang='ko'><p>한</p></body></html>")
                .unwrap();

        assert_eq!(
            japanese.head.font_locale,
            crate::font_pack::FontLocale::Japanese
        );
        assert_eq!(
            korean.head.font_locale,
            crate::font_pack::FontLocale::Korean
        );
    }

    #[test]
    fn adjacent_character_tokens_merge_into_one_text_node() {
        let nodes = parse_html("<p>a&amp;b</p>").unwrap();
        let DomNode::Element(el) = &nodes[0] else {
            panic!("Expected element")
        };
        assert_eq!(el.children.len(), 1);
        assert!(matches!(&el.children[0], DomNode::Text(t) if t == "a&b"));
    }

    #[test]
    fn comment_splits_text_into_two_nodes() {
        let nodes = parse_html("<p>a<!-- x -->b</p>").unwrap();
        let DomNode::Element(el) = &nodes[0] else {
            panic!("Expected element")
        };
        assert_eq!(el.children.len(), 2);
        assert!(matches!(&el.children[0], DomNode::Text(t) if t == "a"));
        assert!(matches!(&el.children[1], DomNode::Text(t) if t == "b"));
    }

    #[test]
    fn foster_parented_text_lands_before_the_table() {
        let nodes = parse_html("<table>x<tr><td>y</td></tr></table>").unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(matches!(&nodes[0], DomNode::Text(t) if t == "x"));
        assert!(matches!(&nodes[1], DomNode::Element(el) if el.raw_tag_name == "table"));
    }

    #[test]
    fn template_contents_stay_out_of_the_tree() {
        let nodes = parse_html("<body><template><p>x</p></template><p>y</p></body>").unwrap();
        assert_eq!(nodes.len(), 2);
        let DomNode::Element(template) = &nodes[0] else {
            panic!("Expected element")
        };
        assert_eq!(template.raw_tag_name, "template");
        assert!(template.children.is_empty());
    }

    #[test]
    fn adoption_agency_reparents_misnested_formatting() {
        let nodes = parse_html("<b>1<p>2</b>3</p>").unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(matches!(&nodes[0], DomNode::Element(b) if b.raw_tag_name == "b"));
        let DomNode::Element(p) = &nodes[1] else {
            panic!("Expected element")
        };
        assert_eq!(p.tag, HtmlTag::P);
        assert_eq!(p.children.len(), 2);
        assert!(matches!(&p.children[0], DomNode::Element(b) if b.raw_tag_name == "b"));
        assert!(matches!(&p.children[1], DomNode::Text(t) if t == "3"));
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
