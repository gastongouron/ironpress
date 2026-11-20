//! Page-scoped values captured from generated-content features.
//!
//! CSS GCPM distinguishes the value entering a page, the first assignment on
//! that page, assignments made by the first element, and the value leaving the
//! page. Keeping those states together prevents renderers from independently
//! approximating `first`, `start`, `last`, and `first-except`.

use super::elements::LayoutNode;
use crate::parser::css::{PageContentPolicy, PageContentReference};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
struct PageAssignments<T> {
    entry: HashMap<String, T>,
    first: HashMap<String, T>,
    exit: HashMap<String, T>,
    assigned_at_start: HashSet<String>,
}

impl<T> Default for PageAssignments<T> {
    fn default() -> Self {
        Self {
            entry: HashMap::new(),
            first: HashMap::new(),
            exit: HashMap::new(),
            assigned_at_start: HashSet::new(),
        }
    }
}

impl<T: Clone> PageAssignments<T> {
    fn assign(&mut self, name: String, value: T, at_page_start: bool) {
        if !self.first.contains_key(&name) {
            if at_page_start {
                self.assigned_at_start.insert(name.clone());
            }
            self.first.insert(name.clone(), value.clone());
        }
        self.exit.insert(name, value);
    }

    fn advance_page(&mut self) {
        self.entry.clone_from(&self.exit);
        self.first.clear();
        self.assigned_at_start.clear();
    }

    fn resolve(&self, reference: &PageContentReference) -> Option<&T> {
        let name = reference.name();
        match reference.policy() {
            PageContentPolicy::First => self.first.get(name).or_else(|| self.entry.get(name)),
            PageContentPolicy::Start => {
                if self.assigned_at_start.contains(name) {
                    self.first.get(name)
                } else {
                    self.entry.get(name)
                }
            }
            PageContentPolicy::Last => self.exit.get(name),
            PageContentPolicy::FirstExcept => {
                (!self.first.contains_key(name)).then(|| self.entry.get(name))?
            }
        }
    }
}

/// An element with an `id` placed on one page: the target of `#id` links
/// and `target-counter()` references.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PageAnchor {
    pub(crate) id: String,
    /// Block offset of the box the anchor precedes, from the page area top.
    pub(crate) block_offset: f32,
}

/// Running elements, named strings and anchors active on one physical page.
///
/// Pagination is the sole writer. Rendering only asks this type to resolve a
/// parsed GCPM reference, so the two PDF passes cannot drift in policy handling.
#[derive(Debug, Clone, Default)]
pub(crate) struct PageGeneratedContent {
    running_elements: PageAssignments<LayoutNode>,
    named_strings: PageAssignments<String>,
    anchors: Vec<PageAnchor>,
}

impl PageGeneratedContent {
    pub(crate) fn capture_running(
        &mut self,
        name: String,
        element: LayoutNode,
        at_page_start: bool,
    ) {
        self.running_elements.assign(name, element, at_page_start);
    }

    pub(crate) fn capture_named_string(
        &mut self,
        name: String,
        value: String,
        at_page_start: bool,
    ) {
        self.named_strings.assign(name, value, at_page_start);
    }

    pub(crate) fn capture_anchor(&mut self, id: String, block_offset: f32) {
        self.anchors.push(PageAnchor { id, block_offset });
    }

    /// The anchors placed on this page, in document order.
    pub(crate) fn anchors(&self) -> &[PageAnchor] {
        &self.anchors
    }

    pub(crate) fn advance_page(&mut self) {
        self.running_elements.advance_page();
        self.named_strings.advance_page();
        self.anchors.clear();
    }

    pub(crate) fn snapshot_and_advance(&mut self) -> Self {
        let page = self.clone();
        self.advance_page();
        page
    }

    pub(crate) fn following_page(&self) -> Self {
        let mut following = self.clone();
        following.advance_page();
        following
    }

    pub(crate) fn running_element(
        &self,
        reference: &PageContentReference,
    ) -> Option<&dyn super::elements::LayoutElement> {
        self.running_elements
            .resolve(reference)
            .map(LayoutNode::as_ref)
    }

    pub(crate) fn running_elements(&self) -> impl Iterator<Item = &LayoutNode> {
        self.running_elements
            .entry
            .values()
            .chain(self.running_elements.first.values())
            .chain(self.running_elements.exit.values())
    }

    pub(crate) fn running_elements_mut(&mut self) -> impl Iterator<Item = &mut LayoutNode> {
        self.running_elements
            .entry
            .values_mut()
            .chain(self.running_elements.first.values_mut())
            .chain(self.running_elements.exit.values_mut())
    }

    pub(crate) fn named_string(&self, reference: &PageContentReference) -> Option<&str> {
        self.named_strings.resolve(reference).map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn named_string_exit(&self, name: &str) -> Option<&str> {
        self.named_strings.exit.get(name).map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn named_string_first(&self, name: &str) -> Option<&str> {
        self.named_strings.first.get(name).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(name: &str, policy: PageContentPolicy) -> PageContentReference {
        PageContentReference::new(name.to_string(), policy)
    }

    #[test]
    fn named_string_policies_distinguish_entry_first_start_and_exit() {
        let mut content = PageGeneratedContent::default();
        content.capture_named_string("section".into(), "A".into(), true);
        content.capture_named_string("section".into(), "B".into(), false);

        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::First)),
            Some("A")
        );
        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::Start)),
            Some("A")
        );
        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::Last)),
            Some("B")
        );

        content.advance_page();
        content.capture_named_string("section".into(), "C".into(), false);
        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::First)),
            Some("C")
        );
        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::Start)),
            Some("B")
        );
        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::Last)),
            Some("C")
        );
        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::FirstExcept)),
            None
        );
    }

    #[test]
    fn first_policy_falls_back_to_the_entry_value() {
        let mut content = PageGeneratedContent::default();
        content.capture_named_string("section".into(), "A".into(), false);
        content.advance_page();

        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::First)),
            Some("A")
        );
        assert_eq!(
            content.named_string(&reference("section", PageContentPolicy::FirstExcept)),
            Some("A")
        );
    }
}
