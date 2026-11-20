//! Grouping of definition-list terms and descriptions into list items.

use super::tree::{Element, StructureTree};
use crate::accessibility::StructureRole;
use std::collections::HashMap;

/// The item of a definition list that is still taking terms and
/// descriptions.
#[derive(Debug, Clone, Copy)]
struct OpenItem {
    element: usize,
    last: StructureRole,
}

/// The open `LI` of every definition list being built, keyed by the list's
/// skeleton node, so that a term and the descriptions after it share one
/// item: ISO 32000-1 Table 335 lets an `LI` hold one or more `Lbl` and
/// `LBody` elements.
#[derive(Debug, Default)]
pub(super) struct DefinitionItems {
    open: HashMap<usize, OpenItem>,
}

impl DefinitionItems {
    /// The item a term (`Lbl`) or description (`LBody`) of `list` belongs
    /// to. A term after a description starts a new item; a term after a
    /// term, or a description, continues the open one.
    pub(super) fn item_for(
        &mut self,
        tree: &mut StructureTree,
        list: usize,
        list_element: usize,
        role: StructureRole,
    ) -> usize {
        let open = self
            .open
            .get(&list)
            .filter(|open| !(role == StructureRole::Lbl && open.last == StructureRole::LBody));
        let element = match open {
            Some(open) => open.element,
            None => tree.push(Element::new("LI", Some(list_element))),
        };
        self.open.insert(
            list,
            OpenItem {
                element,
                last: role,
            },
        );
        element
    }
}
