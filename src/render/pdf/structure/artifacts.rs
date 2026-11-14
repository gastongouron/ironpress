//! Fail-closed artifact normalization of a finished page content stream.
//!
//! Every painting operator that is not inside marked content becomes a
//! `Layout` artifact (ISO 14289-1 7.1: all content is either tagged or an
//! artifact). Decoration painters therefore need no changes. Each inserted
//! sequence wraps exactly one path object, text object, shading, XObject or
//! inline image, so it can never straddle `q`/`Q`, `BT`/`ET` or a path.

/// Wrap unmarked painting operators in `Layout` artifacts.
pub(super) fn artifact_unmarked_paint(stream: &str) -> String {
    const OPEN: &str = "/Artifact << /Type /Layout >> BDC\n";
    const CLOSE: &str = "\nEMC";

    let mut out = String::with_capacity(stream.len() + stream.len() / 8);
    let mut copied = 0;
    let mut insert = |out: &mut String, at: usize, text: &str| {
        out.push_str(&stream[copied..at]);
        out.push_str(text);
        copied = at;
    };

    let mut marked_depth = 0u32;
    let mut operands_start: Option<usize> = None;
    let mut path_start: Option<usize> = None;
    let mut text_object_start: Option<usize> = None;
    let mut inline_image_start: Option<usize> = None;

    for token in Lexer::new(stream) {
        let Token::Operator { start, end, text } = token else {
            if operands_start.is_none() {
                operands_start = Some(token.start());
            }
            continue;
        };
        let item_start = operands_start.take().unwrap_or(start);
        match text {
            "BDC" | "BMC" => marked_depth += 1,
            "EMC" => marked_depth = marked_depth.saturating_sub(1),
            "m" | "re" | "l" | "c" | "v" | "y" | "h" => {
                path_start.get_or_insert(item_start);
            }
            "W" | "W*" => {}
            "n" => path_start = None,
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                if let Some(path_start) = path_start.take()
                    && marked_depth == 0
                {
                    insert(&mut out, path_start, OPEN);
                    insert(&mut out, end, CLOSE);
                }
            }
            "BT" => {
                if marked_depth == 0 {
                    text_object_start = Some(start);
                }
            }
            "ET" => {
                if let Some(text_start) = text_object_start.take() {
                    insert(&mut out, text_start, OPEN);
                    insert(&mut out, end, CLOSE);
                }
            }
            "sh" | "Do" => {
                if marked_depth == 0 {
                    insert(&mut out, item_start, OPEN);
                    insert(&mut out, end, CLOSE);
                }
            }
            "BI" => {
                if marked_depth == 0 {
                    inline_image_start = Some(start);
                }
            }
            "EI" => {
                if let Some(image_start) = inline_image_start.take() {
                    insert(&mut out, image_start, OPEN);
                    insert(&mut out, end, CLOSE);
                }
            }
            _ => {}
        }
    }
    out.push_str(&stream[copied..]);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Operand {
        start: usize,
    },
    Operator {
        start: usize,
        end: usize,
        text: &'a str,
    },
}

impl Token<'_> {
    fn start(self) -> usize {
        match self {
            Self::Operand { start } | Self::Operator { start, .. } => start,
        }
    }
}

/// A lexer for the content-stream syntax this crate itself emits: numbers,
/// names, literal and hex strings, dictionaries, arrays, operators and
/// ASCII-hex inline images.
struct Lexer<'a> {
    stream: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Lexer<'a> {
    fn new(stream: &'a str) -> Self {
        Self {
            stream,
            bytes: stream.as_bytes(),
            position: 0,
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        while let Some(&byte) = self.bytes.get(self.position) {
            if byte.is_ascii_whitespace() {
                self.position += 1;
            } else if byte == b'%' {
                while let Some(&byte) = self.bytes.get(self.position) {
                    self.position += 1;
                    if byte == b'\n' || byte == b'\r' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn skip_literal_string(&mut self) {
        let mut depth = 0u32;
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position += 1;
            match byte {
                b'\\' => self.position += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_until(&mut self, delimiter: u8) {
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position += 1;
            if byte == delimiter {
                return;
            }
        }
    }

    /// Advance over one regular-character run (a number, operator or name
    /// body) and return its end.
    fn regular_run_end(&self, from: usize) -> usize {
        let mut end = from;
        while let Some(&byte) = self.bytes.get(end) {
            if byte.is_ascii_whitespace() || b"()<>[]{}/%".contains(&byte) {
                break;
            }
            end += 1;
        }
        end
    }

    /// After `ID`, skip the image data up to and including the `EI` operator.
    fn inline_image_end(&mut self) -> Option<Token<'a>> {
        let rest = &self.stream[self.position..];
        let mut search = 0;
        while let Some(found) = rest[search..].find("EI") {
            let at = search + found;
            let before = at == 0 || rest.as_bytes()[at - 1].is_ascii_whitespace();
            let after = rest
                .as_bytes()
                .get(at + 2)
                .is_none_or(u8::is_ascii_whitespace);
            if before && after {
                let start = self.position + at;
                self.position = start + 2;
                return Some(Token::Operator {
                    start,
                    end: start + 2,
                    text: "EI",
                });
            }
            search = at + 2;
        }
        self.position = self.stream.len();
        None
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        self.skip_whitespace_and_comments();
        let start = self.position;
        let byte = *self.bytes.get(start)?;
        match byte {
            b'(' => {
                self.skip_literal_string();
                Some(Token::Operand { start })
            }
            b'<' => {
                if self.bytes.get(start + 1) == Some(&b'<') {
                    self.position += 2;
                } else {
                    self.position += 1;
                    self.skip_until(b'>');
                }
                Some(Token::Operand { start })
            }
            b'>' => {
                // Only `>>` reaches here: a hex string end is consumed above.
                self.position += if self.bytes.get(start + 1) == Some(&b'>') {
                    2
                } else {
                    1
                };
                Some(Token::Operand { start })
            }
            b'[' | b']' | b'{' | b'}' => {
                self.position += 1;
                Some(Token::Operand { start })
            }
            b'/' => {
                self.position = self.regular_run_end(start + 1);
                Some(Token::Operand { start })
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                self.position = self.regular_run_end(start);
                Some(Token::Operand { start })
            }
            _ => {
                let end = self.regular_run_end(start);
                self.position = end;
                let text = &self.stream[start..end];
                if text == "ID" {
                    self.position += 1;
                    return self.inline_image_end();
                }
                Some(Token::Operator { start, end, text })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_wrapped_from_their_first_construction_operator() {
        // ISO 32000-1 14.6: marked content may not begin inside a path object,
        // so the artifact opens before `re` and closes after `f`.
        let stream = "q\n1 0 0 rg\n0 0 10 10 re\nf\nQ\n";
        assert_eq!(
            artifact_unmarked_paint(stream),
            "q\n1 0 0 rg\n/Artifact << /Type /Layout >> BDC\n0 0 10 10 re\nf\nEMC\nQ\n"
        );
    }

    #[test]
    fn clipping_paths_are_not_content() {
        // veraPDF's content-item model excludes `n`: a clip is not painted.
        let stream = "q\n0 0 10 10 re\nW n\n0 0 5 5 re\nf\nQ\n";
        assert_eq!(
            artifact_unmarked_paint(stream),
            "q\n0 0 10 10 re\nW n\n/Artifact << /Type /Layout >> BDC\n0 0 5 5 re\nf\nEMC\nQ\n"
        );
    }

    #[test]
    fn text_objects_shadings_xobjects_and_inline_images_are_wrapped_whole() {
        // ISO 32000-1 14.6: a sequence may enclose a text object but may not
        // start or end inside one.
        let stream = "BT\n/F1 12 Tf\n(Hi) Tj\nET\n/Sh0 sh\n/Im1 Do\nBI\n/W 1 /H 1 /CS /DeviceGray /BPC 8 /F /AHx\nID\n00>\nEI\n";
        let expected = "/Artifact << /Type /Layout >> BDC\nBT\n/F1 12 Tf\n(Hi) Tj\nET\nEMC\n\
                        /Artifact << /Type /Layout >> BDC\n/Sh0 sh\nEMC\n\
                        /Artifact << /Type /Layout >> BDC\n/Im1 Do\nEMC\n\
                        /Artifact << /Type /Layout >> BDC\nBI\n/W 1 /H 1 /CS /DeviceGray /BPC 8 /F /AHx\nID\n00>\nEI\nEMC\n";
        assert_eq!(artifact_unmarked_paint(stream), expected);
    }

    #[test]
    fn tagged_content_is_left_alone_and_the_pass_is_idempotent() {
        // veraPDF 7.1-1: an artifact inside tagged content is a failure, so
        // nothing is inserted while a marked-content sequence is open.
        let stream = "/P << /MCID 0 >> BDC\nBT\n(a) Tj\nET\n0 0 1 1 re f\nEMC\n0 0 2 2 re\nS\n";
        let once = artifact_unmarked_paint(stream);
        assert_eq!(
            once,
            "/P << /MCID 0 >> BDC\nBT\n(a) Tj\nET\n0 0 1 1 re f\nEMC\n/Artifact << /Type /Layout >> BDC\n0 0 2 2 re\nS\nEMC\n"
        );
        assert_eq!(artifact_unmarked_paint(&once), once);
    }

    #[test]
    fn operator_like_text_inside_strings_dictionaries_and_names_is_ignored() {
        let stream =
            "BT\n(f re S \\) Do) Tj\n<466f> Tj\n/Span << /ActualText (Do) >> BDC\nEMC\nET\n";
        assert_eq!(
            artifact_unmarked_paint(stream),
            format!("/Artifact << /Type /Layout >> BDC\n{stream}EMC\n"),
        );
        let bare = "/Do 1 0 0 1 0 0 cm\n";
        assert_eq!(artifact_unmarked_paint(bare), bare);
    }

    #[test]
    fn streams_without_paint_are_unchanged() {
        let stream = "q 1 0 0 1 0 0 cm\nq 0 0 100 100 re\nW n\nQ\nQ\n";
        assert_eq!(artifact_unmarked_paint(stream), stream);
    }
}
