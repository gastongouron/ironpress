use super::{
    FontFaceRule, MarginBox, MarginBoxPosition, MarginContentToken, PageRule, PageSelector,
    extract_url_path, preprocess_media_queries,
};

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

/// Classify the text between `@page` and `{` into a [`PageSelector`]
/// (CSS Paged Media 3 §3). A bare `@page { }` is [`PageSelector::None`]; a
/// leading page name yields [`PageSelector::Named`]; otherwise the pseudo-class
/// (`:first`/`:left`/`:right`/`:blank`) is recognised.
pub(crate) fn classify_page_selector(text: &str) -> PageSelector {
    let text = text.trim();
    if text.is_empty() {
        return PageSelector::None;
    }
    // A page name (if any) is the leading identifier before any pseudo-class.
    let name = text.split(':').next().unwrap_or("").trim();
    if !name.is_empty() {
        return PageSelector::Named(name.to_string());
    }
    // No name — classify the first pseudo-class.
    match text.trim_start_matches(':').trim().to_ascii_lowercase().as_str() {
        "first" => PageSelector::First,
        "left" => PageSelector::Left,
        "right" => PageSelector::Right,
        "blank" => PageSelector::Blank,
        _ => PageSelector::None,
    }
}

/// Extract @page rules from preprocessed CSS.
///
/// The `@page` block is captured with a brace-balanced scan so a nested
/// page-margin at-rule (e.g. `@top-center { … }`, CSS Paged Media 3 §5) does
/// NOT truncate the rule at its inner `}` and drop the trailing `size`/`margin`
/// declarations. The selector text between `@page` and `{` is classified into
/// the rule's [`PageRule::selector`].
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
        // The selector is everything between `@page` and the opening brace.
        let selector_text = &after_at[..brace_pos];
        let Some(after_brace) = after_at.get(brace_pos + 1..) else {
            break;
        };
        // Brace-balanced scan: walk forward tracking nesting depth so the whole
        // @page block (including any nested margin-box at-rule) is captured and
        // we close on the MATCHING `}`, not the first one.
        let mut depth = 1usize;
        let mut close_pos = None;
        for (i, ch) in after_brace.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close_pos = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close_pos) = close_pos else {
            break;
        };
        let declarations = &after_brace[..close_pos];
        // Pull the nested page-margin at-rules (`@top-center { … }`, CSS Paged
        // Media 3 §5) out FIRST so they do not corrupt the `;`-split + lowercase
        // pass in `parse_page_declarations`; `clean_decls` is the size/margin/
        // background remainder.
        let (margin_boxes, clean_decls) = split_margin_boxes(declarations);
        match parse_page_declarations(&clean_decls) {
            Some(mut rule) => {
                rule.selector = classify_page_selector(selector_text);
                rule.margin_boxes = margin_boxes;
                page_rules.push(rule);
            }
            None if !margin_boxes.is_empty() => {
                // A `@page` rule carrying ONLY margin boxes (no size/margin/
                // background) must still be retained so the running header/footer
                // is not dropped.
                page_rules.push(PageRule {
                    selector: classify_page_selector(selector_text),
                    margin_boxes,
                    ..PageRule::default()
                });
            }
            None => {}
        }
        remaining = &after_brace[close_pos + 1..];
    }

    page_rules
}

/// Split the page-margin at-rules (`@top-center { … }`, CSS Paged Media 3 §5)
/// out of an `@page` declaration block. Returns the parsed margin boxes plus the
/// remaining declarations (size/margin/background) with the nested at-rules
/// removed, so the remainder is safe to `;`-split.
pub(crate) fn split_margin_boxes(decls: &str) -> (Vec<MarginBox>, String) {
    let mut boxes = Vec::new();
    let mut leftover = String::new();
    let mut i = 0;
    while i < decls.len() {
        let Some(at_rel) = decls[i..].find('@') else {
            leftover.push_str(&decls[i..]);
            break;
        };
        let at = i + at_rel;
        leftover.push_str(&decls[i..at]);
        let after_at = &decls[at + 1..];
        let Some(brace_rel) = after_at.find('{') else {
            // A stray `@` with no block — keep the remainder verbatim.
            leftover.push_str(&decls[at..]);
            break;
        };
        let name = after_at[..brace_rel].trim();
        let body_start = at + 1 + brace_rel + 1;
        let body_region = &decls[body_start..];
        // Brace-balanced scan for the matching close brace.
        let mut depth = 1usize;
        let mut close = None;
        for (j, ch) in body_region.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            // Unbalanced — keep the remainder verbatim and stop.
            leftover.push_str(&decls[at..]);
            break;
        };
        if let Some(position) = MarginBoxPosition::from_at_name(name) {
            if let Some(content) = extract_content_decl(&body_region[..close]) {
                boxes.push(MarginBox { position, content });
            }
        }
        // Resume after the at-rule's matching `}` (drop it from `leftover`).
        i = body_start + close + 1;
    }
    (boxes, leftover)
}

/// Find a `content:` declaration inside a margin-box body and parse its value.
fn extract_content_decl(body: &str) -> Option<Vec<MarginContentToken>> {
    for decl in body.split(';') {
        if let Some((prop, val)) = decl.split_once(':') {
            if prop.trim().eq_ignore_ascii_case("content") {
                return Some(parse_margin_box_content(val.trim()));
            }
        }
    }
    None
}

/// Parse a margin-box `content` value (CSS Paged Media 3 §5.3) into a token
/// list of string literals and the `counter(page)` / `counter(pages)` page
/// counters, e.g. `"Page " counter(page) " of " counter(pages)`.
pub(crate) fn parse_margin_box_content(val: &str) -> Vec<MarginContentToken> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < val.len() {
        let rest = &val[i..];
        let c = rest.chars().next().unwrap();
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if c == '"' || c == '\'' {
            let after = &rest[c.len_utf8()..];
            if let Some(end) = after.find(c) {
                tokens.push(MarginContentToken::Literal(after[..end].to_string()));
                i += c.len_utf8() + end + c.len_utf8();
            } else {
                tokens.push(MarginContentToken::Literal(after.to_string()));
                break;
            }
        } else if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case("counter(") {
            if let Some(end) = rest.find(')') {
                // The optional second arg is the counter style; only `decimal`
                // (the default) is supported, so the style is ignored.
                let name = rest[8..end]
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                match name.as_str() {
                    "page" => tokens.push(MarginContentToken::PageNumber),
                    "pages" => tokens.push(MarginContentToken::PageCount),
                    _ => {}
                }
                i += end + 1;
            } else {
                break;
            }
        } else {
            // Unsupported token (string()/element()/attr()/identifier) — advance
            // one char to keep making progress.
            i += c.len_utf8();
        }
    }
    tokens
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
                // A `background`/`background-*` declaration on `@page` paints the
                // page bleed area (CSS Paged Media 3 §3.1). It is NOT parsed here
                // (the `;`-split + lowercasing above would corrupt data-URI
                // values); instead we flag its presence so the rule is retained,
                // and the value is parsed from `raw_declarations` by a CSS-aware
                // parser in the converter.
                p if p.starts_with("background") => {
                    has_any = true;
                }
                _ => {}
            }
        }
    }

    if has_any {
        rule.raw_declarations = Some(decls.to_string());
        Some(rule)
    } else {
        None
    }
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
    fn parse_page_declarations_captures_background_raw() {
        // The @page background is retained verbatim (not lowercased/`;`-split) so
        // a CSS-aware parser can extract it later; the data-URI case survives.
        let rule = parse_page_declarations(
            "margin: 1cm; background-image: url(\"data:image/svg+xml,%3Csvg%3E\"); background-size: cover",
        )
        .unwrap();
        assert_eq!(rule.margin_top, Some(28.3465));
        let raw = rule.raw_declarations.expect("raw declarations retained");
        assert!(raw.contains("background-image"));
        assert!(raw.contains("%3Csvg"), "data-URI case preserved: {raw}");
    }

    #[test]
    fn parse_page_declarations_background_only_is_retained() {
        // An @page rule carrying ONLY a background (no size/margin) must still be
        // kept so the bleed-area background is not dropped.
        let rule =
            parse_page_declarations("background: #abc").expect("background-only @page retained");
        assert!(rule.width.is_none() && rule.margin_top.is_none());
        assert!(
            rule.raw_declarations
                .as_deref()
                .unwrap()
                .contains("background")
        );
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
    fn extract_page_rules_brace_balanced_keeps_trailing_decls() {
        // A nested page-margin at-rule must NOT truncate the @page block at its
        // inner `}` — the trailing `size`/`margin` declarations after the nested
        // block have to survive (the brace-balance fix).
        let rules = extract_page_rules(
            r#"@page { @top-center { content: "Title" }; size: a4; margin: 2cm }"#,
        );
        assert_eq!(rules.len(), 1, "the @page rule must be captured whole");
        assert!(
            rules[0].width.is_some(),
            "size after the nested margin box must not be dropped"
        );
        let m = 2.0 * 28.3465;
        assert!((rules[0].margin_top.unwrap() - m).abs() < 0.01);
        assert!((rules[0].margin_left.unwrap() - m).abs() < 0.01);
    }

    #[test]
    fn extract_page_rules_nested_block_does_not_leak_into_next_rule() {
        // Two @page rules with a nested margin box in the first: the second rule
        // must still be found (the scan resumes AFTER the first rule's matching
        // close brace, not its inner one).
        let rules = extract_page_rules(
            r#"@page :first { @bottom-right { content: "x" }; margin: 0 } @page { margin: 3cm }"#,
        );
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, PageSelector::First);
        assert_eq!(rules[0].margin_top, Some(0.0));
        assert_eq!(rules[1].selector, PageSelector::None);
        assert!((rules[1].margin_top.unwrap() - 3.0 * 28.3465).abs() < 0.01);
    }

    #[test]
    fn extract_page_rules_captures_selector() {
        let first = extract_page_rules("@page :first { margin: 0 }");
        assert_eq!(first[0].selector, PageSelector::First);

        let left = extract_page_rules("@page :left { margin-left: 2cm }");
        assert_eq!(left[0].selector, PageSelector::Left);

        let right = extract_page_rules("@page :right { margin-right: 2cm }");
        assert_eq!(right[0].selector, PageSelector::Right);

        let blank = extract_page_rules("@page :blank { margin: 1cm }");
        assert_eq!(blank[0].selector, PageSelector::Blank);

        let named = extract_page_rules("@page cover { size: a4; margin: 1cm }");
        assert_eq!(named[0].selector, PageSelector::Named("cover".to_string()));

        let default = extract_page_rules("@page { margin: 1cm }");
        assert_eq!(default[0].selector, PageSelector::None);
    }

    #[test]
    fn classify_page_selector_cases() {
        assert_eq!(classify_page_selector(""), PageSelector::None);
        assert_eq!(classify_page_selector("  "), PageSelector::None);
        assert_eq!(classify_page_selector(":first"), PageSelector::First);
        assert_eq!(classify_page_selector(" :left "), PageSelector::Left);
        assert_eq!(classify_page_selector(":RIGHT"), PageSelector::Right);
        assert_eq!(
            classify_page_selector("cover"),
            PageSelector::Named("cover".to_string())
        );
    }

    #[test]
    fn extract_page_rules_parses_margin_box_counters() {
        let rules = extract_page_rules(
            r#"@page { @bottom-center { content: "Page " counter(page) " of " counter(pages) } }"#,
        );
        assert_eq!(rules.len(), 1, "a margin-box-only @page must be retained");
        assert_eq!(rules[0].margin_boxes.len(), 1);
        let mb = &rules[0].margin_boxes[0];
        assert_eq!(mb.position, MarginBoxPosition::BottomCenter);
        assert_eq!(
            mb.content,
            vec![
                MarginContentToken::Literal("Page ".to_string()),
                MarginContentToken::PageNumber,
                MarginContentToken::Literal(" of ".to_string()),
                MarginContentToken::PageCount,
            ]
        );
    }

    #[test]
    fn extract_page_rules_margin_box_with_geometry() {
        // Margin boxes coexist with size/margin in the same @page block.
        let rules = extract_page_rules(
            r#"@page { size: a4; margin: 2cm; @top-left { content: "Title" }; @top-right { content: counter(page) } }"#,
        );
        assert_eq!(rules.len(), 1);
        assert!(rules[0].width.is_some(), "size must survive margin boxes");
        assert!((rules[0].margin_top.unwrap() - 2.0 * 28.3465).abs() < 0.01);
        assert_eq!(rules[0].margin_boxes.len(), 2);
        assert_eq!(rules[0].margin_boxes[0].position, MarginBoxPosition::TopLeft);
        assert_eq!(
            rules[0].margin_boxes[1].position,
            MarginBoxPosition::TopRight
        );
        assert_eq!(
            rules[0].margin_boxes[1].content,
            vec![MarginContentToken::PageNumber]
        );
    }

    #[test]
    fn parse_margin_box_content_decimal_style_ignored() {
        // `counter(page, decimal)` resolves like bare `counter(page)`.
        let toks = parse_margin_box_content(r##""#" counter(page, decimal)"##);
        assert_eq!(
            toks,
            vec![
                MarginContentToken::Literal("#".to_string()),
                MarginContentToken::PageNumber,
            ]
        );
    }

    #[test]
    fn parse_margin_box_content_single_quotes() {
        let toks = parse_margin_box_content(r#"'p.' counter(page)"#);
        assert_eq!(
            toks,
            vec![
                MarginContentToken::Literal("p.".to_string()),
                MarginContentToken::PageNumber,
            ]
        );
    }

    #[test]
    fn margin_box_position_band_and_align() {
        assert_eq!(
            MarginBoxPosition::BottomCenter.band(),
            Some(crate::parser::css::MarginBoxBand::Bottom)
        );
        assert_eq!(MarginBoxPosition::LeftMiddle.band(), None);
        assert_eq!(
            MarginBoxPosition::TopRight.align(),
            crate::parser::css::MarginBoxAlign::Right
        );
        assert_eq!(
            MarginBoxPosition::TopRightCorner.align(),
            crate::parser::css::MarginBoxAlign::Right
        );
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
