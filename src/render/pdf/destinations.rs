//! Named destinations for links into the document (ISO 32000-1 12.3.2.3).

use super::pdf_text::escape_pdf_string;
use crate::render::pdf_syntax::format_pdf_number;
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Where a link annotation leads (ISO 32000-1 12.5.6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LinkTarget {
    /// A URI action.
    Uri(String),
    /// The element of this document with this `id`.
    Anchor(String),
}

impl LinkTarget {
    pub(super) fn from_href(href: &str) -> Self {
        match href.strip_prefix('#') {
            Some(id) if !id.is_empty() => Self::Anchor(id.to_string()),
            _ => Self::Uri(href.to_string()),
        }
    }

    /// The authored reference, the last resort of a link description.
    pub(super) fn href(&self) -> Cow<'_, str> {
        match self {
            Self::Uri(href) => Cow::Borrowed(href),
            Self::Anchor(id) => Cow::Owned(format!("#{id}")),
        }
    }

    fn uri_action(uri: &str) -> String {
        format!(
            "/A << /Type /Action /S /URI /URI ({}) >>",
            escape_pdf_string(uri)
        )
    }
}

/// Where an anchor was painted.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Location {
    page_index: usize,
    /// Top edge in PDF page coordinates.
    top: f32,
}

/// The `#id` targets of a document and where each was painted, keyed in
/// byte order as the `/Dests` name tree requires (ISO 32000-1 7.9.6).
///
/// Historical output resolves nothing: an `#id` link keeps its relative URI
/// action, so the default output is unchanged. Accessible output resolves an
/// `#id` link to the anchor's page and drops a link to no element at all,
/// since a `Link` whose annotation leads nowhere is a dead link for
/// assistive technology (ISO 14289-1 7.18.5).
#[derive(Debug, Default)]
pub(crate) struct NamedDestinations {
    anchors: Option<BTreeMap<String, Option<Location>>>,
}

impl NamedDestinations {
    /// Resolve `#id` links against these anchors.
    pub(super) fn resolving<'a>(ids: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            anchors: Some(ids.into_iter().map(|id| (id.to_string(), None)).collect()),
        }
    }

    /// Record where an anchor was painted; the first instance of an id wins.
    pub(super) fn locate(&mut self, id: &str, page_index: usize, top: f32) {
        let Some(slot) = self
            .anchors
            .as_mut()
            .and_then(|anchors| anchors.get_mut(id))
        else {
            return;
        };
        if slot.is_none() {
            *slot = Some(Location { page_index, top });
        }
    }

    /// Whether an annotation with this target is written at all.
    pub(super) fn keeps(&self, target: &LinkTarget) -> bool {
        match (&self.anchors, target) {
            (Some(anchors), LinkTarget::Anchor(id)) => anchors.contains_key(id),
            _ => true,
        }
    }

    /// The action or destination entry of an annotation with this target.
    pub(super) fn annotation_entry(&self, target: &LinkTarget) -> String {
        match (&self.anchors, target) {
            (Some(_), LinkTarget::Anchor(id)) => format!("/Dest ({})", escape_pdf_string(id)),
            (_, target) => LinkTarget::uri_action(&target.href()),
        }
    }

    /// Append the `/Dests` name tree and return the catalog's `/Names` entry,
    /// empty when no anchor was located.
    pub(super) fn append_to(&self, objects: &mut Vec<String>, page_ids: &[usize]) -> String {
        let entries: Vec<String> = self
            .anchors
            .iter()
            .flatten()
            .filter_map(|(id, location)| {
                let location = (*location)?;
                let page_id = page_ids.get(location.page_index)?;
                Some(format!(
                    "({}) [{page_id} 0 R /XYZ null {} null]",
                    escape_pdf_string(id),
                    format_pdf_number(location.top)
                ))
            })
            .collect();
        if entries.is_empty() {
            return String::new();
        }
        let tree_id = objects.len() + 1;
        objects.push(format!(
            "{tree_id} 0 obj\n<< /Names [{}] >>\nendobj",
            entries.join(" ")
        ));
        format!(" /Names << /Dests {tree_id} 0 R >>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ISO 32000-1 12.5.6.5: a bare fragment names an element of this
    /// document; anything else is a URI.
    #[test]
    fn bare_fragments_are_anchors() {
        assert_eq!(
            LinkTarget::from_href("#end"),
            LinkTarget::Anchor("end".to_string())
        );
        assert_eq!(LinkTarget::from_href("#end").href(), "#end");
        assert_eq!(LinkTarget::from_href("#"), LinkTarget::Uri("#".to_string()));
        assert_eq!(
            LinkTarget::from_href("https://x/#f"),
            LinkTarget::Uri("https://x/#f".to_string())
        );
    }

    /// Without destinations every link keeps its URI action, so historical
    /// output is unchanged.
    #[test]
    fn historical_output_keeps_relative_uri_actions() {
        let destinations = NamedDestinations::default();
        let anchor = LinkTarget::from_href("#end");
        assert!(destinations.keeps(&anchor));
        assert_eq!(
            destinations.annotation_entry(&anchor),
            "/A << /Type /Action /S /URI /URI (#end) >>"
        );
        assert_eq!(destinations.append_to(&mut Vec::new(), &[3]), "");
    }

    /// ISO 32000-1 12.3.2.3 and 7.9.6: anchors resolve through a `/Dests`
    /// name tree whose keys are in byte order; the first instance of an id
    /// wins, and a link to no element is dropped (ISO 14289-1 7.18.5).
    #[test]
    fn located_anchors_form_a_sorted_name_tree() {
        let mut destinations = NamedDestinations::resolving(["top", "end"]);
        destinations.locate("end", 1, 700.0);
        destinations.locate("end", 1, 100.0);
        destinations.locate("top", 0, 769.89);
        destinations.locate("other", 0, 1.0);
        assert!(destinations.keeps(&LinkTarget::from_href("#end")));
        assert!(!destinations.keeps(&LinkTarget::from_href("#missing")));
        assert!(destinations.keeps(&LinkTarget::from_href("https://x")));
        assert_eq!(
            destinations.annotation_entry(&LinkTarget::from_href("#end")),
            "/Dest (end)"
        );
        assert_eq!(
            destinations.annotation_entry(&LinkTarget::from_href("https://x")),
            "/A << /Type /Action /S /URI /URI (https://x) >>"
        );
        let mut objects = vec!["1 0 obj\n<< >>\nendobj".to_string()];
        let entry = destinations.append_to(&mut objects, &[5, 7]);
        assert_eq!(entry, " /Names << /Dests 2 0 R >>");
        assert_eq!(
            objects[1],
            "2 0 obj\n<< /Names [(end) [7 0 R /XYZ null 700 null] (top) [5 0 R /XYZ null 769.89 null]] >>\nendobj"
        );
    }
}
