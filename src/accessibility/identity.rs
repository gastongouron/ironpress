//! Element identity that travels through layout as a `Copy` value.
//!
//! The layout tree is flat, so structure cannot be reconstructed from it.
//! Instead every computed style, text run and paint group remembers *which
//! element* it came from; the structure skeleton turns that identity back
//! into nesting when the PDF is written.

use super::role::StructureRole;
use crate::parser::dom::ElementNode;
use crate::style::computed::{ComputedStyle, ListStyleType};

/// Position of an element in document pre-order, as assigned by the
/// structure skeleton. Elements the skeleton never saw (synthetic boxes,
/// fragments parsed on their own, every element in untagged mode) are
/// anonymous.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ElementOrdinal(pub(super) u32);

impl ElementOrdinal {
    /// No skeleton node; the element contributes to its nearest ancestor.
    pub(crate) const ANONYMOUS: Self = Self(0);

    pub(crate) fn is_anonymous(self) -> bool {
        self == Self::ANONYMOUS
    }

    /// Index of the skeleton node for this ordinal; `None` when anonymous.
    pub(crate) fn skeleton_index(self) -> Option<usize> {
        (self.0 as usize).checked_sub(1)
    }

    /// The ordinal of the skeleton node stored at `index`.
    pub(crate) fn from_skeleton_index(index: usize) -> Self {
        u32::try_from(index + 1).map_or(Self::ANONYMOUS, Self)
    }
}

/// What painted content belongs to, from the structure tree's point of view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StructureRef {
    /// No element is known. In tagged output such content belongs to the
    /// `Document` element; in untagged output every ref has this value.
    #[default]
    Untagged,
    /// Hidden from assistive technology (`aria-hidden`, `alt=""`): painted
    /// as an artifact.
    Decorative,
    /// Content of a skeleton element.
    Content {
        element: ElementOrdinal,
        role: ContentRole,
    },
}

impl StructureRef {
    /// The ref for content of `element`, given the ref its parent passed down.
    ///
    /// The anonymous test comes first: without a skeleton (untagged output)
    /// every element inherits `Untagged`, so run coalescing is unchanged.
    /// `role=presentation` is transparent rather than decorative because
    /// WAI-ARIA removes only the element's own semantics, not its content.
    pub(crate) fn for_element(element: &ElementNode, inherited: Self) -> Self {
        if element.ordinal.is_anonymous() || inherited == Self::Decorative {
            return inherited;
        }
        match StructureRole::classify(element) {
            StructureRole::Decorative => Self::Decorative,
            StructureRole::Transparent => inherited,
            _ => Self::Content {
                element: element.ordinal,
                role: ContentRole::Body,
            },
        }
    }

    /// The same element, but as its list-item label (`Lbl`).
    pub(crate) fn as_label(self, numbering: ListNumbering) -> Self {
        match self {
            Self::Content { element, .. } => Self::Content {
                element,
                role: ContentRole::Label(numbering),
            },
            other => other,
        }
    }
}

/// Which part of an element a piece of content is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ContentRole {
    /// Ordinary content.
    #[default]
    Body,
    /// A list item's marker.
    Label(ListNumbering),
}

/// The `/ListNumbering` values of ISO 32000-1 Table 347.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ListNumbering {
    #[default]
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    UpperRoman,
    LowerRoman,
    UpperAlpha,
    LowerAlpha,
}

impl From<&ListStyleType> for ListNumbering {
    fn from(list_style_type: &ListStyleType) -> Self {
        match list_style_type {
            ListStyleType::Disc => Self::Disc,
            ListStyleType::Circle => Self::Circle,
            ListStyleType::Square => Self::Square,
            ListStyleType::Decimal | ListStyleType::DecimalLeadingZero => Self::Decimal,
            ListStyleType::LowerAlpha => Self::LowerAlpha,
            ListStyleType::UpperAlpha => Self::UpperAlpha,
            ListStyleType::LowerRoman => Self::LowerRoman,
            ListStyleType::UpperRoman => Self::UpperRoman,
            ListStyleType::CjkDecimal
            | ListStyleType::String(_)
            | ListStyleType::Custom(_)
            | ListStyleType::CounterStyle(_)
            | ListStyleType::None => Self::None,
        }
    }
}

impl ComputedStyle {
    /// Stamp the identity of the element this style was computed for.
    ///
    /// `structure` already holds the parent's ref, because computed styles
    /// start as a clone of the parent and the field inherits deliberately: a
    /// construct that never stamps attributes its text to the nearest known
    /// ancestor instead of orphaning it.
    pub(crate) fn for_element(mut self, element: &ElementNode) -> Self {
        self.structure = StructureRef::for_element(element, self.structure);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::dom::HtmlTag;

    fn element(tag: HtmlTag, attributes: &[(&str, &str)], ordinal: u32) -> ElementNode {
        let mut element = ElementNode::new(tag);
        for (name, value) in attributes {
            element
                .attributes
                .insert((*name).to_string(), (*value).to_string());
        }
        element.ordinal = ElementOrdinal(ordinal);
        element
    }

    #[test]
    fn anonymous_elements_inherit_whatever_the_parent_had() {
        // Default mode: no skeleton ran, so even hidden content keeps the
        // parent's `Untagged` ref and run coalescing is unaffected.
        let hidden = element(HtmlTag::Span, &[("aria-hidden", "true")], 0);
        assert_eq!(
            StructureRef::for_element(&hidden, StructureRef::Untagged),
            StructureRef::Untagged
        );
        let decorative_image = element(HtmlTag::Img, &[("alt", "")], 0);
        assert_eq!(
            StructureRef::for_element(&decorative_image, StructureRef::Untagged),
            StructureRef::Untagged
        );
    }

    #[test]
    fn hidden_content_is_decorative_and_stays_hidden_below() {
        // ISO 14289-1 7.1: content not meaningful to the reader is an artifact.
        let hidden = element(HtmlTag::Span, &[("aria-hidden", "true")], 7);
        assert_eq!(
            StructureRef::for_element(&hidden, StructureRef::Untagged),
            StructureRef::Decorative
        );
        let link_inside = element(HtmlTag::A, &[("href", "https://example.com")], 8);
        assert_eq!(
            StructureRef::for_element(&link_inside, StructureRef::Decorative),
            StructureRef::Decorative
        );
    }

    #[test]
    fn presentation_role_and_inline_formatting_are_transparent() {
        let parent = StructureRef::Content {
            element: ElementOrdinal(3),
            role: ContentRole::Body,
        };
        let layout_table = element(HtmlTag::Table, &[("role", "presentation")], 4);
        assert_eq!(StructureRef::for_element(&layout_table, parent), parent);
        let emphasis = element(HtmlTag::Em, &[], 5);
        assert_eq!(StructureRef::for_element(&emphasis, parent), parent);
    }

    #[test]
    fn structural_elements_get_their_own_identity() {
        let parent = StructureRef::Content {
            element: ElementOrdinal(3),
            role: ContentRole::Body,
        };
        let link = element(HtmlTag::A, &[("href", "#x")], 9);
        assert_eq!(
            StructureRef::for_element(&link, parent),
            StructureRef::Content {
                element: ElementOrdinal(9),
                role: ContentRole::Body,
            }
        );
        assert_eq!(
            StructureRef::for_element(&link, parent).as_label(ListNumbering::Disc),
            StructureRef::Content {
                element: ElementOrdinal(9),
                role: ContentRole::Label(ListNumbering::Disc),
            }
        );
        assert_eq!(
            StructureRef::Untagged.as_label(ListNumbering::Disc),
            StructureRef::Untagged
        );
    }

    #[test]
    fn list_numbering_is_restricted_to_the_iso_32000_values() {
        // ISO 32000-1 Table 347 lists exactly nine values; CSS styles outside
        // that set map to None.
        assert_eq!(
            ListNumbering::from(&ListStyleType::DecimalLeadingZero),
            ListNumbering::Decimal
        );
        assert_eq!(
            ListNumbering::from(&ListStyleType::UpperRoman),
            ListNumbering::UpperRoman
        );
        assert_eq!(
            ListNumbering::from(&ListStyleType::CjkDecimal),
            ListNumbering::None
        );
        assert_eq!(
            ListNumbering::from(&ListStyleType::String("→".to_string())),
            ListNumbering::None
        );
    }
}
