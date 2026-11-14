//! Deterministic XMP packet for the `/Metadata` stream.

use crate::accessibility::DocumentMetadata;

/// The XMP packet carrying the PDF/UA identification and the Dublin Core
/// title and language (ISO 14289-1 5 and 7.1; ISO 16684-1 for the packet).
pub(super) struct XmpPacket<'a> {
    metadata: &'a DocumentMetadata,
    producer: &'a str,
}

impl<'a> XmpPacket<'a> {
    pub(super) fn new(metadata: &'a DocumentMetadata, producer: &'a str) -> Self {
        Self { metadata, producer }
    }

    /// Serialize the packet. No timestamps are written so that output is
    /// reproducible. The `pdfuaid` prefix is spelled literally because
    /// veraPDF resolves it by name as well as by namespace.
    pub(super) fn to_xml(&self) -> String {
        let title = xml_escape(self.metadata.title.as_str());
        let language = xml_escape(self.metadata.language.as_str());
        let producer = xml_escape(self.producer);
        format!(
            "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
             <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
             <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
             <rdf:Description rdf:about=\"\" \
             xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\" \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
             xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">\n\
             <pdfuaid:part>1</pdfuaid:part>\n\
             <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{title}</rdf:li></rdf:Alt></dc:title>\n\
             <dc:language><rdf:Bag><rdf:li>{language}</rdf:li></rdf:Bag></dc:language>\n\
             <pdf:Producer>{producer}</pdf:Producer>\n\
             </rdf:Description>\n\
             </rdf:RDF>\n\
             </x:xmpmeta>\n\
             <?xpacket end=\"w\"?>"
        )
    }
}

fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accessibility::{DocumentLanguage, DocumentTitle};

    fn metadata(title: &str, language: &str) -> DocumentMetadata {
        DocumentMetadata {
            title: title.parse::<DocumentTitle>().unwrap(),
            language: language.parse::<DocumentLanguage>().unwrap(),
        }
    }

    #[test]
    fn packet_identifies_pdf_ua_1_and_carries_the_title() {
        // veraPDF ua1 rules 5-1/5-2 (pdfuaid:part = 1 in the
        // http://www.aiim.org/pdfua/ns/id/ schema) and 7.1-8 (dc:title present).
        let xml = XmpPacket::new(&metadata("Report", "en-GB"), "ironpress test").to_xml();
        assert!(xml.starts_with("<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>"));
        assert!(
            xml.contains("xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\""),
            "{xml}"
        );
        assert!(xml.contains("<pdfuaid:part>1</pdfuaid:part>"), "{xml}");
        assert!(
            xml.contains("<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">Report</rdf:li></rdf:Alt></dc:title>"),
            "{xml}"
        );
        assert!(xml.contains("<rdf:li>en-GB</rdf:li>"), "{xml}");
        assert!(
            xml.contains("<pdf:Producer>ironpress test</pdf:Producer>"),
            "{xml}"
        );
        assert!(xml.ends_with("<?xpacket end=\"w\"?>"), "{xml}");
    }

    #[test]
    fn packet_escapes_markup_in_titles() {
        let xml = XmpPacket::new(&metadata("Q&A <draft> \"v2\"", "en"), "p").to_xml();
        assert!(
            xml.contains(">Q&amp;A &lt;draft&gt; &quot;v2&quot;<"),
            "{xml}"
        );
        assert!(!xml.contains("<draft>"), "{xml}");
    }
}
