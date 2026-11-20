//! Mapping from HTML elements to PDF structure types.
//!
//! The vocabulary is the intersection of PDF/UA-1 and PDF/UA-2 standard
//! structure types (ISO 32000-1 Tables 333 to 338), so a later move to
//! PDF/UA-2 stays additive. `H` (the unnumbered heading) is never used.

use crate::parser::dom::{ElementNode, HtmlTag};

/// A standard structure type, or the two non-emitting outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StructureRole {
    Document,
    Part,
    Art,
    Sect,
    Div,
    BlockQuote,
    P,
    Heading(HeadingLevel),
    L,
    LI,
    /// A definition term (`dt`); the tree builder groups it into an `LI`.
    Lbl,
    /// A definition description (`dd`); grouped into an `LI` with its term.
    LBody,
    Table,
    THead,
    TBody,
    TFoot,
    TR,
    TH,
    TD,
    Caption,
    Figure,
    Formula,
    Link,
    Span,
    /// Contributes no element; its content belongs to the nearest ancestor.
    Transparent,
    /// Hidden from assistive technology together with its descendants.
    Decorative,
}

/// A heading level in `1..=6`, the range PDF/UA-1 can express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HeadingLevel(u8);

impl HeadingLevel {
    /// Clamp an authored level (HTML `h1`–`h6`, ARIA `aria-level`) into range.
    pub(crate) fn clamped(level: u32) -> Self {
        Self(level.clamp(1, 6) as u8)
    }

    pub(crate) fn level(self) -> u8 {
        self.0
    }

    /// `H1` through `H6`; the unnumbered `H` is never used (PDF/UA-1 7.4.2).
    pub(crate) fn pdf_name(self) -> &'static str {
        match self.0 {
            1 => "H1",
            2 => "H2",
            3 => "H3",
            4 => "H4",
            5 => "H5",
            _ => "H6",
        }
    }
}

impl StructureRole {
    /// Whether content of this role gets a structure element of its own.
    pub(crate) fn emits_element(self) -> bool {
        !matches!(self, Self::Transparent | Self::Decorative)
    }

    /// The standard structure type name (ISO 32000-1 Tables 333 to 338).
    pub(crate) fn pdf_name(self) -> &'static str {
        match self {
            Self::Document => "Document",
            Self::Part => "Part",
            Self::Art => "Art",
            Self::Sect => "Sect",
            Self::Div => "Div",
            Self::BlockQuote => "BlockQuote",
            Self::P => "P",
            Self::Heading(level) => level.pdf_name(),
            Self::L => "L",
            Self::LI => "LI",
            Self::Lbl => "Lbl",
            Self::LBody => "LBody",
            Self::Table => "Table",
            Self::THead => "THead",
            Self::TBody => "TBody",
            Self::TFoot => "TFoot",
            Self::TR => "TR",
            Self::TH => "TH",
            Self::TD => "TD",
            Self::Caption => "Caption",
            Self::Figure => "Figure",
            Self::Formula => "Formula",
            Self::Link => "Link",
            Self::Span | Self::Transparent | Self::Decorative => "Span",
        }
    }

    /// Classify an element by its ARIA attributes first, then by its tag.
    pub(crate) fn classify(element: &ElementNode) -> Self {
        let attribute = |name: &str| element.attributes.get(name).map(|value| value.trim());
        if attribute("aria-hidden") == Some("true") {
            return Self::Decorative;
        }
        if element.tag == HtmlTag::Img && attribute("alt") == Some("") {
            return Self::Decorative;
        }
        if element.attributes.contains_key("data-math") {
            return Self::Formula;
        }
        if let Some(role) = attribute("role") {
            match role.to_ascii_lowercase().as_str() {
                "presentation" | "none" => return Self::Transparent,
                "heading" => {
                    // WAI-ARIA 1.2: aria-level defaults to 2 for role=heading.
                    let level = attribute("aria-level")
                        .and_then(|level| level.parse().ok())
                        .unwrap_or(2);
                    return Self::Heading(HeadingLevel::clamped(level));
                }
                "list" => return Self::L,
                "listitem" => return Self::LI,
                "img" => return Self::Figure,
                _ => {}
            }
        }
        match element.tag {
            HtmlTag::H1 => Self::Heading(HeadingLevel(1)),
            HtmlTag::H2 => Self::Heading(HeadingLevel(2)),
            HtmlTag::H3 => Self::Heading(HeadingLevel(3)),
            HtmlTag::H4 => Self::Heading(HeadingLevel(4)),
            HtmlTag::H5 => Self::Heading(HeadingLevel(5)),
            HtmlTag::H6 => Self::Heading(HeadingLevel(6)),
            HtmlTag::P
            | HtmlTag::Pre
            | HtmlTag::Input
            | HtmlTag::Select
            | HtmlTag::Textarea
            | HtmlTag::Video
            | HtmlTag::Audio
            | HtmlTag::Progress
            | HtmlTag::Meter => Self::P,
            HtmlTag::Blockquote => Self::BlockQuote,
            HtmlTag::Section => Self::Sect,
            HtmlTag::Article => Self::Art,
            HtmlTag::Main => Self::Part,
            HtmlTag::Div
            | HtmlTag::Nav
            | HtmlTag::Aside
            | HtmlTag::Header
            | HtmlTag::Footer
            | HtmlTag::Figure
            | HtmlTag::Details
            | HtmlTag::Summary
            | HtmlTag::Address => Self::Div,
            HtmlTag::Ul | HtmlTag::Ol | HtmlTag::Dl => Self::L,
            HtmlTag::Li => Self::LI,
            // ISO 32000-1 Table 335: a term and its descriptions form one LI
            // of Lbl and LBody elements; the tree builder groups them.
            HtmlTag::Dt => Self::Lbl,
            HtmlTag::Dd => Self::LBody,
            HtmlTag::Table => Self::Table,
            HtmlTag::Thead => Self::THead,
            HtmlTag::Tbody => Self::TBody,
            HtmlTag::Tfoot => Self::TFoot,
            HtmlTag::Tr => Self::TR,
            HtmlTag::Th => Self::TH,
            HtmlTag::Td => Self::TD,
            HtmlTag::Caption | HtmlTag::Figcaption => Self::Caption,
            HtmlTag::Img | HtmlTag::Svg => Self::Figure,
            HtmlTag::A if attribute("href").is_some_and(|href| !href.is_empty()) => Self::Link,
            // ISO 32000-1 14.9.5: the expansion lives in /E on a structure
            // element, so an abbreviation with a title gets its own Span.
            HtmlTag::Abbr if attribute("title").is_some_and(|title| !title.is_empty()) => {
                Self::Span
            }
            _ if attribute("lang").is_some_and(|lang| !lang.is_empty()) => Self::Span,
            _ => Self::Transparent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(tag: HtmlTag, attributes: &[(&str, &str)]) -> ElementNode {
        let mut element = ElementNode::new(tag);
        for (name, value) in attributes {
            element
                .attributes
                .insert((*name).to_string(), (*value).to_string());
        }
        element
    }

    #[test]
    fn tags_map_to_the_ua1_ua2_shared_vocabulary() {
        // ISO 32000-1 Tables 334 (grouping), 335 (block), 336 (inline) and
        // 338 (illustration), as used by PDF/UA-1 7.1 to 7.9.
        let cases: &[(HtmlTag, StructureRole)] = &[
            (HtmlTag::H3, StructureRole::Heading(HeadingLevel(3))),
            (HtmlTag::P, StructureRole::P),
            (HtmlTag::Pre, StructureRole::P),
            (HtmlTag::Blockquote, StructureRole::BlockQuote),
            (HtmlTag::Section, StructureRole::Sect),
            (HtmlTag::Article, StructureRole::Art),
            (HtmlTag::Main, StructureRole::Part),
            (HtmlTag::Div, StructureRole::Div),
            (HtmlTag::Figure, StructureRole::Div),
            (HtmlTag::Ul, StructureRole::L),
            (HtmlTag::Ol, StructureRole::L),
            (HtmlTag::Li, StructureRole::LI),
            (HtmlTag::Dl, StructureRole::L),
            (HtmlTag::Dt, StructureRole::Lbl),
            (HtmlTag::Dd, StructureRole::LBody),
            (HtmlTag::Table, StructureRole::Table),
            (HtmlTag::Thead, StructureRole::THead),
            (HtmlTag::Tbody, StructureRole::TBody),
            (HtmlTag::Tfoot, StructureRole::TFoot),
            (HtmlTag::Tr, StructureRole::TR),
            (HtmlTag::Th, StructureRole::TH),
            (HtmlTag::Td, StructureRole::TD),
            (HtmlTag::Caption, StructureRole::Caption),
            (HtmlTag::Figcaption, StructureRole::Caption),
            (HtmlTag::Img, StructureRole::Figure),
            (HtmlTag::Svg, StructureRole::Figure),
            (HtmlTag::Span, StructureRole::Transparent),
            (HtmlTag::Strong, StructureRole::Transparent),
            (HtmlTag::Hr, StructureRole::Transparent),
            (HtmlTag::Br, StructureRole::Transparent),
            (HtmlTag::Unknown, StructureRole::Transparent),
        ];
        for (tag, role) in cases {
            assert_eq!(
                StructureRole::classify(&element(*tag, &[])),
                *role,
                "{tag:?}"
            );
        }
    }

    #[test]
    fn links_need_an_href_and_language_changes_become_spans() {
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::A, &[("href", "https://x")])),
            StructureRole::Link
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::A, &[("name", "anchor")])),
            StructureRole::Transparent
        );
        // ISO 14289-1 7.2: language changes are declared on a structure element.
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Span, &[("lang", "fr")])),
            StructureRole::Span
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::P, &[("lang", "fr")])),
            StructureRole::P
        );
        // ISO 32000-1 14.9.5: an expansion needs an element to carry /E; an
        // abbreviation without one has nothing to say.
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Abbr, &[("title", "PDF")])),
            StructureRole::Span
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Abbr, &[("title", " ")])),
            StructureRole::Transparent
        );
    }

    #[test]
    fn aria_and_alt_override_the_tag() {
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Div, &[("aria-hidden", "true")])),
            StructureRole::Decorative
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Img, &[("alt", " ")])),
            StructureRole::Decorative
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Img, &[("alt", "Chart")])),
            StructureRole::Figure
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Table, &[("role", "presentation")])),
            StructureRole::Transparent
        );
        assert_eq!(
            StructureRole::classify(&element(
                HtmlTag::Div,
                &[("role", "heading"), ("aria-level", "9")]
            )),
            StructureRole::Heading(HeadingLevel(6))
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Div, &[("role", "heading")])),
            StructureRole::Heading(HeadingLevel(2))
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Div, &[("role", "list")])),
            StructureRole::L
        );
        assert_eq!(
            StructureRole::classify(&element(HtmlTag::Div, &[("data-math", "x^2")])),
            StructureRole::Formula
        );
    }
}
