use std::borrow::Cow;
use std::cell::{Ref, RefCell};
use std::collections::HashSet;
use std::sync::LazyLock;

use html5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, QualName, local_name, ns};

/// Index of a node in its [`Document`].
pub(in crate::parser) type NodeId = usize;

pub(in crate::parser) enum NodeData {
    Document,
    Doctype,
    Text(StrTendril),
    Comment,
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
        template_contents: Option<NodeId>,
        mathml_annotation_xml_integration_point: bool,
    },
    ProcessingInstruction,
}

pub(in crate::parser) struct Node {
    pub(in crate::parser) data: NodeData,
    parent: Option<NodeId>,
    pub(in crate::parser) children: Vec<NodeId>,
}

impl Node {
    fn new(data: NodeData) -> Self {
        Self {
            data,
            parent: None,
            children: Vec::new(),
        }
    }
}

/// The tree html5ever built. Template contents hang off their element and
/// never appear among its children.
pub(in crate::parser) struct Document {
    nodes: Vec<Node>,
}

impl Document {
    pub(in crate::parser) const ROOT: NodeId = 0;

    pub(in crate::parser) fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }
}

/// html5ever tree sink backed by a flat node arena.
static NO_NAME: LazyLock<QualName> = LazyLock::new(|| QualName::new(None, ns!(), local_name!("")));

pub(super) struct Sink {
    nodes: RefCell<Vec<Node>>,
}

impl Sink {
    pub(super) fn new() -> Self {
        Self {
            nodes: RefCell::new(vec![Node::new(NodeData::Document)]),
        }
    }
}

fn push(nodes: &mut Vec<Node>, data: NodeData) -> NodeId {
    nodes.push(Node::new(data));
    nodes.len() - 1
}

fn position(nodes: &[Node], parent: NodeId, child: NodeId) -> Option<usize> {
    nodes[parent]
        .children
        .iter()
        .position(|&candidate| candidate == child)
}

fn attach(nodes: &mut [Node], parent: NodeId, child: NodeId) {
    debug_assert!(nodes[child].parent.is_none());
    nodes[child].parent = Some(parent);
    nodes[parent].children.push(child);
}

fn detach(nodes: &mut [Node], child: NodeId) {
    if let Some(parent) = nodes[child].parent.take()
        && let Some(index) = position(nodes, parent, child)
    {
        nodes[parent].children.remove(index);
    }
}

fn append_to_existing_text(nodes: &mut [Node], id: NodeId, text: &str) -> bool {
    match &mut nodes[id].data {
        NodeData::Text(contents) => {
            contents.push_slice(text);
            true
        }
        _ => false,
    }
}

impl TreeSink for Sink {
    type Handle = NodeId;
    type Output = Document;
    type ElemName<'a>
        = Ref<'a, QualName>
    where
        Self: 'a;

    fn finish(self) -> Document {
        Document {
            nodes: self.nodes.into_inner(),
        }
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> NodeId {
        Document::ROOT
    }

    fn elem_name<'a>(&'a self, target: &'a NodeId) -> Ref<'a, QualName> {
        Ref::map(self.nodes.borrow(), |nodes| match &nodes[*target].data {
            NodeData::Element { name, .. } => name,
            _ => &NO_NAME,
        })
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> NodeId {
        let mut nodes = self.nodes.borrow_mut();
        let template_contents = flags.template.then(|| push(&mut nodes, NodeData::Document));
        push(
            &mut nodes,
            NodeData::Element {
                name,
                attrs,
                template_contents,
                mathml_annotation_xml_integration_point: flags
                    .mathml_annotation_xml_integration_point,
            },
        )
    }

    fn create_comment(&self, _text: StrTendril) -> NodeId {
        push(&mut self.nodes.borrow_mut(), NodeData::Comment)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> NodeId {
        push(
            &mut self.nodes.borrow_mut(),
            NodeData::ProcessingInstruction,
        )
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        let mut nodes = self.nodes.borrow_mut();
        let child = match child {
            NodeOrText::AppendText(text) => {
                if let Some(&last) = nodes[*parent].children.last()
                    && append_to_existing_text(&mut nodes, last, &text)
                {
                    return;
                }
                push(&mut nodes, NodeData::Text(text))
            }
            NodeOrText::AppendNode(node) => node,
        };
        attach(&mut nodes, *parent, child);
    }

    fn append_before_sibling(&self, sibling: &NodeId, child: NodeOrText<NodeId>) {
        let mut nodes = self.nodes.borrow_mut();
        let placed = nodes[*sibling]
            .parent
            .and_then(|parent| position(&nodes, parent, *sibling).map(|index| (parent, index)));
        let Some((parent, index)) = placed else {
            drop(nodes);
            self.append(&Document::ROOT, child);
            return;
        };
        let child = match child {
            NodeOrText::AppendText(text) => {
                if index > 0 {
                    let previous = nodes[parent].children[index - 1];
                    if append_to_existing_text(&mut nodes, previous, &text) {
                        return;
                    }
                }
                push(&mut nodes, NodeData::Text(text))
            }
            NodeOrText::AppendNode(node) => node,
        };
        detach(&mut nodes, child);
        nodes[child].parent = Some(parent);
        nodes[parent].children.insert(index, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &NodeId,
        prev_element: &NodeId,
        child: NodeOrText<NodeId>,
    ) {
        let has_parent = self.nodes.borrow()[*element].parent.is_some();
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev_element, child);
        }
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        let mut nodes = self.nodes.borrow_mut();
        let doctype = push(&mut nodes, NodeData::Doctype);
        attach(&mut nodes, Document::ROOT, doctype);
    }

    fn get_template_contents(&self, target: &NodeId) -> NodeId {
        match &self.nodes.borrow()[*target].data {
            NodeData::Element {
                template_contents: Some(contents),
                ..
            } => *contents,
            _ => *target,
        }
    }

    fn same_node(&self, x: &NodeId, y: &NodeId) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn add_attrs_if_missing(&self, target: &NodeId, new_attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        let NodeData::Element { attrs, .. } = &mut nodes[*target].data else {
            return;
        };
        let existing: HashSet<QualName> = attrs.iter().map(|attr| attr.name.clone()).collect();
        attrs.extend(
            new_attrs
                .into_iter()
                .filter(|attr| !existing.contains(&attr.name)),
        );
    }

    fn remove_from_parent(&self, target: &NodeId) {
        detach(&mut self.nodes.borrow_mut(), *target);
    }

    fn reparent_children(&self, node: &NodeId, new_parent: &NodeId) {
        let mut nodes = self.nodes.borrow_mut();
        let children = std::mem::take(&mut nodes[*node].children);
        for &child in &children {
            nodes[child].parent = Some(*new_parent);
        }
        nodes[*new_parent].children.extend(children);
    }

    fn is_mathml_annotation_xml_integration_point(&self, target: &NodeId) -> bool {
        matches!(
            self.nodes.borrow()[*target].data,
            NodeData::Element {
                mathml_annotation_xml_integration_point: true,
                ..
            }
        )
    }
}
