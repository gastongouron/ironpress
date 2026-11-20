//! Resolution of the document-level facts a Tagged PDF must carry.

use super::{AccessibilityError, DocumentLanguage, DocumentTitle, TaggedPdf};
use crate::parser::document_head::{DocumentHead, HtmlLanguage};
use crate::parser::dom::DomNode;

/// Document-level facts resolved once per conversion: the title and language
/// required by ISO 14289-1 7.1 and 7.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentMetadata {
    pub(crate) title: DocumentTitle,
    pub(crate) language: DocumentLanguage,
}

impl DocumentMetadata {
    /// Resolve the title and language from the options, the parsed `<head>`
    /// and, for the title only, the first heading in the body.
    ///
    /// Title: option, then `<title>`, then the first `h1`–`h6` text. Language:
    /// option, then `lang` on `<body>` or `<html>`. There is no implicit
    /// fallback for either; a wrong language makes screen readers pick the
    /// wrong voice, so an opt-in mode fails with a hint instead.
    pub(crate) fn resolve(
        options: &TaggedPdf,
        head: &DocumentHead,
        body: &[DomNode],
    ) -> Result<Self, AccessibilityError> {
        let title = options
            .title
            .clone()
            .or_else(|| head.title.clone())
            .or_else(|| first_heading_title(body))
            .ok_or(AccessibilityError::MissingTitle)?;
        let language = match (&options.language, &head.language) {
            (Some(language), _) | (None, HtmlLanguage::Known(language)) => language.clone(),
            (None, HtmlLanguage::Invalid(reason)) => {
                return Err(AccessibilityError::InvalidLanguage(reason.clone()));
            }
            (None, HtmlLanguage::Absent) => return Err(AccessibilityError::MissingLanguage),
        };
        Ok(Self { title, language })
    }
}

/// Everything the renderer needs to tag a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentStructure {
    pub(crate) metadata: DocumentMetadata,
    /// Roles, parents and facts of every element, keyed by the ordinal that
    /// layout carries on each run and box.
    pub(crate) skeleton: super::StructureSkeleton,
    pub(crate) missing_alt: super::MissingAltPolicy,
}

/// The text of the first heading in document order, if it is non-blank.
fn first_heading_title(nodes: &[DomNode]) -> Option<DocumentTitle> {
    let mut pending: Vec<&DomNode> = nodes.iter().rev().collect();
    while let Some(node) = pending.pop() {
        let DomNode::Element(element) = node else {
            continue;
        };
        if element.tag.is_heading() {
            if let Ok(title) = text_content(&element.children).parse() {
                return Some(title);
            }
            continue;
        }
        pending.extend(element.children.iter().rev());
    }
    None
}

/// Concatenated descendant text, as `Node.textContent` exposes it.
fn text_content(nodes: &[DomNode]) -> String {
    let mut text = String::new();
    let mut pending: Vec<&DomNode> = nodes.iter().rev().collect();
    while let Some(node) = pending.pop() {
        match node {
            DomNode::Text(run) => text.push_str(run),
            DomNode::Element(element) => pending.extend(element.children.iter().rev()),
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::html::parse_html_with_styles;

    fn resolve(options: TaggedPdf, html: &str) -> Result<DocumentMetadata, AccessibilityError> {
        let parsed = parse_html_with_styles(html).unwrap();
        DocumentMetadata::resolve(&options, &parsed.head, &parsed.nodes)
    }

    fn language(tag: &str) -> DocumentLanguage {
        tag.parse().unwrap()
    }

    #[test]
    fn title_prefers_option_then_head_then_first_heading() {
        // ISO 14289-1 7.1: the document title is carried in XMP dc:title; the
        // HTML title element is the authored value, headings are the fallback.
        let html = "<html lang='en'><head><title>Head title</title></head>\
                    <body><h2>First <em>heading</em></h2></body></html>";
        let from_option = resolve(
            TaggedPdf {
                title: Some("Option title".parse().unwrap()),
                ..TaggedPdf::default()
            },
            html,
        )
        .unwrap();
        assert_eq!(from_option.title.as_str(), "Option title");

        let from_head = resolve(TaggedPdf::default(), html).unwrap();
        assert_eq!(from_head.title.as_str(), "Head title");

        let from_heading = resolve(
            TaggedPdf::default(),
            "<html lang='en'><body><p>Intro</p><div><h2>First <em>heading</em></h2></div></body></html>",
        )
        .unwrap();
        assert_eq!(from_heading.title.as_str(), "First heading");
    }

    #[test]
    fn blank_headings_are_skipped_and_no_title_is_an_error() {
        let resolved = resolve(
            TaggedPdf::default(),
            "<html lang='en'><body><h1> </h1><h3>Real</h3></body></html>",
        )
        .unwrap();
        assert_eq!(resolved.title.as_str(), "Real");

        let error = resolve(
            TaggedPdf::default(),
            "<html lang='en'><body><p>No heading</p></body></html>",
        )
        .unwrap_err();
        assert_eq!(error, AccessibilityError::MissingTitle);
    }

    #[test]
    fn language_prefers_option_then_body_then_html() {
        // ISO 14289-1 7.2.3: the default natural language is declared on the
        // catalog; the nearest HTML lang attribute to the content wins.
        let html = "<html lang='en-GB'><body lang='fr'><h1>T</h1></body></html>";
        let from_option = resolve(
            TaggedPdf {
                language: Some(language("de")),
                ..TaggedPdf::default()
            },
            html,
        )
        .unwrap();
        assert_eq!(from_option.language.as_str(), "de");

        let from_body = resolve(TaggedPdf::default(), html).unwrap();
        assert_eq!(from_body.language.as_str(), "fr");

        let from_html = resolve(
            TaggedPdf::default(),
            "<html lang='en-GB'><body><h1>T</h1></body></html>",
        )
        .unwrap();
        assert_eq!(from_html.language.as_str(), "en-GB");
    }

    #[test]
    fn missing_or_malformed_language_is_an_error_unless_overridden() {
        let missing = resolve(TaggedPdf::default(), "<h1>T</h1>").unwrap_err();
        assert_eq!(missing, AccessibilityError::MissingLanguage);

        let malformed = resolve(
            TaggedPdf::default(),
            "<html lang='english!'><body><h1>T</h1></body></html>",
        )
        .unwrap_err();
        assert!(
            matches!(malformed, AccessibilityError::InvalidLanguage(_)),
            "{malformed:?}"
        );

        let overridden = resolve(
            TaggedPdf {
                language: Some(language("en")),
                ..TaggedPdf::default()
            },
            "<html lang='english!'><body><h1>T</h1></body></html>",
        )
        .unwrap();
        assert_eq!(overridden.language.as_str(), "en");
    }
}
