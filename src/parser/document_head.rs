//! Document-level facts read from the parsed tree before `<html>`, `<head>`
//! and `<body>` are unwrapped: the title, the language and the CJK font locale.

use super::html::sink::{Document, Node, NodeData};
use crate::accessibility::{DocumentLanguage, DocumentTitle, InvalidDocumentLanguage};
use crate::font_pack::FontLocale;

/// What the `<head>` and the root elements declare about the whole document.
#[derive(Debug, Clone, Default)]
pub(crate) struct DocumentHead {
    /// The `<title>` under `<head>`, when it has text.
    pub(crate) title: Option<DocumentTitle>,
    /// The nearest `lang` attribute above the body content.
    pub(crate) language: HtmlLanguage,
    /// Regional CJK preference derived from the same `lang` attributes.
    pub(crate) font_locale: FontLocale,
}

/// A `lang` attribute as the HTML declared it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum HtmlLanguage {
    /// No `lang` attribute, or an empty one (HTML: language unknown).
    #[default]
    Absent,
    /// A `lang` attribute that is not well-formed BCP 47.
    Invalid(InvalidDocumentLanguage),
    /// A well-formed language tag.
    Known(DocumentLanguage),
}

impl HtmlLanguage {
    fn from_attribute(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") => Self::Absent,
            Some(tag) => tag
                .parse::<DocumentLanguage>()
                .map_or_else(Self::Invalid, Self::Known),
        }
    }

    /// The nearer declaration wins; an absent inner attribute inherits.
    fn or_outer(self, outer: Self) -> Self {
        match self {
            Self::Absent => outer,
            declared => declared,
        }
    }
}

impl DocumentHead {
    /// Read the head facts from the parsed document before conversion drops
    /// the `<head>` subtree and unwraps `<html>` and `<body>`.
    pub(crate) fn scan(document: &Document) -> Self {
        let tree = Tree(document);
        let Some(html) = tree.find_child(tree.root(), "html") else {
            return Self::default();
        };
        let html_lang = Tree::attribute(html, "lang");
        let body = tree.find_child(html, "body");
        let body_lang = body.and_then(|body| Tree::attribute(body, "lang"));

        let html_locale = FontLocale::from_html_lang(html_lang, FontLocale::Unspecified);
        let font_locale = match body {
            Some(_) => FontLocale::from_html_lang(body_lang, html_locale),
            None => html_locale,
        };
        let language = HtmlLanguage::from_attribute(body_lang)
            .or_outer(HtmlLanguage::from_attribute(html_lang));
        let title = tree
            .find_child(html, "head")
            .and_then(|head| tree.find_child(head, "title"))
            .and_then(|title| tree.text_content(title).parse().ok());

        Self {
            title,
            language,
            font_locale,
        }
    }
}

/// Read-only navigation over the parsed node arena.
#[derive(Clone, Copy)]
struct Tree<'a>(&'a Document);

impl<'a> Tree<'a> {
    fn root(self) -> &'a Node {
        self.0.node(Document::ROOT)
    }

    fn find_child(self, parent: &Node, tag: &str) -> Option<&'a Node> {
        parent
            .children
            .iter()
            .map(|&child| self.0.node(child))
            .find(|child| match &child.data {
                NodeData::Element { name, .. } => name.local.as_ref() == tag,
                _ => false,
            })
    }

    fn attribute(element: &'a Node, name: &str) -> Option<&'a str> {
        let NodeData::Element { attrs, .. } = &element.data else {
            return None;
        };
        attrs
            .iter()
            .find(|attribute| attribute.name.local.as_ref() == name)
            .map(|attribute| attribute.value.as_ref())
    }

    fn text_content(self, element: &Node) -> String {
        element
            .children
            .iter()
            .filter_map(|&child| match &self.0.node(child).data {
                NodeData::Text(contents) => Some(&**contents),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(html: &str) -> DocumentHead {
        crate::parser::html::parse_html_with_styles(html)
            .unwrap()
            .head
    }

    #[test]
    fn title_is_read_from_the_head_only() {
        let head = scan(
            "<html><head><title> A  title </title></head><body><svg><title>svg</title></svg></body></html>",
        );
        assert_eq!(
            head.title.as_ref().map(DocumentTitle::as_str),
            Some("A title")
        );

        let untitled = scan("<html><head></head><body><p>No title</p></body></html>");
        assert_eq!(untitled.title, None);

        let blank = scan("<html><head><title>  </title></head><body></body></html>");
        assert_eq!(blank.title, None);
    }


    #[test]
    fn language_takes_the_nearest_declaration_and_keeps_malformed_values() {
        let nearest = scan("<html lang='en'><body lang='fr-CA'><p>x</p></body></html>");
        assert!(
            matches!(&nearest.language, HtmlLanguage::Known(tag) if tag.as_str() == "fr-CA"),
            "{:?}",
            nearest.language
        );

        let inherited = scan("<html lang='en'><body><p>x</p></body></html>");
        assert!(
            matches!(&inherited.language, HtmlLanguage::Known(tag) if tag.as_str() == "en"),
            "{:?}",
            inherited.language
        );

        let empty = scan("<html lang=''><body><p>x</p></body></html>");
        assert_eq!(empty.language, HtmlLanguage::Absent);

        let malformed = scan("<html lang='en_US'><body><p>x</p></body></html>");
        assert!(
            matches!(malformed.language, HtmlLanguage::Invalid(_)),
            "{:?}",
            malformed.language
        );
    }


    #[test]
    fn font_locale_follows_the_same_attributes() {
        assert_eq!(
            scan("<html lang='ja'><body><p>x</p></body></html>").font_locale,
            FontLocale::Japanese
        );
        assert_eq!(
            scan("<html lang='ja'><body lang='ko'><p>x</p></body></html>").font_locale,
            FontLocale::Korean
        );
        assert_eq!(
            scan("<html lang='ja'><body lang='xx!'><p>x</p></body></html>").font_locale,
            FontLocale::Unspecified
        );
    }
}
