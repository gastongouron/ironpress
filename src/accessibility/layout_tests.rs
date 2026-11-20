//! Identity flowing from the DOM through layout into text runs.

use super::identity::ContentRole;
use super::{ElementOrdinal, ListNumbering, StructureRef, StructureSkeleton};
use crate::layout::elements::LayoutElementTestExt;
use crate::layout::engine::layout;
use crate::parser::dom::DomNode;
use crate::parser::html::parse_html;
use crate::{Margin, PageSize};

const HTML: &str = "<h1>Title</h1>\
    <p>See <a href=\"https://example.com\">link</a> and <span aria-hidden=\"true\">hidden</span>.</p>\
    <ul><li>item</li></ul>";

fn run_refs(nodes: &[DomNode]) -> Vec<(String, StructureRef)> {
    let pages = layout(nodes, PageSize::A4, Margin::default());
    pages
        .iter()
        .flat_map(|page| page.elements.iter())
        .filter_map(|(_, node)| {
            node.inspect_text(|block| {
                block
                    .lines
                    .iter()
                    .flat_map(|line| {
                        line.runs
                            .iter()
                            .map(|run| (run.text.clone(), run.metadata.structure))
                    })
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect()
}

fn body(element: u32) -> StructureRef {
    StructureRef::Content {
        element: ElementOrdinal(element),
        role: ContentRole::Body,
    }
}

#[test]
fn untagged_layout_carries_no_identity() {
    // Default mode never runs the skeleton, so every run stays `Untagged`,
    // including runs inside hidden spans and after the list marker.
    let nodes = parse_html(HTML).unwrap();
    let refs = run_refs(&nodes);
    assert!(!refs.is_empty());
    assert!(
        refs.iter()
            .all(|(_, structure)| *structure == StructureRef::Untagged),
        "{refs:?}"
    );
}

#[test]
fn tagged_layout_stamps_runs_with_their_element() {
    // Pre-order ordinals: h1 = 2, p = 3, a = 4, span = 5, ul = 6, li = 7.
    let mut nodes = parse_html(HTML).unwrap();
    let _skeleton = StructureSkeleton::assign(&mut nodes);
    let refs = run_refs(&nodes);
    let find = |text: &str| {
        refs.iter()
            .find(|(run, _)| run.contains(text))
            .map(|(_, structure)| *structure)
            .unwrap_or_else(|| panic!("no run containing {text:?} in {refs:?}"))
    };

    assert_eq!(find("Title"), body(2));
    assert_eq!(find("See"), body(3));
    assert_eq!(find("link"), body(4));
    assert_eq!(find("hidden"), StructureRef::Decorative);
    assert_eq!(find("item"), body(7));
    assert!(
        refs.iter().any(|(_, structure)| {
            *structure
                == StructureRef::Content {
                    element: ElementOrdinal(7),
                    role: ContentRole::Label(ListNumbering::Disc),
                }
        }),
        "the bullet must be the item's label: {refs:?}"
    );
}
