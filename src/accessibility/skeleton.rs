//! The structure skeleton: every element's role, parent and the attribute
//! facts the structure tree will need, recorded once before layout.

use super::DocumentLanguage;
use super::identity::ElementOrdinal;
use super::role::StructureRole;
use crate::parser::dom::{DomNode, ElementNode, HtmlTag};

/// Roles, parents and facts of every element, indexed by ordinal. Node 0 is
/// the `Document` root; element ordinals start at 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructureSkeleton {
    nodes: Vec<SkeletonNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkeletonNode {
    pub(crate) role: StructureRole,
    pub(crate) parent: ElementOrdinal,
    pub(crate) facts: NodeFacts,
}

/// Attribute-derived facts the structure tree reads back by ordinal, so
/// that no string has to travel through layout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NodeFacts {
    pub(crate) alt: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) aria_label: Option<String>,
    pub(crate) lang: Option<DocumentLanguage>,
    pub(crate) scope: Option<HeaderScope>,
    pub(crate) col_span: u32,
    pub(crate) row_span: u32,
    pub(crate) id: Option<String>,
    /// The `src` attribute, for diagnostics.
    pub(crate) source: Option<String>,
    pub(crate) formula_source: Option<String>,
    /// The `<title>` child of an inline `<svg>`, its accessible name.
    pub(crate) svg_title: Option<String>,
}

/// The HTML `scope` attribute of a `<th>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderScope {
    Row,
    Column,
    RowGroup,
    ColumnGroup,
}

impl HeaderScope {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "row" => Some(Self::Row),
            "col" => Some(Self::Column),
            "rowgroup" => Some(Self::RowGroup),
            "colgroup" => Some(Self::ColumnGroup),
            _ => None,
        }
    }
}

impl StructureSkeleton {
    /// Walk the document in pre-order, stamping every element's ordinal and
    /// recording its skeleton node. Iterative, so deep trees cannot overflow
    /// the stack.
    pub(crate) fn assign(nodes: &mut [DomNode]) -> Self {
        let mut skeleton = Self {
            nodes: vec![SkeletonNode {
                role: StructureRole::Document,
                parent: ElementOrdinal::ANONYMOUS,
                facts: NodeFacts::default(),
            }],
        };
        let document = ElementOrdinal::from_skeleton_index(0);
        let mut pending: Vec<(ElementOrdinal, &mut DomNode)> = nodes
            .iter_mut()
            .rev()
            .map(|node| (document, node))
            .collect();
        while let Some((parent, node)) = pending.pop() {
            let DomNode::Element(element) = node else {
                continue;
            };
            let ordinal = ElementOrdinal::from_skeleton_index(skeleton.nodes.len());
            skeleton.nodes.push(SkeletonNode {
                role: StructureRole::classify(element),
                parent,
                facts: NodeFacts::of(element),
            });
            element.ordinal = ordinal;
            pending.extend(
                element
                    .children
                    .iter_mut()
                    .rev()
                    .map(|child| (ordinal, child)),
            );
        }
        skeleton
    }

    /// Every node in pre-order; index 0 is the `Document`.
    pub(crate) fn nodes(&self) -> &[SkeletonNode] {
        &self.nodes
    }

    /// The node assigned `ordinal`, if the skeleton assigned one.
    pub(crate) fn node(&self, ordinal: ElementOrdinal) -> Option<&SkeletonNode> {
        ordinal
            .skeleton_index()
            .and_then(|index| self.nodes.get(index))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }
}

impl NodeFacts {
    fn of(element: &ElementNode) -> Self {
        let attribute = |name: &str| {
            element
                .attributes
                .get(name)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Self {
            // `alt=""` is kept as the empty string: it means "decorative".
            alt: element.attributes.get("alt").cloned(),
            title: attribute("title"),
            aria_label: attribute("aria-label"),
            lang: attribute("lang").and_then(|lang| lang.parse().ok()),
            scope: attribute("scope").and_then(|scope| HeaderScope::parse(&scope)),
            col_span: span_attribute(element, "colspan", 1, 1000),
            row_span: span_attribute(element, "rowspan", 1, 65_534),
            id: attribute("id"),
            source: attribute("src"),
            formula_source: attribute("data-math"),
            svg_title: (element.tag == HtmlTag::Svg)
                .then(|| svg_title(element))
                .flatten(),
        }
    }
}

impl NodeFacts {
    /// The text alternative of a figure, in HTML accessible-name order:
    /// `alt`, then `aria-label`, `title`, and an inline SVG's `<title>`.
    pub(crate) fn accessible_name(&self) -> Option<&str> {
        [&self.alt, &self.aria_label, &self.title, &self.svg_title]
            .into_iter()
            .filter_map(|name| name.as_deref())
            .map(str::trim)
            .find(|name| !name.is_empty())
    }
}

/// HTML's rules for `colspan`/`rowspan`: a non-negative integer, otherwise
/// the default; zero and overflow are clamped to the allowed range.
fn span_attribute(element: &ElementNode, name: &str, default: u32, max: u32) -> u32 {
    element
        .attributes
        .get(name)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .map_or(default, |value| value.clamp(1, max))
}

fn svg_title(svg: &ElementNode) -> Option<String> {
    svg.children.iter().find_map(|child| match child {
        DomNode::Element(child) if child.raw_tag_name == "title" => {
            let text = child
                .children
                .iter()
                .filter_map(|node| match node {
                    DomNode::Text(text) => Some(text.as_str()),
                    DomNode::Element(_) => None,
                })
                .collect::<String>();
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::html::parse_html;

    fn ordinals(nodes: &[DomNode]) -> Vec<(String, u32)> {
        let mut out = Vec::new();
        let mut pending: Vec<&DomNode> = nodes.iter().rev().collect();
        while let Some(node) = pending.pop() {
            if let DomNode::Element(element) = node {
                out.push((element.raw_tag_name.clone(), element.ordinal.0));
                pending.extend(element.children.iter().rev());
            }
        }
        out
    }

    #[test]
    fn ordinals_follow_document_pre_order_and_parents_are_recorded() {
        let mut nodes =
            parse_html("<h1>T</h1><ul><li>a<em>b</em></li><li>c</li></ul><p>d</p>").unwrap();
        let skeleton = StructureSkeleton::assign(&mut nodes);

        assert_eq!(
            ordinals(&nodes),
            vec![
                ("h1".to_string(), 2),
                ("ul".to_string(), 3),
                ("li".to_string(), 4),
                ("em".to_string(), 5),
                ("li".to_string(), 6),
                ("p".to_string(), 7),
            ]
        );
        assert_eq!(skeleton.len(), 7);
        let em = skeleton.node(ElementOrdinal(5)).unwrap();
        assert_eq!(em.role, StructureRole::Transparent);
        assert_eq!(em.parent, ElementOrdinal(4));
        let li = skeleton.node(ElementOrdinal(4)).unwrap();
        assert_eq!(li.role, StructureRole::LI);
        assert_eq!(li.parent, ElementOrdinal(3));
        let ul = skeleton.node(ElementOrdinal(3)).unwrap();
        assert_eq!(ul.parent, ElementOrdinal(1));
        assert_eq!(
            skeleton.node(ElementOrdinal(1)).unwrap().role,
            StructureRole::Document
        );
        assert_eq!(skeleton.node(ElementOrdinal::ANONYMOUS), None);
        assert_eq!(skeleton.node(ElementOrdinal(99)), None);
    }

    #[test]
    fn facts_are_read_from_attributes_with_html_parsing_rules() {
        let mut nodes = parse_html(
            r#"<table><tr><th scope="col" colspan="2" rowspan="0">H</th>
               <td colspan="x" rowspan="70000" id="c1" lang="fr">x</td></tr></table>
               <img alt="" title=" Chart ">
               <svg><title> Sales </title><rect/></svg>
               <div data-math="x^2" aria-label="squared"></div>"#,
        )
        .unwrap();
        let skeleton = StructureSkeleton::assign(&mut nodes);
        let facts = |tag: &str| {
            let mut pending: Vec<&DomNode> = nodes.iter().rev().collect();
            while let Some(node) = pending.pop() {
                if let DomNode::Element(element) = node {
                    if element.raw_tag_name == tag {
                        return skeleton.node(element.ordinal).unwrap().facts.clone();
                    }
                    pending.extend(element.children.iter().rev());
                }
            }
            panic!("no <{tag}>")
        };

        let th = facts("th");
        assert_eq!(th.scope, Some(HeaderScope::Column));
        assert_eq!((th.col_span, th.row_span), (2, 1));

        let td = facts("td");
        assert_eq!((td.col_span, td.row_span), (1, 65_534));
        assert_eq!(td.id.as_deref(), Some("c1"));
        assert_eq!(td.lang.as_ref().map(|l| l.as_str()), Some("fr"));

        let img = facts("img");
        assert_eq!(img.alt.as_deref(), Some(""));
        assert_eq!(img.title.as_deref(), Some("Chart"));

        assert_eq!(facts("svg").svg_title.as_deref(), Some("Sales"));

        let formula = facts("div");
        assert_eq!(formula.formula_source.as_deref(), Some("x^2"));
        assert_eq!(formula.aria_label.as_deref(), Some("squared"));
    }

    #[test]
    fn deep_nesting_does_not_recurse() {
        let depth = 400;
        let html = format!("{}x{}", "<div>".repeat(depth), "</div>".repeat(depth));
        let mut nodes = parse_html(&html).unwrap();
        let skeleton = StructureSkeleton::assign(&mut nodes);
        assert_eq!(skeleton.len(), depth + 1);
    }
}
