//! Opt-in accessible output: Tagged PDF conforming to PDF/UA-1 (ISO 14289-1).
//!
//! Nothing in this module runs unless [`TaggedPdf`] options are attached to a
//! converter. Default output stays byte-identical to an untagged conversion.

#[cfg(test)]
mod conversion_tests;
#[cfg(test)]
mod fixture_tests;
mod identity;
#[cfg(test)]
mod layout_tests;
mod metadata;
mod role;
mod skeleton;

pub(crate) use identity::{ContentRole, ElementOrdinal, ListNumbering, StructureRef};
pub(crate) use metadata::{DocumentMetadata, DocumentStructure};
pub(crate) use role::{HeadingLevel, StructureRole};
pub(crate) use skeleton::{HeaderScope, NodeFacts, SkeletonNode, StructureSkeleton};

use std::fmt;
use std::str::FromStr;

/// Options for the accessible (Tagged PDF, PDF/UA-1) output mode.
///
/// Attach with [`HtmlConverter::tagged_pdf`](crate::HtmlConverter::tagged_pdf)
/// or the scalar setters [`accessible`](crate::HtmlConverter::accessible),
/// [`document_title`](crate::HtmlConverter::document_title) and
/// [`document_language`](crate::HtmlConverter::document_language).
///
/// ```
/// use ironpress::{DocumentLanguage, TaggedPdf};
///
/// let options = TaggedPdf {
///     language: Some("en-GB".parse::<DocumentLanguage>().unwrap()),
///     ..TaggedPdf::default()
/// };
/// assert!(options.title.is_none());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaggedPdf {
    /// Natural language of the document. Overrides `lang` on `<html>`/`<body>`.
    /// ISO 14289-1 7.2 requires one; the conversion fails without it.
    pub language: Option<DocumentLanguage>,
    /// Document title for XMP `dc:title`. Overrides `<title>` and the first
    /// heading. ISO 14289-1 7.1 requires one; the conversion fails without it.
    pub title: Option<DocumentTitle>,
    /// What to do with an `<img>` that has no `alt` attribute.
    pub missing_alt: MissingAltPolicy,
}

/// Policy for images without an `alt` attribute in accessible output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MissingAltPolicy {
    /// Fail the conversion with a message naming the image (default).
    #[default]
    Error,
    /// Mark the image as a decorative artifact, hiding it from assistive
    /// technology.
    Artifact,
}

/// A well-formed BCP 47 language tag naming a document's natural language.
///
/// ```
/// use ironpress::DocumentLanguage;
///
/// let language: DocumentLanguage = "en-GB".parse().unwrap();
/// assert_eq!(language.as_str(), "en-GB");
/// assert!("not a tag!".parse::<DocumentLanguage>().is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentLanguage(language_tags::LanguageTag);

impl DocumentLanguage {
    /// The tag as written, for the PDF `/Lang` entry and XMP `dc:language`.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for DocumentLanguage {
    type Err = InvalidDocumentLanguage;

    fn from_str(tag: &str) -> Result<Self, Self::Err> {
        let tag = tag.trim();
        language_tags::LanguageTag::parse(tag)
            .map(Self)
            .map_err(|reason| InvalidDocumentLanguage {
                tag: tag.to_string(),
                reason: reason.to_string(),
            })
    }
}

impl fmt::Display for DocumentLanguage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Error returned when a language tag is not well-formed BCP 47.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidDocumentLanguage {
    tag: String,
    reason: String,
}

impl fmt::Display for InvalidDocumentLanguage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} is not a BCP 47 language tag ({}); use a tag such as en, en-GB or zh-Hans",
            self.tag, self.reason
        )
    }
}

impl std::error::Error for InvalidDocumentLanguage {}

/// A non-empty document title with collapsed whitespace, as HTML exposes
/// `document.title`.
///
/// ```
/// use ironpress::DocumentTitle;
///
/// let title: DocumentTitle = "  Quarterly\n  report ".parse().unwrap();
/// assert_eq!(title.as_str(), "Quarterly report");
/// assert!("   ".parse::<DocumentTitle>().is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentTitle(String);

impl DocumentTitle {
    /// The title text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DocumentTitle {
    type Err = EmptyDocumentTitle;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            return Err(EmptyDocumentTitle);
        }
        Ok(Self(collapsed))
    }
}

impl fmt::Display for DocumentTitle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Error returned when a document title is empty or whitespace only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyDocumentTitle;

impl fmt::Display for EmptyDocumentTitle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a document title must contain at least one non-whitespace character")
    }
}

impl std::error::Error for EmptyDocumentTitle {}

/// Why a document cannot be produced as an accessible PDF.
///
/// Reported through [`IronpressError::RenderError`](crate::IronpressError::RenderError)
/// with a `PDF/UA: ` prefix, so that the public error enum keeps its variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccessibilityError {
    /// No title option, `<title>` element or heading (ISO 14289-1 7.1,
    /// veraPDF rule 7.1-8: XMP `dc:title` is required).
    MissingTitle,
    /// No language option and no `lang` on `<html>` or `<body>`
    /// (ISO 14289-1 7.2, veraPDF rule 7.2-2: the catalog `/Lang` is required).
    MissingLanguage,
    /// The HTML carried a `lang` attribute that is not BCP 47.
    InvalidLanguage(InvalidDocumentLanguage),
    /// An image or formula has no text alternative (ISO 14289-1 7.3 and 7.7,
    /// veraPDF rules 7.3-1 and 7.7-1: Figure and Formula need /Alt).
    MissingAltText {
        /// The element, as `<img src="…">` or `<svg>`.
        element: String,
    },
}

impl fmt::Display for AccessibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTitle => formatter.write_str(
                "PDF/UA: the document has no title; add <title> to the HTML head, \
                 start the content with a heading, or set HtmlConverter::document_title \
                 (CLI: --title)",
            ),
            Self::MissingLanguage => formatter.write_str(
                "PDF/UA: the document language is unknown; add lang to <html>, \
                 or set HtmlConverter::document_language (CLI: --lang)",
            ),
            Self::InvalidLanguage(reason) => write!(
                formatter,
                "PDF/UA: the HTML lang attribute {reason}; fix the attribute, \
                 or override it with HtmlConverter::document_language (CLI: --lang)"
            ),
            Self::MissingAltText { element } => write!(
                formatter,
                "PDF/UA: {element} has no text alternative; add an alt attribute \
                 (alt=\"\" for a purely decorative image), or set \
                 TaggedPdf::missing_alt to MissingAltPolicy::Artifact to hide such images"
            ),
        }
    }
}

impl std::error::Error for AccessibilityError {}

impl From<AccessibilityError> for crate::error::IronpressError {
    fn from(error: AccessibilityError) -> Self {
        Self::RenderError(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags_are_parsed_as_bcp47() {
        // ISO 14289-1 7.2.3 / ISO 32000-1 14.9.2: /Lang is an RFC 3066 (BCP 47)
        // language identifier; a well-formed tag round-trips unchanged.
        let language: DocumentLanguage = " zh-Hans-CN ".parse().unwrap();
        assert_eq!(language.as_str(), "zh-Hans-CN");
        assert_eq!(language.to_string(), "zh-Hans-CN");

        let error = "en_GB".parse::<DocumentLanguage>().unwrap_err();
        assert!(error.to_string().contains("\"en_GB\""), "{error}");
        assert!(error.to_string().contains("BCP 47"), "{error}");
    }

    #[test]
    fn titles_collapse_whitespace_and_reject_blank_text() {
        // HTML's document.title strips and collapses ASCII whitespace; an empty
        // dc:title fails veraPDF rule 7.1-9, so blank titles are rejected.
        let title: DocumentTitle = "\n  Annual \t report\n".parse().unwrap();
        assert_eq!(title.as_str(), "Annual report");
        assert_eq!("".parse::<DocumentTitle>(), Err(EmptyDocumentTitle));
        assert_eq!(" \n\t".parse::<DocumentTitle>(), Err(EmptyDocumentTitle));
    }

    #[test]
    fn accessibility_errors_name_the_fix_and_the_pdf_ua_prefix() {
        let as_ironpress: crate::error::IronpressError = AccessibilityError::MissingTitle.into();
        let message = as_ironpress.to_string();
        assert!(
            message.contains("PDF/UA: the document has no title"),
            "{message}"
        );
        assert!(message.contains("--title"), "{message}");

        let message = AccessibilityError::MissingLanguage.to_string();
        assert!(message.starts_with("PDF/UA: "), "{message}");
        assert!(message.contains("--lang"), "{message}");

        let invalid = "en_GB".parse::<DocumentLanguage>().unwrap_err();
        let message = AccessibilityError::InvalidLanguage(invalid).to_string();
        assert!(message.contains("\"en_GB\""), "{message}");
        assert!(message.contains("--lang"), "{message}");
    }
}
