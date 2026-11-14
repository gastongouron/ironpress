//! Building the structure hierarchy from the skeleton and the recorded marks.
//!
//! Nesting comes from the skeleton; content comes from the marks each page
//! recorded. Elements without content are pruned, except the cells and rows
//! a table needs to stay regular.

use super::super::geometry::PdfRect;
use super::super::pdf_text::PdfTextString;
use super::recorder::{MarkKind, Tagging};
use super::tables::{CellFacts, regularize_tables};
use crate::accessibility::{
    AccessibilityError, ContentRole, HeadingLevel, ListNumbering, NodeFacts, SkeletonNode,
    StructureRef, StructureRole,
};

pub(super) struct StructureTree {
    elements: Vec<Element>,
    /// Per recorded page, the element that owns each MCID.
    page_entries: Vec<Vec<usize>>,
    /// Per recorded page, the `Link` element of each annotation, in order.
    annotation_elements: Vec<Vec<usize>>,
}

pub(super) struct Element {
    pub(super) name: String,
    heading: Option<HeadingLevel>,
    parent: Option<usize>,
    pub(super) children: Vec<usize>,
    marks: Vec<Mark>,
    annotations: Vec<Annotation>,
    /// Extra dictionary entries, each starting with a space.
    pub(super) attributes: String,
    pub(super) cell: Option<CellFacts>,
    /// For an `LI`: the numbering its label marks were painted with.
    numbering: Option<ListNumbering>,
}

#[derive(Debug, Clone, Copy)]
struct Mark {
    page: usize,
    mcid: usize,
    sequence: usize,
}

#[derive(Debug, Clone, Copy)]
struct Annotation {
    page: usize,
    object_id: usize,
}

impl Element {
    #[cfg(test)]
    pub(super) fn parent_index(&self) -> Option<usize> {
        self.parent
    }

    pub(super) fn new(name: &str, parent: Option<usize>) -> Self {
        Self {
            name: name.to_string(),
            heading: None,
            parent,
            children: Vec::new(),
            marks: Vec::new(),
            annotations: Vec::new(),
            attributes: String::new(),
            cell: None,
            numbering: None,
        }
    }
}

/// Where a recorded mark's content lands.
#[derive(Clone, Copy)]
struct Target {
    node: usize,
    label: bool,
}

impl StructureTree {
    pub(super) fn build(tagging: &Tagging) -> Result<Self, AccessibilityError> {
        let nodes = tagging.skeleton.nodes();
        let count = nodes.len();
        let parent_of = |index: usize| nodes[index].parent.skeleton_index().unwrap_or(0);

        // 1. Distribute the recorded marks and annotations over skeleton nodes.
        let mut body: Vec<Vec<Mark>> = vec![Vec::new(); count];
        let mut labels: Vec<Vec<Mark>> = vec![Vec::new(); count];
        let mut figure_boxes: Vec<Option<PdfRect>> = vec![None; count];
        let mut numbering: Vec<Option<ListNumbering>> = vec![None; count];
        let mut annotations: Vec<Vec<Annotation>> = vec![Vec::new(); count];
        let mut page_targets: Vec<Vec<Target>> = Vec::with_capacity(tagging.pages.len());
        let mut annotation_targets: Vec<Vec<usize>> = Vec::with_capacity(tagging.pages.len());
        let mut sequence = 0;
        for (page, record) in tagging.pages.iter().enumerate() {
            let mut targets = Vec::with_capacity(record.marks.len());
            for (mcid, mark) in record.marks.iter().enumerate() {
                let node = tagging
                    .emitting_element(mark.owner)
                    .skeleton_index()
                    .unwrap_or(0);
                let label = match mark.owner {
                    StructureRef::Content {
                        role: ContentRole::Label(list_numbering),
                        ..
                    } if nodes[node].role == StructureRole::LI => {
                        numbering[node].get_or_insert(list_numbering);
                        true
                    }
                    _ => false,
                };
                let recorded = Mark {
                    page,
                    mcid,
                    sequence,
                };
                sequence += 1;
                if label {
                    labels[node].push(recorded);
                } else {
                    body[node].push(recorded);
                }
                if let MarkKind::Figure { bbox } = mark.kind {
                    figure_boxes[node].get_or_insert(bbox);
                }
                targets.push(Target { node, label });
            }
            let mut owners = Vec::with_capacity(record.annotations.len());
            for annotation in &record.annotations {
                let node = tagging
                    .emitting_element(annotation.owner)
                    .skeleton_index()
                    .unwrap_or(0);
                annotations[node].push(Annotation {
                    page,
                    object_id: annotation.object_id,
                });
                owners.push(node);
            }
            page_targets.push(targets);
            annotation_targets.push(owners);
        }

        // 2. Keep nodes with content and their ancestors, plus the rows and
        //    cells a surviving table needs for a regular grid.
        let mut alive = vec![false; count];
        let mut has_alive_child = vec![false; count];
        alive[0] = true;
        for index in (1..count).rev() {
            if !body[index].is_empty()
                || !labels[index].is_empty()
                || !annotations[index].is_empty()
            {
                alive[index] = true;
            }
            if alive[index] {
                alive[parent_of(index)] = true;
                has_alive_child[parent_of(index)] = true;
            }
        }
        for index in 1..count {
            let parent = parent_of(index);
            if alive[index] || !alive[parent] {
                continue;
            }
            let revived = matches!(
                (nodes[parent].role, nodes[index].role),
                (
                    StructureRole::Table
                        | StructureRole::THead
                        | StructureRole::TBody
                        | StructureRole::TFoot,
                    StructureRole::TR
                ) | (StructureRole::TR, StructureRole::TH | StructureRole::TD)
            );
            if revived {
                alive[index] = true;
                has_alive_child[parent] = true;
            }
        }

        // 3. Create one element per surviving emitting node; list items get
        //    their Lbl and LBody.
        let mut tree = Self {
            elements: Vec::new(),
            page_entries: Vec::new(),
            annotation_elements: Vec::new(),
        };
        let mut own: Vec<Option<usize>> = vec![None; count];
        let mut attach: Vec<Option<usize>> = vec![None; count];
        let mut label_of: Vec<Option<usize>> = vec![None; count];
        let document = tree.push(Element::new("Document", None));
        tree.elements[document].attributes = format!(
            " /Lang {}",
            PdfTextString::from(tagging.metadata.language.as_str())
        );
        own[0] = Some(document);
        attach[0] = Some(document);
        for index in 1..count {
            if !alive[index] {
                continue;
            }
            let node = &nodes[index];
            let parent_attach = attach[parent_of(index)].unwrap_or(document);
            if !node.role.emits_element() {
                own[index] = Some(parent_attach);
                attach[index] = Some(parent_attach);
                continue;
            }
            let mut element = Element::new(node.role.pdf_name(), Some(parent_attach));
            element.heading = match node.role {
                StructureRole::Heading(level) => Some(level),
                _ => None,
            };
            element.attributes = node_attributes(node, figure_boxes[index])?;
            element.cell = cell_facts(node, nodes, parent_of(index));
            element.numbering = numbering[index];
            let element = tree.push(element);
            own[index] = Some(element);
            attach[index] = Some(element);
            if node.role == StructureRole::LI {
                if !labels[index].is_empty() {
                    label_of[index] = Some(tree.push(Element::new("Lbl", Some(element))));
                }
                if !body[index].is_empty() || has_alive_child[index] {
                    let body_element = tree.push(Element::new("LBody", Some(element)));
                    own[index] = Some(body_element);
                    attach[index] = Some(body_element);
                }
            }
        }

        // 4. Attach marks and annotations.
        for index in 0..count {
            let Some(owner) = own[index] else {
                continue;
            };
            tree.elements[owner]
                .marks
                .extend(body[index].iter().copied());
            let label_owner = label_of[index].unwrap_or(owner);
            tree.elements[label_owner]
                .marks
                .extend(labels[index].iter().copied());
        }
        let mut link_of: Vec<Option<usize>> = vec![None; count];
        for index in 0..count {
            if annotations[index].is_empty() {
                continue;
            }
            // ISO 14289-1 7.18.5: every link annotation is a kid of a Link.
            let link = match own[index] {
                Some(element) if tree.elements[element].name == "Link" => element,
                _ => {
                    let parent = attach[index].unwrap_or(document);
                    tree.push(Element::new("Link", Some(parent)))
                }
            };
            tree.elements[link]
                .annotations
                .extend(annotations[index].iter().copied());
            link_of[index] = Some(link);
        }
        tree.page_entries = page_targets
            .iter()
            .map(|targets| {
                targets
                    .iter()
                    .map(|target| {
                        let owner = own[target.node].unwrap_or(document);
                        if target.label {
                            label_of[target.node].unwrap_or(owner)
                        } else {
                            owner
                        }
                    })
                    .collect()
            })
            .collect();
        tree.annotation_elements = annotation_targets
            .iter()
            .map(|owners| {
                owners
                    .iter()
                    .map(|node| link_of[*node].unwrap_or(document))
                    .collect()
            })
            .collect();

        regularize_tables(&mut tree.elements);
        tree.attribute_lists();
        tree.normalize_headings(document);
        Ok(tree)
    }

    /// PDF/UA-1 7.2 (veraPDF 7.2-40): every `L` declares its numbering, taken
    /// from the first item whose marker was painted.
    fn attribute_lists(&mut self) {
        for index in 0..self.elements.len() {
            if self.elements[index].name != "L" {
                continue;
            }
            let numbering = self.elements[index]
                .children
                .iter()
                .find_map(|child| self.elements[*child].numbering)
                .unwrap_or_default();
            self.elements[index].attributes.push_str(&format!(
                " /A << /O /List /ListNumbering /{} >>",
                numbering.pdf_name()
            ));
        }
    }

    pub(super) fn push(&mut self, element: Element) -> usize {
        let index = self.elements.len();
        if let Some(parent) = element.parent {
            self.elements[parent].children.push(index);
        }
        self.elements.push(element);
        index
    }

    /// PDF/UA-1 7.4.2 (veraPDF 7.4.4-x): the first heading is H1 and no
    /// heading skips a level on the way down, in structure pre-order.
    fn normalize_headings(&mut self, root: usize) {
        let mut last = 0u8;
        let mut pending = vec![root];
        while let Some(index) = pending.pop() {
            if let Some(level) = self.elements[index].heading {
                let normalized = level.level().min(last + 1);
                last = normalized;
                self.elements[index].name = HeadingLevel::clamped(u32::from(normalized))
                    .pdf_name()
                    .to_string();
            }
            pending.extend(self.elements[index].children.iter().rev());
        }
    }

    /// Append the StructTreeRoot and every element; returns the root's id.
    pub(super) fn write(
        &self,
        objects: &mut Vec<String>,
        tagging: &Tagging,
        page_ids: &[usize],
    ) -> usize {
        let root_id = objects.len() + 1;
        let element_id = |index: usize| root_id + 1 + index;
        let page_object = |page: usize| page_ids.get(page).copied().unwrap_or(1);

        let mut first_sequence: Vec<Option<usize>> = vec![None; self.elements.len()];
        for index in (0..self.elements.len()).rev() {
            let own = self.elements[index]
                .marks
                .iter()
                .map(|mark| mark.sequence)
                .min();
            let children = self.elements[index]
                .children
                .iter()
                .filter_map(|child| first_sequence[*child])
                .min();
            first_sequence[index] = [own, children].into_iter().flatten().min();
        }

        let mut nums = String::new();
        for (page, record) in tagging.pages.iter().enumerate() {
            let entries = self.page_entries[page]
                .iter()
                .map(|element| format!("{} 0 R", element_id(*element)))
                .collect::<Vec<_>>()
                .join(" ");
            nums.push_str(&format!("{} [{entries}] ", record.key));
            for (annotation, element) in record
                .annotations
                .iter()
                .zip(&self.annotation_elements[page])
            {
                nums.push_str(&format!("{} {} 0 R ", annotation.key, element_id(*element)));
            }
        }
        objects.push(format!(
            "{root_id} 0 obj\n<< /Type /StructTreeRoot /K [{} 0 R] /ParentTree << /Nums [{}] >> /ParentTreeNextKey {} >>\nendobj",
            element_id(0),
            nums.trim_end(),
            tagging.next_parent_key(),
        ));

        for (index, element) in self.elements.iter().enumerate() {
            let parent_id = element.parent.map_or(root_id, element_id);
            let mut marks = element.marks.clone();
            marks.sort_by_key(|mark| mark.sequence);
            let page = marks.first().map(|mark| mark.page);
            let mark_kid = |mark: &Mark| {
                if Some(mark.page) == page {
                    mark.mcid.to_string()
                } else {
                    format!(
                        "<< /Type /MCR /Pg {} 0 R /MCID {} >>",
                        page_object(mark.page),
                        mark.mcid
                    )
                }
            };
            let mut kids = Vec::new();
            let mut next_mark = 0;
            for child in &element.children {
                if let Some(child_sequence) = first_sequence[*child] {
                    while marks
                        .get(next_mark)
                        .is_some_and(|mark| mark.sequence < child_sequence)
                    {
                        kids.push(mark_kid(&marks[next_mark]));
                        next_mark += 1;
                    }
                }
                kids.push(format!("{} 0 R", element_id(*child)));
            }
            kids.extend(marks[next_mark..].iter().map(mark_kid));
            kids.extend(element.annotations.iter().map(|annotation| {
                format!(
                    "<< /Type /OBJR /Obj {} 0 R /Pg {} 0 R >>",
                    annotation.object_id,
                    page_object(annotation.page)
                )
            }));
            let pg = page.map_or_else(String::new, |page| {
                format!(" /Pg {} 0 R", page_object(page))
            });
            let kids = if kids.is_empty() {
                String::new()
            } else {
                format!(" /K [{}]", kids.join(" "))
            };
            objects.push(format!(
                "{} 0 obj\n<< /Type /StructElem /S /{} /P {parent_id} 0 R{pg}{}{kids} >>\nendobj",
                element_id(index),
                element.name,
                element.attributes,
            ));
        }
        root_id
    }
}

/// `/Lang`, `/Alt` and the Figure bounding box of one element.
fn node_attributes(
    node: &SkeletonNode,
    figure_box: Option<PdfRect>,
) -> Result<String, AccessibilityError> {
    let mut attributes = String::new();
    if let Some(lang) = &node.facts.lang {
        attributes.push_str(&format!(" /Lang {}", PdfTextString::from(lang.as_str())));
    }
    match node.role {
        StructureRole::Figure => {
            let name =
                node.facts
                    .accessible_name()
                    .ok_or_else(|| AccessibilityError::MissingAltText {
                        element: describe_figure(&node.facts),
                    })?;
            attributes.push_str(&format!(" /Alt {}", PdfTextString::from(name)));
            if let Some(bbox) = figure_box {
                attributes.push_str(&format!(
                    " /A << /O /Layout /BBox [{} {} {} {}] >>",
                    crate::render::pdf_syntax::format_pdf_number(bbox.left),
                    crate::render::pdf_syntax::format_pdf_number(bbox.bottom),
                    crate::render::pdf_syntax::format_pdf_number(bbox.right()),
                    crate::render::pdf_syntax::format_pdf_number(bbox.top()),
                ));
            }
        }
        StructureRole::Formula => {
            let name = node
                .facts
                .formula_source
                .as_deref()
                .or(node.facts.accessible_name())
                .ok_or_else(|| AccessibilityError::MissingAltText {
                    element: "a formula without source text".to_string(),
                })?;
            attributes.push_str(&format!(" /Alt {}", PdfTextString::from(name)));
        }
        _ => {}
    }
    Ok(attributes)
}

fn describe_figure(facts: &NodeFacts) -> String {
    match &facts.source {
        Some(source) => format!("<img src=\"{source}\">"),
        None => "an <img> without src, or an inline <svg> without <title>".to_string(),
    }
}

fn cell_facts(node: &SkeletonNode, nodes: &[SkeletonNode], parent: usize) -> Option<CellFacts> {
    let header = match node.role {
        StructureRole::TH => true,
        StructureRole::TD => false,
        _ => return None,
    };
    let in_head = nodes[parent]
        .parent
        .skeleton_index()
        .is_some_and(|group| nodes[group].role == StructureRole::THead);
    Some(CellFacts {
        header,
        in_head,
        col_span: node.facts.col_span,
        row_span: node.facts.row_span,
        scope: node.facts.scope,
    })
}
