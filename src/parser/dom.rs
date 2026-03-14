use std::collections::HashMap;

/// Supported HTML tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlTag {
    Html,
    Head,
    Body,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    P,
    Div,
    Span,
    Strong,
    B,
    Em,
    I,
    U,
    A,
    Br,
    Hr,
    Table,
    Thead,
    Tbody,
    Tfoot,
    Tr,
    Td,
    Th,
    Caption,
    Ul,
    Ol,
    Li,
    Dl,
    Dt,
    Dd,
    Img,
    Blockquote,
    Pre,
    Code,
    Small,
    Sub,
    Sup,
    Del,
    S,
    Ins,
    Mark,
    Abbr,
    Section,
    Article,
    Nav,
    Header,
    Footer,
    Main,
    Aside,
    Figure,
    Figcaption,
    Address,
    Details,
    Summary,
    Unknown,
}

impl HtmlTag {
    pub fn from_tag_name(tag: &str) -> Self {
        match tag.to_ascii_lowercase().as_str() {
            "html" => Self::Html,
            "head" => Self::Head,
            "body" => Self::Body,
            "h1" => Self::H1,
            "h2" => Self::H2,
            "h3" => Self::H3,
            "h4" => Self::H4,
            "h5" => Self::H5,
            "h6" => Self::H6,
            "p" => Self::P,
            "div" => Self::Div,
            "span" => Self::Span,
            "strong" => Self::Strong,
            "b" => Self::B,
            "em" => Self::Em,
            "i" => Self::I,
            "u" => Self::U,
            "a" => Self::A,
            "br" => Self::Br,
            "hr" => Self::Hr,
            "table" => Self::Table,
            "thead" => Self::Thead,
            "tbody" => Self::Tbody,
            "tfoot" => Self::Tfoot,
            "tr" => Self::Tr,
            "td" => Self::Td,
            "th" => Self::Th,
            "caption" => Self::Caption,
            "ul" => Self::Ul,
            "ol" => Self::Ol,
            "li" => Self::Li,
            "dl" => Self::Dl,
            "dt" => Self::Dt,
            "dd" => Self::Dd,
            "img" => Self::Img,
            "blockquote" => Self::Blockquote,
            "pre" => Self::Pre,
            "code" => Self::Code,
            "small" => Self::Small,
            "sub" => Self::Sub,
            "sup" => Self::Sup,
            "del" | "strike" => Self::Del,
            "s" => Self::S,
            "ins" => Self::Ins,
            "mark" => Self::Mark,
            "abbr" => Self::Abbr,
            "section" => Self::Section,
            "article" => Self::Article,
            "nav" => Self::Nav,
            "header" => Self::Header,
            "footer" => Self::Footer,
            "main" => Self::Main,
            "aside" => Self::Aside,
            "figure" => Self::Figure,
            "figcaption" => Self::Figcaption,
            "address" => Self::Address,
            "details" => Self::Details,
            "summary" => Self::Summary,
            _ => Self::Unknown,
        }
    }

    pub fn is_block(&self) -> bool {
        matches!(
            self,
            Self::H1
                | Self::H2
                | Self::H3
                | Self::H4
                | Self::H5
                | Self::H6
                | Self::P
                | Self::Div
                | Self::Table
                | Self::Thead
                | Self::Tbody
                | Self::Tfoot
                | Self::Tr
                | Self::Ul
                | Self::Ol
                | Self::Li
                | Self::Dl
                | Self::Dt
                | Self::Dd
                | Self::Hr
                | Self::Body
                | Self::Html
                | Self::Blockquote
                | Self::Pre
                | Self::Caption
                | Self::Section
                | Self::Article
                | Self::Nav
                | Self::Header
                | Self::Footer
                | Self::Main
                | Self::Aside
                | Self::Figure
                | Self::Figcaption
                | Self::Address
                | Self::Details
                | Self::Summary
        )
    }

    pub fn is_inline(&self) -> bool {
        matches!(
            self,
            Self::Span
                | Self::Strong
                | Self::B
                | Self::Em
                | Self::I
                | Self::U
                | Self::A
                | Self::Code
                | Self::Small
                | Self::Sub
                | Self::Sup
                | Self::Del
                | Self::S
                | Self::Ins
                | Self::Mark
                | Self::Abbr
        )
    }
}

/// A node in the internal DOM tree.
#[derive(Debug)]
pub enum DomNode {
    Element(ElementNode),
    Text(String),
}

/// An HTML element with tag, attributes, and children.
#[derive(Debug)]
pub struct ElementNode {
    pub tag: HtmlTag,
    pub attributes: HashMap<String, String>,
    pub children: Vec<DomNode>,
}

impl ElementNode {
    pub fn new(tag: HtmlTag) -> Self {
        Self {
            tag,
            attributes: HashMap::new(),
            children: Vec::new(),
        }
    }

    pub fn style_attr(&self) -> Option<&str> {
        self.attributes.get("style").map(|s| s.as_str())
    }

    pub fn class_list(&self) -> Vec<&str> {
        self.attributes
            .get("class")
            .map(|s| s.split_whitespace().collect())
            .unwrap_or_default()
    }

    pub fn id(&self) -> Option<&str> {
        self.attributes.get("id").map(|s| s.as_str())
    }

    pub fn tag_name(&self) -> &'static str {
        match self.tag {
            HtmlTag::Html => "html",
            HtmlTag::Head => "head",
            HtmlTag::Body => "body",
            HtmlTag::H1 => "h1",
            HtmlTag::H2 => "h2",
            HtmlTag::H3 => "h3",
            HtmlTag::H4 => "h4",
            HtmlTag::H5 => "h5",
            HtmlTag::H6 => "h6",
            HtmlTag::P => "p",
            HtmlTag::Div => "div",
            HtmlTag::Span => "span",
            HtmlTag::Strong => "strong",
            HtmlTag::B => "b",
            HtmlTag::Em => "em",
            HtmlTag::I => "i",
            HtmlTag::U => "u",
            HtmlTag::A => "a",
            HtmlTag::Br => "br",
            HtmlTag::Hr => "hr",
            HtmlTag::Table => "table",
            HtmlTag::Thead => "thead",
            HtmlTag::Tbody => "tbody",
            HtmlTag::Tfoot => "tfoot",
            HtmlTag::Tr => "tr",
            HtmlTag::Td => "td",
            HtmlTag::Th => "th",
            HtmlTag::Caption => "caption",
            HtmlTag::Ul => "ul",
            HtmlTag::Ol => "ol",
            HtmlTag::Li => "li",
            HtmlTag::Dl => "dl",
            HtmlTag::Dt => "dt",
            HtmlTag::Dd => "dd",
            HtmlTag::Img => "img",
            HtmlTag::Blockquote => "blockquote",
            HtmlTag::Pre => "pre",
            HtmlTag::Code => "code",
            HtmlTag::Small => "small",
            HtmlTag::Sub => "sub",
            HtmlTag::Sup => "sup",
            HtmlTag::Del => "del",
            HtmlTag::S => "s",
            HtmlTag::Ins => "ins",
            HtmlTag::Mark => "mark",
            HtmlTag::Abbr => "abbr",
            HtmlTag::Section => "section",
            HtmlTag::Article => "article",
            HtmlTag::Nav => "nav",
            HtmlTag::Header => "header",
            HtmlTag::Footer => "footer",
            HtmlTag::Main => "main",
            HtmlTag::Aside => "aside",
            HtmlTag::Figure => "figure",
            HtmlTag::Figcaption => "figcaption",
            HtmlTag::Address => "address",
            HtmlTag::Details => "details",
            HtmlTag::Summary => "summary",
            HtmlTag::Unknown => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_from_name() {
        assert_eq!(HtmlTag::from_tag_name("h1"), HtmlTag::H1);
        assert_eq!(HtmlTag::from_tag_name("H1"), HtmlTag::H1);
        assert_eq!(HtmlTag::from_tag_name("blockquote"), HtmlTag::Blockquote);
        assert_eq!(HtmlTag::from_tag_name("pre"), HtmlTag::Pre);
        assert_eq!(HtmlTag::from_tag_name("code"), HtmlTag::Code);
        assert_eq!(HtmlTag::from_tag_name("small"), HtmlTag::Small);
        assert_eq!(HtmlTag::from_tag_name("sub"), HtmlTag::Sub);
        assert_eq!(HtmlTag::from_tag_name("sup"), HtmlTag::Sup);
        assert_eq!(HtmlTag::from_tag_name("del"), HtmlTag::Del);
        assert_eq!(HtmlTag::from_tag_name("strike"), HtmlTag::Del);
        assert_eq!(HtmlTag::from_tag_name("s"), HtmlTag::S);
        assert_eq!(HtmlTag::from_tag_name("ins"), HtmlTag::Ins);
        assert_eq!(HtmlTag::from_tag_name("mark"), HtmlTag::Mark);
        assert_eq!(HtmlTag::from_tag_name("abbr"), HtmlTag::Abbr);
        assert_eq!(HtmlTag::from_tag_name("section"), HtmlTag::Section);
        assert_eq!(HtmlTag::from_tag_name("article"), HtmlTag::Article);
        assert_eq!(HtmlTag::from_tag_name("nav"), HtmlTag::Nav);
        assert_eq!(HtmlTag::from_tag_name("header"), HtmlTag::Header);
        assert_eq!(HtmlTag::from_tag_name("footer"), HtmlTag::Footer);
        assert_eq!(HtmlTag::from_tag_name("main"), HtmlTag::Main);
        assert_eq!(HtmlTag::from_tag_name("aside"), HtmlTag::Aside);
        assert_eq!(HtmlTag::from_tag_name("figure"), HtmlTag::Figure);
        assert_eq!(HtmlTag::from_tag_name("figcaption"), HtmlTag::Figcaption);
        assert_eq!(HtmlTag::from_tag_name("address"), HtmlTag::Address);
        assert_eq!(HtmlTag::from_tag_name("details"), HtmlTag::Details);
        assert_eq!(HtmlTag::from_tag_name("summary"), HtmlTag::Summary);
        assert_eq!(HtmlTag::from_tag_name("thead"), HtmlTag::Thead);
        assert_eq!(HtmlTag::from_tag_name("tbody"), HtmlTag::Tbody);
        assert_eq!(HtmlTag::from_tag_name("tfoot"), HtmlTag::Tfoot);
        assert_eq!(HtmlTag::from_tag_name("caption"), HtmlTag::Caption);
        assert_eq!(HtmlTag::from_tag_name("dl"), HtmlTag::Dl);
        assert_eq!(HtmlTag::from_tag_name("dt"), HtmlTag::Dt);
        assert_eq!(HtmlTag::from_tag_name("dd"), HtmlTag::Dd);
        assert_eq!(HtmlTag::from_tag_name("img"), HtmlTag::Img);
        assert_eq!(HtmlTag::from_tag_name("table"), HtmlTag::Table);
        assert_eq!(HtmlTag::from_tag_name("nonsense"), HtmlTag::Unknown);
    }

    #[test]
    fn block_elements() {
        assert!(HtmlTag::P.is_block());
        assert!(HtmlTag::Div.is_block());
        assert!(HtmlTag::Blockquote.is_block());
        assert!(HtmlTag::Pre.is_block());
        assert!(HtmlTag::Section.is_block());
        assert!(HtmlTag::Article.is_block());
        assert!(HtmlTag::Details.is_block());
        assert!(HtmlTag::Dl.is_block());
        assert!(!HtmlTag::Span.is_inline() || HtmlTag::Span.is_inline());
        assert!(!HtmlTag::Code.is_block());
    }

    #[test]
    fn inline_elements() {
        assert!(HtmlTag::Span.is_inline());
        assert!(HtmlTag::Strong.is_inline());
        assert!(HtmlTag::Code.is_inline());
        assert!(HtmlTag::Small.is_inline());
        assert!(HtmlTag::Sub.is_inline());
        assert!(HtmlTag::Sup.is_inline());
        assert!(HtmlTag::Del.is_inline());
        assert!(HtmlTag::S.is_inline());
        assert!(HtmlTag::Ins.is_inline());
        assert!(HtmlTag::Mark.is_inline());
        assert!(HtmlTag::Abbr.is_inline());
        assert!(!HtmlTag::P.is_inline());
    }

    #[test]
    fn element_node_new() {
        let node = ElementNode::new(HtmlTag::P);
        assert_eq!(node.tag, HtmlTag::P);
        assert!(node.attributes.is_empty());
        assert!(node.children.is_empty());
        assert!(node.style_attr().is_none());
    }

    #[test]
    fn element_node_with_style() {
        let mut node = ElementNode::new(HtmlTag::Div);
        node.attributes
            .insert("style".to_string(), "color: red".to_string());
        assert_eq!(node.style_attr(), Some("color: red"));
    }
}
