use super::{FontFaceRule, PageRule, extract_url_path, preprocess_media_queries};

/// Parse a CSS stylesheet and extract `@page` rules.
pub fn parse_page_rules(css: &str) -> Vec<PageRule> {
    let preprocessed = preprocess_media_queries(css);
    extract_page_rules(&preprocessed)
}

/// Parse a CSS stylesheet and extract `@font-face` rules.
///
/// Only local file paths are supported in `src: url(...)`. Remote URLs
/// (http/https) are rejected for security reasons.
pub fn parse_font_face_rules(css: &str) -> Vec<FontFaceRule> {
    let preprocessed = preprocess_media_queries(css);
    extract_font_face_rules(&preprocessed)
}

/// Extract @font-face rules from preprocessed CSS.
pub(crate) fn extract_font_face_rules(css: &str) -> Vec<FontFaceRule> {
    let mut rules = Vec::new();
    let mut remaining = css;

    while let Some(at_pos) = remaining.to_ascii_lowercase().find("@font-face") {
        let Some(after_at) = remaining.get(at_pos + 10..) else {
            break;
        };
        let Some(brace_pos) = after_at.find('{') else {
            break;
        };
        let Some(after_brace) = after_at.get(brace_pos + 1..) else {
            break;
        };
        let Some(close_pos) = after_brace.find('}') else {
            break;
        };
        let declarations = &after_brace[..close_pos];
        if let Some(rule) = parse_font_face_declarations(declarations) {
            rules.push(rule);
        }
        remaining = &after_brace[close_pos + 1..];
    }

    rules
}

/// Parse the declarations inside an @font-face block.
pub(crate) fn parse_font_face_declarations(decls: &str) -> Option<FontFaceRule> {
    let mut font_family: Option<String> = None;
    let mut src_path: Option<String> = None;

    for declaration in decls.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }

        if let Some((prop, val)) = declaration.split_once(':') {
            let prop = prop.trim().to_ascii_lowercase();
            let val = val.trim();

            match prop.as_str() {
                "font-family" => {
                    let name = val.trim_matches('"').trim_matches('\'').trim().to_string();
                    if !name.is_empty() {
                        font_family = Some(name);
                    }
                }
                "src" => {
                    if let Some(path) = extract_url_path(val) {
                        src_path = Some(path);
                    }
                }
                _ => {}
            }
        }
    }

    match (font_family, src_path) {
        (Some(family), Some(path)) => Some(FontFaceRule {
            font_family: family,
            src_path: path,
        }),
        _ => None,
    }
}

/// Extract @page rules from preprocessed CSS.
pub(crate) fn extract_page_rules(css: &str) -> Vec<PageRule> {
    let mut page_rules = Vec::new();
    let mut remaining = css;

    while let Some(at_pos) = remaining.find("@page") {
        let Some(after_at) = remaining.get(at_pos + 5..) else {
            break;
        };
        let Some(brace_pos) = after_at.find('{') else {
            break;
        };
        let Some(after_brace) = after_at.get(brace_pos + 1..) else {
            break;
        };
        let Some(close_pos) = after_brace.find('}') else {
            break;
        };
        let declarations = &after_brace[..close_pos];
        if let Some(rule) = parse_page_declarations(declarations) {
            page_rules.push(rule);
        }
        remaining = &after_brace[close_pos + 1..];
    }

    page_rules
}

/// Parse the declarations inside an @page block.
pub(crate) fn parse_page_declarations(decls: &str) -> Option<PageRule> {
    let mut rule = PageRule::default();
    let mut has_any = false;

    for declaration in decls.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }

        if let Some((prop, val)) = declaration.split_once(':') {
            let prop = prop.trim().to_ascii_lowercase();
            let val = val.trim().to_ascii_lowercase();

            match prop.as_str() {
                "size" => {
                    if let Some((w, h)) = parse_page_size(&val) {
                        rule.width = Some(w);
                        rule.height = Some(h);
                        has_any = true;
                    }
                }
                "margin" => {
                    let parts: Vec<&str> = val.split_whitespace().collect();
                    match parts.len() {
                        1 => {
                            if let Some(v) = parse_page_length(parts[0]) {
                                rule.margin_top = Some(v);
                                rule.margin_right = Some(v);
                                rule.margin_bottom = Some(v);
                                rule.margin_left = Some(v);
                                has_any = true;
                            }
                        }
                        2 => {
                            if let (Some(tb), Some(lr)) =
                                (parse_page_length(parts[0]), parse_page_length(parts[1]))
                            {
                                rule.margin_top = Some(tb);
                                rule.margin_bottom = Some(tb);
                                rule.margin_right = Some(lr);
                                rule.margin_left = Some(lr);
                                has_any = true;
                            }
                        }
                        4 => {
                            if let (Some(t), Some(r), Some(b), Some(l)) = (
                                parse_page_length(parts[0]),
                                parse_page_length(parts[1]),
                                parse_page_length(parts[2]),
                                parse_page_length(parts[3]),
                            ) {
                                rule.margin_top = Some(t);
                                rule.margin_right = Some(r);
                                rule.margin_bottom = Some(b);
                                rule.margin_left = Some(l);
                                has_any = true;
                            }
                        }
                        _ => {}
                    }
                }
                "margin-top" => {
                    if let Some(v) = parse_page_length(&val) {
                        rule.margin_top = Some(v);
                        has_any = true;
                    }
                }
                "margin-right" => {
                    if let Some(v) = parse_page_length(&val) {
                        rule.margin_right = Some(v);
                        has_any = true;
                    }
                }
                "margin-bottom" => {
                    if let Some(v) = parse_page_length(&val) {
                        rule.margin_bottom = Some(v);
                        has_any = true;
                    }
                }
                "margin-left" => {
                    if let Some(v) = parse_page_length(&val) {
                        rule.margin_left = Some(v);
                        has_any = true;
                    }
                }
                _ => {}
            }
        }
    }

    if has_any { Some(rule) } else { None }
}

/// Parse a page size value. Returns (width, height) in points.
pub(crate) fn parse_page_size(val: &str) -> Option<(f32, f32)> {
    let val = val.trim();
    match val {
        "a4" => return Some((595.28, 841.89)),
        "a3" => return Some((841.89, 1190.55)),
        "a5" => return Some((419.53, 595.28)),
        "letter" => return Some((612.0, 792.0)),
        "legal" => return Some((612.0, 1008.0)),
        "b5" => return Some((498.9, 708.66)),
        "portrait" => return parse_page_size("a4"),
        "landscape" => return parse_page_size("a4").map(|(width, height)| (height, width)),
        _ => {}
    }

    let parts: Vec<&str> = val.split_whitespace().collect();
    if parts.len() == 2 {
        if let (Some(w), Some(h)) = (parse_page_length(parts[0]), parse_page_length(parts[1])) {
            return Some((w, h));
        }
    }

    if parts.len() == 2 {
        let (size_name, orientation) = (parts[0], parts[1]);
        if let Some((w, h)) = parse_page_size(size_name) {
            return match orientation {
                "landscape" => Some((h, w)),
                _ => Some((w, h)),
            };
        }
    }

    None
}

/// Parse a length value for @page rules (supports mm, in, cm, pt, px).
pub(crate) fn parse_page_length(val: &str) -> Option<f32> {
    let val = val.trim();
    if let Some(n) = val.strip_suffix("mm") {
        n.trim().parse::<f32>().ok().map(|v| v * 2.83465)
    } else if let Some(n) = val.strip_suffix("cm") {
        n.trim().parse::<f32>().ok().map(|v| v * 28.3465)
    } else if let Some(n) = val.strip_suffix("in") {
        n.trim().parse::<f32>().ok().map(|v| v * 72.0)
    } else if let Some(n) = val.strip_suffix("pt") {
        n.trim().parse::<f32>().ok()
    } else if let Some(n) = val.strip_suffix("px") {
        n.trim().parse::<f32>().ok().map(|v| v * 0.75)
    } else {
        val.parse::<f32>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_page_size_accepts_bare_orientation_keywords() {
        assert_eq!(parse_page_size("portrait"), Some((595.28, 841.89)));
        assert_eq!(parse_page_size("landscape"), Some((841.89, 595.28)));
    }

    #[test]
    fn parse_page_size_named_formats() {
        assert!(parse_page_size("a4").is_some());
        assert!(parse_page_size("a3").is_some());
        assert!(parse_page_size("a5").is_some());
        assert!(parse_page_size("letter").is_some());
        assert!(parse_page_size("legal").is_some());
        assert!(parse_page_size("b5").is_some());
    }

    #[test]
    fn parse_page_size_custom_dimensions() {
        let (w, h) = parse_page_size("200mm 300mm").unwrap();
        assert!((w - 200.0 * 2.83465).abs() < 0.1);
        assert!((h - 300.0 * 2.83465).abs() < 0.1);
    }

    #[test]
    fn parse_page_size_named_with_landscape() {
        let (w, h) = parse_page_size("a4 landscape").unwrap();
        assert!(w > h); // landscape: width > height
    }

    #[test]
    fn parse_page_size_invalid() {
        assert!(parse_page_size("bogus").is_none());
        assert!(parse_page_size("").is_none());
    }

    #[test]
    fn parse_page_length_units() {
        assert!((parse_page_length("10mm").unwrap() - 28.3465).abs() < 0.01);
        assert!((parse_page_length("1cm").unwrap() - 28.3465).abs() < 0.01);
        assert!((parse_page_length("1in").unwrap() - 72.0).abs() < 0.01);
        assert!((parse_page_length("72pt").unwrap() - 72.0).abs() < 0.01);
        assert!((parse_page_length("96px").unwrap() - 72.0).abs() < 0.01);
        assert!((parse_page_length("100").unwrap() - 100.0).abs() < 0.01);
    }

    #[test]
    fn parse_page_length_invalid() {
        assert!(parse_page_length("abc").is_none());
    }

    #[test]
    fn parse_page_declarations_margin_1() {
        let rule = parse_page_declarations("margin: 72pt").unwrap();
        assert_eq!(rule.margin_top, Some(72.0));
        assert_eq!(rule.margin_right, Some(72.0));
        assert_eq!(rule.margin_bottom, Some(72.0));
        assert_eq!(rule.margin_left, Some(72.0));
    }

    #[test]
    fn parse_page_declarations_margin_2() {
        let rule = parse_page_declarations("margin: 36pt 72pt").unwrap();
        assert_eq!(rule.margin_top, Some(36.0));
        assert_eq!(rule.margin_bottom, Some(36.0));
        assert_eq!(rule.margin_right, Some(72.0));
        assert_eq!(rule.margin_left, Some(72.0));
    }

    #[test]
    fn parse_page_declarations_margin_4() {
        let rule = parse_page_declarations("margin: 10pt 20pt 30pt 40pt").unwrap();
        assert_eq!(rule.margin_top, Some(10.0));
        assert_eq!(rule.margin_right, Some(20.0));
        assert_eq!(rule.margin_bottom, Some(30.0));
        assert_eq!(rule.margin_left, Some(40.0));
    }

    #[test]
    fn parse_page_declarations_individual_margins() {
        let rule = parse_page_declarations(
            "margin-top: 10pt; margin-right: 20pt; margin-bottom: 30pt; margin-left: 40pt",
        )
        .unwrap();
        assert_eq!(rule.margin_top, Some(10.0));
        assert_eq!(rule.margin_right, Some(20.0));
        assert_eq!(rule.margin_bottom, Some(30.0));
        assert_eq!(rule.margin_left, Some(40.0));
    }

    #[test]
    fn parse_page_declarations_size() {
        let rule = parse_page_declarations("size: a4").unwrap();
        assert!(rule.width.is_some());
        assert!(rule.height.is_some());
    }

    #[test]
    fn parse_page_declarations_empty() {
        assert!(parse_page_declarations("").is_none());
        assert!(parse_page_declarations("  ;  ;  ").is_none());
    }

    #[test]
    fn parse_page_declarations_margin_3_ignored() {
        // 3-value margin is not supported, should not set margins
        assert!(parse_page_declarations("margin: 10pt 20pt 30pt").is_none());
    }

    #[test]
    fn extract_font_face_rules_basic() {
        let rules = extract_font_face_rules(
            r#"@font-face { font-family: "MyFont"; src: url("font.ttf"); }"#,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].font_family, "MyFont");
        assert_eq!(rules[0].src_path, "font.ttf");
    }

    #[test]
    fn extract_font_face_rules_multiple() {
        let css = r#"
            @font-face { font-family: "A"; src: url("a.ttf"); }
            @font-face { font-family: "B"; src: url("b.ttf"); }
        "#;
        assert_eq!(extract_font_face_rules(css).len(), 2);
    }

    #[test]
    fn parse_font_face_declarations_missing_family() {
        assert!(parse_font_face_declarations("src: url(\"f.ttf\")").is_none());
    }

    #[test]
    fn parse_font_face_declarations_missing_src() {
        assert!(parse_font_face_declarations("font-family: \"F\"").is_none());
    }

    #[test]
    fn extract_page_rules_basic() {
        let rules = extract_page_rules("@page { size: a4; margin: 1in }");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].width.is_some());
        assert_eq!(rules[0].margin_top, Some(72.0));
    }

    #[test]
    fn extract_page_rules_malformed() {
        assert!(extract_page_rules("@page { bogus }").is_empty());
        assert!(extract_page_rules("@page no-brace").is_empty());
    }

    #[test]
    fn parse_page_rules_integration() {
        let rules = parse_page_rules("body {} @page { size: letter; margin: 1in }");
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn parse_font_face_rules_integration() {
        let rules = parse_font_face_rules(r#"@font-face { font-family: "X"; src: url("x.ttf"); }"#);
        assert_eq!(rules.len(), 1);
    }
}
