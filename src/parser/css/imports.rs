use std::path::Path;

use crate::util::decode_base64;

use super::{ImportRule, preprocess_media_queries};

/// Maximum recursion depth for @import processing.
pub const MAX_IMPORT_DEPTH: usize = 10;

/// Maximum cumulative size (in bytes) of all imported CSS content (10 MB).
pub const MAX_IMPORT_TOTAL_SIZE: usize = 10 * 1024 * 1024;

/// Extract a file path from a CSS `url(...)` value.
///
/// Handles: `url("path")`, `url('path')`, `url(path)`
pub(crate) fn extract_url_path(val: &str) -> Option<String> {
    let val = val.trim();
    let lower = val.to_ascii_lowercase();
    let start = lower.find("url(")?;
    let after_url = val.get(start + 4..)?;
    let end = after_url.find(')')?;
    let path = after_url
        .get(..end)?
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();

    if path.is_empty() { None } else { Some(path) }
}

pub(crate) fn extract_svg_data_uri(val: &str) -> Option<String> {
    let lower = val.to_ascii_lowercase();
    let start = lower.find("url(")?;
    let after_url = val.get(start + 4..)?;
    let end = after_url.rfind(')')?;
    let inner = after_url.get(..end)?.trim();
    let inner = inner.trim_matches('"').trim_matches('\'').trim();

    let inner_lower = inner.to_ascii_lowercase();
    if !inner_lower.starts_with("data:image/svg+xml") {
        return None;
    }

    let after_mime = inner.get("data:image/svg+xml".len()..)?;
    let (params, data) = after_mime.split_once(',')?;

    if params
        .split(';')
        .any(|param| param.eq_ignore_ascii_case("base64"))
    {
        let decoded = decode_base64(data)?;
        String::from_utf8(decoded).ok()
    } else {
        Some(percent_decode(data))
    }
}

/// Parse `@import` rules from a CSS string.
///
/// Returns the list of import rules found. Only local file paths are
/// supported; remote URLs (http/https) are rejected for security.
pub fn parse_import_rules(css: &str) -> Vec<ImportRule> {
    let preprocessed = preprocess_media_queries(css);
    extract_import_rules(&preprocessed)
}

/// Resolve `@import` rules in a CSS string by reading and inlining local files.
///
/// The `base_dir` is the directory relative to which import paths are resolved.
/// Recursion is limited to [`MAX_IMPORT_DEPTH`] levels to prevent infinite loops.
pub fn resolve_imports(css: &str, base_dir: &Path, depth: usize) -> String {
    let mut total_imported = 0usize;
    resolve_imports_inner(
        css,
        base_dir,
        depth,
        &mut total_imported,
        MAX_IMPORT_TOTAL_SIZE,
    )
}

/// Check whether a joined path stays within the given base directory.
///
/// Returns `true` only when the canonical form of `path` starts with the
/// canonical form of `base`. Any I/O error causes the function to return `false`.
pub fn is_path_within(path: &Path, base: &Path) -> bool {
    let Ok(canonical_base) = base.canonicalize() else {
        return false;
    };
    let Ok(canonical_path) = path.canonicalize() else {
        return false;
    };

    canonical_path.starts_with(&canonical_base)
}

fn extract_import_rules(css: &str) -> Vec<ImportRule> {
    let mut rules = Vec::new();

    for line in css.split(';').map(str::trim) {
        let lower = line.to_ascii_lowercase();
        if !lower.starts_with("@import") {
            continue;
        }

        let after_import = line.get(7..).unwrap_or("").trim();
        let path = if after_import.to_ascii_lowercase().starts_with("url(") {
            extract_url_path(after_import)
        } else {
            let trimmed = after_import
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        };

        if let Some(path) = path {
            if !path.starts_with("http://") && !path.starts_with("https://") {
                rules.push(ImportRule { path });
            }
        }
    }

    rules
}

fn percent_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + b - b'a'),
        b'A'..=b'F' => Some(10 + b - b'A'),
        _ => None,
    }
}

pub(crate) fn resolve_imports_inner(
    css: &str,
    base_dir: &Path,
    depth: usize,
    total_imported: &mut usize,
    max_total: usize,
) -> String {
    if depth >= MAX_IMPORT_DEPTH {
        return css.to_string();
    }

    let import_rules = parse_import_rules(css);
    if import_rules.is_empty() {
        return css.to_string();
    }

    let mut result = String::new();

    for import in &import_rules {
        let path = base_dir.join(&import.path);
        if !is_path_within(&path, base_dir) {
            continue;
        }
        if let Ok(imported_css) = std::fs::read_to_string(&path) {
            *total_imported += imported_css.len();
            if *total_imported > max_total {
                break;
            }

            let imported_base = path.parent().unwrap_or(base_dir);
            let resolved = resolve_imports_inner(
                &imported_css,
                imported_base,
                depth + 1,
                total_imported,
                max_total,
            );
            result.push_str(&resolved);
            result.push('\n');
        }
    }

    result.push_str(&strip_import_rules(css));
    result
}

/// Remove @import rules from CSS text, leaving all other content intact.
pub(crate) fn strip_import_rules(css: &str) -> String {
    let mut result = String::new();
    let mut remaining = css;

    while !remaining.is_empty() {
        let trimmed = remaining.trim_start();
        if trimmed.to_ascii_lowercase().starts_with("@import") {
            let Some(semi_pos) = trimmed.find(';') else {
                break;
            };
            remaining = &trimmed[semi_pos + 1..];
            continue;
        }

        if let Some(at_pos) = remaining.find('@') {
            result.push_str(&remaining[..at_pos]);
            remaining = &remaining[at_pos..];
            if !remaining.to_ascii_lowercase().starts_with("@import") {
                result.push('@');
                remaining = &remaining[1..];
            }
        } else {
            result.push_str(remaining);
            break;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_import_quoted_string() {
        let css = r#"@import "styles.css";"#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "styles.css");
    }

    #[test]
    fn parse_import_single_quoted() {
        let css = "@import 'other.css';";
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "other.css");
    }

    #[test]
    fn parse_import_url_function() {
        let css = r#"@import url("path/to/styles.css");"#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "path/to/styles.css");
    }

    #[test]
    fn parse_import_url_single_quotes() {
        let css = "@import url('path/to/styles.css');";
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "path/to/styles.css");
    }

    #[test]
    fn parse_import_url_no_quotes() {
        let css = "@import url(styles.css);";
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "styles.css");
    }

    #[test]
    fn parse_import_multiple() {
        let css = r#"
            @import "a.css";
            @import url("b.css");
            body { color: black; }
        "#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].path, "a.css");
        assert_eq!(rules[1].path, "b.css");
    }

    #[test]
    fn parse_import_rejects_https() {
        let css = r#"@import "https://example.com/styles.css";"#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 0, "Remote HTTPS URLs should be rejected");
    }

    #[test]
    fn parse_import_rejects_http() {
        let css = r#"@import url("http://example.com/styles.css");"#;
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 0, "Remote HTTP URLs should be rejected");
    }

    #[test]
    fn parse_import_no_rules_for_regular_css() {
        let css = "body { color: red; } p { font-size: 14px; }";
        let rules = parse_import_rules(css);
        assert_eq!(rules.len(), 0);
    }

    #[test]
    fn strip_import_preserves_regular_rules() {
        let css = r#"@import "a.css"; body { color: red; }"#;
        let stripped = strip_import_rules(css);
        assert!(stripped.contains("body"));
        assert!(stripped.contains("color: red"));
        assert!(!stripped.contains("@import"));
    }

    #[test]
    fn strip_import_multiple() {
        let css = r#"@import "a.css"; @import "b.css"; p { margin: 0; }"#;
        let stripped = strip_import_rules(css);
        assert!(!stripped.contains("@import"));
        assert!(stripped.contains("margin: 0"));
    }

    #[test]
    fn resolve_imports_no_imports() {
        let css = "body { color: red; }";
        let resolved = resolve_imports(css, std::path::Path::new("/tmp"), 0);
        assert_eq!(resolved.trim(), css);
    }

    #[test]
    fn resolve_imports_depth_limit() {
        let css = r#"@import "a.css"; body { color: red; }"#;
        let resolved = resolve_imports(css, std::path::Path::new("/tmp"), MAX_IMPORT_DEPTH);
        assert!(resolved.contains("@import"));
    }

    #[test]
    fn resolve_imports_missing_file() {
        let css = r#"@import "nonexistent.css"; body { color: red; }"#;
        let resolved = resolve_imports(css, std::path::Path::new("/tmp/nonexistent"), 0);
        assert!(resolved.contains("body"));
    }

    #[test]
    fn extract_url_path_double_quotes() {
        assert_eq!(
            extract_url_path(r#"url("fonts/test.ttf")"#),
            Some("fonts/test.ttf".to_string())
        );
    }

    #[test]
    fn extract_url_path_single_quotes() {
        assert_eq!(
            extract_url_path("url('fonts/test.ttf')"),
            Some("fonts/test.ttf".to_string())
        );
    }

    #[test]
    fn extract_url_path_no_quotes() {
        assert_eq!(
            extract_url_path("url(fonts/test.ttf)"),
            Some("fonts/test.ttf".to_string())
        );
    }

    #[test]
    fn extract_url_path_empty() {
        assert_eq!(extract_url_path("url()"), None);
    }

    #[test]
    fn extract_url_path_no_url_function() {
        assert_eq!(extract_url_path("fonts/test.ttf"), None);
    }

    #[test]
    fn parse_import_rules_empty_path() {
        let rules = parse_import_rules("@import \"\";");
        assert!(rules.is_empty());
    }

    #[test]
    fn strip_import_rules_malformed_no_semicolon() {
        let result = strip_import_rules("@import url(test.css)");
        assert!(result.is_empty());
    }

    #[test]
    fn strip_import_rules_non_import_at_rule() {
        let result = strip_import_rules("@charset 'utf-8'; p { color: red; }");
        assert!(result.contains("@charset"));
        assert!(result.contains("p { color: red; }"));
    }

    #[test]
    fn resolve_imports_with_real_file() {
        let dir = std::env::temp_dir().join("ironpress_css_test");
        std::fs::create_dir_all(&dir).unwrap();
        let imported_file = dir.join("imported.css");
        std::fs::write(&imported_file, "body { color: green; }").unwrap();
        let css = "@import \"imported.css\";\np { font-size: 12pt; }";
        let result = resolve_imports(css, &dir, 0);
        assert!(result.contains("body { color: green; }"));
        assert!(result.contains("p { font-size: 12pt; }"));
        std::fs::remove_file(&imported_file).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn path_traversal_blocked() {
        let dir = std::env::temp_dir().join("ironpress_traversal_test");
        std::fs::create_dir_all(&dir).unwrap();
        let css = "@import \"../../etc/passwd\";\nbody { color: red; }";
        let result = resolve_imports(css, &dir, 0);
        assert!(
            !result.contains("root:"),
            "path traversal import should be silently skipped"
        );
        assert!(result.contains("body { color: red; }"));
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn path_traversal_dot_dot_in_middle() {
        let dir = std::env::temp_dir().join("ironpress_traversal_mid_test");
        let subdir = dir.join("subdir");
        std::fs::create_dir_all(&subdir).unwrap();
        let css = "@import \"subdir/../../etc/passwd\";\np { margin: 0; }";
        let result = resolve_imports(css, &dir, 0);
        assert!(
            !result.contains("root:"),
            "path traversal via subdir/.. should be blocked"
        );
        assert!(result.contains("p { margin: 0; }"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn normal_subdirectory_import_allowed() {
        let dir = std::env::temp_dir().join("ironpress_subdir_import_test");
        let subdir = dir.join("subdir");
        std::fs::create_dir_all(&subdir).unwrap();
        let sub_file = subdir.join("styles.css");
        std::fs::write(&sub_file, "h1 { color: blue; }").unwrap();
        let css = "@import \"subdir/styles.css\";\np { font-size: 10pt; }";
        let result = resolve_imports(css, &dir, 0);
        assert!(
            result.contains("h1 { color: blue; }"),
            "subdirectory import should work"
        );
        assert!(result.contains("p { font-size: 10pt; }"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_imports_size_limit() {
        let dir = std::env::temp_dir().join("ironpress_css_size_limit_test");
        std::fs::create_dir_all(&dir).unwrap();

        let content_a = format!("a {{ padding: {}; }}", "x".repeat(80));
        let content_b = format!("b {{ padding: {}; }}", "y".repeat(80));
        let content_c = format!("c {{ padding: {}; }}", "z".repeat(80));
        std::fs::write(dir.join("a.css"), &content_a).unwrap();
        std::fs::write(dir.join("b.css"), &content_b).unwrap();
        std::fs::write(dir.join("c.css"), &content_c).unwrap();

        let css =
            "@import \"a.css\";\n@import \"b.css\";\n@import \"c.css\";\nbody { color: red; }";

        let mut total = 0usize;
        let result_all = resolve_imports_inner(css, &dir, 0, &mut total, 10 * 1024 * 1024);
        assert!(result_all.contains("padding:"));
        let full_count = result_all.matches("padding:").count();
        assert_eq!(full_count, 3);

        let mut total2 = 0usize;
        let result_limited = resolve_imports_inner(css, &dir, 0, &mut total2, 150);
        let limited_count = result_limited.matches("padding:").count();
        assert!(
            limited_count < 3,
            "expected fewer than 3 imports with size limit, got {limited_count}"
        );
        assert!(result_limited.contains("body { color: red; }"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
