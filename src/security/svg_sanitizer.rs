//! SVG sanitizer — strips dangerous elements and attributes before parsing.

/// Maximum number of SVG elements allowed.
pub const MAX_SVG_ELEMENTS: usize = 10_000;

/// Maximum SVG nesting depth.
pub const MAX_SVG_DEPTH: usize = 50;

/// Allowlisted SVG elements (everything else is stripped, content preserved if safe).
const ALLOWED_ELEMENTS: &[&str] = &[
    "svg", "g", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "title",
    "desc", "defs",
];

/// Blocklisted elements (removed WITH their content — these are dangerous).
const BLOCKED_ELEMENTS: &[&str] = &[
    "script",
    "foreignobject",
    "use",
    "image",
    "a",
    "animate",
    "set",
    "animatemotion",
    "animatetransform",
    "iframe",
    "embed",
    "object",
    "style",
    "handler",
    "listener",
];

/// Sanitize SVG markup string. Returns cleaned SVG.
pub fn sanitize_svg(svg: &str) -> String {
    // 1. Remove blocked elements and their content
    let mut result = svg.to_string();
    for tag in BLOCKED_ELEMENTS {
        result = remove_tag_with_content(&result, tag);
    }

    // 2. Remove event handler attributes and dangerous href attributes
    result = remove_dangerous_attributes(&result);

    // 3. Remove javascript: in attribute values
    result = remove_javascript_urls(&result);

    // 4. Strip unknown elements (keep content)
    result = strip_unknown_elements(&result);

    // 5. Check element count limit
    let count = count_elements(&result);
    if count > MAX_SVG_ELEMENTS {
        return String::from("<svg></svg>");
    }

    result
}

/// Remove a tag and all its content (case-insensitive).
fn remove_tag_with_content(input: &str, tag: &str) -> String {
    let mut result = input.to_string();
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    loop {
        let lower = result.to_ascii_lowercase();
        let start = lower.find(&open);
        let end = lower.find(&close);

        match (start, end) {
            (Some(s), Some(e)) => {
                let end_pos = e + close.len();
                result = format!("{}{}", &result[..s], &result[end_pos..]);
            }
            (Some(s), None) => {
                // Self-closing or unclosed — remove from start to end of tag
                if let Some(gt) = result[s..].find('>') {
                    result = format!("{}{}", &result[..s], &result[s + gt + 1..]);
                } else {
                    break;
                }
            }
            _ => break,
        }
    }

    result
}

/// Remove on* event handler attributes, href, and xlink:href inside tags.
fn remove_dangerous_attributes(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    let mut in_tag = false;

    while i < bytes.len() {
        let c = bytes[i] as char;

        if c == '<' && !in_tag {
            in_tag = true;
            result.push(c);
            i += 1;
            continue;
        }

        if c == '>' {
            in_tag = false;
            result.push(c);
            i += 1;
            continue;
        }

        if in_tag {
            // Check for on* event handlers
            if (c == 'o' || c == 'O') && i + 2 < bytes.len() {
                let next = bytes[i + 1] as char;
                if (next == 'n' || next == 'N') && (bytes[i + 2] as char).is_ascii_alphabetic() {
                    let prev = if i > 0 { bytes[i - 1] as char } else { ' ' };
                    if prev == ' ' || prev == '\t' || prev == '\n' {
                        i = skip_attribute(bytes, i);
                        continue;
                    }
                }
            }

            // Check for href attribute
            if (c == 'h' || c == 'H') && i + 4 < bytes.len() {
                let chunk: String = bytes[i..i + 4]
                    .iter()
                    .map(|&b| (b as char).to_ascii_lowercase())
                    .collect();
                if chunk == "href" {
                    let prev = if i > 0 { bytes[i - 1] as char } else { ' ' };
                    if prev == ' ' || prev == '\t' || prev == '\n' || prev == ':' {
                        i = skip_attribute(bytes, i);
                        continue;
                    }
                }
            }

            // Check for xlink:href attribute
            if (c == 'x' || c == 'X') && i + 10 < bytes.len() {
                let chunk: String = bytes[i..i + 10]
                    .iter()
                    .map(|&b| (b as char).to_ascii_lowercase())
                    .collect();
                if chunk == "xlink:href" {
                    let prev = if i > 0 { bytes[i - 1] as char } else { ' ' };
                    if prev == ' ' || prev == '\t' || prev == '\n' {
                        i = skip_attribute(bytes, i);
                        continue;
                    }
                }
            }
        }

        result.push(c);
        i += 1;
    }

    result
}

/// Skip past an attribute name and its value (name="value" or name='value' or name=value).
fn skip_attribute(bytes: &[u8], start: usize) -> usize {
    let mut j = start;
    // Skip attribute name
    while j < bytes.len() && bytes[j] != b'=' && bytes[j] != b' ' && bytes[j] != b'>' {
        j += 1;
    }
    // Skip = and value
    if j < bytes.len() && bytes[j] == b'=' {
        j += 1;
        // Skip whitespace
        while j < bytes.len() && (bytes[j] as char).is_whitespace() {
            j += 1;
        }
        if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
            let quote = bytes[j];
            j += 1;
            while j < bytes.len() && bytes[j] != quote {
                j += 1;
            }
            if j < bytes.len() {
                j += 1; // skip closing quote
            }
        } else {
            // Unquoted — skip to space or >
            while j < bytes.len() && bytes[j] != b' ' && bytes[j] != b'>' {
                j += 1;
            }
        }
    }
    j
}

/// Remove javascript: from attribute values.
fn remove_javascript_urls(html: &str) -> String {
    // Case-insensitive replacement
    let mut result = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let target = "javascript:";
    let mut last = 0;

    for (pos, _) in lower.match_indices(target) {
        result.push_str(&html[last..pos]);
        last = pos + target.len();
    }
    result.push_str(&html[last..]);
    result
}

/// Strip tags that are not in the allowlist, but keep their text content.
fn strip_unknown_elements(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Find end of tag
            if let Some(gt_offset) = html[i..].find('>') {
                let tag_str = &html[i + 1..i + gt_offset];
                let tag_name = extract_tag_name(tag_str);
                let tag_lower = tag_name.to_ascii_lowercase();

                if is_allowed_element(&tag_lower) {
                    // Keep the whole tag
                    result.push_str(&html[i..=i + gt_offset]);
                }
                // else: skip the tag (content will be kept by subsequent iterations)
                i += gt_offset + 1;
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Extract the tag name from the content between < and >.
fn extract_tag_name(tag_content: &str) -> &str {
    let s = tag_content.trim_start_matches('/').trim();
    // Tag name ends at first space, /, or end
    let end = s
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(s.len());
    &s[..end]
}

/// Check if a tag name (lowercase) is in the allowlist.
fn is_allowed_element(tag_lower: &str) -> bool {
    ALLOWED_ELEMENTS.contains(&tag_lower)
}

/// Count the number of opening tags in the markup.
fn count_elements(html: &str) -> usize {
    let mut count = 0;
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] != b'/' {
            count += 1;
        }
        i += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_script() {
        let input = r#"<svg><rect width="10" height="10"/><script>alert(1)</script></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("script"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn sanitize_strips_foreignobject() {
        let input = r#"<svg><foreignObject><div>evil</div></foreignObject><rect/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("foreignObject"));
        assert!(!result.contains("foreignobject"));
        assert!(!result.contains("evil"));
    }

    #[test]
    fn sanitize_strips_use_element() {
        let input = r##"<svg><use href="#evil"/></svg>"##;
        let result = sanitize_svg(input);
        assert!(!result.contains("use"));
    }

    #[test]
    fn sanitize_strips_event_handlers() {
        let input = r#"<svg><rect onclick="alert(1)" width="10" height="10"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("onclick"));
        assert!(!result.contains("alert"));
        assert!(result.contains("rect"));
    }

    #[test]
    fn sanitize_preserves_basic_shapes() {
        let input = r#"<svg><rect width="10" height="10"/><circle cx="5" cy="5" r="3"/><path d="M0 0L10 10"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(result.contains("rect"));
        assert!(result.contains("circle"));
        assert!(result.contains("path"));
    }

    #[test]
    fn sanitize_strips_href() {
        let input = r#"<svg><rect href="http://evil.com" width="10"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("href"));
        assert!(!result.contains("evil.com"));
    }

    #[test]
    fn sanitize_exceeds_max_elements_returns_empty_svg() {
        // Build an SVG with more than MAX_SVG_ELEMENTS opening tags
        let mut input = String::from("<svg>");
        for _ in 0..MAX_SVG_ELEMENTS + 1 {
            input.push_str("<rect/>");
        }
        input.push_str("</svg>");
        let result = sanitize_svg(&input);
        assert_eq!(result, "<svg></svg>");
    }

    #[test]
    fn sanitize_nested_blocked_elements() {
        let input = "<svg><script><script>inner</script></script><rect/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("script"));
        assert!(!result.contains("inner"));
        assert!(result.contains("rect"));
    }

    #[test]
    fn sanitize_unclosed_blocked_element_no_gt() {
        // A blocked tag with no closing tag and no '>' — triggers the break branch (line 81)
        // The tag persists because there's no '>' to close it, but the break is exercised.
        let input = "<svg><rect/><script";
        let result = sanitize_svg(input);
        // The unclosed fragment remains since there is no '>' to find
        assert!(result.contains("<script"));
    }

    #[test]
    fn sanitize_self_closing_blocked_element() {
        // A blocked tag with opening but no closing, has '>' — triggers (Some(s), None) branch
        let input = "<svg><image src='evil.png'/><rect/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("image"));
        assert!(!result.contains("evil.png"));
        assert!(result.contains("rect"));
    }

    #[test]
    fn sanitize_strips_xlink_href() {
        let input = r#"<svg><rect xlink:href="http://evil.com" width="10"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("xlink:href"));
        assert!(!result.contains("evil.com"));
    }

    #[test]
    fn sanitize_strips_xlink_href_mixed_case() {
        let input = r#"<svg><rect Xlink:Href="http://evil.com" width="10"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("Xlink:Href"));
        assert!(!result.contains("evil.com"));
    }

    #[test]
    fn sanitize_event_handler_mixed_case() {
        let input = r#"<svg><rect OnClick="alert(1)" width="10"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("OnClick"));
        assert!(!result.contains("alert"));
        assert!(result.contains("rect"));
    }

    #[test]
    fn sanitize_event_handler_with_tab_separator() {
        let input = "<svg><rect\tonclick=\"alert(1)\" width=\"10\"/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("onclick"));
    }

    #[test]
    fn sanitize_event_handler_with_newline_separator() {
        let input = "<svg><rect\nonclick=\"alert(1)\" width=\"10\"/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("onclick"));
    }

    #[test]
    fn sanitize_removes_javascript_urls() {
        let input = r#"<svg><rect fill="javascript:void(0)"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("javascript:"));
        // The "void(0)" part remains since only the "javascript:" prefix is stripped
    }

    #[test]
    fn sanitize_removes_javascript_urls_mixed_case() {
        let input = r#"<svg><rect fill="JavaScript:void(0)"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("JavaScript:"));
        assert!(!result.contains("javascript:"));
    }

    #[test]
    fn sanitize_removes_multiple_javascript_urls() {
        let input = r#"<svg><rect fill="javascript:x" stroke="javascript:y"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("javascript:"));
    }

    #[test]
    fn sanitize_strips_unknown_elements_keeps_text() {
        let input = "<svg><div>hello</div><rect/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("<div"));
        assert!(!result.contains("</div>"));
        assert!(result.contains("hello"));
        assert!(result.contains("rect"));
    }

    #[test]
    fn sanitize_empty_svg() {
        let result = sanitize_svg("");
        assert_eq!(result, "");
    }

    #[test]
    fn sanitize_text_only_content() {
        let result = sanitize_svg("just plain text");
        assert_eq!(result, "just plain text");
    }

    #[test]
    fn sanitize_unclosed_tag_in_strip_unknown() {
        // An unclosed '<' with no '>' triggers the else branch in strip_unknown_elements
        let input = "<svg><rect/></svg><broken";
        let result = sanitize_svg(input);
        // The '<' is preserved as-is since there's no closing '>'
        assert!(result.contains("<broken"));
    }

    #[test]
    fn sanitize_unquoted_attribute_value() {
        // An event handler with an unquoted value exercises the unquoted branch in skip_attribute
        let input = "<svg><rect onclick=alert width=\"10\"/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("onclick"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn sanitize_attribute_with_whitespace_after_equals() {
        // Whitespace between = and the quoted value exercises that branch in skip_attribute
        let input = "<svg><rect onclick= \"alert(1)\" width=\"10\"/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("onclick"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn sanitize_attribute_with_single_quotes() {
        let input = "<svg><rect onclick='alert(1)' width='10'/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("onclick"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn sanitize_at_element_limit_passes() {
        // Exactly MAX_SVG_ELEMENTS should be allowed
        let mut input = String::from("<svg>");
        for _ in 0..MAX_SVG_ELEMENTS - 1 {
            input.push_str("<rect/>");
        }
        input.push_str("</svg>");
        let result = sanitize_svg(&input);
        assert!(result.contains("rect"));
        assert_ne!(result, "<svg></svg>");
    }

    #[test]
    fn sanitize_xlink_href_with_tab_prefix() {
        let input = "<svg><rect\txlink:href=\"http://evil.com\" width=\"10\"/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("xlink:href"));
    }

    #[test]
    fn sanitize_xlink_href_with_newline_prefix() {
        let input = "<svg><rect\nxlink:href=\"http://evil.com\" width=\"10\"/></svg>";
        let result = sanitize_svg(input);
        assert!(!result.contains("xlink:href"));
    }

    #[test]
    fn sanitize_href_after_colon() {
        // href preceded by ':' (like in xlink:href) should also be stripped
        let input = r#"<svg><rect xlink:href="http://evil.com" width="10"/></svg>"#;
        let result = sanitize_svg(input);
        assert!(!result.contains("href"));
    }
}
